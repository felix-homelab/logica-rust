//! Shared helpers for the acceptance tests: load a capture, detect, decode in every batching.
#![allow(dead_code, clippy::cast_possible_truncation)]

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use logica_core::capture::CaptureMeta;
use logica_core::import::{self, StoreTarget};
use logica_core::io::MemFile;
use logica_core::store::CaptureStore;
use logica_protocols::Registry;
use logica_protocols::analyse::{Batching, CaptureAnalysis, analyse_capture, decode_store};
use logica_protocols::types::*;

pub struct Loaded {
    pub store: CaptureStore,
    pub meta: CaptureMeta,
    pub analysis: CaptureAnalysis,
}

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Prints past the test harness's output capture, so skips are visible in a plain `cargo test`.
pub fn say(msg: &str) {
    let _ = writeln!(std::io::stderr(), "{msg}");
}

pub fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(logica_protocols::registry)
}

/// A sample capture, imported and analysed once per test binary. `None` (with a visible message)
/// for the local-only FNIRSI files when they are absent.
pub fn sample(name: &str) -> Option<Arc<Loaded>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Loaded>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(l) = cache.lock().unwrap().get(name) {
        return Some(l.clone());
    }
    let local = ["I2C.demo", "SPI.demo", "UART.demo"].contains(&name);
    let path =
        fixtures().join(if local { format!("captures/local/{name}") } else { format!("captures/{name}") });
    if !path.exists() {
        assert!(local, "{name} is missing");
        say(&format!("skipped: FNIRSI example files not present ({name})"));
        return None;
    }
    let file = MemFile::from_bytes(std::fs::read(&path).unwrap());
    let imp = import::demo::import(&file, name, &StoreTarget::memory(), &mut |_, _| {}).unwrap();
    let analysis = analyse_capture(&imp.store, &imp.meta, registry()).unwrap();
    let l = Arc::new(Loaded { store: imp.store, meta: imp.meta, analysis });
    cache.lock().unwrap().insert(name.to_string(), l.clone());
    Some(l)
}

pub fn chosen(l: &Loaded) -> Vec<Detection> {
    l.analysis.chosen().cloned().collect()
}

/// The one chosen detection of `protocol` (on `channel`, if given).
pub fn only(l: &Loaded, protocol: &str, channel: Option<u32>) -> Detection {
    let list: Vec<Detection> = chosen(l)
        .into_iter()
        .filter(|d| d.protocol_id == protocol && channel.is_none_or(|c| d.channels.contains(&c)))
        .collect();
    assert_eq!(
        list.len(),
        1,
        "{protocol} on {}: {:#?}",
        l.meta.title,
        l.analysis
            .buses
            .iter()
            .map(|b| (
                &b.bus.channels,
                b.chosen.iter().map(|d| (&d.protocol_id, d.confidence)).collect::<Vec<_>>(),
                b.alternatives.iter().map(|d| (&d.protocol_id, d.confidence)).collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>()
    );
    list.into_iter().next().unwrap()
}

/// Decodes in one batch, in batches of 1 and 3 edges, and with snapshot and restore after every
/// batch of 7; asserts all four are identical and returns the result (doc 06).
pub fn decode_with(
    store: &CaptureStore,
    meta: &CaptureMeta,
    protocol: &str,
    roles: &Roles,
    settings: &Settings,
) -> Result<DecodeResult, String> {
    let whole = decode_store(registry(), store, meta, protocol, roles, settings, Batching::Whole)?;
    for b in [Batching::Edges(1), Batching::Edges(3), Batching::WithSnapshots(7)] {
        let other = decode_store(registry(), store, meta, protocol, roles, settings, b)?;
        assert_eq!(other, whole, "{protocol} on {} differs with {b:?}", meta.title);
    }
    Ok(whole)
}

pub fn decode(l: &Loaded, d: &Detection) -> DecodeResult {
    decode_with(&l.store, &l.meta, &d.protocol_id, &d.roles, &d.settings).unwrap()
}

pub fn decode_as(l: &Loaded, protocol: &str, roles: &Roles, settings: &Settings) -> DecodeResult {
    decode_with(&l.store, &l.meta, protocol, roles, settings).unwrap()
}

pub fn with(d: &Detection, key: &str, v: SettingValue) -> Settings {
    let mut s = d.settings.clone();
    s.insert(key.into(), v);
    s
}

pub fn at(l: &Loaded, p: &Packet, digits: i32) -> f64 {
    let t = p.start as f64 / l.meta.sample_rate_hz;
    let f = 10f64.powi(digits);
    (t * f).round() / f
}

pub fn texts(r: &DecodeResult) -> Vec<String> {
    r.packets
        .iter()
        .map(|p| logica_protocols::format::as_text(p.streams.first().map_or(&[][..], |s| &s.bytes)))
        .collect()
}

pub fn hexes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

pub fn stream(p: &Packet, label: &str) -> String {
    hexes(p.stream(label).map_or(&[][..], |s| &s.bytes))
}

pub fn evidence_text(d: &Detection) -> String {
    d.evidence.iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join(" ")
}

pub fn n(d: &Detection, key: &str) -> f64 {
    match d.settings.get(key) {
        Some(SettingValue::Number(x)) => *x,
        other => panic!("{key} is {other:?}"),
    }
}

pub fn t<'a>(d: &'a Detection, key: &str) -> &'a str {
    match d.settings.get(key) {
        Some(SettingValue::Text(x)) => x,
        other => panic!("{key} is {other:?}"),
    }
}

pub fn b(d: &Detection, key: &str) -> bool {
    match d.settings.get(key) {
        Some(SettingValue::Bool(x)) => *x,
        other => panic!("{key} is {other:?}"),
    }
}

pub fn no_problems(r: &DecodeResult) {
    assert!(r.problem_counts.is_empty(), "problems: {:?}", r.problem_counts);
}

pub fn count(r: &DecodeResult, code: ProblemCode) -> u64 {
    r.problem_counts.get(&code).copied().unwrap_or(0)
}

pub fn role(d: &Detection, key: &str) -> Option<u32> {
    d.roles.get(key).copied().flatten()
}

/// A generated capture, stored and analysed.
pub fn generated(g: &logica_protocols::synth::GenCapture) -> Loaded {
    let store = g.store().unwrap();
    let meta = g.meta();
    let analysis = analyse_capture(&store, &meta, registry()).unwrap();
    Loaded { store, meta, analysis }
}

/// Labels of frames in annotation row `row`.
pub fn labels(r: &DecodeResult, row: u8) -> Vec<String> {
    r.frames.iter().filter(|f| f.row == row).map(|f| f.label.clone()).collect()
}
