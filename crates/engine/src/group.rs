//! Several devices into one capture (doc 05, "Several devices in one capture"). Each device runs on
//! its own thread into a tagged sink; this merger lines their data up and hands one stream to the
//! capture's sink, so the rest of the engine sees one device.
//!
//! Devices do not share a clock. Without help they are lined up at their starts: each device's
//! sample 0 at the capture's sample 0. Given a signal every device sees (a shared clock, or any
//! shared line) and the channel it is on, the merger matches its rising edges: first the shift
//! (the best match of the first edges; between equally good shifts, as with a steady clock, the
//! smallest, so devices must start within half a period of each other), then, while the capture
//! runs, the drift of each clock against the first device's, as a straight line through every
//! matched pair. The first device's clock is the capture's.
//!
//! The capture moves on only as far as every device has delivered (on the capture's clock), so a
//! slow device holds the others' data in memory until it catches up.

use std::collections::VecDeque;
use std::sync::mpsc::{SyncSender, sync_channel};

use logica_core::capture::{ChannelInfo, Note, NoteLevel};
use logica_core::device::{Block, BlockSink, Device, DeviceError, RunOutcome, StopSignal};
use logica_core::{Level, Sample};
use logica_protocols::num;

use crate::engine::RunResult;

/// Messages in flight from the devices to the merger.
const QUEUE: usize = 1024;
/// How much signal (seconds, on every device) the merger waits for to line the devices up before
/// it lines them up at their starts instead.
const LOCK_WINDOW_S: f64 = 1.0;
/// Signal (seconds, on every device) collected before lining up, so the first edges already get
/// the drift fit, not only the shift.
const LOCK_READY_S: f64 = 0.2;
/// Rising edges of each device tried as the pair that lines them up.
const LOCK_EDGES: usize = 16;
/// Matched rising edges needed to accept a shift.
const LOCK_MIN: usize = 4;
/// Signal (seconds) a drift fit must span before its slope is used.
const FIT_SPAN_S: f64 = 0.05;

pub struct Member {
    pub id: String,
    /// Short name, put before its channel names ("Second: TX").
    pub label: String,
    pub device: Box<dyn Device>,
    /// The channel (physical number) that carries the shared signal, when lining up by one.
    pub align: Option<u32>,
}

/// What a device delivered, owned, from its thread.
enum Msg {
    Begin { channels: Vec<ChannelInfo>, initial: Vec<Level>, rate: f64 },
    Edges { slot: usize, edges: Vec<Sample> },
    Advance(Sample),
    Note(Note),
    Done(Result<RunOutcome, String>),
}

/// A device's sink: everything becomes level changes, tagged with the device.
struct Tagged {
    k: usize,
    tx: SyncSender<(usize, Msg)>,
    levels: Vec<Level>,
}

impl Tagged {
    fn send(&self, m: Msg) -> Result<(), DeviceError> {
        self.tx
            .send((self.k, m))
            .map_err(|_| DeviceError::new("The capture was closed while the device was sending."))
    }

    fn level(&mut self, slot: usize) -> Result<&mut Level, DeviceError> {
        self.levels.get_mut(slot).ok_or_else(|| {
            DeviceError::new(format!("The device sent data for slot {slot}, which it did not begin."))
        })
    }

    fn edges(&mut self, slot: usize, edges: Vec<Sample>) -> Result<(), DeviceError> {
        if edges.is_empty() {
            return Ok(());
        }
        if !edges.len().is_multiple_of(2) {
            let l = self.level(slot)?;
            *l = !*l;
        }
        self.send(Msg::Edges { slot, edges })
    }
}

impl BlockSink for Tagged {
    fn begin(&mut self, channels: &[ChannelInfo], initial: &[Level], rate: f64) -> Result<(), DeviceError> {
        self.levels = (0..channels.len()).map(|i| initial.get(i).copied().unwrap_or_default()).collect();
        self.send(Msg::Begin { channels: channels.to_vec(), initial: self.levels.clone(), rate })
    }

    fn block(&mut self, b: Block<'_>) -> Result<(), DeviceError> {
        match b {
            Block::Transitions { slot, edges } => self.edges(slot, edges.to_vec()),
            Block::Bits { slot, start, words, count } => {
                let mut level = *self.level(slot)?;
                let mut edges = Vec::new();
                for i in 0..count {
                    let word = words.get(usize::try_from(i / 64).unwrap_or(usize::MAX)).copied().unwrap_or(0);
                    let l = Level::from_bit(word >> (i % 64));
                    if l != level {
                        edges.push(start + i);
                        level = l;
                    }
                }
                self.send_converted(slot, edges)
            }
            Block::Packed { start, unit_size, data, bits } => {
                let n = data.len().checked_div(unit_size).unwrap_or(0);
                for (slot, &bit) in bits.iter().enumerate() {
                    let (byte, shift) = (usize::try_from(bit / 8).unwrap_or(usize::MAX), bit % 8);
                    let mut level = *self.level(slot)?;
                    let mut edges = Vec::new();
                    for j in 0..n {
                        let v = data.get(j * unit_size + byte).copied().unwrap_or(0);
                        let l = Level::from_bit(u64::from(v >> shift));
                        if l != level {
                            edges.push(start + j as Sample);
                            level = l;
                        }
                    }
                    self.send_converted(slot, edges)?;
                }
                Ok(())
            }
            Block::Advance { end } => self.send(Msg::Advance(end)),
        }
    }

    fn note(&mut self, note: Note) {
        let _ = self.send(Msg::Note(note));
    }
}

impl Tagged {
    /// Edges found from levels: the level after them is already right in `edges`' count.
    fn send_converted(&mut self, slot: usize, edges: Vec<Sample>) -> Result<(), DeviceError> {
        self.edges(slot, edges)
    }
}

/// A device's samples on the capture's clock: `slope * x + offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Line {
    slope: f64,
    offset: f64,
}

impl Line {
    const SAME: Line = Line { slope: 1.0, offset: 0.0 };
    fn at(&self, x: Sample) -> f64 {
        self.slope * x as f64 + self.offset
    }
}

/// A straight line through matched (device, capture) samples, kept as running means so large
/// sample numbers stay exact enough.
#[derive(Default)]
struct Fit {
    n: f64,
    mx: f64,
    my: f64,
    cxx: f64,
    cxy: f64,
    lo: f64,
    hi: f64,
}

impl Fit {
    fn add(&mut self, x: f64, y: f64) {
        if self.n == 0.0 {
            (self.lo, self.hi) = (x, x);
        }
        self.lo = self.lo.min(x);
        self.hi = self.hi.max(x);
        self.n += 1.0;
        let dx = x - self.mx;
        self.mx += dx / self.n;
        self.my += (y - self.my) / self.n;
        self.cxx += dx * (x - self.mx);
        self.cxy += dx * (y - self.my);
    }

    /// The line, with its slope once the pairs span `span` samples; before that only the shift.
    fn line(&self, span: f64) -> Option<Line> {
        if self.n == 0.0 {
            return None;
        }
        if self.hi - self.lo >= span && self.cxx > 0.0 {
            let slope = self.cxy / self.cxx;
            return Some(Line { slope, offset: self.my - slope * self.mx });
        }
        Some(Line { slope: 1.0, offset: self.my - self.mx })
    }
}

/// The shift `d` (capture = device + `d`, in samples) that puts most of the device's rising edges
/// within `tol` of the reference's, and how many matched; between equally good shifts the smallest.
/// None when fewer than [`LOCK_MIN`] match, or fewer than three quarters of those that could.
pub(crate) fn best_shift(reference: &[Sample], device: &[Sample], tol: f64) -> Option<(f64, usize)> {
    let (first, last) = (*reference.first()? as f64, *reference.last()? as f64);
    let near = |y: f64| {
        let i = reference.partition_point(|&a| (a as f64) < y - tol);
        reference.get(i).is_some_and(|&a| (a as f64 - y).abs() <= tol)
    };
    let mut best: Option<(f64, usize)> = None;
    for &a in reference.iter().take(LOCK_EDGES) {
        for &b in device.iter().take(LOCK_EDGES) {
            let d = a as f64 - b as f64;
            let mut could = 0;
            let mut hit = 0;
            for &x in device.iter().take(4 * LOCK_EDGES) {
                let y = x as f64 + d;
                if y >= first - tol && y <= last + tol {
                    could += 1;
                    if near(y) {
                        hit += 1;
                    }
                }
            }
            if hit < LOCK_MIN || hit * 4 < could * 3 {
                continue;
            }
            let better = match best {
                None => true,
                Some((bd, bh)) => hit > bh || (hit == bh && d.abs() < bd.abs()),
            };
            if better {
                best = Some((d, hit));
            }
        }
    }
    best
}

/// How close two edges of the shared signal must be to count as the same: a twentieth of the
/// reference's usual spacing, at least two samples.
fn tolerance(reference: &[Sample]) -> f64 {
    let mut gaps: Vec<Sample> = reference.windows(2).map(|w| w[1] - w[0]).collect();
    if gaps.is_empty() {
        return 2.0;
    }
    let mid = gaps.len() / 2;
    let (_, m, _) = gaps.select_nth_unstable(mid);
    (*m as f64 / 20.0).max(2.0)
}

/// What the merger knows of one device.
struct Part {
    label: String,
    align: Option<u32>,
    /// Channel numbers of this device start here in the capture.
    index_base: u32,
    begun: Option<(Vec<ChannelInfo>, Vec<Level>, f64)>,
    /// Its first slot in the capture.
    slot_base: usize,
    /// Edges not yet in the capture, per slot, on its own clock.
    pending: Vec<VecDeque<Sample>>,
    known: Sample,
    done: Option<Result<RunOutcome, String>>,
    sync_slot: Option<usize>,
    sync_level: Level,
    /// Rising edges of the shared signal not yet matched, on its own clock.
    rising: VecDeque<Sample>,
    line: Line,
    fit: Fit,
    lined_up: bool,
}

impl Part {
    fn end(&self) -> f64 {
        self.line.at(self.known)
    }
}

struct Merger<'a> {
    parts: Vec<Part>,
    sink: &'a mut dyn BlockSink,
    stop: &'a StopSignal,
    rate: f64,
    tol: f64,
    /// Rising edges of the reference's shared signal, on the capture's clock, kept while needed.
    reference: VecDeque<Sample>,
    /// Whether the capture began (all devices began and, if asked, are lined up).
    begun: bool,
    /// Every slot's last edge in the capture, so edges stay in order after a correction.
    last: Vec<Option<Sample>>,
    emitted: Sample,
    error: Option<String>,
    sink_gone: bool,
}

impl Merger<'_> {
    fn fail(&mut self, why: String) {
        if self.error.is_none() {
            self.error = Some(why);
        }
        self.stop.stop();
    }

    fn on(&mut self, k: usize, m: Msg) {
        match m {
            Msg::Begin { channels, initial, rate } => {
                let p = &mut self.parts[k];
                p.pending = vec![VecDeque::new(); channels.len()];
                p.sync_slot = p.align.and_then(|ch| channels.iter().position(|c| c.index == ch));
                p.sync_level = p.sync_slot.and_then(|s| initial.get(s).copied()).unwrap_or_default();
                if p.align.is_some() && p.sync_slot.is_none() {
                    let why = format!("{} did not capture the channel chosen to line it up by.", p.label);
                    p.begun = Some((channels, initial, rate));
                    self.fail(why);
                    return;
                }
                p.begun = Some((channels, initial, rate));
                if self.parts.iter().all(|p| p.begun.is_some()) {
                    self.all_begun();
                }
            }
            Msg::Edges { slot, edges } => {
                let reference = k == 0;
                let p = &mut self.parts[k];
                if p.sync_slot == Some(slot) {
                    for &e in &edges {
                        p.sync_level = !p.sync_level;
                        if p.sync_level == Level::High {
                            if reference {
                                self.reference.push_back(e);
                            } else {
                                p.rising.push_back(e);
                            }
                        }
                    }
                }
                let p = &mut self.parts[k];
                match p.pending.get_mut(slot) {
                    Some(q) => q.extend(edges),
                    None => {
                        let why = format!("{} sent data for a channel it did not begin.", p.label);
                        self.fail(why);
                    }
                }
            }
            Msg::Advance(end) => {
                self.parts[k].known = self.parts[k].known.max(end);
                self.step();
            }
            Msg::Note(n) => {
                let label = &self.parts[k].label;
                self.sink.note(Note { level: n.level, message: format!("{label}: {}", n.message) });
            }
            Msg::Done(r) => {
                match &r {
                    Err(e) => {
                        let why = format!("{}: {e}", self.parts[k].label);
                        self.fail(why);
                    }
                    // one device stopping ends the capture: the others cannot go further
                    Ok(o) if o.stopped => self.stop.stop(),
                    Ok(_) => {}
                }
                self.parts[k].done = Some(r);
            }
        }
    }

    /// Every device began: check they agree, then begin the capture unless they still need lining up.
    fn all_begun(&mut self) {
        let rates: Vec<f64> = self.parts.iter().filter_map(|p| p.begun.as_ref().map(|b| b.2)).collect();
        let first = rates[0];
        if let Some(k) = rates.iter().position(|&r| (r - first).abs() > first * 1e-9) {
            let why = format!(
                "{} captures at {}, {} at {}; one capture has one rate. Configure them alike.",
                self.parts[k].label,
                logica_protocols::format::rate(rates[k]),
                self.parts[0].label,
                logica_protocols::format::rate(first)
            );
            self.fail(why);
            return;
        }
        self.rate = first;
        let mut base = 0;
        for p in &mut self.parts {
            p.slot_base = base;
            base += p.begun.as_ref().map_or(0, |b| b.0.len());
        }
        self.last = vec![None; base];
        self.step();
    }

    /// Lines up what can be lined up, begins the capture once everything is, and moves it on.
    fn step(&mut self) {
        if self.error.is_some() || !self.parts.iter().all(|p| p.begun.is_some()) {
            return;
        }
        if !self.begun {
            self.line_up();
            if !self.parts.iter().all(|p| p.lined_up) {
                return;
            }
            // the drift over what arrived so far, before the first edge goes out
            self.track();
            self.begin();
        }
        self.track();
        let end = self.parts.iter().map(Part::end).fold(f64::INFINITY, f64::min);
        self.emit(end);
    }

    /// Finds each device's shift against the first, or gives up after [`LOCK_WINDOW_S`].
    fn line_up(&mut self) {
        let window = LOCK_WINDOW_S * self.rate;
        let reference: Vec<Sample> = self.reference.iter().copied().collect();
        if self.parts[0].align.is_none() {
            for p in &mut self.parts {
                p.lined_up = true;
            }
            return;
        }
        self.parts[0].lined_up = true;
        self.tol = tolerance(&reference);
        let give_up = self.parts.iter().all(|p| p.known as f64 >= window);
        let ready = self.parts.iter().all(|p| p.known as f64 >= LOCK_READY_S * self.rate);
        if !ready && !give_up {
            return;
        }
        let reference_label = self.parts[0].label.clone();
        let reference_channel = self.channel_name(0);
        for k in 1..self.parts.len() {
            if self.parts[k].lined_up {
                continue;
            }
            let rising: Vec<Sample> = self.parts[k].rising.iter().copied().collect();
            let found = (reference.len() >= LOCK_MIN && rising.len() >= LOCK_MIN)
                .then(|| best_shift(&reference, &rising, self.tol))
                .flatten();
            let channel = self.channel_name(k);
            let p = &mut self.parts[k];
            match found {
                Some((d, _)) => {
                    // the matched edges themselves go into the fit (track); the note follows at
                    // the beginning, with the fitted offset
                    p.line = Line { slope: 1.0, offset: d };
                    p.lined_up = true;
                }
                None if give_up => {
                    p.lined_up = true;
                    p.align = None;
                    p.rising.clear();
                    let message = format!(
                        "{} could not be lined up with {reference_label}: {channel} and {reference_channel} have no edges in common in the first {LOCK_WINDOW_S} s. It is lined up at its start instead; check that both see the same signal.",
                        p.label
                    );
                    self.sink.note(Note { level: NoteLevel::Warning, message });
                }
                None => {}
            }
        }
    }

    fn channel_name(&self, k: usize) -> String {
        let p = &self.parts[k];
        match (&p.begun, p.sync_slot) {
            (Some((channels, _, _)), Some(s)) => format!("CH{} {}", channels[s].index, channels[s].name),
            _ => "its shared channel".into(),
        }
    }

    /// Begins the capture: every device's channels, numbered on and named by device; the levels at
    /// the capture's sample 0 take in each device's edges from before it.
    fn begin(&mut self) {
        let mut channels = Vec::new();
        let mut initial = Vec::new();
        for p in &mut self.parts {
            let Some((chs, levels, _)) = &p.begun else { continue };
            for (s, c) in chs.iter().enumerate() {
                let mut level = levels.get(s).copied().unwrap_or_default();
                let q = &mut p.pending[s];
                while q.front().is_some_and(|&x| p.line.at(x) < 0.5) {
                    q.pop_front();
                    level = !level;
                }
                channels.push(ChannelInfo {
                    index: p.index_base + c.index,
                    name: format!("{}: {}", p.label, c.name),
                    ..c.clone()
                });
                initial.push(level);
            }
            // the first matched pairs stay; earlier rising edges are of no more use
            let line = p.line;
            p.rising.retain(|&x| line.at(x) >= 0.0);
        }
        if let Err(e) = self.sink.begin(&channels, &initial, self.rate) {
            self.sink_gone = true;
            self.fail(e.0);
            return;
        }
        self.begun = true;
        let reference = format!("{} {}", self.parts[0].label, self.channel_name(0));
        for k in 1..self.parts.len() {
            if self.parts[k].align.is_none() {
                continue;
            }
            let channel = self.channel_name(k);
            let p = &self.parts[k];
            let at = p.line.offset;
            let message = format!(
                "{} is lined up with {reference} by its {channel}: its sample 0 is at sample {} ({:.3} ms) of this capture, and its clock is followed as the capture runs.",
                p.label,
                at.round(),
                at / self.rate * 1e3
            );
            self.sink.note(Note { level: NoteLevel::Info, message });
        }
    }

    /// Matches each device's new rising edges of the shared signal with the reference's and
    /// follows its clock.
    fn track(&mut self) {
        let span = FIT_SPAN_S * self.rate;
        let known = self.parts[0].known as f64;
        for p in self.parts.iter_mut().skip(1) {
            if p.align.is_none() {
                continue;
            }
            while let Some(&x) = p.rising.front() {
                let y = p.line.at(x);
                // the reference must have delivered past it before it can be matched
                if y + self.tol > known {
                    break;
                }
                p.rising.pop_front();
                let i = self.reference.partition_point(|&a| (a as f64) < y - self.tol);
                if let Some(&a) = self.reference.get(i)
                    && (a as f64 - y).abs() <= self.tol
                {
                    p.fit.add(x as f64, a as f64);
                    if let Some(l) = p.fit.line(span) {
                        p.line = l;
                    }
                }
            }
        }
        // the reference's edges are needed only as far back as any device still looks
        // (a device still to deliver needs them from where it got to)
        let oldest = self
            .parts
            .iter()
            .skip(1)
            .filter(|p| p.align.is_some())
            .map(|p| p.rising.front().map_or(p.end(), |&x| p.line.at(x).min(p.end())))
            .fold(known, f64::min);
        while self.reference.front().is_some_and(|&a| (a as f64) < oldest - 2.0 * self.tol - self.rate) {
            self.reference.pop_front();
        }
    }

    /// Hands every edge before `end` (capture clock) to the capture and advances it there.
    fn emit(&mut self, end: f64) {
        if !self.begun || self.sink_gone || !end.is_finite() {
            return;
        }
        let end = num::floor(end.max(0.0));
        if end <= self.emitted {
            return;
        }
        for p in &mut self.parts {
            for (s, q) in p.pending.iter_mut().enumerate() {
                let slot = p.slot_base + s;
                let mut out = Vec::new();
                while let Some(&x) = q.front() {
                    let y = p.line.at(x);
                    if y >= end as f64 {
                        break;
                    }
                    q.pop_front();
                    // what the capture already reached is final; a clock correction never
                    // puts an edge before it, nor before the channel's last edge
                    let mut y = num::round(y.max(0.0)).max(self.emitted);
                    if let Some(l) = self.last[slot] {
                        y = y.max(l + 1);
                    }
                    self.last[slot] = Some(y);
                    out.push(y);
                }
                if !out.is_empty()
                    && let Err(e) = self.sink.block(Block::Transitions { slot, edges: &out })
                {
                    self.sink_gone = true;
                    self.error.get_or_insert(e.0);
                    self.stop.stop();
                    return;
                }
            }
        }
        if let Err(e) = self.sink.block(Block::Advance { end }) {
            self.sink_gone = true;
            self.error.get_or_insert(e.0);
            self.stop.stop();
            return;
        }
        self.emitted = end;
    }

    /// The drift each device's clock had against the first one's, for the notes at the end.
    fn drift_notes(&mut self) {
        let span = FIT_SPAN_S * self.rate;
        let reference = self.parts[0].label.clone();
        for p in self.parts.iter().skip(1) {
            if p.align.is_none() || p.fit.n < 2.0 || p.fit.hi - p.fit.lo < span {
                continue;
            }
            let ppm = (1.0 / p.line.slope - 1.0) * 1e6;
            let message = format!(
                "{}'s clock ran {ppm:+.1} ppm against {reference}'s; that was corrected along the way ({} edges of the shared signal matched).",
                p.label, p.fit.n
            );
            self.sink.note(Note { level: NoteLevel::Info, message });
        }
    }
}

/// Runs every member at once into `sink` as one capture and returns the devices with the result:
/// the samples the capture reached, whether it was stopped, or why it failed.
pub fn run(members: Vec<Member>, sink: &mut dyn BlockSink, stop: &StopSignal) -> RunResult {
    let (tx, rx) = sync_channel::<(usize, Msg)>(QUEUE);
    let mut parts = Vec::new();
    let mut handles = Vec::new();
    let mut index_base = 0;
    for (k, m) in members.into_iter().enumerate() {
        let next = m.device.capabilities().channels.iter().max().map_or(0, |c| c + 1);
        parts.push(Part {
            label: m.label,
            align: m.align,
            index_base,
            begun: None,
            slot_base: 0,
            pending: Vec::new(),
            known: 0,
            done: None,
            sync_slot: None,
            sync_level: Level::Low,
            rising: VecDeque::new(),
            line: Line::SAME,
            fit: Fit::default(),
            lined_up: false,
        });
        index_base += next;
        let mut tagged = Tagged { k, tx: tx.clone(), levels: Vec::new() };
        let mut dev = m.device;
        let stop = stop.clone();
        let id = m.id;
        handles.push(std::thread::spawn(move || {
            let r = dev.run(&mut tagged, &stop).map_err(|e| e.0);
            let _ = tagged.send(Msg::Done(r));
            (id, dev)
        }));
    }
    drop(tx);
    let mut m = Merger {
        parts,
        sink,
        stop,
        rate: 0.0,
        tol: 2.0,
        reference: VecDeque::new(),
        begun: false,
        last: Vec::new(),
        emitted: 0,
        error: None,
        sink_gone: false,
    };
    // every device sends Done last; the channel closes when all threads ended
    for (k, msg) in rx {
        m.on(k, msg);
    }
    let devices: Vec<(String, Box<dyn Device>)> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
    if devices.len() < m.parts.len() && m.error.is_none() {
        m.error = Some("A device driver stopped unexpectedly.".into());
    }
    if m.parts.iter().any(|p| p.done.is_none()) && m.error.is_none() {
        m.error = Some("A device ended without saying how.".into());
    }
    let result = if m.begun {
        // as far as every device got
        let end = m.parts.iter().map(Part::end).fold(f64::INFINITY, f64::min);
        m.emit(end);
        m.drift_notes();
        match m.error.take() {
            Some(e) => Err(e),
            None => Ok(RunOutcome {
                samples: m.emitted,
                stopped: stop.is_stopped()
                    || m.parts.iter().any(|p| matches!(&p.done, Some(Ok(o)) if o.stopped)),
            }),
        }
    } else {
        Err(m.error.take().unwrap_or_else(|| {
            "The devices ended before they could be lined up; nothing was captured.".into()
        }))
    };
    (devices, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use logica_core::device::{Capabilities, CaptureConfig, CaptureMode, DeviceInfo};

    /// A scripted device with two channels: CH0 sees a clock shared by every fake (its edges at
    /// whole periods of the shared time), CH1 one pulse at shared time 1001.5 s. Its sample 0 is at
    /// shared time `start_s`, and its sample clock runs `ppm` fast.
    struct Fake {
        info: DeviceInfo,
        caps: Capabilities,
        rate: f64,
        start_s: f64,
        ppm: f64,
        period_s: f64,
        pulse_s: f64,
        samples: Sample,
        fail_after: Option<Sample>,
    }

    fn fake(name: &str, start_s: f64, ppm: f64) -> Fake {
        Fake {
            info: DeviceInfo {
                id: format!("fake/{name}"),
                driver: "fake".into(),
                model: format!("Virtual analyser: {name}"),
                serial: None,
                firmware: None,
                channels: 2,
            },
            caps: Capabilities {
                channels: vec![0, 1],
                channel_names: vec!["SYNC".into(), "DATA".into()],
                rates_hz: vec![1e6],
                limits: Vec::new(),
                max_samples: None,
                modes: vec![CaptureMode::Stream],
                threshold: None,
                outputs: 0,
            },
            rate: 1e6,
            start_s,
            ppm,
            period_s: 0.01,
            pulse_s: 1001.5,
            samples: 2_000_000,
            fail_after: None,
        }
    }

    impl Fake {
        /// The sample at shared time `t`.
        fn at(&self, t: f64) -> f64 {
            (t - self.start_s) * self.rate * (1.0 + self.ppm * 1e-6)
        }
        fn edges(&self) -> [Vec<Sample>; 2] {
            let last = self.start_s + self.samples as f64 / self.rate + 1.0;
            let mut sync = Vec::new();
            let half = self.period_s / 2.0;
            let mut k = (self.start_s / half).floor() + 1.0;
            while k * half < last {
                let x = self.at(k * half);
                if x >= 1.0 {
                    sync.push(num::round(x));
                }
                k += 1.0;
            }
            let data = vec![num::round(self.at(self.pulse_s)), num::round(self.at(self.pulse_s + 0.001))];
            [sync, data]
        }
    }

    impl Device for Fake {
        fn info(&self) -> &DeviceInfo {
            &self.info
        }
        fn capabilities(&self) -> &Capabilities {
            &self.caps
        }
        fn configure(&mut self, _: &CaptureConfig) -> Result<(), DeviceError> {
            Ok(())
        }
        fn close(&mut self) {}
        fn run(&mut self, sink: &mut dyn BlockSink, stop: &StopSignal) -> Result<RunOutcome, DeviceError> {
            let half = self.period_s / 2.0;
            let high = num::floor(self.start_s / half).is_multiple_of(2);
            let channels: Vec<ChannelInfo> = [(0, "SYNC"), (1, "DATA")]
                .iter()
                .map(|&(i, n)| ChannelInfo {
                    index: i,
                    name: (*n).into(),
                    colour: None,
                    enabled: true,
                    threshold_volts: None,
                })
                .collect();
            sink.begin(&channels, &[Level::from(high), Level::Low], self.rate)?;
            let all = self.edges();
            let tile = 10_000;
            let mut end = 0;
            while end < self.samples {
                if stop.is_stopped() {
                    return Ok(RunOutcome { samples: end, stopped: true });
                }
                let next = (end + tile).min(self.samples);
                if self.fail_after.is_some_and(|f| next > f) {
                    return Err(DeviceError::new("the cable came loose"));
                }
                for (slot, e) in all.iter().enumerate() {
                    let part: Vec<Sample> = e.iter().copied().filter(|&x| x >= end && x < next).collect();
                    sink.block(Block::Transitions { slot, edges: &part })?;
                }
                sink.block(Block::Advance { end: next })?;
                end = next;
            }
            Ok(RunOutcome { samples: end, stopped: false })
        }
    }

    /// What the capture received.
    #[derive(Default)]
    struct Rec {
        channels: Vec<ChannelInfo>,
        initial: Vec<Level>,
        edges: Vec<Vec<Sample>>,
        end: Sample,
        notes: Vec<String>,
    }

    impl BlockSink for Rec {
        fn begin(&mut self, channels: &[ChannelInfo], initial: &[Level], _: f64) -> Result<(), DeviceError> {
            self.channels = channels.to_vec();
            self.initial = initial.to_vec();
            self.edges = vec![Vec::new(); channels.len()];
            Ok(())
        }
        fn block(&mut self, b: Block<'_>) -> Result<(), DeviceError> {
            match b {
                Block::Transitions { slot, edges } => {
                    for &e in edges {
                        assert!(self.edges[slot].last().is_none_or(|&l| e > l), "edges in order");
                        assert!(e >= self.end, "no edge before what the capture reached");
                        self.edges[slot].push(e);
                    }
                }
                Block::Advance { end } => {
                    assert!(end > self.end, "the capture only moves on");
                    self.end = end;
                }
                _ => panic!("the merger hands on level changes only"),
            }
            Ok(())
        }
        fn note(&mut self, note: Note) {
            self.notes.push(note.message);
        }
    }

    fn members(devices: Vec<Fake>, align: bool) -> Vec<Member> {
        devices
            .into_iter()
            .enumerate()
            .map(|(k, d)| Member {
                id: format!("fake/{k}"),
                label: d.info.model.trim_start_matches("Virtual analyser: ").into(),
                device: Box::new(d),
                align: align.then_some(0),
            })
            .collect()
    }

    fn rising(r: &Rec, slot: usize) -> Vec<Sample> {
        let mut level = r.initial[slot];
        r.edges[slot]
            .iter()
            .filter(|_| {
                level = !level;
                level == Level::High
            })
            .copied()
            .collect()
    }

    #[test]
    fn a_shared_clock_lines_devices_up_and_follows_their_drift() {
        // B started 3.1 ms after A, and its clock runs 300 ppm fast
        let (a, b) = (fake("A", 1000.0123, 0.0), fake("B", 1000.0154, 300.0));
        let mut rec = Rec::default();
        let (devices, r) = run(members(vec![a, b], true), &mut rec, &StopSignal::new());
        assert_eq!(devices.len(), 2);
        assert_eq!(r.unwrap(), RunOutcome { samples: 2_000_000, stopped: false });
        let names: Vec<(u32, &str)> = rec.channels.iter().map(|c| (c.index, c.name.as_str())).collect();
        assert_eq!(names, [(0, "A: SYNC"), (1, "A: DATA"), (2, "B: SYNC"), (3, "B: DATA")]);
        // the same pulse lands on the same sample, 1.5 s in, where the drift alone is 450 samples
        assert_eq!(rec.edges[1].len(), 2);
        let (pa, pb) = (rec.edges[1][0] as i64, rec.edges[3][0] as i64);
        assert!((pa - pb).abs() <= 2, "A {pa}, B {pb}");
        // every edge of the shared clock lines up, from the first to the last
        let (ra, rb) = (rising(&rec, 0), rising(&rec, 2));
        assert!(ra.len() > 190, "{}", ra.len());
        for x in &ra {
            let near = rb.iter().map(|y| (*y as i64 - *x as i64).abs()).min().unwrap();
            assert!(near <= 2, "A's edge at {x} has B's {near} samples away");
        }
        assert!(
            rec.notes.iter().any(|n| n.starts_with(
                "B is lined up with A CH0 SYNC by its CH0 SYNC: its sample 0 is at sample 3100 (3.100 ms)"
            )),
            "{:?}",
            rec.notes
        );
        let matched = rec.notes.iter().find_map(|n| n.split("corrected along the way (").nth(1)).unwrap();
        let n: u32 = matched.split(' ').next().unwrap().parse().unwrap();
        assert!(n >= 195, "every edge of the shared clock is used: {matched}");
        let drift = rec.notes.iter().find_map(|n| n.strip_prefix("B's clock ran ")).unwrap();
        let ppm: f64 = drift.split(' ').next().unwrap().parse().unwrap();
        assert!((ppm - 300.0).abs() < 2.0, "{drift}");
    }

    #[test]
    fn without_a_shared_signal_devices_line_up_at_their_starts() {
        let (a, b) = (fake("A", 1000.0123, 0.0), fake("A", 1000.0154, 300.0));
        let mut rec = Rec::default();
        let (_, r) = run(members(vec![a, b], false), &mut rec, &StopSignal::new());
        assert_eq!(r.unwrap().samples, 2_000_000);
        // B's pulse stays at its own sample: 1.4846 s after its start, on a fast clock
        let own = num::round((1001.5 - 1000.0154) * 1e6 * 1.0003) as i64;
        assert!((rec.edges[3][0] as i64 - own).abs() <= 1);
        assert!(rec.notes.iter().all(|n| !n.contains("lined up")), "{:?}", rec.notes);
    }

    #[test]
    fn signals_without_common_edges_are_lined_up_at_the_start_with_a_warning() {
        let a = fake("A", 1000.0123, 0.0);
        let mut b = fake("B", 1000.0154, 0.0);
        b.period_s = 0.0137;
        let mut rec = Rec::default();
        let (_, r) = run(members(vec![a, b], true), &mut rec, &StopSignal::new());
        assert_eq!(r.unwrap().samples, 2_000_000);
        assert!(
            rec.notes.iter().any(|n| n.starts_with(
                "B could not be lined up with A: CH0 SYNC and CH0 SYNC have no edges in common"
            )),
            "{:?}",
            rec.notes
        );
    }

    #[test]
    fn different_rates_are_refused_and_a_failing_device_ends_the_capture() {
        let a = fake("A", 0.0, 0.0);
        let mut b = fake("B", 0.0, 0.0);
        b.rate = 2e6;
        let mut rec = Rec::default();
        let (devices, r) = run(members(vec![a, b], false), &mut rec, &StopSignal::new());
        assert_eq!(devices.len(), 2, "the devices come back");
        assert!(r.unwrap_err().contains("B captures at 2 MS/s, A at 1 MS/s; one capture has one rate"));

        let a = fake("A", 0.0, 0.0);
        let mut b = fake("B", 0.0, 0.0);
        b.fail_after = Some(500_000);
        let mut rec = Rec::default();
        let (_, r) = run(members(vec![a, b], false), &mut rec, &StopSignal::new());
        assert_eq!(r.unwrap_err(), "B: the cable came loose");
        // what every device delivered is kept: B's failure stops A too, so the capture ends
        // where the one that got less far was, at most where B failed
        assert!(rec.end <= 500_000, "{}", rec.end);
    }

    #[test]
    fn stopping_stops_every_device() {
        let stop = StopSignal::new();
        stop.stop();
        let mut rec = Rec::default();
        let (_, r) = run(members(vec![fake("A", 0.0, 0.0), fake("B", 0.0, 0.0)], false), &mut rec, &stop);
        let o = r.unwrap();
        assert!(o.stopped);
        assert_eq!(o.samples, 0);
    }

    fn clock(start: Sample, period: Sample, n: usize) -> Vec<Sample> {
        (0..n as Sample).map(|i| start + i * period).collect()
    }

    #[test]
    fn a_steady_clock_lines_up_by_the_smallest_shift() {
        // the device started 300 samples after the reference: its edges come 300 samples earlier
        let reference = clock(1_000, 4_000, 40);
        let device = clock(700, 4_000, 40);
        let (d, hits) = best_shift(&reference, &device, tolerance(&reference)).unwrap();
        assert_eq!(d, 300.0);
        assert!(hits >= 30, "{hits}");
        // started 300 samples before: the shift is negative, not a period away
        let device = clock(1_300, 4_000, 40);
        assert_eq!(best_shift(&reference, &device, tolerance(&reference)).unwrap().0, -300.0);
    }

    #[test]
    fn an_irregular_signal_lines_up_however_far_apart() {
        let mut t = 0;
        let reference: Vec<Sample> = (0..60u64)
            .map(|i| {
                t += 1_000 + (i * 7919 % 13) * 300;
                t
            })
            .collect();
        // a shift of 25 000 samples, more than any gap between edges
        let device: Vec<Sample> = reference.iter().filter(|&&a| a > 25_000).map(|a| a - 25_000).collect();
        assert_eq!(best_shift(&reference, &device, 2.0).unwrap().0, 25_000.0);
    }

    #[test]
    fn signals_with_nothing_in_common_do_not_line_up() {
        let reference = clock(1_000, 4_000, 40);
        let device: Vec<Sample> = (0..40u64).map(|i| 500 + i * i * 37).collect();
        assert_eq!(best_shift(&reference, &device, tolerance(&reference)), None);
        assert_eq!(best_shift(&[], &device, 2.0), None);
    }

    #[test]
    fn the_fit_finds_a_drifting_clock() {
        let mut f = Fit::default();
        // the device runs 200 ppm fast and started 1000 samples later
        for i in 0..200u64 {
            let y = 1_000.0 + i as f64 * 40_000.0;
            let x = (y - 1_000.0) * (1.0 + 200e-6);
            f.add(x.round(), y);
        }
        let l = f.line(1e5).unwrap();
        assert!(((1.0 / l.slope - 1.0) * 1e6 - 200.0).abs() < 0.5, "{l:?}");
        assert!((l.offset - 1_000.0).abs() < 1.0, "{l:?}");
        // a short span gives only the shift
        let mut g = Fit::default();
        g.add(10.0, 110.0);
        g.add(20.0, 121.0);
        assert_eq!(g.line(1e3).unwrap(), Line { slope: 1.0, offset: 100.5 });
    }
}
