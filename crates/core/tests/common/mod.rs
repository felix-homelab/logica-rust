//! Shared test helpers: a reference model of a channel and signal generators.
#![allow(dead_code, clippy::cast_possible_truncation)]

use logica_core::store::{CHUNK_SAMPLES, CaptureStore};
use logica_core::{Level, Sample};

pub const C: u64 = CHUNK_SAMPLES;

/// The reference: what a channel is, in the simplest form.
#[derive(Clone, Debug)]
pub struct Model {
    pub initial: Level,
    pub edges: Vec<Sample>,
}

impl Model {
    pub fn level_at(&self, s: Sample) -> Level {
        let n = self.edges.partition_point(|&e| e <= s);
        if n % 2 == 1 { !self.initial } else { self.initial }
    }
    pub fn edges_in(&self, a: Sample, b: Sample) -> Vec<Sample> {
        self.edges.iter().copied().filter(|&e| e >= a && e < b).collect()
    }
    pub fn transitions_in(&self, a: Sample, b: Sample) -> u64 {
        (self.edges.partition_point(|&e| e < b) - self.edges.partition_point(|&e| e < a)) as u64
    }
    /// Bits of samples [from, from + n), sample k in bit k % 64 of word k / 64.
    pub fn bits(&self, from: Sample, n: u64) -> Vec<u64> {
        let mut out = vec![0u64; n.div_ceil(64) as usize];
        let mut level = self.level_at(from);
        let mut i = self.edges.partition_point(|&e| e <= from);
        let mut p = from;
        while p < from + n {
            let next = self.edges.get(i).copied().unwrap_or(u64::MAX).min(from + n);
            if level.is_high() {
                for k in p - from..next - from {
                    out[(k / 64) as usize] |= 1 << (k % 64);
                }
            }
            p = next;
            if p < from + n {
                level = !level;
                i += 1;
            }
        }
        out
    }
}

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Edges in `[from, to)` with random gaps averaging `mean`.
pub fn random_edges(rng: &mut Rng, from: Sample, to: Sample, mean: u64, out: &mut Vec<Sample>) {
    let mut p = from;
    loop {
        p += 1 + rng.below(2 * mean);
        if p >= to {
            break;
        }
        out.push(p);
    }
}

/// A mixed signal over `count` samples starting at `base`: sparse, quiet, dense (bits), an edge on
/// chunk starts and right before them, and a burst toggling on every sample.
pub fn mixed_model(seed: u64, base: Sample, count: u64, initial: Level) -> Model {
    let mut rng = Rng(seed);
    let mut e = Vec::new();
    let end = base + count;
    let q = count / 8;
    random_edges(&mut rng, base, base + q, 5000, &mut e); // sparse
    // quiet from q to 2q
    random_edges(&mut rng, base + 2 * q, base + 3 * q, 5, &mut e); // dense: well over 2^19 per chunk
    for k in (base / C + 1)..=(end / C) {
        let b = k * C;
        if b > base + 3 * q && b + 2 < end {
            e.extend([b - 1, b]);
        }
    }
    let burst = base + 5 * q;
    e.extend(burst..burst + 70_000); // saturates a coarse bucket
    random_edges(&mut rng, base + 6 * q, end, 300, &mut e);
    e.retain(|&x| x > base && x < end);
    e.sort_unstable();
    e.dedup();
    Model { initial, edges: e }
}

/// Asserts that two stores hold the same signal: every edge of every channel, and LODs.
pub fn assert_same(a: &CaptureStore, b: &CaptureStore) {
    assert_eq!(a.slots(), b.slots());
    assert_eq!(a.sample_count(), b.sample_count());
    let n = a.sample_count();
    for slot in 0..a.slots() {
        assert_eq!(a.initial_level(slot).unwrap(), b.initial_level(slot).unwrap(), "slot {slot} initial");
        assert_eq!(a.edges_in(slot, 0, n).unwrap(), b.edges_in(slot, 0, n).unwrap(), "slot {slot} edges");
        for bucket in [1u64 << 16, 1 << 8, 999_999] {
            assert_eq!(
                a.lod(slot, 0, bucket, 200).unwrap(),
                b.lod(slot, 0, bucket, 200).unwrap(),
                "slot {slot} lod {bucket}"
            );
        }
    }
}
