//! Reading the store (doc 04, "Reading" and "Level of detail"): level at a sample, a level cursor,
//! edges in a range, an incremental edge iterator, merged edge batches for decoders, and exact
//! LOD queries for any bucket size.

use std::sync::Arc;

use super::bits::{self, idx};
use super::channels::CaptureStore;
use super::chunk::{ChunkKind, Payload};
use super::edges::{Edge, EdgeBatch};
use super::error::{StoreError, StoreResult};
use super::lod::LodBucket;
use super::write::{OpenData, check_slot};
use super::{CHUNK_SAMPLES, COARSE_SHIFT, FINE_SHIFT, Level, Sample, chunk_base, chunk_of};

enum ViewData<'a> {
    Quiet,
    Shared(Arc<Payload>),
    Open(&'a OpenData),
}

/// One chunk of one channel, ready to read: a completed chunk (payload from the cache or the
/// backing) or the chunk being written.
pub(crate) struct ChunkView<'a> {
    pub chunk: u64,
    pub entry: Level,
    /// Samples of the chunk that may be read.
    pub len: u64,
    data: ViewData<'a>,
}

impl ChunkView<'_> {
    fn offsets(&self) -> Option<&[u32]> {
        match &self.data {
            ViewData::Shared(p) => match &**p {
                Payload::Offsets(o) => Some(o),
                Payload::Bits(_) => None,
            },
            ViewData::Open(OpenData::Offsets(o)) => Some(o),
            _ => None,
        }
    }

    fn words(&self) -> Option<&[u64]> {
        match &self.data {
            ViewData::Shared(p) => match &**p {
                Payload::Bits(w) => Some(w),
                Payload::Offsets(_) => None,
            },
            ViewData::Open(OpenData::Bits(w)) => Some(w),
            _ => None,
        }
    }

    /// Level of sample `off` of the chunk.
    fn level_at(&self, off: u64) -> Level {
        if let Some(o) = self.offsets() {
            let n = o.partition_point(|&x| u64::from(x) <= off);
            if n % 2 == 1 { !self.entry } else { self.entry }
        } else if let Some(w) = self.words() {
            bits::get(w, off)
        } else {
            self.entry
        }
    }

    /// Transitions at samples in `[a, b)` of the chunk.
    fn transitions_in(&self, a: u64, b: u64) -> u64 {
        if a >= b {
            return 0;
        }
        if let Some(o) = self.offsets() {
            (o.partition_point(|&x| u64::from(x) < b) - o.partition_point(|&x| u64::from(x) < a)) as u64
        } else if let Some(w) = self.words() {
            let prev = if a == 0 { self.entry } else { bits::get(w, a - 1) };
            bits::count_transitions(w, a, b - a, prev).0
        } else {
            0
        }
    }

    /// The first transition at or after `from` and before `lim`, as a chunk offset.
    fn next_transition(&self, from: u64, lim: u64, idx_hint: &mut usize) -> Option<u64> {
        if let Some(o) = self.offsets() {
            while *idx_hint < o.len() && u64::from(o[*idx_hint]) < from {
                *idx_hint += 1;
            }
            let x = u64::from(*o.get(*idx_hint)?);
            return (x < lim).then_some(x);
        }
        let w = self.words()?;
        let mut prev = if from == 0 {
            self.entry as u64
        } else {
            bits::get(w, from - 1) as u64
        };
        let mut p = from;
        while p < lim {
            let valid = (lim - p).min(64);
            let x = bits::window(w, p) & bits::low_mask(valid);
            let t = (x ^ ((x << 1) | prev)) & bits::low_mask(valid);
            if t != 0 {
                return Some(p + u64::from(t.trailing_zeros()));
            }
            prev = (x >> (valid - 1)) & 1;
            p += valid;
        }
        None
    }
}

fn round_up(v: u64, shift: u32) -> u64 {
    v.div_ceil(1 << shift) << shift
}

fn round_down(v: u64, shift: u32) -> u64 {
    (v >> shift) << shift
}

impl CaptureStore {
    fn check_sample(&self, s: Sample) -> StoreResult<()> {
        if s < self.sample_count() {
            Ok(())
        } else {
            Err(StoreError::OutOfRange {
                sample: s,
                count: self.sample_count(),
            })
        }
    }

    /// A readable view of chunk `chunk` of `slot`.
    pub(crate) fn view(&self, slot: usize, chunk: u64) -> StoreResult<ChunkView<'_>> {
        let ch = &self.channels[slot];
        let base = chunk_base(chunk);
        let avail = ch.written_to.saturating_sub(base).min(CHUNK_SAMPLES);
        if let Some(s) = ch.chunks.get(idx(chunk)) {
            let data = if s.kind == ChunkKind::Quiet {
                ViewData::Quiet
            } else {
                ViewData::Shared(self.payload(slot, chunk)?)
            };
            return Ok(ChunkView {
                chunk,
                entry: s.entry,
                len: avail,
                data,
            });
        }
        if chunk == ch.chunks.len() as u64 {
            if let Some(open) = &ch.open {
                return Ok(ChunkView {
                    chunk,
                    entry: open.entry,
                    len: avail,
                    data: ViewData::Open(&open.data),
                });
            }
            return Ok(ChunkView {
                chunk,
                entry: ch.level,
                len: 0,
                data: ViewData::Quiet,
            });
        }
        Err(StoreError::OutOfRange {
            sample: base,
            count: self.sample_count(),
        })
    }

    /// Level of `slot` at sample `s`.
    pub fn level_at(&self, slot: usize, s: Sample) -> StoreResult<Level> {
        check_slot(slot, self.slots())?;
        self.check_sample(s)?;
        let k = chunk_of(s);
        let off = s - chunk_base(k);
        if let Some(sum) = self.channels[slot].chunks.get(idx(k)) {
            if sum.kind == ChunkKind::Quiet {
                return Ok(sum.entry);
            }
            if let Some(c) = &sum.coarse
                && off.is_multiple_of(1 << COARSE_SHIFT)
            {
                return Ok(c.start_level(idx(off >> COARSE_SHIFT)));
            }
        }
        Ok(self.view(slot, k)?.level_at(off))
    }

    /// A cursor answering levels of `slot`, amortised O(1) for increasing positions.
    pub fn cursor(&self, slot: usize) -> StoreResult<LevelCursor<'_>> {
        check_slot(slot, self.slots())?;
        Ok(LevelCursor {
            store: self,
            slot,
            view: None,
            idx: 0,
            last: 0,
        })
    }

    /// Edges (first samples at a new level) of `slot` with samples in `[from, to)`; `to` is
    /// limited to the samples written so far.
    pub fn edges(&self, slot: usize, from: Sample, to: Sample) -> StoreResult<EdgeIter<'_>> {
        check_slot(slot, self.slots())?;
        Ok(EdgeIter {
            store: self,
            slot,
            pos: from,
            to: to.min(self.sample_count()),
            view: None,
            idx: 0,
        })
    }

    /// All edges of `slot` in `[from, to)`.
    pub fn edges_in(&self, slot: usize, from: Sample, to: Sample) -> StoreResult<Vec<Sample>> {
        self.edges(slot, from, to)?.collect()
    }

    /// Merged edges of `slots` after sample `start` and before `end`, in batches (doc 06). The
    /// levels at `start` are [`EdgeMerger::start_levels`].
    pub fn merger(&self, slots: &[usize], start: Sample, end: Sample) -> StoreResult<EdgeMerger<'_>> {
        if slots.len() > 256 {
            return Err(StoreError::Invalid(
                "A decoder can use at most 256 channels.".into(),
            ));
        }
        let end = end.min(self.sample_count());
        let mut levels = Vec::with_capacity(slots.len());
        let mut iters = Vec::with_capacity(slots.len());
        for &slot in slots {
            levels.push(self.level_at(slot, start)?);
            iters.push(self.edges(slot, start + 1, end)?);
        }
        let mut heads = Vec::with_capacity(slots.len());
        for it in &mut iters {
            heads.push(it.next().transpose()?);
        }
        Ok(EdgeMerger {
            start_levels: levels.clone(),
            levels,
            iters,
            heads,
            end,
            done: false,
        })
    }

    /// Exact number of transitions of `slot` at samples in `[from, to)`. Uses the coarse LOD for
    /// whole 2^16 buckets, the fine LOD for whole 2^8 buckets of dense chunks, and the payload for
    /// the rest.
    pub fn transitions_in(&self, slot: usize, from: Sample, to: Sample) -> StoreResult<u64> {
        check_slot(slot, self.slots())?;
        let to = to.min(self.sample_count());
        let mut total = 0u64;
        let mut k = chunk_of(from);
        while from < to && chunk_base(k) < to {
            let base = chunk_base(k);
            let lo = from.max(base) - base;
            let hi = to.min(chunk_base(k + 1)) - base;
            total += self.chunk_transitions(slot, k, lo, hi)?;
            k += 1;
        }
        Ok(total)
    }

    fn chunk_transitions(&self, slot: usize, k: u64, lo: u64, hi: u64) -> StoreResult<u64> {
        if lo >= hi {
            return Ok(0);
        }
        let Some(s) = self.channels[slot].chunks.get(idx(k)) else {
            return Ok(self.view(slot, k)?.transitions_in(lo, hi));
        };
        if s.kind == ChunkKind::Quiet {
            return Ok(0);
        }
        if lo == 0 && hi == CHUNK_SAMPLES {
            return Ok(u64::from(s.transitions));
        }
        let (c0, c1) = (round_up(lo, COARSE_SHIFT), round_down(hi, COARSE_SHIFT));
        if let Some(coarse) = &s.coarse
            && c0 < c1
        {
            let mut sum = 0u64;
            for j in idx(c0 >> COARSE_SHIFT)..idx(c1 >> COARSE_SHIFT) {
                sum += if coarse.saturated(j) {
                    let b = (j as u64) << COARSE_SHIFT;
                    self.fine_transitions(slot, k, b, b + (1 << COARSE_SHIFT))?
                } else {
                    u64::from(coarse.counts[j])
                };
            }
            return Ok(sum
                + self.fine_transitions(slot, k, lo, c0)?
                + self.fine_transitions(slot, k, c1, hi)?);
        }
        self.fine_transitions(slot, k, lo, hi)
    }

    /// Like `chunk_transitions` below the coarse level: fine LOD where it exists and the range
    /// covers whole fine buckets, the payload for the rest.
    fn fine_transitions(&self, slot: usize, k: u64, lo: u64, hi: u64) -> StoreResult<u64> {
        if lo >= hi {
            return Ok(0);
        }
        let s = &self.channels[slot].chunks[idx(k)];
        let (f0, f1) = (round_up(lo, FINE_SHIFT), round_down(hi, FINE_SHIFT));
        if s.has_fine && f0 < f1 {
            let fine = self.fine_lod(slot, k)?;
            let mid: u64 = fine.counts[idx(f0 >> FINE_SHIFT)..idx(f1 >> FINE_SHIFT)]
                .iter()
                .map(|&c| u64::from(c))
                .sum();
            let mut edge_parts = 0;
            if lo < f0 || f1 < hi {
                let v = self.view(slot, k)?;
                edge_parts = v.transitions_in(lo, f0) + v.transitions_in(f1, hi);
            }
            return Ok(mid + edge_parts);
        }
        Ok(self.view(slot, k)?.transitions_in(lo, hi))
    }

    /// LOD of `slot`: `n` buckets of `bucket` samples from `start`, exact, stopping at the end of
    /// the samples written so far (the last bucket may be short).
    pub fn lod(&self, slot: usize, start: Sample, bucket: u64, n: usize) -> StoreResult<Vec<LodBucket>> {
        check_slot(slot, self.slots())?;
        if bucket == 0 {
            return Err(StoreError::Invalid(
                "A LOD bucket must hold at least one sample.".into(),
            ));
        }
        let mut out = Vec::with_capacity(n.min(1 << 20));
        let end = self.sample_count();
        let mut b0 = start;
        for _ in 0..n {
            if b0 >= end {
                break;
            }
            let b1 = b0.saturating_add(bucket).min(end);
            out.push(LodBucket {
                transitions: self.transitions_in(slot, b0, b1)?,
                start_level: self.level_at(slot, b0)?,
            });
            b0 = b1;
        }
        Ok(out)
    }
}

/// Levels of one channel at increasing (or any) positions.
pub struct LevelCursor<'a> {
    store: &'a CaptureStore,
    slot: usize,
    view: Option<ChunkView<'a>>,
    /// For offsets: how many offsets are at or before `last`.
    idx: usize,
    last: u64,
}

impl LevelCursor<'_> {
    pub fn level_at(&mut self, s: Sample) -> StoreResult<Level> {
        self.store.check_sample(s)?;
        let k = chunk_of(s);
        let off = s - chunk_base(k);
        let fresh = !matches!(&self.view, Some(v) if v.chunk == k) || off < self.last;
        if fresh {
            if !matches!(&self.view, Some(v) if v.chunk == k) {
                self.view = Some(self.store.view(self.slot, k)?);
            }
            self.idx = 0;
        }
        self.last = off;
        let Some(v) = &self.view else {
            return Err(StoreError::Invalid("no chunk".into()));
        };
        if let Some(o) = v.offsets() {
            if fresh {
                self.idx = o.partition_point(|&x| u64::from(x) <= off);
            } else {
                while self.idx < o.len() && u64::from(o[self.idx]) <= off {
                    self.idx += 1;
                }
            }
            return Ok(if self.idx % 2 == 1 { !v.entry } else { v.entry });
        }
        Ok(v.level_at(off))
    }
}

/// Edges of one channel in a range, one at a time.
pub struct EdgeIter<'a> {
    store: &'a CaptureStore,
    slot: usize,
    /// Next sample to look at.
    pos: Sample,
    to: Sample,
    view: Option<ChunkView<'a>>,
    idx: usize,
}

impl Iterator for EdgeIter<'_> {
    type Item = StoreResult<Sample>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.to {
            let k = chunk_of(self.pos);
            let base = chunk_base(k);
            let next_chunk = chunk_base(k + 1).min(self.to);
            if !matches!(&self.view, Some(v) if v.chunk == k) {
                let chunks = &self.store.channels[self.slot].chunks;
                if chunks.get(idx(k)).is_some_and(|s| s.transitions == 0) {
                    self.pos = next_chunk;
                    continue;
                }
                match self.store.view(self.slot, k) {
                    Ok(v) => {
                        // Start the offset index at the first position, not at 0.
                        self.idx = v
                            .offsets()
                            .map_or(0, |o| o.partition_point(|&x| u64::from(x) < self.pos - base));
                        self.view = Some(v);
                    }
                    Err(e) => {
                        self.pos = self.to;
                        return Some(Err(e));
                    }
                }
            }
            let v = self.view.as_ref()?;
            let lim = (self.to - base).min(v.len);
            match v.next_transition(self.pos - base, lim, &mut self.idx) {
                Some(off) => {
                    self.pos = base + off + 1;
                    return Some(Ok(base + off));
                }
                None => self.pos = next_chunk,
            }
        }
        None
    }
}

/// The merged, time-ordered edges of a decoder's channels, delivered in batches (doc 06).
pub struct EdgeMerger<'a> {
    start_levels: Vec<Level>,
    levels: Vec<Level>,
    iters: Vec<EdgeIter<'a>>,
    heads: Vec<Option<Sample>>,
    end: Sample,
    done: bool,
}

impl EdgeMerger<'_> {
    /// Levels of the channels at the start sample.
    pub fn start_levels(&self) -> &[Level] {
        &self.start_levels
    }

    /// Where the stream ends: no edge at or after it is delivered.
    pub fn end(&self) -> Sample {
        self.end
    }

    /// The next batch: edges before `until`, at most `max_edges` (at least 1) unless more edges
    /// share the last sample — a batch never splits the edges of one sample. Every edge in it is
    /// before its `end`, and every edge before `end` has been delivered. `None` once the stream's
    /// end has been reached.
    pub fn next_batch(&mut self, max_edges: usize, until: Sample) -> StoreResult<Option<EdgeBatch>> {
        if self.done {
            return Ok(None);
        }
        let until = until.min(self.end);
        let max_edges = max_edges.max(1);
        let mut edges = Vec::new();
        let end = loop {
            let mut best: Option<(Sample, usize)> = None;
            for (i, h) in self.heads.iter().enumerate() {
                if let Some(s) = *h
                    && best.is_none_or(|(b, _)| s < b)
                {
                    best = Some((s, i));
                }
            }
            let Some((s, i)) = best else { break until };
            if s >= until {
                break until;
            }
            if edges.len() >= max_edges && edges.last().is_some_and(|e: &Edge| e.sample < s) {
                break s;
            }
            self.levels[i] = !self.levels[i];
            edges.push(Edge {
                sample: s,
                slot: u8::try_from(i).unwrap_or(u8::MAX),
                level: self.levels[i],
            });
            self.heads[i] = self.iters[i].next().transpose()?;
        };
        if end >= self.end {
            self.done = true;
        }
        Ok(Some(EdgeBatch { edges, end }))
    }

    /// Every remaining edge in one batch.
    pub fn rest(&mut self) -> StoreResult<EdgeBatch> {
        let mut all = EdgeBatch {
            edges: Vec::new(),
            end: self.end,
        };
        while let Some(b) = self.next_batch(usize::MAX, Sample::MAX)? {
            all.edges.extend(b.edges);
            all.end = b.end;
        }
        Ok(all)
    }
}
