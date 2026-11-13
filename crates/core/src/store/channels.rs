//! The store of one capture: per-channel chunk summaries, the open chunks being written, the cache
//! and the backing.
//!
//! Planned surface (built in the next step, shown here for review):
//!
//! - Writing (`&mut self`, one writer): `push_edges(slot, &[Sample])` for transition lists,
//!   `push_bits(slot, start, &[u64], count)` for per-channel bit blocks (counted first, then kept as
//!   bits, turned into offsets, or left quiet), `push_packed(start, unit_size, &[u8])` for packed
//!   samples (XOR with the predecessor, runs of equal words skipped), `advance_to(sample_count)` and
//!   `finish()`. A chunk is completed when every channel has passed its end: summary and coarse LOD
//!   built, fine LOD for dense chunks, payload into the cache as dirty.
//! - Reading (`&self`, many readers): `level_at`, `LevelCursor` (amortised O(1) for increasing
//!   positions), `edges_in(slot, range)`, `EdgeIter` (incremental), `EdgeMerger` producing
//!   `EdgeBatch`es for a decoder's channels, and `lod(slot, start, bucket_size, n)` returning exact
//!   `LodBucket`s. Readers see completed chunks and the open ones (the newest data while capturing).
//! - Eviction: a reader that loads a payload may push a dirty one out; it appends that blob to the
//!   backing (space reserved with an atomic append position) and records where in the summary's
//!   `OnceLock`, so no write lock is needed.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use super::cache::{CacheStats, ChunkCache};
use super::chunk::ChunkSummary;
use super::{Level, Sample, chunk_of};
use crate::io::RandomAccess;

/// One channel's chunks.
#[derive(Debug)]
pub struct ChannelChunks {
    /// Level at sample 0.
    pub initial: Level,
    /// Completed chunks, in order; chunk k covers samples [k * 2^24, (k + 1) * 2^24).
    pub chunks: Vec<ChunkSummary>,
}

/// The transitions of every channel of one capture.
pub struct CaptureStore {
    channels: Vec<ChannelChunks>,
    /// Samples readable on every channel.
    sample_count: Sample,
    cache: Mutex<ChunkCache>,
    backing: Arc<dyn RandomAccess>,
    /// Where the next blob goes in the backing.
    append_at: AtomicU64,
}

impl CaptureStore {
    /// An empty store for channels starting at the given levels. `append_at` is where blobs may be
    /// appended to `backing` (after the `.lacap` header of a working file).
    pub fn new(
        initial: &[Level],
        backing: Arc<dyn RandomAccess>,
        append_at: u64,
        cache_budget: usize,
    ) -> Self {
        CaptureStore {
            channels: initial
                .iter()
                .map(|&l| ChannelChunks {
                    initial: l,
                    chunks: Vec::new(),
                })
                .collect(),
            sample_count: 0,
            cache: Mutex::new(ChunkCache::new(cache_budget)),
            backing,
            append_at: AtomicU64::new(append_at),
        }
    }

    pub fn slots(&self) -> usize {
        self.channels.len()
    }

    pub fn sample_count(&self) -> Sample {
        self.sample_count
    }

    pub fn channel(&self, slot: usize) -> Option<&ChannelChunks> {
        self.channels.get(slot)
    }

    /// The summary of the chunk holding `sample` on `slot`, if that chunk is complete.
    pub fn summary_at(&self, slot: usize, sample: Sample) -> Option<&ChunkSummary> {
        let k = usize::try_from(chunk_of(sample)).ok()?;
        self.channels.get(slot)?.chunks.get(k)
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).stats()
    }

    pub fn backing(&self) -> &Arc<dyn RandomAccess> {
        &self.backing
    }

    pub fn append_position(&self) -> u64 {
        self.append_at.load(std::sync::atomic::Ordering::Acquire)
    }
}
