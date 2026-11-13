//! The M0 scale exit criterion (doc 10): a synthetic capture of 5e9 samples on 8 channels is
//! created into a working `.lacap` on disk, saved, reopened and queried (level at a sample, edges
//! in a window, LOD lookup), with the whole process's peak resident memory under 1 GB. This test
//! has a binary of its own so no other test shares the process.

use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use logica_core::capture::{Source, SourceKind};
use logica_core::device::{CaptureConfig, CaptureMode, StopSignal, StoreSink, VirtualDriver};
use logica_core::import::StoreTarget;
use logica_core::io::RandomAccess;
use logica_core::lacap;
use logica_core::store::CHUNK_SAMPLES;
use logica_core::{Level, Sample};
use logica_drivers::synthetic::{DENSE_LEN, DENSE_START, MIX_TILE, SyntheticDriver, programs};
use logica_fileio::StdFile;

const SAMPLES: Sample = 5_000_000_000;
/// Small enough that chunks are evicted to the file while capturing (the data is about 14 MB).
const BUDGET: usize = 8 << 20;
const MEMORY_LIMIT: u64 = 1 << 30;

fn say(msg: &str) {
    let _ = writeln!(std::io::stderr(), "{msg}");
}

/// Peak resident memory of this process (Linux).
fn peak_rss() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// Edges of program channel `c` in `[a, b)`, from the program itself.
fn expected(c: usize, a: Sample, b: Sample) -> Vec<Sample> {
    let p = programs().into_iter().find(|p| p.name == "mix-dense").unwrap();
    let lane = &p.tile.channels[c].2;
    let mut out = Vec::new();
    let first = a / MIX_TILE;
    for k in first..=b / MIX_TILE {
        let base = k * MIX_TILE;
        out.extend(lane.edges.iter().map(|&e| base + e).filter(|&e| e >= a && e < b && e > 0));
        if c == 7 {
            let from = DENSE_START.max(base).max(a);
            let to = (DENSE_START + DENSE_LEN).min(base + MIX_TILE).min(b);
            let mut s = from + (from % 2);
            while s < to {
                out.push(s);
                s += 2;
            }
        }
    }
    out.sort_unstable();
    out
}

#[test]
fn five_billion_samples_on_8_channels_within_1_gb() {
    let path = std::env::temp_dir().join(format!("logica-scale-{}.lacap", std::process::id()));
    let t0 = Instant::now();
    // create into a working .lacap on disk
    let file: Arc<dyn RandomAccess> = Arc::new(StdFile::create(&path).unwrap());
    let mut device = SyntheticDriver.open("synthetic/mix-dense").unwrap();
    device
        .configure(&CaptureConfig {
            channels: (0..8).collect(),
            sample_rate_hz: 100e6,
            samples: SAMPLES,
            mode: CaptureMode::Stream,
            threshold_volts: None,
        })
        .unwrap();
    let mut sink = StoreSink::new(StoreTarget { backing: file.clone(), cache_budget: BUDGET });
    let out = device.run(&mut sink, &StopSignal::new()).unwrap();
    let (store, meta) = sink
        .finish(&out, Source { kind: SourceKind::Synthetic, detail: "mix-dense".into() }, "scale")
        .unwrap();
    let created = t0.elapsed();
    let transitions: u64 =
        (0..8).map(|s| store.chunks(s).unwrap().iter().map(|c| u64::from(c.transitions)).sum::<u64>()).sum();
    assert_eq!(store.sample_count(), SAMPLES);
    let stats = store.cache_stats();
    let high_water = stats.high_water;
    assert!(high_water <= BUDGET + (2 << 20) + (136 << 10), "cache high water {high_water}");
    assert!(stats.evictions > 0, "chunks were evicted to the file while capturing");

    // save in place
    let t1 = Instant::now();
    lacap::save(&store, &meta, &serde_json::Value::Null).unwrap();
    let saved = t1.elapsed();
    drop(store);
    drop(file);
    let size = std::fs::metadata(&path).unwrap().len();

    // reopen
    let t2 = Instant::now();
    let opened = lacap::open(Arc::new(StdFile::open(&path).unwrap()), BUDGET).unwrap();
    let reopened = t2.elapsed();
    let s = &opened.store;
    assert!(!opened.recovered);
    assert_eq!(s.sample_count(), SAMPLES);

    // queries: level at a sample, edges in a window (across 2^32), LOD lookup
    let t3 = Instant::now();
    let b32: Sample = 1 << 32;
    for c in 0..8 {
        let (a, b) = (b32 - 3 * MIX_TILE / 2, b32 + MIX_TILE);
        assert_eq!(s.edges_in(c, a, b).unwrap(), expected(c, a, b), "channel {c} around 2^32");
    }
    let dense = s.edges_in(7, DENSE_START + DENSE_LEN - 100, DENSE_START + DENSE_LEN + 100).unwrap();
    assert_eq!(dense, expected(7, DENSE_START + DENSE_LEN - 100, DENSE_START + DENSE_LEN + 100));
    // PWM is high for the first quarter of every millisecond
    for &at in &[0u64, 24_999, 25_000, 4_999_999_999, b32 + 12_345] {
        let want = if at % 100_000 < 25_000 { Level::High } else { Level::Low };
        assert_eq!(s.level_at(6, at).unwrap(), want, "PWM at {at}");
    }
    let lod = s.lod(6, 0, CHUNK_SAMPLES, 300).unwrap();
    assert_eq!(lod.len(), 299, "5e9 samples are 298 whole chunks and a part");
    assert_eq!(lod[1].transitions, expected(6, CHUNK_SAMPLES, 2 * CHUNK_SAMPLES).len() as u64);
    let dense_lod = s.lod(7, DENSE_START, 1 << 16, 4).unwrap();
    assert!(dense_lod.iter().all(|b| b.transitions == 1 << 15), "{dense_lod:?}");
    let queried = t3.elapsed();
    drop(opened);
    std::fs::remove_file(&path).unwrap();

    say(&format!(
        "scale: 5e9 samples x 8 channels, {transitions} transitions; create {created:.2?}, save {saved:.2?}, reopen {reopened:.2?}, queries {queried:.2?}; file {:.1} MB; cache high water {:.1} MB, {} evictions",
        size as f64 / 1e6,
        high_water as f64 / f64::from(1u32 << 20),
        stats.evictions
    ));
    match peak_rss() {
        Some(peak) => {
            say(&format!("scale: peak resident memory {:.0} MB", peak as f64 / f64::from(1u32 << 20)));
            assert!(peak < MEMORY_LIMIT, "peak resident memory {peak} bytes is over 1 GB");
        }
        None => say("skipped: peak memory is measured on Linux only"),
    }
}
