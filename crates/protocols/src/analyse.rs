//! Detection and decoding of a capture (doc 06): measure channels → group them into buses → run
//! every detector → choose the best non-overlapping explanation; and whole-capture decodes through
//! the streaming decoders, in any batching.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use logica_core::capture::CaptureMeta;
use logica_core::measure::{ChannelMeasurement, measure_channel};
use logica_core::store::CaptureStore;
use logica_core::store::edges::{Edge, EdgeBatch};
use logica_core::{Level, Sample};

use crate::buses::{Bus, group_buses};
use crate::format;
use crate::protocol::{DetectContext, Protocol, Registry};
use crate::stream::{BoundChannel, Collected, DecoderSetup, StreamDecoder};
use crate::types::{DecodeResult, Detection, Rejection, Roles, SettingSpec, SettingValue, Settings};
use crate::window::Window;

/// Below this confidence a detection is listed as an alternative only.
pub const MIN_CONFIDENCE: f64 = 0.4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BusAnalysis {
    pub bus: Bus,
    /// The best non-overlapping explanation of the bus, most confident first.
    pub chosen: Vec<Detection>,
    /// Other candidates, for comparison and learning.
    pub alternatives: Vec<Detection>,
    pub rejections: Vec<Rejection>,
    /// Channels of the bus that no detection explains.
    pub unexplained: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureAnalysis {
    /// The samples detection looked at.
    pub window: (Sample, Sample),
    pub measurements: Vec<ChannelMeasurement>,
    pub buses: Vec<BusAnalysis>,
    /// Observations across buses (a TX/RX pair), and why the window is short if it is.
    pub notes: Vec<String>,
}

impl CaptureAnalysis {
    pub fn chosen(&self) -> impl Iterator<Item = &Detection> {
        self.buses.iter().flat_map(|b| b.chosen.iter())
    }
}

fn rank(d: &Detection) -> f64 {
    d.confidence + 0.08 * (d.channels.len() as f64 - 1.0)
}

/// Detects protocols on a window.
pub fn analyse_window(window: &Window, sample_rate_hz: f64, registry: &Registry) -> CaptureAnalysis {
    let measurements: BTreeMap<u32, ChannelMeasurement> = window
        .channels
        .iter()
        .map(|c| (c.index, measure_channel(c.index, &c.edges, c.initial, window.start, window.end)))
        .collect();
    let buses = group_buses(window, &measurements);
    let mut results = Vec::with_capacity(buses.len());
    for bus in buses {
        let channels: Vec<_> = bus.channels.iter().filter_map(|&i| window.channel(i)).collect();
        if channels.iter().all(|c| c.edges.is_empty()) {
            results.push(BusAnalysis {
                unexplained: bus.channels.clone(),
                bus,
                chosen: vec![],
                alternatives: vec![],
                rejections: vec![],
            });
            continue;
        }
        let ctx = DetectContext { sample_rate_hz, window, bus: &bus, channels, measurements: &measurements };
        let mut all: Vec<Detection> = Vec::new();
        let mut rejections = Vec::new();
        for p in registry.all() {
            if let Some(out) = p.detect(&ctx) {
                all.extend(out.detections);
                rejections.extend(out.rejections);
            }
        }
        // stable: equal ranks keep registry order
        all.sort_by(|a, b| rank(b).total_cmp(&rank(a)));
        let mut used: Vec<u32> = Vec::new();
        let (mut chosen, mut alternatives) = (Vec::new(), Vec::new());
        for d in all {
            if d.confidence >= MIN_CONFIDENCE && d.channels.iter().all(|c| !used.contains(c)) {
                used.extend(&d.channels);
                chosen.push(d);
            } else {
                alternatives.push(d);
            }
        }
        let unexplained = bus.channels.iter().copied().filter(|c| !used.contains(c)).collect();
        results.push(BusAnalysis { bus, chosen, alternatives, rejections, unexplained });
    }

    let mut notes: Vec<String> = window.note.iter().cloned().collect();
    // TX/RX pairs: two UART lines on separate buses with the same settings
    let uarts: Vec<&Detection> =
        results.iter().flat_map(|r| r.chosen.iter()).filter(|d| d.protocol_id == "uart").collect();
    for i in 0..uarts.len() {
        for j in i + 1..uarts.len() {
            let (a, b) = (uarts[i], uarts[j]);
            if ["baud", "parity", "invert"].iter().all(|k| a.settings.get(*k) == b.settings.get(*k)) {
                let baud = match a.settings.get("baud") {
                    Some(SettingValue::Number(n)) => *n,
                    _ => 0.0,
                };
                notes.push(format!(
                    "Channels {} and {} are both UART at {} baud and take turns. They are probably the two directions (TX and RX) of one link.",
                    a.channels[0],
                    b.channels[0],
                    format::int(baud)
                ));
            }
        }
    }
    CaptureAnalysis {
        window: (window.start, window.end),
        measurements: measurements.into_values().collect(),
        buses: results,
        notes,
    }
}

/// Where analysis of a capture starts: after its start-up artefact, if any.
pub fn analysis_start(meta: &CaptureMeta) -> Sample {
    meta.artefact.as_ref().map_or(0, |a| a.end)
}

/// Detects protocols on a capture (on its detection window).
pub fn analyse_capture(
    store: &CaptureStore,
    meta: &CaptureMeta,
    registry: &Registry,
) -> Result<CaptureAnalysis, String> {
    let window = Window::from_store(store, meta, analysis_start(meta)).map_err(|e| e.to_string())?;
    Ok(analyse_window(&window, meta.sample_rate_hz, registry))
}

// ------------------------------------------------------------------ decoding

/// How edges are handed to a decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Batching {
    /// One batch with every edge.
    Whole,
    /// Batches of at most this many edges.
    Edges(usize),
    /// Batches of at most this many edges, with a snapshot and restore after every batch.
    WithSnapshots(usize),
}

/// The protocol that decodes `protocol_id` (following an alias).
pub fn decoding_protocol<'r>(registry: &'r Registry, protocol_id: &str) -> Result<&'r dyn Protocol, String> {
    let def = registry.get(protocol_id).ok_or_else(|| format!("There is no decoder for {protocol_id}."))?;
    let target = match def.alias_of() {
        Some(a) => registry.get(a).ok_or_else(|| format!("There is no decoder for {}.", def.name()))?,
        None => def,
    };
    if !target.has_decoder() {
        return Err(format!("There is no decoder for {}.", def.name()));
    }
    Ok(target.as_ref())
}

/// Binds roles to the capture's channels: the bound channels in role order with their store
/// slots, or a message for the person.
pub fn bind_roles(
    protocol: &dyn Protocol,
    meta: &CaptureMeta,
    roles: &Roles,
) -> Result<Vec<(BoundChannel, usize)>, String> {
    let mut out = Vec::new();
    for r in protocol.roles() {
        let Some(&Some(ch)) = roles.get(&r.key) else {
            if r.optional {
                continue;
            }
            return Err(format!("Choose a channel for {}.", r.noun));
        };
        match meta.channels.iter().position(|c| c.index == ch) {
            Some(slot) => out.push((
                BoundChannel { role: r.key.clone(), channel: ch, name: meta.channels[slot].name.clone() },
                slot,
            )),
            None if r.optional => {}
            None => return Err(format!("Channel {ch} ({}) is not in this capture.", r.noun)),
        }
    }
    Ok(out)
}

/// Runs a decoder over merged edge batches and collects everything it emits.
fn run_decoder(
    protocol: &dyn Protocol,
    setup: &DecoderSetup,
    mut next: impl FnMut(usize) -> Result<Option<EdgeBatch>, String>,
    end: Sample,
    batching: Batching,
) -> Result<DecodeResult, String> {
    let mut dec: Box<dyn StreamDecoder> = protocol.decoder(setup, None)?;
    let mut col = Collected::default();
    let size = match batching {
        Batching::Whole => usize::MAX,
        Batching::Edges(n) | Batching::WithSnapshots(n) => n.max(1),
    };
    while let Some(b) = next(size)? {
        dec.push(&b);
        col.take(dec.drain());
        if let Batching::WithSnapshots(_) = batching {
            let snap = dec.snapshot();
            dec = protocol.decoder(setup, Some(snap))?;
        }
    }
    dec.finish(end);
    col.take(dec.drain());
    Ok(col.result)
}

/// Decodes a whole capture from its store with `protocol_id`, from the end of the start-up
/// artefact, handing the decoder its edges in the given batching.
pub fn decode_store(
    registry: &Registry,
    store: &CaptureStore,
    meta: &CaptureMeta,
    protocol_id: &str,
    roles: &Roles,
    settings: &Settings,
    batching: Batching,
) -> Result<DecodeResult, String> {
    let protocol = decoding_protocol(registry, protocol_id)?;
    let bound = bind_roles(protocol, meta, roles)?;
    let start = analysis_start(meta).min(store.sample_count());
    let count = store.sample_count();
    let slots: Vec<usize> = bound.iter().map(|(_, s)| *s).collect();
    let settings = SettingSpec::with_defaults(protocol.settings(), settings);
    if count == 0 || start >= count {
        let levels = slots
            .iter()
            .map(|&s| store.initial_level(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let setup = DecoderSetup {
            sample_rate_hz: meta.sample_rate_hz,
            start,
            start_levels: levels,
            channels: bound.into_iter().map(|(b, _)| b).collect(),
            settings,
        };
        return run_decoder(protocol, &setup, |_| Ok(None), count, batching);
    }
    // levels just before `start`, edges from `start` on
    let mut merger = store.merger(&slots, start.saturating_sub(1), count).map_err(|e| e.to_string())?;
    let start_levels = if start == 0 {
        slots
            .iter()
            .map(|&s| store.initial_level(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    } else {
        merger.start_levels().to_vec()
    };
    let setup = DecoderSetup {
        sample_rate_hz: meta.sample_rate_hz,
        start,
        start_levels,
        channels: bound.into_iter().map(|(b, _)| b).collect(),
        settings,
    };
    run_decoder(
        protocol,
        &setup,
        |n| merger.next_batch(n, Sample::MAX).map_err(|e| e.to_string()),
        count,
        batching,
    )
}

/// Decodes the edges of a detection window in one batch (detectors' full decodes).
pub fn decode_window(
    protocol: &dyn Protocol,
    window: &Window,
    sample_rate_hz: f64,
    roles: &Roles,
    settings: &Settings,
) -> Result<DecodeResult, String> {
    let mut bound = Vec::new();
    for r in protocol.roles() {
        if let Some(&Some(ch)) = roles.get(&r.key)
            && let Some(c) = window.channel(ch)
        {
            bound.push((BoundChannel { role: r.key.clone(), channel: ch, name: c.name.clone() }, c));
        } else if !r.optional {
            return Err(format!("Choose a channel for {}.", r.noun));
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    for (slot, (_, c)) in bound.iter().enumerate() {
        let mut level = c.initial;
        for &e in &c.edges {
            level = !level;
            edges.push(Edge { sample: e, slot: u8::try_from(slot).unwrap_or(u8::MAX), level });
        }
    }
    edges.sort_by_key(|e| (e.sample, e.slot));
    let setup = DecoderSetup {
        sample_rate_hz,
        start: window.start,
        start_levels: bound.iter().map(|(_, c)| c.initial).collect::<Vec<Level>>(),
        channels: bound.iter().map(|(b, _)| b.clone()).collect(),
        settings: SettingSpec::with_defaults(protocol.settings(), settings),
    };
    let mut batch = Some(EdgeBatch { edges, end: window.end });
    run_decoder(protocol, &setup, |_| Ok(batch.take()), window.end, Batching::Whole)
}
