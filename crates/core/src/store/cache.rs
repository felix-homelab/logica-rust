//! The chunk cache: payloads and fine LODs, least recently used first out, within a byte budget
//! (doc 04, "Storage").
//!
//! The cache does no I/O. Inserting can push entries out; clean ones (already in the backing) are
//! dropped, dirty ones (not yet written) are handed back to the caller, which appends them to the
//! backing and records where they went.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::chunk::Payload;
use super::lod::FineLod;

/// Which part of a chunk an entry holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    Payload,
    FineLod,
}

/// Identifies one cached item: a channel slot of the store, a chunk index and the part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CacheKey {
    pub slot: u32,
    pub chunk: u64,
    pub part: Part,
}

/// A cached item. Readers keep the `Arc` while they use it.
#[derive(Clone, Debug)]
pub enum Cached {
    Payload(Arc<Payload>),
    FineLod(Arc<FineLod>),
}

impl Cached {
    pub fn heap_bytes(&self) -> usize {
        match self {
            Cached::Payload(p) => p.heap_bytes(),
            Cached::FineLod(f) => f.heap_bytes(),
        }
    }
}

#[derive(Debug)]
struct Entry {
    value: Cached,
    bytes: usize,
    tick: u64,
    dirty: bool,
}

/// Cache statistics, for tests, benchmarks and the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub budget: usize,
    pub used: usize,
    pub high_water: usize,
    pub entries: usize,
    pub evictions: u64,
}

#[derive(Debug)]
pub struct ChunkCache {
    budget: usize,
    used: usize,
    high_water: usize,
    evictions: u64,
    tick: u64,
    entries: HashMap<CacheKey, Entry>,
    /// Least recently used first.
    order: BTreeMap<u64, CacheKey>,
}

impl ChunkCache {
    pub fn new(budget: usize) -> Self {
        ChunkCache {
            budget,
            used: 0,
            high_water: 0,
            evictions: 0,
            tick: 0,
            entries: HashMap::new(),
            order: BTreeMap::new(),
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            budget: self.budget,
            used: self.used,
            high_water: self.high_water,
            entries: self.entries.len(),
            evictions: self.evictions,
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// The item for `key`, marked as most recently used.
    pub fn get(&mut self, key: &CacheKey) -> Option<Cached> {
        let tick = self.next_tick();
        let e = self.entries.get_mut(key)?;
        self.order.remove(&e.tick);
        e.tick = tick;
        self.order.insert(tick, *key);
        Some(e.value.clone())
    }

    /// Adds or replaces an item. `dirty` means it is not in the backing yet. Returns the dirty
    /// items pushed out to stay within the budget; the item just inserted is never pushed out,
    /// even if it alone is over the budget.
    pub fn insert(&mut self, key: CacheKey, value: Cached, dirty: bool) -> Vec<(CacheKey, Cached)> {
        self.remove(&key);
        let tick = self.next_tick();
        let bytes = value.heap_bytes();
        self.used += bytes;
        self.high_water = self.high_water.max(self.used);
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                tick,
                dirty,
            },
        );
        self.order.insert(tick, key);

        let mut out = Vec::new();
        while self.used > self.budget {
            let Some((&oldest, &victim)) = self.order.iter().next() else {
                break;
            };
            if victim == key {
                break;
            }
            self.order.remove(&oldest);
            if let Some(e) = self.entries.remove(&victim) {
                self.used -= e.bytes;
                self.evictions += 1;
                if e.dirty {
                    out.push((victim, e.value));
                }
            }
        }
        out
    }

    /// Removes an item, returning it with its dirty flag.
    pub fn remove(&mut self, key: &CacheKey) -> Option<(Cached, bool)> {
        let e = self.entries.remove(key)?;
        self.order.remove(&e.tick);
        self.used -= e.bytes;
        Some((e.value, e.dirty))
    }

    /// Keys of the items not yet in the backing, in key order (for saving).
    pub fn dirty_keys(&self) -> Vec<CacheKey> {
        let mut keys: Vec<CacheKey> = self
            .entries
            .iter()
            .filter(|(_, e)| e.dirty)
            .map(|(k, _)| *k)
            .collect();
        keys.sort();
        keys
    }

    /// Records that an item has been written to the backing.
    pub fn mark_clean(&mut self, key: &CacheKey) {
        if let Some(e) = self.entries.get_mut(key) {
            e.dirty = false;
        }
    }

    /// Changes the budget; returns the dirty items pushed out.
    pub fn set_budget(&mut self, budget: usize) -> Vec<(CacheKey, Cached)> {
        self.budget = budget;
        let mut out = Vec::new();
        while self.used > self.budget {
            let Some((&oldest, &victim)) = self.order.iter().next() else {
                break;
            };
            self.order.remove(&oldest);
            if let Some(e) = self.entries.remove(&victim) {
                self.used -= e.bytes;
                self.evictions += 1;
                if e.dirty {
                    out.push((victim, e.value));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(chunk: u64) -> CacheKey {
        CacheKey {
            slot: 0,
            chunk,
            part: Part::Payload,
        }
    }
    fn offsets(n: usize) -> Cached {
        Cached::Payload(Arc::new(Payload::Offsets(vec![0; n].into_boxed_slice())))
    }

    #[test]
    fn evicts_least_recently_used_and_returns_only_dirty_items() {
        let mut c = ChunkCache::new(40);
        assert!(c.insert(key(0), offsets(4), true).is_empty()); // 16 bytes
        assert!(c.insert(key(1), offsets(4), false).is_empty()); // 32
        c.get(&key(0)); // 1 is now the oldest
        let out = c.insert(key(2), offsets(4), true); // 48 > 40: 1 goes, clean
        assert!(out.is_empty());
        assert!(c.get(&key(1)).is_none());
        let out = c.insert(key(3), offsets(4), true); // 0 goes, dirty
        assert_eq!(out.iter().map(|(k, _)| k.chunk).collect::<Vec<_>>(), vec![0]);
        assert_eq!(c.stats().used, 32);
        assert_eq!(c.stats().high_water, 48);
        assert_eq!(c.stats().evictions, 2);
    }

    #[test]
    fn an_item_larger_than_the_budget_stays_until_the_next_insert() {
        let mut c = ChunkCache::new(8);
        assert!(c.insert(key(0), offsets(100), true).is_empty());
        assert!(c.get(&key(0)).is_some());
        let out = c.insert(key(1), offsets(1), false);
        assert_eq!(out.len(), 1);
        assert_eq!(c.stats().used, 4);
    }

    #[test]
    fn dirty_tracking_for_save() {
        let mut c = ChunkCache::new(1000);
        c.insert(key(2), offsets(1), true);
        c.insert(key(1), offsets(1), true);
        c.insert(key(0), offsets(1), false);
        assert_eq!(c.dirty_keys(), vec![key(1), key(2)]);
        c.mark_clean(&key(1));
        assert_eq!(c.dirty_keys(), vec![key(2)]);
        let out = c.set_budget(0);
        assert_eq!(out.len(), 1);
        assert_eq!(c.stats().entries, 0);
    }

    #[test]
    fn replacing_an_item_keeps_the_byte_count_right() {
        let mut c = ChunkCache::new(1000);
        c.insert(key(0), offsets(10), true);
        c.insert(key(0), offsets(2), false);
        assert_eq!(c.stats().used, 8);
        assert_eq!(c.remove(&key(0)).map(|(_, d)| d), Some(false));
        assert_eq!(c.stats().used, 0);
    }
}
