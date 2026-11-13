//! Streaming decoders (doc 06): create from a setup, push edge batches, finish, drain, snapshot.
//!
//! A decoder is a [`Machine`]: a resumable loop over an [`Edges`] buffer. Every read of data that
//! has not arrived yet returns [`Pending`]; a step that meets it stops before emitting anything and
//! runs again after the next push. So the output is the same for any batching, and for snapshot and
//! restore between batches. Edges a machine no longer needs are released, so memory follows its
//! look-ahead, not the capture length.

use std::collections::VecDeque;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use logica_core::store::edges::EdgeBatch;
use logica_core::{Level, Sample};

use crate::types::{DecodeResult, Frame, FrameKind, Packet, PacketStream, Problem, Settings};

/// A read that needs data which has not arrived yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending;

pub type Res<T> = Result<T, Pending>;

/// One channel bound to a decoder role.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundChannel {
    pub role: String,
    /// Physical channel number (the lane frames are drawn under).
    pub channel: u32,
    pub name: String,
}

/// What a decoder is created from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecoderSetup {
    pub sample_rate_hz: f64,
    /// First sample of the decode; edges from here on are pushed.
    pub start: Sample,
    /// Level of each bound channel just before `start`.
    pub start_levels: Vec<Level>,
    /// Bound channels in slot order (the slot of an edge is its index here).
    pub channels: Vec<BoundChannel>,
    /// Settings with defaults applied.
    pub settings: Settings,
}

impl DecoderSetup {
    /// Slot of the channel bound to `role`.
    pub fn slot(&self, role: &str) -> Option<usize> {
        self.channels.iter().position(|c| c.role == role)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SlotEdges {
    start_level: Level,
    /// Absolute index (counted from the decode start) of `edges[0]`.
    base: u64,
    edges: VecDeque<Sample>,
}

/// The edges of a decoder's channels received so far.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edges {
    slots: Vec<SlotEdges>,
    /// Every edge before this has been received.
    known_end: Sample,
    finished: bool,
}

impl Edges {
    pub fn new(start_levels: &[Level], start: Sample) -> Self {
        Edges {
            slots: start_levels
                .iter()
                .map(|&l| SlotEdges { start_level: l, base: 0, edges: VecDeque::new() })
                .collect(),
            known_end: start,
            finished: false,
        }
    }

    /// A complete buffer (for detection on a window): every channel's level before `start` and
    /// its edges in `[start, end)`.
    pub fn complete(channels: Vec<(Level, Vec<Sample>)>, end: Sample) -> Self {
        Edges {
            slots: channels
                .into_iter()
                .map(|(l, e)| SlotEdges { start_level: l, base: 0, edges: e.into() })
                .collect(),
            known_end: end,
            finished: true,
        }
    }

    pub fn push(&mut self, batch: &EdgeBatch) {
        for e in &batch.edges {
            if let Some(s) = self.slots.get_mut(usize::from(e.slot)) {
                s.edges.push_back(e.sample);
            }
        }
        self.known_end = self.known_end.max(batch.end);
    }

    pub fn finish(&mut self, end: Sample) {
        self.known_end = self.known_end.max(end);
        self.finished = true;
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Every edge before this has been received (the capture end once finished).
    pub fn known_end(&self) -> Sample {
        self.known_end
    }

    pub fn slots(&self) -> usize {
        self.slots.len()
    }

    fn known(&self, s: Sample) -> bool {
        self.finished || s < self.known_end
    }

    /// Level of `slot` at sample `s` (at or after the last release point).
    pub fn level_at(&self, slot: usize, s: Sample) -> Res<Level> {
        if !self.known(s) {
            return Err(Pending);
        }
        let c = &self.slots[slot];
        let n = c.base + c.edges.partition_point(|&e| e <= s) as u64;
        Ok(level_after_count(c.start_level, n))
    }

    /// Level of `slot` after its edge with absolute index `i`.
    pub fn level_after(&self, slot: usize, i: u64) -> Level {
        level_after_count(self.slots[slot].start_level, i + 1)
    }

    /// Level of `slot` before any edge.
    pub fn start_level(&self, slot: usize) -> Level {
        self.slots[slot].start_level
    }

    /// Absolute index of the first edge at or after `s`. Needs every edge before `s`.
    pub fn lower_bound(&self, slot: usize, s: Sample) -> Res<u64> {
        if !(self.finished || s <= self.known_end) {
            return Err(Pending);
        }
        let c = &self.slots[slot];
        Ok(c.base + c.edges.partition_point(|&e| e < s) as u64)
    }

    /// The edge with absolute index `i`; `None` when the stream has ended before it.
    pub fn edge(&self, slot: usize, i: u64) -> Res<Option<Sample>> {
        let c = &self.slots[slot];
        debug_assert!(i >= c.base, "edge {i} was released");
        match usize::try_from(i - c.base).ok().and_then(|k| c.edges.get(k)) {
            Some(&e) => Ok(Some(e)),
            None if self.finished => Ok(None),
            None => Err(Pending),
        }
    }

    /// Edges of `slot` received so far, counted from the decode start.
    pub fn count(&self, slot: usize) -> u64 {
        let c = &self.slots[slot];
        c.base + c.edges.len() as u64
    }

    /// Whether every edge of `slot` has been received (the stream ended).
    pub fn total(&self, slot: usize) -> Option<u64> {
        self.finished.then(|| self.count(slot))
    }

    /// The first edge of `slot` at or after `s`.
    pub fn next_edge(&self, slot: usize, s: Sample) -> Res<Option<Sample>> {
        let c = &self.slots[slot];
        let k = c.edges.partition_point(|&e| e < s);
        match c.edges.get(k) {
            Some(&e) => Ok(Some(e)),
            None if self.finished => Ok(None),
            None => Err(Pending),
        }
    }

    /// The first edge of `slot` in `[from, to)`, or `None` if there is none.
    pub fn edge_in(&self, slot: usize, from: Sample, to: Sample) -> Res<Option<Sample>> {
        let c = &self.slots[slot];
        let k = c.edges.partition_point(|&e| e < from);
        match c.edges.get(k) {
            Some(&e) if e < to => Ok(Some(e)),
            Some(_) => Ok(None),
            None if self.finished || to <= self.known_end => Ok(None),
            None => Err(Pending),
        }
    }

    /// The earliest of the next edges of several slots (ties: lowest slot), where `next[i]` is
    /// the absolute index of the next unread edge of slot `i`. `None` once the stream has ended.
    pub fn next_merged(&self, next: &[u64]) -> Res<Option<(usize, Sample)>> {
        let mut best: Option<(usize, Sample)> = None;
        let mut pending = false;
        for (slot, &i) in next.iter().enumerate() {
            match self.edge(slot, i) {
                Ok(Some(e)) if best.is_none_or(|(_, b)| e < b) => best = Some((slot, e)),
                Ok(_) => {}
                Err(Pending) => pending = true,
            }
        }
        // A received edge is before `known_end`, and a slot still waiting has nothing before it.
        match best {
            Some(b) => Ok(Some(b)),
            None if pending => Err(Pending),
            None => Ok(None),
        }
    }

    /// Forgets edges before `s` on every slot; reads before `s` are not allowed afterwards.
    pub fn release_before(&mut self, s: Sample) {
        for c in &mut self.slots {
            while c.edges.front().is_some_and(|&e| e < s) {
                c.edges.pop_front();
                c.base += 1;
            }
        }
    }

    /// Forgets edges of `slot` with absolute index below `i`.
    pub fn release_index(&mut self, slot: usize, i: u64) {
        let c = &mut self.slots[slot];
        while c.base < i && !c.edges.is_empty() {
            c.edges.pop_front();
            c.base += 1;
        }
    }

    /// Edges held in memory (for tests of bounded look-ahead).
    pub fn buffered(&self) -> usize {
        self.slots.iter().map(|c| c.edges.len()).sum()
    }
}

fn level_after_count(start: Level, n: u64) -> Level {
    if n % 2 == 1 { !start } else { start }
}

// ------------------------------------------------------------------ output

/// What a decoder emitted since the last drain.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub frames: Vec<Frame>,
    /// Packets that are complete (a later packet has started, or the stream ended).
    pub packets: Vec<Packet>,
    pub notes: Vec<String>,
}

/// A frame before it is attached to a packet.
pub struct NewFrame {
    pub start: Sample,
    pub end: Sample,
    pub lane: u32,
    pub kind: FrameKind,
    pub value: Option<f64>,
    pub label: String,
    pub detail: Option<String>,
    pub problems: Vec<Problem>,
    pub row: u8,
}

impl NewFrame {
    pub fn new(start: Sample, end: Sample, lane: u32, kind: FrameKind, label: impl Into<String>) -> Self {
        NewFrame {
            start,
            end,
            lane,
            kind,
            value: None,
            label: label.into(),
            detail: None,
            problems: Vec::new(),
            row: 0,
        }
    }
    pub fn value(mut self, v: f64) -> Self {
        self.value = Some(v);
        self
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    pub fn problems(mut self, p: Vec<Problem>) -> Self {
        self.problems = p;
        self
    }
    pub fn problem(mut self, p: Problem) -> Self {
        self.problems.push(p);
        self
    }
    pub fn row(mut self, r: u8) -> Self {
        self.row = r;
        self
    }
}

/// Collects frames and packets while a machine runs. With `keep` off (trial decodes during
/// detection) nothing is stored.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Out {
    keep: bool,
    frames: Vec<Frame>,
    done: Vec<Packet>,
    current: Option<Packet>,
    /// Packets opened so far.
    opened: u64,
    notes: Vec<String>,
}

impl Out {
    pub fn new(keep: bool) -> Self {
        Out { keep, ..Default::default() }
    }

    /// Index of the current packet, if one was opened.
    pub fn current_packet(&self) -> Option<u64> {
        self.opened.checked_sub(1)
    }

    pub fn packets_opened(&self) -> u64 {
        self.opened
    }

    /// Starts a packet; the previous one is complete.
    pub fn open_packet(&mut self, start: Sample, title: impl Into<String>) {
        let index = self.opened;
        self.opened += 1;
        if !self.keep {
            return;
        }
        if let Some(p) = self.current.take() {
            self.done.push(p);
        }
        self.current = Some(Packet { index, start, end: start, title: title.into(), streams: Vec::new() });
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        if let Some(p) = &mut self.current {
            p.title = title.into();
        }
    }

    /// Adds a byte to a stream of the current packet.
    pub fn byte(&mut self, label: &str, lane: u32, value: u8, end: Sample) {
        let Some(p) = &mut self.current else { return };
        match p.streams.iter_mut().find(|s| s.label == label) {
            Some(s) => s.bytes.push(value),
            None => p.streams.push(PacketStream { label: label.into(), lane, bytes: vec![value] }),
        }
        p.end = p.end.max(end);
    }

    /// Extends the current packet to `end`.
    pub fn close_packet(&mut self, end: Sample) {
        if let Some(p) = &mut self.current {
            p.end = p.end.max(end);
        }
    }

    pub fn add(&mut self, f: NewFrame) {
        if !self.keep {
            return;
        }
        if let Some(p) = &mut self.current {
            p.end = p.end.max(f.end);
        }
        self.frames.push(Frame {
            start: f.start,
            end: f.end,
            lane: f.lane,
            kind: f.kind,
            value: f.value,
            label: f.label,
            detail: f.detail,
            problems: f.problems,
            packet: self.opened.saturating_sub(1),
            row: f.row,
        });
    }

    pub fn note(&mut self, text: impl Into<String>) {
        if self.keep {
            self.notes.push(text.into());
        }
    }

    /// The stream ended: the current packet is complete.
    pub fn finish(&mut self) {
        if let Some(p) = self.current.take() {
            self.done.push(p);
        }
    }

    pub fn drain(&mut self) -> Output {
        Output {
            frames: std::mem::take(&mut self.frames),
            packets: std::mem::take(&mut self.done),
            notes: std::mem::take(&mut self.notes),
        }
    }
}

// ------------------------------------------------------------------ decoders

/// A streaming decoder (doc 06).
pub trait StreamDecoder: Send {
    /// Edges of the bound channels in time order, complete up to `batch.end`.
    fn push(&mut self, batch: &EdgeBatch);
    /// No more data: the capture ends at `end`. Emits anything pending.
    fn finish(&mut self, end: Sample);
    /// Frames, complete packets and notes since the last drain.
    fn drain(&mut self) -> Output;
    /// The whole state as data, to resume later.
    fn snapshot(&self) -> serde_json::Value;
}

/// The protocol-specific part of a decoder: a resumable loop.
pub trait Machine: Send + Serialize + DeserializeOwned + 'static {
    /// Advances as far as the received edges allow. Returns when a read is [`Pending`] or the
    /// stream is exhausted; must not emit anything for a step it could not finish.
    fn run(&mut self, edges: &mut Edges, out: &mut Out);
    /// Called once after the last `run`, when the stream has ended.
    fn at_end(&mut self, _edges: &Edges, _out: &mut Out) {}
}

/// Runs `step` until it is pending or reports that nothing is left (`Ok(false)`).
pub fn run_steps(mut step: impl FnMut() -> Res<bool>) {
    while let Ok(true) = step() {}
}

#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
struct Driver<M: Machine> {
    machine: M,
    edges: Edges,
    out: Out,
    ended: bool,
}

impl<M: Machine> StreamDecoder for Driver<M> {
    fn push(&mut self, batch: &EdgeBatch) {
        if self.ended {
            return;
        }
        self.edges.push(batch);
        self.machine.run(&mut self.edges, &mut self.out);
    }

    fn finish(&mut self, end: Sample) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.edges.finish(end);
        self.machine.run(&mut self.edges, &mut self.out);
        self.machine.at_end(&self.edges, &mut self.out);
        self.out.finish();
    }

    fn drain(&mut self) -> Output {
        self.out.drain()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Wraps a machine as a streaming decoder, or restores one from a snapshot.
pub fn decoder<M: Machine>(
    machine: M,
    setup: &DecoderSetup,
    snapshot: Option<serde_json::Value>,
) -> Result<Box<dyn StreamDecoder>, String> {
    if let Some(v) = snapshot {
        let d: Driver<M> = serde_json::from_value(v)
            .map_err(|e| format!("The saved decoder state cannot be read ({e})."))?;
        return Ok(Box::new(d));
    }
    Ok(Box::new(Driver {
        machine,
        edges: Edges::new(&setup.start_levels, setup.start),
        out: Out::new(true),
        ended: false,
    }))
}

/// Collects everything a decoder emits into one result.
#[derive(Default)]
pub struct Collected {
    pub result: DecodeResult,
}

impl Collected {
    pub fn take(&mut self, o: Output) {
        for f in &o.frames {
            for p in &f.problems {
                *self.result.problem_counts.entry(p.code).or_default() += 1;
            }
        }
        self.result.frames.extend(o.frames);
        self.result.packets.extend(o.packets);
        self.result.notes.extend(o.notes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logica_core::store::edges::Edge;

    fn batch(edges: &[(Sample, u8)], end: Sample) -> EdgeBatch {
        EdgeBatch {
            edges: edges.iter().map(|&(s, slot)| Edge { sample: s, slot, level: Level::Low }).collect(),
            end,
        }
    }

    #[test]
    fn reads_wait_for_data_and_release_keeps_levels() {
        let mut e = Edges::new(&[Level::High, Level::Low], 0);
        assert_eq!(e.level_at(0, 0), Err(Pending));
        e.push(&batch(&[(5, 0), (7, 1), (9, 0)], 10));
        assert_eq!(e.level_at(0, 4), Ok(Level::High));
        assert_eq!(e.level_at(0, 5), Ok(Level::Low));
        assert_eq!(e.level_at(0, 9), Ok(Level::High));
        assert_eq!(e.level_at(0, 10), Err(Pending));
        assert_eq!(e.edge_in(1, 8, 10), Ok(None));
        assert_eq!(e.edge_in(1, 8, 11), Err(Pending));
        assert_eq!(e.next_merged(&[0, 0]), Ok(Some((0, 5))));
        assert_eq!(e.next_merged(&[2, 1]), Err(Pending));
        assert_eq!(e.lower_bound(0, 6), Ok(1));
        assert_eq!(e.lower_bound(0, 11), Err(Pending));
        e.release_before(8);
        assert_eq!(e.level_at(0, 8), Ok(Level::Low));
        assert_eq!(e.edge(0, 1), Ok(Some(9)));
        assert_eq!(e.level_after(0, 1), Level::High);
        e.finish(20);
        assert_eq!(e.edge(0, 2), Ok(None));
        assert_eq!(e.level_at(0, 100), Ok(Level::High));
        assert_eq!(e.next_merged(&[2, 1]), Ok(None));
    }

    #[test]
    fn packets_complete_when_the_next_one_opens() {
        let mut o = Out::new(true);
        o.add(NewFrame::new(0, 1, 0, FrameKind::Info, "before"));
        o.open_packet(10, "a");
        o.byte("RX", 0, 0x41, 20);
        o.add(NewFrame::new(10, 25, 0, FrameKind::Data, "41"));
        let d = o.drain();
        assert_eq!(d.frames.len(), 2);
        assert_eq!(d.frames[0].packet, 0);
        assert!(d.packets.is_empty(), "still open");
        o.open_packet(30, "b");
        let d = o.drain();
        assert_eq!(d.packets.len(), 1);
        assert_eq!((d.packets[0].end, d.packets[0].streams[0].bytes.clone()), (25, vec![0x41]));
        o.finish();
        assert_eq!(o.drain().packets[0].index, 1);
    }
}
