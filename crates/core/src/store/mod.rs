//! The chunked transition store (doc 04, ADR 0004).
//!
//! Each channel is a sequence of chunks of [`CHUNK_SAMPLES`] samples. A chunk is quiet (no
//! payload), a list of chunk-relative transition offsets, or raw bits; every chunk records its
//! entry level, so it can be read on its own. Summaries and coarse LODs are always resident;
//! payloads and fine LODs live in a byte-budgeted cache ([`cache::ChunkCache`]) and are evicted to a
//! backing file (memory or the `.lacap` itself).
//!
//! An edge is the first sample at a new level. Levels are binary, so a channel is fully described by
//! its level at sample 0 and its ascending edge positions.

pub mod bits;
pub(crate) mod blob;
pub mod cache;
pub mod channels;
pub mod chunk;
pub mod edges;
pub mod error;
pub mod lod;
mod read;
mod write;

pub use channels::CaptureStore;
pub use error::{StoreError, StoreResult};
pub use read::{EdgeIter, EdgeMerger, LevelCursor};

use std::ops::Not;

/// A sample index: the unit of time inside the engine. Stays below 2^53 (doc 04).
pub type Sample = u64;

/// log2 of the chunk length.
pub const CHUNK_SHIFT: u32 = 24;
/// Samples per chunk: 2^24 = 16,777,216.
pub const CHUNK_SAMPLES: u64 = 1 << CHUNK_SHIFT;
/// 64-bit words of raw bits per chunk (2 MiB).
pub const CHUNK_WORDS: usize = 1 << (CHUNK_SHIFT - 6);
/// Bytes of raw bits per chunk.
pub const CHUNK_BITS_BYTES: usize = CHUNK_WORDS * 8;
/// Above this many transitions, offsets would be larger than the raw bits, so the chunk is bits.
pub const MAX_OFFSETS: u32 = 1 << 19;

/// log2 of the coarse LOD bucket: 2^16 samples, 256 buckets per chunk.
pub const COARSE_SHIFT: u32 = 16;
pub const COARSE_BUCKETS: usize = 1 << (CHUNK_SHIFT - COARSE_SHIFT);
/// log2 of the fine LOD bucket: 2^8 samples, 65,536 buckets per chunk.
pub const FINE_SHIFT: u32 = 8;
pub const FINE_BUCKETS: usize = 1 << (CHUNK_SHIFT - FINE_SHIFT);
/// Chunks with more transitions than this get a fine LOD.
pub const FINE_LOD_MIN_TRANSITIONS: u32 = 4096;

/// Default byte budget of the chunk cache (doc 04).
pub const DEFAULT_CACHE_BUDGET: usize = 512 << 20;

/// Index of the chunk holding `sample`.
pub const fn chunk_of(sample: Sample) -> u64 {
    sample >> CHUNK_SHIFT
}

/// First sample of chunk `chunk`.
pub const fn chunk_base(chunk: u64) -> Sample {
    chunk << CHUNK_SHIFT
}

/// Position of `sample` inside its chunk. Always below 2^24, so it fits a `u32`.
pub const fn offset_in_chunk(sample: Sample) -> u32 {
    (sample & (CHUNK_SAMPLES - 1)) as u32
}

/// A digital level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Level {
    #[default]
    Low = 0,
    High = 1,
}

impl Level {
    /// The level of bit 0 of `bit`.
    pub const fn from_bit(bit: u64) -> Self {
        if bit & 1 == 1 { Level::High } else { Level::Low }
    }

    pub const fn is_high(self) -> bool {
        matches!(self, Level::High)
    }

    /// All zeros for low, all ones for high: the word a run of this level fills.
    pub const fn fill_word(self) -> u64 {
        match self {
            Level::Low => 0,
            Level::High => u64::MAX,
        }
    }
}

impl Not for Level {
    type Output = Level;
    fn not(self) -> Level {
        match self {
            Level::Low => Level::High,
            Level::High => Level::Low,
        }
    }
}

impl From<bool> for Level {
    fn from(high: bool) -> Self {
        if high { Level::High } else { Level::Low }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_arithmetic_works_past_2_pow_32() {
        let s: Sample = (1u64 << 32) + 5;
        assert_eq!(chunk_of(s), 256);
        assert_eq!(chunk_base(256), 1u64 << 32);
        assert_eq!(offset_in_chunk(s), 5);
        assert_eq!(offset_in_chunk(chunk_base(7) - 1), (1 << 24) - 1);
    }

    #[test]
    fn sizes_match_doc_04() {
        assert_eq!(CHUNK_SAMPLES, 16_777_216);
        assert_eq!(CHUNK_BITS_BYTES, 2 << 20);
        // offsets above MAX_OFFSETS would take more than the raw bits
        assert_eq!(MAX_OFFSETS as usize * 4, CHUNK_BITS_BYTES);
        assert_eq!(COARSE_BUCKETS, 256);
        assert_eq!(FINE_BUCKETS, 65_536);
    }

    #[test]
    fn levels() {
        assert_eq!(!Level::Low, Level::High);
        assert_eq!(Level::from_bit(3), Level::High);
        assert_eq!(Level::from(false), Level::Low);
        assert_eq!(Level::High.fill_word(), u64::MAX);
    }
}
