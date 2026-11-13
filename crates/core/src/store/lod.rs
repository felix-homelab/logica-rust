//! Level of detail per chunk (doc 04, "Level of detail").
//!
//! Per bucket: the transition count and the level at the bucket start. Coarse buckets (2^16
//! samples) are resident for every non-quiet chunk; fine buckets (2^8 samples) exist only for
//! dense chunks and live in the cache. Counts saturate at `u16::MAX`; only a coarse bucket can
//! reach that (it can hold 65,536 transitions), and it is then answered from the fine LOD.

use super::{COARSE_BUCKETS, FINE_BUCKETS, Level};

/// Coarse LOD of one chunk: 256 buckets of 2^16 samples. 544 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoarseLod {
    pub counts: [u16; COARSE_BUCKETS],
    /// Bit k of byte k / 8: the level at the start of bucket k.
    pub start_levels: [u8; COARSE_BUCKETS / 8],
}

/// Fine LOD of one dense chunk: 65,536 buckets of 2^8 samples. 136 KiB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FineLod {
    pub counts: Box<[u16]>,
    pub start_levels: Box<[u8]>,
}

impl CoarseLod {
    pub fn start_level(&self, bucket: usize) -> Level {
        packed_level(&self.start_levels, bucket)
    }

    /// Whether the count of `bucket` may be larger than it says.
    pub fn saturated(&self, bucket: usize) -> bool {
        self.counts[bucket] == u16::MAX
    }
}

impl FineLod {
    pub fn new() -> Self {
        FineLod {
            counts: vec![0; FINE_BUCKETS].into_boxed_slice(),
            start_levels: vec![0; FINE_BUCKETS / 8].into_boxed_slice(),
        }
    }

    pub fn start_level(&self, bucket: usize) -> Level {
        packed_level(&self.start_levels, bucket)
    }

    /// Bytes in memory, for the cache budget.
    pub fn heap_bytes(&self) -> usize {
        self.counts.len() * 2 + self.start_levels.len()
    }
}

impl Default for FineLod {
    fn default() -> Self {
        Self::new()
    }
}

fn packed_level(bits: &[u8], k: usize) -> Level {
    Level::from_bit(u64::from(bits[k / 8] >> (k % 8)))
}

/// One bucket of a LOD query of any bucket size, exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LodBucket {
    /// Transitions inside the bucket (an edge on its first sample included).
    pub transitions: u64,
    /// Level of the bucket's first sample.
    pub start_level: Level,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_levels_are_lsb_first() {
        let mut c = CoarseLod { counts: [0; COARSE_BUCKETS], start_levels: [0; COARSE_BUCKETS / 8] };
        c.start_levels[1] = 0b0000_0100;
        assert_eq!(c.start_level(10), Level::High);
        assert_eq!(c.start_level(9), Level::Low);
        c.counts[3] = u16::MAX;
        assert!(c.saturated(3));
        assert_eq!(FineLod::new().heap_bytes(), 136 * 1024);
    }
}
