//! The store of one capture: per-channel chunks, the cache and the backing.
//!
//! Writing takes `&mut self` (one writer); reading takes `&self` (any number of readers). Readers
//! see every sample below [`CaptureStore::sample_count`], including the chunk being written, so a
//! live view is never a whole chunk behind. The engine (M1) wraps the store for live capture.
//!
//! Eviction: inserting into the cache can push out items not yet in the backing; whoever inserted
//! (the writer completing a chunk, or a reader loading one) appends them to the backing while
//! holding the cache lock, and records where in the summary's `OnceLock`.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::bits::idx;
use super::blob::{self, BLOB_HEADER};
use super::cache::{CacheKey, CacheStats, Cached, ChunkCache, Part};
use super::chunk::{BlobLoc, ChunkSummary, Payload};
use super::error::{StoreError, StoreResult};
use super::lod::FineLod;
use super::write::{ChannelState, Completed, check_slot};
use super::{Level, Sample};
use crate::io::RandomAccess;

/// The transitions of every channel of one capture.
pub struct CaptureStore {
    pub(crate) channels: Vec<ChannelState>,
    /// Samples readable on every channel.
    sample_count: Sample,
    finished: bool,
    cache: Mutex<ChunkCache>,
    backing: Arc<dyn RandomAccess>,
    /// Where the next blob goes in the backing.
    append_at: AtomicU64,
}

impl std::fmt::Debug for CaptureStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureStore")
            .field("channels", &self.channels.len())
            .field("sample_count", &self.sample_count)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

fn io_err(context: impl Into<String>) -> impl FnOnce(io::Error) -> StoreError {
    let context = context.into();
    move |source| StoreError::Io { context, source }
}

impl CaptureStore {
    /// An empty store with one channel per entry of `initial` (its level at sample 0, until data
    /// at sample 0 says otherwise). Blobs are appended to `backing` from `append_at` on (after the
    /// `.lacap` header of a working file).
    pub fn new(
        initial: &[Level],
        backing: Arc<dyn RandomAccess>,
        append_at: u64,
        cache_budget: usize,
    ) -> Self {
        CaptureStore {
            channels: initial.iter().map(|&l| ChannelState::new(l)).collect(),
            sample_count: 0,
            finished: false,
            cache: Mutex::new(ChunkCache::new(cache_budget)),
            backing,
            append_at: AtomicU64::new(append_at),
        }
    }

    /// A finished store from saved summaries (opening a `.lacap`): every chunk is in `backing`.
    pub(crate) fn from_parts(
        channels: Vec<(Level, Vec<ChunkSummary>)>,
        sample_count: Sample,
        backing: Arc<dyn RandomAccess>,
        append_at: u64,
        cache_budget: usize,
    ) -> Self {
        let channels = channels
            .into_iter()
            .map(|(initial, chunks)| {
                let level = chunks.last().map_or(initial, |c| c.exit);
                ChannelState {
                    initial,
                    chunks,
                    open: None,
                    level,
                    written_to: sample_count,
                }
            })
            .collect();
        CaptureStore {
            channels,
            sample_count,
            finished: true,
            cache: Mutex::new(ChunkCache::new(cache_budget)),
            backing,
            append_at: AtomicU64::new(append_at),
        }
    }

    // ------------------------------------------------------------------ accessors

    pub fn slots(&self) -> usize {
        self.channels.len()
    }

    pub fn sample_count(&self) -> Sample {
        self.sample_count
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Level of channel `slot` at sample 0.
    pub fn initial_level(&self, slot: usize) -> StoreResult<Level> {
        check_slot(slot, self.channels.len())?;
        Ok(self.channels[slot].initial)
    }

    /// Completed chunks of `slot` (all of them once the store is finished).
    pub fn chunks(&self, slot: usize) -> StoreResult<&[ChunkSummary]> {
        check_slot(slot, self.channels.len())?;
        Ok(&self.channels[slot].chunks)
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.lock().stats()
    }

    pub fn backing(&self) -> &Arc<dyn RandomAccess> {
        &self.backing
    }

    /// Where the next blob will be appended to the backing.
    pub fn append_position(&self) -> u64 {
        self.append_at.load(Ordering::Acquire)
    }

    /// Changes the cache budget; items pushed out are written to the backing first.
    pub fn set_cache_budget(&self, budget: usize) -> StoreResult<()> {
        let mut cache = self.lock();
        let out = cache.set_budget(budget);
        self.write_evicted(out)
    }

    // ------------------------------------------------------------------ writing

    fn check_writable(&self) -> StoreResult<()> {
        if self.finished {
            Err(StoreError::Finished)
        } else {
            Ok(())
        }
    }

    /// Appends level changes of `slot`: ascending samples, each at or after the end of what was
    /// written before. An edge at sample 0 sets the initial level instead.
    pub fn push_edges(&mut self, slot: usize, edges: &[Sample]) -> StoreResult<()> {
        self.check_writable()?;
        check_slot(slot, self.channels.len())?;
        let mut done = Vec::new();
        let ch = &mut self.channels[slot];
        let mut result = Ok(());
        for &e in edges {
            if e < ch.written_to {
                result = Err(StoreError::Order {
                    slot,
                    expected: ch.written_to,
                    got: e,
                });
                break;
            }
            ch.toggle_at(e, &mut done);
        }
        self.after_write(slot, done)?;
        result
    }

    /// Appends `count` samples of `slot`, given as bits of `words` (sample k in bit k % 64 of word
    /// k / 64), starting at sample `start`, which must be where the channel's data ends.
    pub fn push_bits(&mut self, slot: usize, start: Sample, words: &[u64], count: u64) -> StoreResult<()> {
        self.check_writable()?;
        check_slot(slot, self.channels.len())?;
        if count > words.len() as u64 * 64 {
            return Err(StoreError::Invalid(format!(
                "{count} samples were announced but only {} were given.",
                words.len() as u64 * 64
            )));
        }
        let ch = &mut self.channels[slot];
        if start != ch.written_to {
            return Err(StoreError::Order {
                slot,
                expected: ch.written_to,
                got: start,
            });
        }
        let mut done = Vec::new();
        ch.push_bits(words, 0, count, &mut done);
        self.after_write(slot, done)
    }

    /// Appends packed samples: `unit_size` bytes per sample, little-endian, one bit per channel;
    /// channel `slot` is bit `bit_of_slot[slot]`. Every channel's data must end at `start`.
    pub fn push_packed(
        &mut self,
        start: Sample,
        unit_size: usize,
        data: &[u8],
        bit_of_slot: &[u32],
    ) -> StoreResult<()> {
        self.check_writable()?;
        if !(1..=8).contains(&unit_size) || !data.len().is_multiple_of(unit_size) {
            return Err(StoreError::Invalid(format!(
                "Packed samples need 1 to 8 bytes per sample and whole samples; got {} bytes with {unit_size} per sample.",
                data.len()
            )));
        }
        if bit_of_slot.len() != self.channels.len() {
            return Err(StoreError::Invalid(
                "Every channel needs a bit in the packed samples.".into(),
            ));
        }
        let unit_bits = u32::try_from(unit_size * 8).unwrap_or(u32::MAX);
        if let Some(&b) = bit_of_slot.iter().find(|&&b| b >= unit_bits) {
            return Err(StoreError::Invalid(format!(
                "Bit {b} does not exist in samples of {unit_size} byte(s)."
            )));
        }
        for (slot, ch) in self.channels.iter().enumerate() {
            if ch.written_to != start {
                return Err(StoreError::Order {
                    slot,
                    expected: ch.written_to,
                    got: start,
                });
            }
        }
        let n = (data.len() / unit_size) as u64;
        if n == 0 {
            return Ok(());
        }
        let read = |i: usize| -> u64 {
            let mut b = [0u8; 8];
            b[..unit_size].copy_from_slice(&data[i * unit_size..(i + 1) * unit_size]);
            u64::from_le_bytes(b)
        };
        let mask = bit_of_slot.iter().fold(0u64, |m, &b| m | (1 << b));
        let mut done: Vec<(usize, Completed)> = Vec::new();
        let mut local = Vec::new();
        let mut prev = 0u64;
        let first = if start == 0 {
            // The first sample defines the initial levels.
            let w = read(0) & mask;
            for (ch, &b) in self.channels.iter_mut().zip(bit_of_slot) {
                ch.initial = Level::from_bit(w >> b);
                ch.level = ch.initial;
            }
            prev = w;
            1
        } else {
            for (ch, &b) in self.channels.iter().zip(bit_of_slot) {
                prev |= (ch.level as u64) << b;
            }
            0
        };
        let samples = data.chunks_exact(unit_size);
        for (i, chunk) in (first..).zip(samples.skip(first)) {
            let mut b = [0u8; 8];
            b[..unit_size].copy_from_slice(chunk);
            let w = u64::from_le_bytes(b) & mask;
            if w != prev {
                let diff = w ^ prev;
                for (slot, &bit) in bit_of_slot.iter().enumerate() {
                    if (diff >> bit) & 1 == 1 {
                        self.channels[slot].toggle_at(start + i as u64, &mut local);
                        done.extend(local.drain(..).map(|c| (slot, c)));
                    }
                }
                prev = w;
            }
        }
        for (slot, ch) in self.channels.iter_mut().enumerate() {
            ch.fill_to(start + n, &mut local);
            done.extend(local.drain(..).map(|c| (slot, c)));
        }
        self.store_completed(done)?;
        self.update_sample_count();
        Ok(())
    }

    /// Declares that `slot` keeps its level up to `end` (for example a block a file leaves out).
    pub fn extend(&mut self, slot: usize, end: Sample) -> StoreResult<()> {
        self.check_writable()?;
        check_slot(slot, self.channels.len())?;
        let mut done = Vec::new();
        self.channels[slot].fill_to(end, &mut done);
        self.after_write(slot, done)
    }

    /// Where the data of `slot` ends so far.
    pub fn written_to(&self, slot: usize) -> StoreResult<Sample> {
        check_slot(slot, self.channels.len())?;
        Ok(self.channels[slot].written_to)
    }

    /// Declares that every channel is known up to `end`: channels without data there keep their
    /// level.
    pub fn advance_to(&mut self, end: Sample) -> StoreResult<()> {
        self.check_writable()?;
        let mut done = Vec::new();
        let mut local = Vec::new();
        for (slot, ch) in self.channels.iter_mut().enumerate() {
            ch.fill_to(end, &mut local);
            done.extend(local.drain(..).map(|c| (slot, c)));
        }
        self.store_completed(done)?;
        self.update_sample_count();
        Ok(())
    }

    /// Ends the capture at `end` samples: the last chunk is completed and nothing more can be
    /// added. Data already written past `end` is an error.
    pub fn finish(&mut self, end: Sample) -> StoreResult<()> {
        self.check_writable()?;
        for (slot, ch) in self.channels.iter().enumerate() {
            if ch.written_to > end {
                return Err(StoreError::Invalid(format!(
                    "Channel {slot} has data up to sample {}, past the end at sample {end}.",
                    ch.written_to
                )));
            }
        }
        let mut done = Vec::new();
        let mut local = Vec::new();
        for (slot, ch) in self.channels.iter_mut().enumerate() {
            ch.finish(end, &mut local);
            done.extend(local.drain(..).map(|c| (slot, c)));
        }
        self.store_completed(done)?;
        self.sample_count = end;
        self.finished = true;
        Ok(())
    }

    fn after_write(&mut self, slot: usize, done: Vec<Completed>) -> StoreResult<()> {
        self.store_completed(done.into_iter().map(|c| (slot, c)).collect())?;
        self.update_sample_count();
        Ok(())
    }

    fn update_sample_count(&mut self) {
        self.sample_count = self.channels.iter().map(|c| c.written_to).min().unwrap_or(0);
    }

    /// Puts completed payloads and fine LODs into the cache as not yet written.
    fn store_completed(&self, done: Vec<(usize, Completed)>) -> StoreResult<()> {
        let mut cache = self.lock();
        for (slot, c) in done {
            let slot32 = u32::try_from(slot).unwrap_or(u32::MAX);
            if let Some(p) = c.payload {
                let key = CacheKey {
                    slot: slot32,
                    chunk: c.chunk,
                    part: Part::Payload,
                };
                let out = cache.insert(key, Cached::Payload(Arc::new(p)), true);
                self.write_evicted(out)?;
            }
            if let Some(f) = c.fine {
                let key = CacheKey {
                    slot: slot32,
                    chunk: c.chunk,
                    part: Part::FineLod,
                };
                let out = cache.insert(key, Cached::FineLod(Arc::new(f)), true);
                self.write_evicted(out)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ cache and backing

    pub(crate) fn lock(&self) -> MutexGuard<'_, ChunkCache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn summary(&self, slot: usize, chunk: u64) -> &ChunkSummary {
        &self.channels[slot].chunks[idx(chunk)]
    }

    fn write_evicted(&self, items: Vec<(CacheKey, Cached)>) -> StoreResult<()> {
        for (key, item) in items {
            self.write_item(key, &item)?;
        }
        Ok(())
    }

    /// Appends one cached item to the backing and records where it went.
    fn write_item(&self, key: CacheKey, item: &Cached) -> StoreResult<BlobLoc> {
        let slot = key.slot as usize;
        let s = self.summary(slot, key.chunk);
        let (blob, cell) = match item {
            Cached::Payload(p) => (
                blob::encode_chunk(key.slot, key.chunk, s.entry, s.transitions, p),
                &s.payload_at,
            ),
            Cached::FineLod(f) => (blob::encode_fine(key.slot, key.chunk, f), &s.fine_at),
        };
        let len = blob.len() as u64;
        let at = self.append_at.fetch_add(len, Ordering::AcqRel);
        self.backing.write_all_at(at, &blob).map_err(io_err(format!(
            "Could not write chunk {} of channel {slot} to the capture file",
            key.chunk
        )))?;
        let loc = BlobLoc { offset: at, len };
        let _ = cell.set(loc);
        Ok(loc)
    }

    /// Appends raw bytes (a `.lacap` blob) to the backing.
    pub(crate) fn append_raw(&self, bytes: &[u8]) -> StoreResult<BlobLoc> {
        let len = bytes.len() as u64;
        let at = self.append_at.fetch_add(len, Ordering::AcqRel);
        self.backing
            .write_all_at(at, bytes)
            .map_err(io_err("Could not write to the capture file"))?;
        Ok(BlobLoc { offset: at, len })
    }

    /// Moves the append position (after an index and footer were written behind the blobs).
    pub(crate) fn set_append_position(&self, at: u64) {
        self.append_at.store(at, Ordering::Release);
    }

    /// Writes every cached item that is not in the backing yet (before saving).
    pub fn flush(&self) -> StoreResult<()> {
        let mut cache = self.lock();
        for key in cache.dirty_keys() {
            if let Some(item) = cache.get(&key) {
                self.write_item(key, &item)?;
                cache.mark_clean(&key);
            }
        }
        Ok(())
    }

    fn read_blob(&self, slot: usize, chunk: u64, loc: BlobLoc) -> StoreResult<Vec<u8>> {
        if loc.len < (BLOB_HEADER + blob::BLOB_TRAILER) as u64 || loc.len > (64 << 20) {
            return Err(StoreError::Damaged {
                slot,
                chunk,
                reason: "the blob length is impossible".into(),
            });
        }
        let mut buf = vec![0u8; idx(loc.len)];
        self.backing.read_exact_at(loc.offset, &mut buf).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                StoreError::Damaged {
                    slot,
                    chunk,
                    reason: "the file ends inside it".into(),
                }
            } else {
                StoreError::Io {
                    context: format!("Could not read chunk {chunk} of channel {slot}"),
                    source: e,
                }
            }
        })?;
        Ok(buf)
    }

    /// The payload of a completed, non-quiet chunk, from the cache or the backing.
    pub(crate) fn payload(&self, slot: usize, chunk: u64) -> StoreResult<Arc<Payload>> {
        let key = CacheKey {
            slot: u32::try_from(slot).unwrap_or(u32::MAX),
            chunk,
            part: Part::Payload,
        };
        let mut cache = self.lock();
        if let Some(Cached::Payload(p)) = cache.get(&key) {
            return Ok(p);
        }
        let s = self.summary(slot, chunk);
        let damaged = |reason: String| StoreError::Damaged { slot, chunk, reason };
        let loc = *s
            .payload_at
            .get()
            .ok_or_else(|| damaged("it is neither in memory nor in the file".into()))?;
        let bytes = self.read_blob(slot, chunk, loc)?;
        let (p, entry, transitions) = blob::decode_chunk(&bytes, key.slot, chunk).map_err(damaged)?;
        if entry != s.entry || transitions != s.transitions || p.kind() != s.kind {
            return Err(damaged("it does not match the channel table".into()));
        }
        let p = Arc::new(p);
        let out = cache.insert(key, Cached::Payload(p.clone()), false);
        self.write_evicted(out)?;
        Ok(p)
    }

    /// The fine LOD of a dense completed chunk, from the cache or the backing.
    pub(crate) fn fine_lod(&self, slot: usize, chunk: u64) -> StoreResult<Arc<FineLod>> {
        let key = CacheKey {
            slot: u32::try_from(slot).unwrap_or(u32::MAX),
            chunk,
            part: Part::FineLod,
        };
        let mut cache = self.lock();
        if let Some(Cached::FineLod(f)) = cache.get(&key) {
            return Ok(f);
        }
        let s = self.summary(slot, chunk);
        let damaged = |reason: String| StoreError::Damaged { slot, chunk, reason };
        let loc = *s
            .fine_at
            .get()
            .ok_or_else(|| damaged("its fine LOD is neither in memory nor in the file".into()))?;
        let bytes = self.read_blob(slot, chunk, loc)?;
        let f = Arc::new(blob::decode_fine(&bytes, key.slot, chunk).map_err(damaged)?);
        let out = cache.insert(key, Cached::FineLod(f.clone()), false);
        self.write_evicted(out)?;
        Ok(f)
    }
}
