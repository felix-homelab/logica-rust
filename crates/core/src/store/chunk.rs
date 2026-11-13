//! Chunk summaries and payloads (doc 04, "Transitions, chunked").

use std::sync::OnceLock;

use super::lod::CoarseLod;
use super::{CHUNK_BITS_BYTES, CHUNK_WORDS, Level, MAX_OFFSETS};

/// How a chunk's transitions are stored. The values are the `kind` byte of `.lacap` (doc 09).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChunkKind {
    /// No transitions; no payload.
    Quiet = 0,
    /// Ascending chunk-relative transition positions.
    Offsets = 1,
    /// Raw samples, least significant bit first.
    Bits = 2,
}

impl ChunkKind {
    /// The cheapest representation for a chunk with `transitions` transitions.
    pub const fn for_transitions(transitions: u32) -> Self {
        if transitions == 0 {
            ChunkKind::Quiet
        } else if transitions <= MAX_OFFSETS {
            ChunkKind::Offsets
        } else {
            ChunkKind::Bits
        }
    }

    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(ChunkKind::Quiet),
            1 => Some(ChunkKind::Offsets),
            2 => Some(ChunkKind::Bits),
            _ => None,
        }
    }
}

/// Where a blob lives in the backing file: the offset of its blob header and the length of the
/// whole blob (header, payload and CRC).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlobLoc {
    pub offset: u64,
    pub len: u64,
}

/// What is always resident about one chunk of one channel: about 40 bytes, plus 544 bytes of
/// coarse LOD when the chunk is not quiet. 5e9 samples on 8 channels are 2,400 summaries.
#[derive(Debug)]
pub struct ChunkSummary {
    pub kind: ChunkKind,
    /// Level of the sample before the chunk (for chunk 0, the level at sample 0).
    pub entry: Level,
    /// Level of the chunk's last sample.
    pub exit: Level,
    pub transitions: u32,
    /// Whether the chunk has a fine LOD (more than `FINE_LOD_MIN_TRANSITIONS` transitions).
    pub has_fine: bool,
    /// Where the payload is in the backing, once it has been written there (on eviction or
    /// save). Set once from `&self`, so readers that evict can record it without a write lock.
    pub payload_at: OnceLock<BlobLoc>,
    /// Where the fine LOD is in the backing, once written.
    pub fine_at: OnceLock<BlobLoc>,
    /// Coarse LOD; `None` for quiet chunks.
    pub coarse: Option<Box<CoarseLod>>,
}

impl ChunkSummary {
    /// A chunk without transitions at `level`.
    pub fn quiet(level: Level) -> Self {
        ChunkSummary {
            kind: ChunkKind::Quiet,
            entry: level,
            exit: level,
            transitions: 0,
            has_fine: false,
            payload_at: OnceLock::new(),
            fine_at: OnceLock::new(),
            coarse: None,
        }
    }
}

/// A chunk's transitions, as held in the cache. Immutable once the chunk is complete; readers
/// hold it through an `Arc`, so eviction never pulls data from under a reader.
#[derive(Debug, PartialEq, Eq)]
pub enum Payload {
    /// Ascending chunk-relative positions, each below 2^24; at most `MAX_OFFSETS` of them.
    Offsets(Box<[u32]>),
    /// `CHUNK_WORDS` words, sample k of the chunk in bit k % 64 of word k / 64. In a chunk that
    /// ends before 2^24 samples (the last one), bits past the end repeat the last level, so they
    /// never look like transitions.
    Bits(Box<[u64]>),
}

impl Payload {
    /// Bytes this payload occupies in memory, for the cache budget.
    pub fn heap_bytes(&self) -> usize {
        match self {
            Payload::Offsets(o) => o.len() * 4,
            Payload::Bits(_) => CHUNK_BITS_BYTES,
        }
    }

    pub fn kind(&self) -> ChunkKind {
        match self {
            Payload::Offsets(_) => ChunkKind::Offsets,
            Payload::Bits(_) => ChunkKind::Bits,
        }
    }

    /// A bits payload with every sample at `level`.
    pub fn bits_filled(level: Level) -> Self {
        Payload::Bits(vec![level.fill_word(); CHUNK_WORDS].into_boxed_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representation_follows_density() {
        assert_eq!(ChunkKind::for_transitions(0), ChunkKind::Quiet);
        assert_eq!(ChunkKind::for_transitions(1), ChunkKind::Offsets);
        assert_eq!(ChunkKind::for_transitions(MAX_OFFSETS), ChunkKind::Offsets);
        assert_eq!(ChunkKind::for_transitions(MAX_OFFSETS + 1), ChunkKind::Bits);
    }

    #[test]
    fn kind_bytes_round_trip() {
        for k in [ChunkKind::Quiet, ChunkKind::Offsets, ChunkKind::Bits] {
            assert_eq!(ChunkKind::from_byte(k as u8), Some(k));
        }
        assert_eq!(ChunkKind::from_byte(3), None);
    }

    #[test]
    fn payload_memory_is_never_above_raw() {
        let o = Payload::Offsets(vec![0; MAX_OFFSETS as usize].into_boxed_slice());
        assert_eq!(o.heap_bytes(), CHUNK_BITS_BYTES);
        assert_eq!(Payload::bits_filled(Level::High).heap_bytes(), CHUNK_BITS_BYTES);
    }
}
