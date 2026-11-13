//! Writing into the store: transition lists, per-channel bit blocks and packed samples (doc 04,
//! "Ingest"), and completing chunks (summary, coarse LOD, fine LOD for dense chunks).

use super::bits::{self, idx, off32};
use super::chunk::{ChunkKind, ChunkSummary, Payload};
use super::error::{StoreError, StoreResult};
use super::lod::{CoarseLod, FineLod};
use super::{
    CHUNK_SAMPLES, CHUNK_WORDS, COARSE_BUCKETS, COARSE_SHIFT, FINE_BUCKETS, FINE_LOD_MIN_TRANSITIONS,
    FINE_SHIFT, Level, MAX_OFFSETS, Sample, chunk_base, chunk_of,
};

/// The data of the chunk being written.
#[derive(Debug)]
pub(crate) enum OpenData {
    /// Chunk-relative transition positions so far.
    Offsets(Vec<u32>),
    /// Raw bits; valid up to the channel's `written_to`.
    Bits(Box<[u64]>),
}

#[derive(Debug)]
pub(crate) struct OpenChunk {
    pub entry: Level,
    pub data: OpenData,
    pub transitions: u32,
}

/// A chunk just completed: its payload and fine LOD go into the cache.
pub(crate) struct Completed {
    pub chunk: u64,
    pub payload: Option<Payload>,
    pub fine: Option<FineLod>,
}

/// One channel's chunks and its write position.
#[derive(Debug)]
pub(crate) struct ChannelState {
    /// Level at sample 0.
    pub initial: Level,
    /// Completed chunks; chunk k covers samples [k * 2^24, (k + 1) * 2^24).
    pub chunks: Vec<ChunkSummary>,
    /// The chunk being written, always chunk `chunks.len()`. `None` means `written_to` is at the
    /// start of that chunk and nothing of it has been written.
    pub open: Option<OpenChunk>,
    /// Level of the last written sample (the initial level before anything is written).
    pub level: Level,
    /// Every sample before this is known.
    pub written_to: Sample,
}

impl ChannelState {
    pub fn new(initial: Level) -> Self {
        ChannelState {
            initial,
            chunks: Vec::new(),
            open: None,
            level: initial,
            written_to: 0,
        }
    }

    fn open_chunk(&mut self) -> &mut OpenChunk {
        let entry = self.level;
        self.open.get_or_insert_with(|| OpenChunk {
            entry,
            data: OpenData::Offsets(Vec::new()),
            transitions: 0,
        })
    }

    /// Completes the open chunk with `len` samples (2^24 except at the end of a capture).
    fn complete_open(&mut self, len: u64, done: &mut Vec<Completed>) {
        let chunk = self.chunks.len() as u64;
        let (summary, payload, fine) = match self.open.take() {
            Some(open) => finalise(open, len, self.level),
            None => (ChunkSummary::quiet(self.level), None, None),
        };
        self.chunks.push(summary);
        if payload.is_some() || fine.is_some() {
            done.push(Completed { chunk, payload, fine });
        }
    }

    /// Extends the channel at its current level up to `target`.
    pub fn fill_to(&mut self, target: Sample, done: &mut Vec<Completed>) {
        while self.written_to < target {
            let k = chunk_of(self.written_to);
            let base = chunk_base(k);
            let end = chunk_base(k + 1);
            if self.open.is_none() && target >= end {
                self.chunks.push(ChunkSummary::quiet(self.level));
                self.written_to = end;
                continue;
            }
            let to = target.min(end);
            let (from, level) = (self.written_to - base, self.level);
            if let OpenData::Bits(w) = &mut self.open_chunk().data {
                bits::fill(w, from, to - base, level);
            }
            self.written_to = to;
            if to == end {
                self.complete_open(CHUNK_SAMPLES, done);
            }
        }
    }

    /// The level changes at sample `e` (at or after `written_to`). At sample 0 this sets the
    /// initial level instead: the first data defines it, and chunk 0 has no edge at offset 0.
    pub fn toggle_at(&mut self, e: Sample, done: &mut Vec<Completed>) {
        if e == 0 {
            self.initial = !self.initial;
            self.level = self.initial;
            self.fill_to(1, done);
            return;
        }
        self.fill_to(e, done);
        let base = chunk_base(chunk_of(e));
        let off = off32(e - base);
        let level = !self.level;
        // The open chunk must exist before the level flips: its entry is the level before the edge.
        let open = self.open_chunk();
        open.transitions += 1;
        match &mut open.data {
            OpenData::Offsets(v) => {
                v.push(off);
                if v.len() > MAX_OFFSETS as usize {
                    open.data = OpenData::Bits(offsets_to_bits(open.entry, v, u64::from(off) + 1));
                }
            }
            OpenData::Bits(w) => bits::fill(w, u64::from(off), u64::from(off) + 1, level),
        }
        self.level = level;
        self.written_to = e + 1;
        if self.written_to == chunk_base(chunk_of(e) + 1) {
            self.complete_open(CHUNK_SAMPLES, done);
        }
    }

    /// Appends `n` samples given as bits of `src` from bit `off`. Each piece is counted first;
    /// it is extracted to offsets while the chunk stays at or below `MAX_OFFSETS` transitions,
    /// otherwise the chunk becomes raw bits (doc 04).
    pub fn push_bits(&mut self, src: &[u64], off: u64, n: u64, done: &mut Vec<Completed>) {
        if n == 0 {
            return;
        }
        if self.written_to == 0 {
            self.initial = bits::get(src, off);
            self.level = self.initial;
        }
        let mut p = 0u64;
        while p < n {
            let k = chunk_of(self.written_to);
            let base = chunk_base(k);
            let end = chunk_base(k + 1);
            let m = (n - p).min(end - self.written_to);
            let at = self.written_to - base;
            let prev = self.level;
            let (t, last) = bits::count_transitions(src, off + p, m, prev);
            let t = u32::try_from(t).unwrap_or(u32::MAX);
            let open = self.open_chunk();
            let fits = matches!(open.data, OpenData::Offsets(_)) && open.transitions + t <= MAX_OFFSETS;
            if fits {
                if let OpenData::Offsets(v) = &mut open.data {
                    bits::for_each_transition(src, off + p, m, prev, |r| v.push(off32(at + r)));
                }
            } else {
                if let OpenData::Offsets(v) = &open.data {
                    open.data = OpenData::Bits(offsets_to_bits(open.entry, v, at));
                }
                if let OpenData::Bits(w) = &mut open.data {
                    bits::copy(w, at, src, off + p, m);
                }
            }
            open.transitions += t;
            self.level = last;
            self.written_to += m;
            p += m;
            if self.written_to == end {
                self.complete_open(CHUNK_SAMPLES, done);
            }
        }
    }

    /// Completes the last, partial chunk at `end` (the capture's sample count).
    pub fn finish(&mut self, end: Sample, done: &mut Vec<Completed>) {
        self.fill_to(end, done);
        if self.open.is_some() {
            let len = end - chunk_base(self.chunks.len() as u64);
            self.complete_open(len, done);
        }
    }
}

/// Raw bits from offsets, valid up to `upto`; the rest is filled with the last level.
fn offsets_to_bits(entry: Level, offs: &[u32], upto: u64) -> Box<[u64]> {
    let mut w = vec![0u64; CHUNK_WORDS].into_boxed_slice();
    let mut level = entry;
    let mut from = 0u64;
    for &o in offs {
        bits::fill(&mut w, from, u64::from(o), level);
        level = !level;
        from = u64::from(o);
    }
    bits::fill(&mut w, from, upto.max(from), level);
    bits::fill(&mut w, upto.max(from), CHUNK_SAMPLES, level);
    w
}

/// LOD buckets of `1 << shift` samples from ascending offsets.
fn lod_from_offsets(offs: &[u32], entry: Level, shift: u32, buckets: usize) -> (Vec<u16>, Vec<u8>) {
    let mut counts = vec![0u16; buckets];
    let mut levels = vec![0u8; buckets / 8];
    for &o in offs {
        let c = &mut counts[(o >> shift) as usize];
        *c = c.saturating_add(1);
    }
    let mut level = entry;
    let mut i = 0usize;
    for (j, slot) in (0..buckets).zip(0u64..) {
        let start = slot << shift;
        while i < offs.len() && u64::from(offs[i]) <= start {
            level = !level;
            i += 1;
        }
        levels[j / 8] |= (level as u8) << (j % 8);
    }
    (counts, levels)
}

fn coarse_from(counts: Vec<u16>, levels: Vec<u8>) -> Box<CoarseLod> {
    let mut c = CoarseLod {
        counts: [0; COARSE_BUCKETS],
        start_levels: [0; COARSE_BUCKETS / 8],
    };
    c.counts.copy_from_slice(&counts);
    c.start_levels.copy_from_slice(&levels);
    Box::new(c)
}

/// Builds the summary, payload and fine LOD of a completed chunk of `len` samples whose last
/// sample is at level `exit`.
fn finalise(open: OpenChunk, len: u64, exit: Level) -> (ChunkSummary, Option<Payload>, Option<FineLod>) {
    let entry = open.entry;
    let mut summary = ChunkSummary::quiet(entry);
    summary.exit = exit;
    match open.data {
        OpenData::Offsets(offs) => {
            let t = off32(offs.len() as u64);
            if t == 0 {
                return (summary, None, None);
            }
            let (cc, cl) = lod_from_offsets(&offs, entry, COARSE_SHIFT, COARSE_BUCKETS);
            summary.coarse = Some(coarse_from(cc, cl));
            let fine = (t > FINE_LOD_MIN_TRANSITIONS).then(|| {
                let (fc, fl) = lod_from_offsets(&offs, entry, FINE_SHIFT, FINE_BUCKETS);
                FineLod {
                    counts: fc.into_boxed_slice(),
                    start_levels: fl.into_boxed_slice(),
                }
            });
            summary.kind = ChunkKind::Offsets;
            summary.transitions = t;
            summary.has_fine = fine.is_some();
            (summary, Some(Payload::Offsets(offs.into_boxed_slice())), fine)
        }
        OpenData::Bits(mut w) => {
            bits::fill(&mut w, len, CHUNK_SAMPLES, exit);
            // Fine counts from popcounts: a fine bucket is 4 words.
            let mut fine = FineLod::new();
            let mut prev = entry as u64;
            let mut total = 0u64;
            for (j, words) in w.as_chunks::<4>().0.iter().enumerate() {
                let mut c = 0u32;
                for &x in words {
                    c += (x ^ ((x << 1) | prev)).count_ones();
                    prev = x >> 63;
                }
                fine.counts[j] = u16::try_from(c).unwrap_or(u16::MAX);
                fine.start_levels[j / 8] |= u8::from(words[0] & 1 == 1) << (j % 8);
                total += u64::from(c);
            }
            let mut cc = vec![0u16; COARSE_BUCKETS];
            let mut cl = vec![0u8; COARSE_BUCKETS / 8];
            let per = FINE_BUCKETS / COARSE_BUCKETS;
            for j in 0..COARSE_BUCKETS {
                let sum: u32 = fine.counts[j * per..(j + 1) * per]
                    .iter()
                    .map(|&c| u32::from(c))
                    .sum();
                cc[j] = u16::try_from(sum).unwrap_or(u16::MAX);
                cl[j / 8] |= (fine.start_level(j * per) as u8) << (j % 8);
            }
            let t = u32::try_from(total).unwrap_or(u32::MAX);
            if t == 0 {
                return (summary, None, None);
            }
            summary.coarse = Some(coarse_from(cc, cl));
            summary.transitions = t;
            summary.has_fine = t > FINE_LOD_MIN_TRANSITIONS;
            let fine = summary.has_fine.then_some(fine);
            if t <= MAX_OFFSETS {
                let mut offs = Vec::with_capacity(idx(u64::from(t)));
                bits::for_each_transition(&w, 0, CHUNK_SAMPLES, entry, |r| offs.push(off32(r)));
                summary.kind = ChunkKind::Offsets;
                (summary, Some(Payload::Offsets(offs.into_boxed_slice())), fine)
            } else {
                summary.kind = ChunkKind::Bits;
                (summary, Some(Payload::Bits(w)), fine)
            }
        }
    }
}

/// Checks that `slot` exists.
pub(crate) fn check_slot(slot: usize, channels: usize) -> StoreResult<()> {
    if slot < channels {
        Ok(())
    } else {
        Err(StoreError::NoSuchChannel { slot, channels })
    }
}
