//! The chunked store against a plain reference model (an initial level plus a sorted edge list),
//! for every way of writing data, with and without eviction, and past sample 2^32.

#![allow(clippy::cast_possible_truncation)]

use std::sync::Arc;

use logica_core::io::{MemFile, RandomAccess};
use logica_core::store::chunk::ChunkKind;
use logica_core::store::edges::Edge;
use logica_core::store::{CaptureStore, MAX_OFFSETS, StoreError};
use logica_core::{Level, Sample};

mod common;
use common::*;

fn new_store(levels: &[Level], budget: usize) -> (CaptureStore, Arc<MemFile>) {
    let file = Arc::new(MemFile::new());
    let store = CaptureStore::new(levels, file.clone(), 16, budget);
    (store, file)
}

#[derive(Clone, Copy, Debug)]
enum Ingest {
    EdgesAtOnce,
    EdgesInPieces,
    Bits { piece: u64 },
}

/// Writes `models` (all starting at sample 0) into a store, `count` samples long.
fn build(models: &[Model], count: u64, how: Ingest, budget: usize) -> (CaptureStore, Arc<MemFile>) {
    let (mut s, f) = new_store(&models.iter().map(|m| m.initial).collect::<Vec<_>>(), budget);
    for (slot, m) in models.iter().enumerate() {
        match how {
            Ingest::EdgesAtOnce => s.push_edges(slot, &m.edges).unwrap(),
            Ingest::EdgesInPieces => {
                for piece in m.edges.chunks(997) {
                    s.push_edges(slot, piece).unwrap();
                }
            }
            Ingest::Bits { piece } => {
                let mut p = 0;
                while p < count {
                    let n = piece.min(count - p);
                    s.push_bits(slot, p, &m.bits(p, n), n).unwrap();
                    p += n;
                }
            }
        }
    }
    s.finish(count).unwrap();
    (s, f)
}

fn check(store: &CaptureStore, models: &[Model], count: u64, seed: u64) {
    assert_eq!(store.sample_count(), count);
    let mut rng = Rng(seed);
    for (slot, m) in models.iter().enumerate() {
        assert_eq!(store.initial_level(slot).unwrap(), m.initial);
        // every edge, and windows
        assert_eq!(store.edges_in(slot, 0, count).unwrap(), m.edges, "slot {slot} all edges");
        let mut points: Vec<Sample> = vec![0, count - 1];
        for k in 0..=count / C {
            for d in [0, 1, 2, 65_535, 65_536, 65_537] {
                points.push(k * C + d);
                points.push((k * C).saturating_sub(d));
            }
        }
        for _ in 0..300 {
            points.push(rng.below(count));
        }
        for &e in m.edges.iter().step_by(1 + m.edges.len() / 200) {
            points.extend([e.saturating_sub(1), e, e + 1]);
        }
        points.retain(|&p| p < count);
        for &p in &points {
            assert_eq!(store.level_at(slot, p).unwrap(), m.level_at(p), "slot {slot} level at {p}");
        }
        // a cursor, forwards then backwards
        let mut sorted = points.clone();
        sorted.sort_unstable();
        let mut cur = store.cursor(slot).unwrap();
        for &p in sorted.iter().chain(sorted.iter().rev()) {
            assert_eq!(cur.level_at(p).unwrap(), m.level_at(p), "slot {slot} cursor at {p}");
        }
        for _ in 0..100 {
            let a = rng.below(count);
            let b = (a + rng.below(3 * C)).min(count);
            assert_eq!(store.edges_in(slot, a, b).unwrap(), m.edges_in(a, b), "slot {slot} edges {a}..{b}");
            assert_eq!(
                store.transitions_in(slot, a, b).unwrap(),
                m.transitions_in(a, b),
                "slot {slot} count {a}..{b}"
            );
        }
        // LOD for aligned and unaligned bucket sizes
        for (start, bucket) in [(0, C), (0, 1 << 16), (0, 256), (7, 1 << 16), (0, 1_000_003), (C - 300, 777)]
        {
            let n = 300;
            let lod = store.lod(slot, start, bucket, n).unwrap();
            for (i, b) in lod.iter().enumerate() {
                let b0 = start + i as u64 * bucket;
                let b1 = (b0 + bucket).min(count);
                assert_eq!(b.transitions, m.transitions_in(b0, b1), "slot {slot} lod {b0}..{b1}");
                assert_eq!(b.start_level, m.level_at(b0), "slot {slot} lod level {b0}");
            }
        }
    }
}

fn reference_merge(models: &[Model], start: Sample, end: Sample) -> Vec<Edge> {
    let mut all: Vec<Edge> = Vec::new();
    for (i, m) in models.iter().enumerate() {
        for e in m.edges_in(start + 1, end) {
            all.push(Edge { sample: e, slot: i as u8, level: m.level_at(e) });
        }
    }
    all.sort_by_key(|e| (e.sample, e.slot));
    all
}

fn merged(store: &CaptureStore, slots: &[usize], start: Sample, end: Sample, max: usize) -> Vec<Edge> {
    let mut m = store.merger(slots, start, end).unwrap();
    let mut out = Vec::new();
    let mut last_end = start;
    while let Some(b) = m.next_batch(max, Sample::MAX).unwrap() {
        // more than `max` only when the extra edges share the last sample
        let over = b.edges.len().saturating_sub(max.max(1));
        assert!(over < slots.len());
        if over > 0 {
            let last = b.edges.last().unwrap().sample;
            assert!(b.edges[b.edges.len() - over - 1..].iter().all(|e| e.sample == last));
        }
        assert!(b.end >= last_end, "batch ends never go back");
        assert!(b.edges.iter().all(|e| e.sample < b.end), "edges are before `end`");
        last_end = b.end;
        out.extend(b.edges);
    }
    assert_eq!(last_end, end.min(store.sample_count()));
    out
}

#[test]
fn every_ingest_path_reads_back_like_the_reference() {
    let count = 3 * C + 12_345;
    let models = vec![
        mixed_model(1, 0, count, Level::Low),
        mixed_model(2, 0, count, Level::High),
        Model { initial: Level::High, edges: vec![] },
        Model { initial: Level::Low, edges: vec![1, C - 1, C, C + 1, 2 * C, count - 1] },
    ];
    for how in [
        Ingest::EdgesAtOnce,
        Ingest::EdgesInPieces,
        Ingest::Bits { piece: C },
        Ingest::Bits { piece: 1_000_003 },
        Ingest::Bits { piece: 64 * 4099 },
    ] {
        let (s, _) = build(&models, count, how, 1 << 30);
        check(&s, &models, count, 11);
        // representation follows density
        let kinds: Vec<ChunkKind> = s.chunks(0).unwrap().iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&ChunkKind::Bits), "{how:?}: dense part kept as bits: {kinds:?}");
        assert!(kinds.contains(&ChunkKind::Offsets), "{how:?}: {kinds:?}");
        assert!(
            s.chunks(2).unwrap().iter().all(|c| c.kind == ChunkKind::Quiet),
            "{how:?}: quiet bits stay quiet"
        );
    }
}

#[test]
fn eviction_to_the_backing_gives_the_same_answers() {
    let count = 3 * C + 999;
    let models = vec![mixed_model(5, 0, count, Level::Low), mixed_model(6, 0, count, Level::High)];
    // A budget of one byte: every payload and fine LOD goes to the backing at once.
    let (s, f) = build(&models, count, Ingest::EdgesAtOnce, 1);
    assert!(f.size().unwrap() > 1 << 20, "blobs were written");
    check(&s, &models, count, 12);
    let st = s.cache_stats();
    assert!(st.evictions > 0);
    // and with a budget of a few chunks, payloads are read back and evicted again
    s.set_cache_budget(5 << 20).unwrap();
    check(&s, &models, count, 13);
    assert!(s.cache_stats().high_water <= (5 << 20) + (2 << 20) + (136 << 10));
}

#[test]
fn a_damaged_blob_is_reported_by_chunk_and_channel() {
    let count = 2 * C;
    let models = vec![Model { initial: Level::Low, edges: vec![10, 20, C + 5] }];
    let (s, f) = build(&models, count, Ingest::EdgesAtOnce, 1);
    // The newest item stays in the cache even over budget; push everything out first.
    s.set_cache_budget(0).unwrap();
    let loc = *s.chunks(0).unwrap()[1].payload_at.get().unwrap();
    let mut bytes = f.to_bytes();
    bytes[(loc.offset + 30) as usize] ^= 0xff;
    f.write_all_at(0, &bytes).unwrap();
    assert_eq!(s.level_at(0, 15).unwrap(), Level::High, "chunk 0 is fine");
    let err = s.level_at(0, C + 6).unwrap_err();
    assert!(matches!(err, StoreError::Damaged { slot: 0, chunk: 1, .. }));
    assert_eq!(
        err.to_string(),
        "Chunk 1 of channel 0 is damaged (checksum mismatch); the capture cannot be read there."
    );
    let err = s.edges_in(0, 0, count).unwrap_err();
    assert!(err.to_string().contains("Chunk 1 of channel 0"));
}

#[test]
fn captures_longer_than_2_pow_32_samples() {
    let b = 1u64 << 32;
    let count = b + 2 * C + 77;
    let m = Model { initial: Level::Low, edges: vec![5, b - 1, b, b + 1, b + C - 1, b + C, count - 1] };
    let mut dense = mixed_model(9, b - C, 3 * C, Level::Low);
    dense.edges.retain(|&e| e < count);
    dense.edges.insert(0, 3);
    let models = vec![m, dense];
    let (s, _) = build(&models, count, Ingest::EdgesAtOnce, 1 << 30);
    assert_eq!(s.chunks(0).unwrap().len() as u64, count.div_ceil(C));
    check(&s, &models, count, 14);
    // the merged stream across 2^32
    let all = merged(&s, &[0, 1], b - 100, b + 100, 3);
    assert_eq!(all, reference_merge(&models, b - 100, b + 100));
    // bits pushed across 2^32
    let (mut s2, _) = new_store(&[Level::Low], 1 << 30);
    s2.advance_to(b - C / 2).unwrap();
    let m2 = Model { initial: Level::Low, edges: vec![b - 3, b, b + 64, b + C / 2] };
    let n = C + 100;
    s2.push_bits(0, b - C / 2, &m2.bits(b - C / 2, n), n).unwrap();
    s2.finish(b - C / 2 + n).unwrap();
    assert_eq!(s2.edges_in(0, 0, u64::MAX).unwrap(), m2.edges);
}

#[test]
fn batching_never_changes_the_merged_stream() {
    let count = 2 * C + 5;
    let models = vec![
        mixed_model(21, 0, count, Level::Low),
        Model { initial: Level::High, edges: vec![100, 200, C, C + 7] },
        // edges at the same samples as channel 1, to pin the order by slot
        Model { initial: Level::Low, edges: vec![100, 150, 200, C] },
    ];
    let (s, _) = build(&models, count, Ingest::EdgesAtOnce, 1 << 30);
    let start = 50;
    let want = reference_merge(&models, start, count);
    for max in [1, 3, 1000, usize::MAX] {
        assert_eq!(merged(&s, &[0, 1, 2], start, count, max), want, "batches of {max}");
    }
    // time-limited batches: `end` advances to `until` even without edges
    let mut m = s.merger(&[1, 2], 0, count).unwrap();
    assert_eq!(m.start_levels(), &[Level::High, Level::Low]);
    let b = m.next_batch(10, 100).unwrap().unwrap();
    assert_eq!((b.edges.len(), b.end), (0, 100));
    // A batch never splits the edges of one sample, even beyond `max_edges`.
    let b = m.next_batch(1, 150).unwrap().unwrap();
    assert_eq!(
        b.edges,
        vec![
            Edge { sample: 100, slot: 0, level: Level::Low },
            Edge { sample: 100, slot: 1, level: Level::High }
        ]
    );
    assert_eq!(b.end, 150, "until");
    let rest = m.rest().unwrap();
    assert_eq!(rest.edges.first().map(|e| (e.sample, e.slot)), Some((150, 1)));
    assert_eq!(rest.end, count);
    assert!(m.next_batch(10, u64::MAX).unwrap().is_none());
}

#[test]
fn representation_switches_at_the_documented_thresholds() {
    let (mut s, _) = new_store(&[Level::Low, Level::Low, Level::Low], 1 << 30);
    let at_max: Vec<Sample> = (1..=u64::from(MAX_OFFSETS)).map(|i| i * 2).collect();
    let above: Vec<Sample> = (1..=u64::from(MAX_OFFSETS) + 1).map(|i| i * 2).collect();
    s.push_edges(0, &at_max).unwrap();
    s.push_edges(1, &above).unwrap();
    // channel 2: a whole coarse bucket toggling on every sample, at the start of chunk 1
    s.push_edges(2, &(C..C + 65_536).collect::<Vec<_>>()).unwrap();
    s.finish(2 * C).unwrap();
    assert_eq!(s.chunks(0).unwrap()[0].kind, ChunkKind::Offsets);
    assert_eq!(s.chunks(1).unwrap()[0].kind, ChunkKind::Bits);
    let c1 = &s.chunks(2).unwrap()[1];
    assert!(c1.has_fine);
    assert!(c1.coarse.as_ref().unwrap().saturated(0));
    assert_eq!(s.transitions_in(2, C, C + 65_536).unwrap(), 65_536, "exact despite saturation");
    assert_eq!(s.lod(2, C, 1 << 16, 2).unwrap()[0].transitions, 65_536);
    assert_eq!(s.lod(2, 0, C, 2).unwrap()[1].transitions, 65_536);
}

#[test]
fn packed_samples_split_into_channels() {
    let count = C + 5000;
    let models = vec![
        mixed_model(31, 0, count, Level::Low),
        Model { initial: Level::High, edges: vec![1, 2, 3, C, C + 1] },
        Model { initial: Level::Low, edges: vec![] },
    ];
    // channel slot i lives in bit [3, 0, 9][i] of 2-byte samples; bit 5 is noise to be ignored
    let bit_of_slot = [3u32, 0, 9];
    let mut data = Vec::with_capacity(count as usize * 2);
    let mut levels: Vec<Level> = models.iter().map(|m| m.initial).collect();
    let mut idx = vec![0usize; models.len()];
    for s in 0..count {
        let mut w = 0u16;
        for (i, m) in models.iter().enumerate() {
            while idx[i] < m.edges.len() && m.edges[idx[i]] <= s {
                levels[i] = !levels[i];
                idx[i] += 1;
            }
            w |= u16::from(levels[i].is_high()) << bit_of_slot[i];
        }
        w |= ((s / 3) as u16 & 1) << 5;
        data.extend_from_slice(&w.to_le_bytes());
    }
    let (mut s, _) = new_store(&[Level::Low; 3], 1 << 30);
    let piece = 2 * 777_777;
    let mut at = 0u64;
    for part in data.chunks(piece) {
        s.push_packed(at, 2, part, &bit_of_slot).unwrap();
        at += (part.len() / 2) as u64;
        assert_eq!(s.sample_count(), at);
    }
    s.finish(count).unwrap();
    check(&s, &models, count, 15);
}

#[test]
fn reading_while_writing_sees_the_open_chunk() {
    let (mut s, _) = new_store(&[Level::Low], 1 << 30);
    s.push_edges(0, &[10, 20]).unwrap();
    assert_eq!(s.sample_count(), 21);
    assert_eq!(s.level_at(0, 15).unwrap(), Level::High);
    assert_eq!(s.edges_in(0, 0, 100).unwrap(), vec![10, 20]);
    s.advance_to(C + 10).unwrap();
    assert_eq!(s.chunks(0).unwrap().len(), 1);
    assert_eq!(s.level_at(0, C + 9).unwrap(), Level::Low);
    assert_eq!(s.lod(0, 0, 16, 2).unwrap()[0].transitions, 1);
}

#[test]
fn mistakes_get_clear_errors() {
    let (mut s, _) = new_store(&[Level::Low, Level::Low], 1 << 20);
    s.push_edges(0, &[5, 9]).unwrap();
    let e = s.push_edges(0, &[7]).unwrap_err();
    assert_eq!(e.to_string(), "Data for channel 0 must continue at sample 10, but it starts at sample 7.");
    let e = s.push_edges(2, &[1]).unwrap_err();
    assert_eq!(e.to_string(), "There is no channel 2; this capture has 2 channel(s).");
    let e = s.push_bits(1, 3, &[0], 10).unwrap_err();
    assert!(matches!(e, StoreError::Order { slot: 1, expected: 0, got: 3 }));
    assert!(s.push_bits(1, 0, &[0], 65).unwrap_err().to_string().contains("only 64 were given"));
    assert!(s.push_packed(0, 0, &[], &[0, 1]).is_err());
    assert!(s.push_packed(10, 1, &[1, 2, 3], &[0, 1]).is_err(), "channel 1 is not at sample 10");
    assert!(s.push_packed(0, 1, &[1], &[0, 8]).unwrap_err().to_string().contains("Bit 8"));
    assert_eq!(s.sample_count(), 0, "channel 1 has nothing yet");
    let e = s.level_at(0, 0).unwrap_err();
    assert_eq!(e.to_string(), "Sample 0 is past the end of the capture, which has 0 sample(s) so far.");
    let e = s.finish(8).unwrap_err();
    assert!(e.to_string().contains("past the end at sample 8"));
    s.finish(100).unwrap();
    assert!(matches!(s.push_edges(0, &[200]), Err(StoreError::Finished)));
    assert!(s.lod(0, 0, 0, 1).is_err());
    assert_eq!(s.lod(0, 90, 7, 10).unwrap().len(), 2, "stops at the end, last bucket short");
}

#[test]
fn data_at_sample_0_defines_the_initial_level() {
    let (mut s, _) = new_store(&[Level::Low, Level::Low, Level::Low], 1 << 20);
    s.push_edges(0, &[0, 4]).unwrap();
    s.push_bits(1, 0, &[0b1110], 8).unwrap();
    s.push_edges(2, &[]).unwrap();
    s.finish(8).unwrap();
    assert_eq!(s.initial_level(0).unwrap(), Level::High);
    assert_eq!(s.edges_in(0, 0, 8).unwrap(), vec![4]);
    assert_eq!(s.initial_level(1).unwrap(), Level::Low);
    assert_eq!(s.edges_in(1, 0, 8).unwrap(), vec![1, 4]);
    assert_eq!(s.edges_in(2, 0, 8).unwrap(), Vec::<u64>::new());
}

#[test]
fn an_empty_capture_is_readable_as_empty() {
    let (mut s, _) = new_store(&[Level::High], 1 << 20);
    s.finish(0).unwrap();
    assert_eq!(s.sample_count(), 0);
    assert_eq!(s.edges_in(0, 0, 10).unwrap(), Vec::<u64>::new());
    assert!(s.lod(0, 0, 10, 5).unwrap().is_empty());
    assert!(s.merger(&[0], 0, 10).is_err());
}
