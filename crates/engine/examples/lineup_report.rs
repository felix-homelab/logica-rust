//! Measures how well two devices line up in one capture, with and without a shared signal, and
//! writes the numbers as JSON for `tools/reports/render-lineup.mjs`.
//!
//! Two virtual analysers (`logica-vla`) run on loopback ports, as fast as the merger takes the
//! data. Both see the same `shared` clock (100 Hz, on the computer's clock). Device B starts later
//! by a set delay (as a slower device would) and its sample clock runs off by a set ppm. Each
//! setting is captured twice through the engine's own merger (`logica_engine::group::run`): lined
//! up by the shared clock, and lined up at the starts.
//!
//! The truth comes from the analysers: each says where its sample 0 is on the computer's clock
//! (nanoseconds since 1970), so every rising edge of the shared clock belongs to a known period.
//! The error of a period is where B's edge landed in the capture minus where A's did.
//!
//! `cargo run --release -p logica-engine --example lineup_report -- <out.json>`

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use logica_core::capture::{ChannelInfo, Note};
use logica_core::device::{
    Block, BlockSink, Capabilities, CaptureConfig, CaptureMode, Device, DeviceError, DeviceInfo, RunOutcome,
    StopSignal, VirtualDriver,
};
use logica_core::{Level, Sample};
use logica_drivers::vla::VlaDriver;
use logica_engine::group::{self, Member};
use logica_vla::Config;
use serde_json::{Value, json};

const RATE: f64 = 4e6;
/// Length of the drift captures; the start captures are one second.
/// The shared clock's period, in nanoseconds (100 Hz).
const PERIOD_NS: f64 = 1e7;
const SYNC: u32 = 10;

/// Starts an analyser on a free port, as fast as it is read; returns its address.
fn analyser(name: &str, ppm: f64) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let at = listener.local_addr().expect("an address").to_string();
    let mut c = Config::example();
    c.name = name.into();
    c.seed = Some(7);
    c.speed = 0.0;
    c.clock_ppm = ppm;
    c.listen.clone_from(&at);
    std::thread::spawn(move || logica_vla::server::serve(listener, c, Arc::new(|_: &str| {})));
    at
}

/// A device that waits before it starts, as a slower one would, and keeps its own rising edges of
/// the shared clock (on its own clock), so every edge in the capture can be traced to its period.
struct Late {
    inner: Box<dyn Device>,
    delay: Duration,
    rising: Arc<Mutex<Vec<Sample>>>,
}

/// Passes a device's data on and keeps the rising edges of its only channel.
struct Tap<'a> {
    inner: &'a mut dyn BlockSink,
    level: Level,
    rising: Arc<Mutex<Vec<Sample>>>,
}

impl BlockSink for Tap<'_> {
    fn begin(&mut self, channels: &[ChannelInfo], initial: &[Level], rate: f64) -> Result<(), DeviceError> {
        self.level = initial.first().copied().unwrap_or_default();
        self.inner.begin(channels, initial, rate)
    }
    fn block(&mut self, b: Block<'_>) -> Result<(), DeviceError> {
        if let Block::Transitions { slot: 0, edges } = b {
            let mut r = self.rising.lock().expect("the tap");
            for &e in edges {
                self.level = !self.level;
                if self.level == Level::High {
                    r.push(e);
                }
            }
        }
        self.inner.block(b)
    }
    fn note(&mut self, note: Note) {
        self.inner.note(note);
    }
}

impl Device for Late {
    fn info(&self) -> &DeviceInfo {
        self.inner.info()
    }
    fn capabilities(&self) -> &Capabilities {
        self.inner.capabilities()
    }
    fn configure(&mut self, c: &CaptureConfig) -> Result<(), DeviceError> {
        self.inner.configure(c)
    }
    fn run(&mut self, sink: &mut dyn BlockSink, stop: &StopSignal) -> Result<RunOutcome, DeviceError> {
        std::thread::sleep(self.delay);
        let mut tap = Tap { inner: sink, level: Level::Low, rising: self.rising.clone() };
        self.inner.run(&mut tap, stop)
    }
    fn close(&mut self) {
        self.inner.close();
    }
}

/// What the capture received.
#[derive(Default)]
struct Rec {
    initial: Vec<Level>,
    edges: Vec<Vec<Sample>>,
    notes: Vec<String>,
}

impl BlockSink for Rec {
    fn begin(&mut self, channels: &[ChannelInfo], initial: &[Level], _: f64) -> Result<(), DeviceError> {
        self.initial = initial.to_vec();
        self.edges = vec![Vec::new(); channels.len()];
        Ok(())
    }
    fn block(&mut self, b: Block<'_>) -> Result<(), DeviceError> {
        if let Block::Transitions { slot, edges } = b {
            self.edges[slot].extend_from_slice(edges);
        }
        Ok(())
    }
    fn note(&mut self, note: Note) {
        self.notes.push(note.message);
    }
}

impl Rec {
    fn rising(&self, slot: usize) -> Vec<Sample> {
        let mut level = self.initial[slot];
        let mut out = Vec::new();
        for &e in &self.edges[slot] {
            level = !level;
            if level == Level::High {
                out.push(e);
            }
        }
        out
    }

    /// Where a device's sample 0 is on the computer's clock, from its note.
    fn epoch(&self, label: &str) -> f64 {
        self.notes
            .iter()
            .filter(|n| n.starts_with(&format!("{label}: ")))
            .find_map(|n| n.split("Sample 0 is at ").nth(1)?.split(' ').next()?.parse::<f64>().ok())
            .expect("the analyser says where its sample 0 is")
    }
}

/// One capture of A and B together; the error per shared-clock period, and what the merger said.
fn capture(ppm: f64, delay_ms: f64, align: bool, seconds: f64) -> Value {
    let (a_at, b_at) = (analyser("A", 0.0), analyser("B", ppm));
    let open = |at: &str| {
        let d = VlaDriver::new(vec![at.to_string()]);
        let mut dev = d.open(&format!("vla/{at}")).expect("the analyser answers");
        dev.configure(&CaptureConfig {
            channels: vec![SYNC],
            sample_rate_hz: RATE,
            samples: logica_protocols::num::round(seconds * RATE),
            mode: CaptureMode::Stream,
            threshold_volts: None,
        })
        .expect("a configuration the analyser takes");
        dev
    };
    let a = open(&a_at);
    let raw = Arc::new(Mutex::new(Vec::new()));
    let b: Box<dyn Device> = Box::new(Late {
        inner: open(&b_at),
        delay: Duration::from_secs_f64(delay_ms / 1e3),
        rising: raw.clone(),
    });
    let members = vec![
        Member { id: "a".into(), label: "A".into(), device: a, align: align.then_some(SYNC) },
        Member { id: "b".into(), label: "B".into(), device: b, align: align.then_some(SYNC) },
    ];
    let mut rec = Rec::default();
    let (_, r) = group::run(members, &mut rec, &StopSignal::new());
    let outcome = r.expect("the capture completes");
    let (ea, eb) = (rec.epoch("A"), rec.epoch("B"));
    let scale = 1.0 + ppm * 1e-6;

    // A's clock is the capture's: its edge of period n is where it is
    let period = |t_ns: f64| logica_protocols::num::round(t_ns / PERIOD_NS);
    let a_edges: BTreeMap<u64, f64> =
        rec.rising(0).into_iter().map(|x| (period(ea + x as f64 / RATE * 1e9), x as f64)).collect();
    let found_offset = rec
        .notes
        .iter()
        .find_map(|n| n.split("its sample 0 is at sample ").nth(1)?.split(' ').next()?.parse::<f64>().ok());
    // B's edges in the capture are its own edges in order, less those before the capture's
    // sample 0: find how many were dropped from where the merger put its sample 0, then name
    // each edge's period by B's own clock (so a slip by whole periods shows as an error)
    let raw = raw.lock().expect("the tap").clone();
    let comb = rec.rising(1);
    let shift = found_offset.unwrap_or(0.0);
    let dropped = (0..raw.len().min(50))
        .min_by(|&i, &j| {
            let d = |k: usize| (comb[0] as f64 - raw[k] as f64 - shift).abs();
            d(i).total_cmp(&d(j))
        })
        .unwrap_or(0);
    let b_edges: BTreeMap<u64, f64> = comb
        .iter()
        .zip(&raw[dropped..])
        .map(|(&y, &x)| (period(eb + x as f64 / (RATE * scale) * 1e9), y as f64))
        .collect();
    let mut points = Vec::new();
    for (n, x) in &a_edges {
        if let Some(y) = b_edges.get(n) {
            // seconds into the capture, and B minus A in microseconds
            points.push(json!([x / RATE, (y - x) / RATE * 1e6]));
        }
    }
    // the truth: where B's sample 0 is on A's clock, in samples
    let true_offset = (eb - ea) / 1e9 * RATE;
    let drift = rec
        .notes
        .iter()
        .find_map(|n| n.strip_prefix("B's clock ran ")?.split(' ').next()?.parse::<f64>().ok());
    json!({
        "ppm": ppm,
        "delayMs": delay_ms,
        "align": align,
        "samples": outcome.samples,
        "trueOffsetUs": true_offset / RATE * 1e6,
        "foundOffsetUs": found_offset.map(|o| o / RATE * 1e6),
        "measuredPpm": drift,
        "points": points,
    })
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "lineup.json".into());
    let mut runs = Vec::new();
    // drift: B starts 2 ms late, its clock 0 to 500 ppm off
    for ppm in [0.0, 20.0, 100.0, 500.0] {
        for align in [false, true] {
            eprintln!("drift {ppm} ppm, lined up by the shared clock: {align}");
            runs.push(capture(ppm, 2.0, align, 10.0));
        }
    }
    // starts: forty one-second captures each way, B asked to start 0 to 3.9 ms late, its clock
    // 50 ppm off; the start gap that results is measured, not assumed
    let mut starts = Vec::new();
    for k in 0..40 {
        let delay = f64::from(k % 40) * 0.1;
        for align in [false, true] {
            eprintln!("start {delay:.1} ms late, lined up by the shared clock: {align}");
            let mut r = capture(50.0, delay, align, 1.0);
            // the error at the start is what matters here
            if let Some(p) = r["points"].as_array_mut() {
                p.truncate(5);
            }
            starts.push(r);
        }
    }
    let report = json!({
        "rateHz": RATE,
        "seconds": 10.0,
        "sharedClockHz": 1e9 / PERIOD_NS,
        "drift": runs,
        "starts": starts,
    });
    std::fs::write(&out, serde_json::to_string(&report).expect("JSON")).expect("the report is written");
    eprintln!("wrote {out}");
}
