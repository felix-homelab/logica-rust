//! Writes a large `.lacap` for the window's performance budgets (doc 08): by default 1e9 samples
//! on 16 channels at 100 MHz. Each channel is a square wave of its own period with a dense burst
//! in the middle, so every zoom level has something to draw.
//!
//! `cargo run --release -p logica-drivers --example make_capture -- <out.lacap> [samples] [channels]`

use std::sync::Arc;

use logica_core::capture::{CaptureMeta, CaptureState, ChannelInfo, Source, SourceKind};
use logica_core::io::RandomAccess;
use logica_core::lacap;
use logica_core::{Level, Sample};
use logica_fileio::StdFile;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("an output path ending in .lacap");
    let count: Sample = args.next().map_or(1_000_000_000, |s| s.parse().expect("a sample count"));
    let channels: usize = args.next().map_or(16, |s| s.parse().expect("a channel count"));
    let file: Arc<dyn RandomAccess> =
        Arc::new(StdFile::create(std::path::Path::new(&out)).expect("the output can be created"));
    let mut store =
        lacap::new_capture(file, &vec![Level::Low; channels], 256 << 20).expect("a working capture");
    let mut total = 0usize;
    for slot in 0..channels {
        // half periods from 5 µs (ch 0) up, at 100 MHz; a burst of 50 ns pulses in the middle
        let half = 500 * (slot as u64 + 1);
        let burst = count / 2..count / 2 + 2_000_000;
        let mut edges = Vec::new();
        let mut s = half;
        while s < count {
            if burst.contains(&s) {
                let mut b = burst.start;
                while b < burst.end {
                    edges.push(b);
                    b += 5;
                }
                s = burst.end + (half - burst.end % half) % half;
                if edges.len() % 2 == 1 {
                    s += half;
                }
                continue;
            }
            edges.push(s);
            s += half;
            if edges.len() >= 4_000_000 {
                store.push_edges(slot, &edges).expect("edges in order");
                total += edges.len();
                edges.clear();
            }
        }
        store.push_edges(slot, &edges).expect("edges in order");
        total += edges.len();
    }
    store.finish(count).expect("the capture ends");
    let meta = CaptureMeta {
        id: "c1".into(),
        title: "16 channels, 1e9 samples".into(),
        source: Source { kind: SourceKind::Synthetic, detail: "make_capture example".into() },
        sample_rate_hz: 100e6,
        trigger: None,
        channels: (0..channels)
            .map(|i| ChannelInfo {
                index: u32::try_from(i).expect("few channels"),
                name: format!("CH{i}"),
                colour: None,
                enabled: true,
                threshold_volts: None,
            })
            .collect(),
        segments: std::iter::once(0..count).collect(),
        notes: vec![],
        state: CaptureState::Complete,
        artefact: None,
        saved_decoders: vec![],
    };
    lacap::save(&store, &meta, &serde_json::Value::Null).expect("the capture is saved");
    println!("{out}: {count} samples, {channels} channels, {total} edges");
}
