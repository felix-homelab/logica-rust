//! Serves the virtual analyser on a TCP listener (`PROTOCOL.md`): every connection is greeted, so
//! Logica can list the analyser while it is capturing; one capture runs at a time, like on a real
//! device.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use logica_drivers::vla::wire::{
    self, Hello, HelloChannel, MAX_LINE, Message, PROTOCOL, Request, Start, VERSION,
};

use crate::config::Config;
use crate::generator::Generator;

/// Where progress lines go (the terminal, or nowhere in tests).
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

struct Shared {
    config: Config,
    rates: Vec<f64>,
    busy: AtomicBool,
    captures: AtomicU64,
    log: Log,
}

/// Accepts connections until the listener fails; each runs on its own thread.
pub fn serve(listener: TcpListener, config: Config, log: Log) -> std::io::Result<()> {
    let rates = config.usable_rates();
    let shared =
        Arc::new(Shared { config, rates, busy: AtomicBool::new(false), captures: AtomicU64::new(0), log });
    for stream in listener.incoming() {
        let stream = stream?;
        let shared = shared.clone();
        std::thread::spawn(move || {
            let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            if let Err(e) = connection(stream, &shared) {
                (shared.log)(&format!("connection from {peer} ended: {e}"));
            }
        });
    }
    Ok(())
}

pub fn hello(config: &Config, rates: &[f64], port: u16) -> Hello {
    Hello {
        protocol: PROTOCOL.into(),
        version: VERSION,
        model: config.name.clone(),
        serial: format!("vla-{port}"),
        channels: config
            .channel_names()
            .into_iter()
            .enumerate()
            .map(|(i, name)| HelloChannel { index: u32::try_from(i).unwrap_or(u32::MAX), name })
            .collect(),
        sample_rates_hz: rates.to_vec(),
    }
}

fn refuse(stream: &mut TcpStream, text: String) -> std::io::Result<()> {
    let mut b = Vec::new();
    Message::Error { text }.encode(&mut b);
    stream.write_all(&b)
}

fn rate_text(hz: f64) -> String {
    if hz >= 1e6 { format!("{} MS/s", hz / 1e6) } else { format!("{} kS/s", hz / 1e3) }
}

/// Why a start request cannot be served, if it cannot.
fn check(s: &Start, config: &Config, rates: &[f64]) -> Option<String> {
    if s.samples == 0 {
        return Some("Choose how many samples to capture.".into());
    }
    if !rates.iter().any(|&r| (r - s.sample_rate_hz).abs() <= r * 1e-9) {
        let list = rates.iter().map(|&r| rate_text(r)).collect::<Vec<_>>().join(", ");
        return Some(format!(
            "{} is not a rate this analyser offers. Choose one of: {list}.",
            rate_text(s.sample_rate_hz)
        ));
    }
    if s.channels.is_empty() {
        return Some("Enable at least one channel.".into());
    }
    if s.channels.windows(2).any(|w| w[0] >= w[1]) {
        return Some("List the channels once each, in ascending order.".into());
    }
    if let Some(c) = s.channels.iter().find(|&&c| c >= config.channel_count) {
        return Some(format!("This analyser has no channel {c}."));
    }
    None
}

fn connection(mut stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let _ = stream.set_nodelay(true);
    let port = stream.local_addr()?.port();
    stream.write_all(&wire::line(&hello(&shared.config, &shared.rates, port)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = Vec::new();
    // a client that only wanted the greeting (Logica listing devices) closes here
    if (&mut reader).take(MAX_LINE as u64).read_until(b'\n', &mut line)? == 0 {
        return Ok(());
    }
    let start = match serde_json::from_slice::<Request>(&line) {
        Ok(Request::Start(s)) => s,
        Ok(Request::Stop(_)) => return Ok(()),
        Err(e) => return refuse(&mut stream, format!("The analyser did not understand the request: {e}.")),
    };
    if let Some(why) = check(&start, &shared.config, &shared.rates) {
        return refuse(&mut stream, why);
    }
    if shared.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return refuse(
            &mut stream,
            "The virtual analyser is busy with another capture; stop that one first.".into(),
        );
    }
    let r = capture(&mut stream, reader, &start, shared);
    shared.busy.store(false, Ordering::Release);
    let _ = stream.shutdown(Shutdown::Both);
    r
}

fn capture(
    stream: &mut TcpStream,
    mut reader: BufReader<TcpStream>,
    s: &Start,
    shared: &Shared,
) -> std::io::Result<()> {
    let n = shared.captures.fetch_add(1, Ordering::Relaxed) + 1;
    let seed = shared.config.seed.unwrap_or_else(|| {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos() & u128::from(u64::MAX)).unwrap_or(0));
        t ^ n.rotate_left(32)
    });
    (shared.log)(&format!(
        "capture {n}: {} channels at {} for {} samples, seed {seed}",
        s.channels.len(),
        rate_text(s.sample_rate_hz),
        s.samples
    ));
    // the client asks to stop with a line, or by going away
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if matches!(serde_json::from_str::<Request>(&line), Ok(Request::Stop(true))) {
                        break;
                    }
                }
            }
        }
        flag.store(true, Ordering::Release);
    });

    let mut g = Generator::new(&shared.config, s.sample_rate_hz, seed);
    let mut out = Vec::new();
    Message::Begin { initial: g.initial(&s.channels) }.encode(&mut out);
    Message::Note {
        text: format!(
            "Generated by the virtual analyser \"{}\" (logica-vla) with seed {seed}; its timing is perfect.",
            shared.config.name
        ),
    }
    .encode(&mut out);
    stream.write_all(&out)?;

    let tile = shared.config.tile_ms / 1e3;
    let began = Instant::now();
    let mut end = 0;
    let mut k: u64 = 0;
    let stopped = loop {
        if stop.load(Ordering::Acquire) {
            break true;
        }
        k += 1;
        let t = k as f64 * tile;
        g.fill(t);
        end = g.sample_at(t).min(s.samples);
        out.clear();
        for (slot, &c) in s.channels.iter().enumerate() {
            let samples = g.take(c, end);
            if !samples.is_empty() {
                Message::Edges { slot: u16::try_from(slot).unwrap_or(u16::MAX), samples }.encode(&mut out);
            }
        }
        Message::Advance { end }.encode(&mut out);
        stream.write_all(&out)?;
        if end >= s.samples {
            break false;
        }
        if shared.config.speed > 0.0 {
            let due = began + Duration::from_secs_f64(t / shared.config.speed);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
    };
    out.clear();
    Message::End { samples: end }.encode(&mut out);
    stream.write_all(&out)?;
    (shared.log)(&format!(
        "capture {n} {} after {end} samples ({:.1} s of signal)",
        if stopped { "stopped" } else { "finished" },
        end as f64 / s.sample_rate_hz
    ));
    Ok(())
}
