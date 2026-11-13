//! Benchmarks against the baseline in `docs/benchmarks.md` (own harness, best of 5; the large
//! capture runs once, as in the baseline). Run with `cargo bench`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use logica_core::capture::{Source, SourceKind};
use logica_core::device::{CaptureConfig, CaptureMode, StopSignal, StoreSink, VirtualDriver};
use logica_core::import::{self, StoreTarget};
use logica_core::io::{MemFile, RandomAccess};
use logica_core::lacap;
use logica_core::store::{CHUNK_SAMPLES, CaptureStore};
use logica_core::{Level, Sample};
use logica_drivers::synthetic::SyntheticDriver;
use logica_fileio::StdFile;
use logica_protocols::analyse::{Batching, analyse_capture, decode_store};

fn best_of<T>(n: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut last = None;
    for _ in 0..n {
        let t = Instant::now();
        let v = f();
        best = best.min(t.elapsed());
        last = Some(v);
    }
    (best, last.unwrap())
}

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1e3)
}

fn row(what: &str, result: String) {
    println!("| {what} | {result} |");
}

fn main() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/captures");
    let bytes = std::fs::read(fixtures.join("mystery_D.demo")).expect("mystery_D.demo");
    let registry = logica_protocols::registry();
    println!("| What | Result |\n|---|---|");

    let (t, imported) = best_of(5, || {
        let f = MemFile::from_bytes(bytes.clone());
        import::demo::import(&f, "mystery_D.demo", &StoreTarget::memory(), &mut |_, _| {}).unwrap()
    });
    row("Import `mystery_D.demo` (50 M samples, 6 channels) into the store", ms(t));

    let (t, analysis) = best_of(5, || analyse_capture(&imported.store, &imported.meta, &registry).unwrap());
    row("Detection on `mystery_D` (all protocols, 6 channels)", ms(t));

    let spi = analysis.chosen().find(|d| d.protocol_id == "spi").unwrap().clone();
    let (t, _) = best_of(5, || {
        decode_store(
            &registry,
            &imported.store,
            &imported.meta,
            "spi",
            &spi.roles,
            &spi.settings,
            Batching::Whole,
        )
        .unwrap()
    });
    row("Streaming SPI decode of `mystery_D`", ms(t));

    let (save, file) = best_of(5, || {
        let f = MemFile::new();
        lacap::save_as(&imported.store, &imported.meta, &serde_json::Value::Null, &f).unwrap();
        f
    });
    let size = file.size().unwrap();
    let file: Arc<dyn RandomAccess> = Arc::new(file);
    let (open, _) = best_of(5, || lacap::open(file.clone(), 64 << 20).unwrap());
    row(
        &format!("`.lacap` save / open of `mystery_D` (in memory, {:.1} KB)", size as f64 / 1e3),
        format!("{} / {}", ms(save), ms(open)),
    );

    // packed-sample ingest: 8 channels, one byte per sample, a counter that steps every 16 samples
    let n: usize = 256 << 20;
    let data: Vec<u8> = (0..n).map(|i| u8::try_from((i >> 4) & 0xff).unwrap_or(0)).collect();
    let bits: Vec<u32> = (0..8).collect();
    let (t, _) = best_of(5, || {
        let mut s = CaptureStore::new(&[Level::Low; 8], Arc::new(MemFile::new()), 16, 1 << 30);
        for (k, part) in data.chunks(1 << 22).enumerate() {
            s.push_packed((k << 22) as Sample, 1, part, &bits).unwrap();
        }
        s
    });
    row(
        "Packed-sample ingest, 8 channels, 1 byte per sample (256 MB; a counter stepping every 16 samples)",
        format!("{:.0} MB/s", n as f64 / t.as_secs_f64() / 1e6),
    );

    // 5e9 samples x 8 channels, mix with a dense stretch, into a working .lacap on disk
    let path = std::env::temp_dir().join(format!("logica-bench-{}.lacap", std::process::id()));
    let t0 = Instant::now();
    let backing: Arc<dyn RandomAccess> = Arc::new(StdFile::create(&path).unwrap());
    let mut d = SyntheticDriver.open("synthetic/mix-dense").unwrap();
    d.configure(&CaptureConfig {
        channels: (0..8).collect(),
        sample_rate_hz: 100e6,
        samples: 5_000_000_000,
        mode: CaptureMode::Stream,
        threshold_volts: None,
    })
    .unwrap();
    let budget = 64 << 20;
    let mut sink = StoreSink::new(StoreTarget { backing, cache_budget: budget });
    let out = d.run(&mut sink, &StopSignal::new()).unwrap();
    let (store, meta) = sink
        .finish(&out, Source { kind: SourceKind::Synthetic, detail: "mix-dense".into() }, "bench")
        .unwrap();
    let create = t0.elapsed();
    let transitions: u64 =
        (0..8).map(|s| store.chunks(s).unwrap().iter().map(|c| u64::from(c.transitions)).sum::<u64>()).sum();
    let high = store.cache_stats().high_water;
    let t1 = Instant::now();
    lacap::save(&store, &meta, &serde_json::Value::Null).unwrap();
    let save = t1.elapsed();
    drop(store);
    let size = std::fs::metadata(&path).unwrap().len();
    let t2 = Instant::now();
    let o = lacap::open(Arc::new(StdFile::open(&path).unwrap()), budget).unwrap();
    let reopen = t2.elapsed();
    let t3 = Instant::now();
    for c in 0..8 {
        o.store.level_at(c, 4_321_000_000).unwrap();
        o.store.edges_in(c, (1 << 32) - 1_000_000, (1 << 32) + 1_000_000).unwrap();
        o.store.lod(c, 0, CHUNK_SAMPLES, 300).unwrap();
    }
    let queries = t3.elapsed();
    drop(o);
    std::fs::remove_file(&path).unwrap();
    row(
        &format!(
            "5e9 samples × 8 channels, synthetic `mix-dense` ({:.1} M transitions): create into a working `.lacap` on disk",
            transitions as f64 / 1e6
        ),
        ms(create),
    );
    row(
        "… save in place / reopen / a set of queries (level at a sample, edges in a window, LOD lookup)",
        format!("{} / {} / {}", ms(save), ms(reopen), ms(queries)),
    );
    row("… file size", format!("{:.1} MB", size as f64 / 1e6));
    row(
        &format!("… chunk cache high-water mark (budget {} MB)", budget >> 20),
        format!("{:.1} MB", high as f64 / f64::from(1u32 << 20)),
    );
}
