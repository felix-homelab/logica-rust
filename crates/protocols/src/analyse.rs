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
use crate::stream::{BoundChannel, Collected, DecoderSetup, StreamDecoder, collect_into};
use crate::types::{
    DecodeResult, Detection, Rejection, RoleSpec, Roles, SettingSpec, SettingValue, Settings,
};
use crate::window::Window;

/// Below this confidence a detection is listed as an alternative only.
pub const MIN_CONFIDENCE: f64 = 0.4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
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

/// What a decode of a capture needs: roles, settings and a streaming decoder. Protocols (through
/// [`Decoding`]) and virtual analog sources both provide it, so they share one driver.
pub trait Decodes {
    fn role_specs(&self) -> &[RoleSpec];
    fn setting_specs(&self) -> &[SettingSpec];
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String>;
}

/// A protocol as something that decodes.
pub struct Decoding<'a>(pub &'a dyn Protocol);

impl Decodes for Decoding<'_> {
    fn role_specs(&self) -> &[RoleSpec] {
        self.0.roles()
    }
    fn setting_specs(&self) -> &[SettingSpec] {
        self.0.settings()
    }
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        self.0.decoder(setup, snapshot)
    }
}

/// A virtual analog source as something that decodes.
pub struct Sourcing<'a>(pub &'a dyn crate::analog::Source);

impl Decodes for Sourcing<'_> {
    fn role_specs(&self) -> &[RoleSpec] {
        self.0.roles()
    }
    fn setting_specs(&self) -> &[SettingSpec] {
        self.0.settings()
    }
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        self.0.create(setup, snapshot)
    }
}

/// Binds roles to the capture's channels: the bound channels in role order with their store
/// slots, or a message for the person.
pub fn bind_roles(
    protocol: &dyn Protocol,
    meta: &CaptureMeta,
    roles: &Roles,
) -> Result<Vec<(BoundChannel, usize)>, String> {
    bind_role_specs(protocol.roles(), meta, roles)
}

/// [`bind_roles`] for any list of roles.
pub fn bind_role_specs(
    specs: &[RoleSpec],
    meta: &CaptureMeta,
    roles: &Roles,
) -> Result<Vec<(BoundChannel, usize)>, String> {
    let mut out = Vec::new();
    for r in specs {
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
    protocol: &dyn Decodes,
    setup: &DecoderSetup,
    mut next: impl FnMut(usize) -> Result<Option<EdgeBatch>, String>,
    end: Sample,
    batching: Batching,
) -> Result<DecodeResult, String> {
    let mut dec: Box<dyn StreamDecoder> = protocol.create(setup, None)?;
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
            dec = protocol.create(setup, Some(snap))?;
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
    decode_store_with(&Decoding(protocol), store, meta, roles, settings, batching)
}

/// [`decode_store`] for anything that decodes: a protocol or a virtual analog source.
pub fn decode_store_with(
    protocol: &dyn Decodes,
    store: &CaptureStore,
    meta: &CaptureMeta,
    roles: &Roles,
    settings: &Settings,
    batching: Batching,
) -> Result<DecodeResult, String> {
    let bound = bind_role_specs(protocol.role_specs(), meta, roles)?;
    let start = analysis_start(meta).min(store.sample_count());
    let count = store.sample_count();
    let slots: Vec<usize> = bound.iter().map(|(_, s)| *s).collect();
    let settings = SettingSpec::with_defaults(protocol.setting_specs(), settings);
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

/// A decode that follows a capture while it grows (live capture, doc 06): every
/// [`LiveDecode::advance`] pushes the edges that arrived since the last one into the same
/// streaming decoder, so the work per update follows the new data, not the capture's length.
pub struct LiveDecode {
    decoder: Box<dyn StreamDecoder>,
    slots: Vec<usize>,
    /// Every edge before this sample has been pushed.
    pos: Sample,
}

impl LiveDecode {
    /// Starts a decode of `store` as it is now; nothing is decoded until the first `advance`.
    pub fn new(
        registry: &Registry,
        store: &CaptureStore,
        meta: &CaptureMeta,
        protocol_id: &str,
        roles: &Roles,
        settings: &Settings,
    ) -> Result<Self, String> {
        Self::with(&Decoding(decoding_protocol(registry, protocol_id)?), store, meta, roles, settings)
    }

    /// [`LiveDecode::new`] for anything that decodes: a protocol or a virtual analog source.
    pub fn with(
        protocol: &dyn Decodes,
        store: &CaptureStore,
        meta: &CaptureMeta,
        roles: &Roles,
        settings: &Settings,
    ) -> Result<Self, String> {
        let bound = bind_role_specs(protocol.role_specs(), meta, roles)?;
        let slots: Vec<usize> = bound.iter().map(|(_, s)| *s).collect();
        let start = analysis_start(meta).min(store.sample_count());
        let start_levels = slots
            .iter()
            .map(|&s| if start == 0 { store.initial_level(s) } else { store.level_at(s, start - 1) })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let setup = DecoderSetup {
            sample_rate_hz: meta.sample_rate_hz,
            start,
            start_levels,
            channels: bound.into_iter().map(|(b, _)| b).collect(),
            settings: SettingSpec::with_defaults(protocol.setting_specs(), settings),
        };
        Ok(LiveDecode { decoder: protocol.create(&setup, None)?, slots, pos: start })
    }

    /// Pushes the edges up to the store's current end and appends what the decoder emitted to
    /// `into`. Returns whether anything was appended.
    pub fn advance(&mut self, store: &CaptureStore, into: &mut DecodeResult) -> Result<bool, String> {
        let count = store.sample_count();
        if count <= self.pos {
            return Ok(false);
        }
        // the merger delivers edges after its start sample: from `pos` on
        let mut merger =
            store.merger(&self.slots, self.pos.saturating_sub(1), count).map_err(|e| e.to_string())?;
        let before = (into.frames.len(), into.packets.len(), into.notes.len(), into.points.len());
        while let Some(b) = merger.next_batch(1 << 16, Sample::MAX).map_err(|e| e.to_string())? {
            self.decoder.push(&b);
            collect_into(into, self.decoder.drain());
        }
        self.pos = count;
        Ok((into.frames.len(), into.packets.len(), into.notes.len(), into.points.len()) != before)
    }

    /// The capture ended at `end`: pushes the rest and lets the decoder emit what is pending.
    pub fn finish(
        mut self,
        store: &CaptureStore,
        end: Sample,
        into: &mut DecodeResult,
    ) -> Result<(), String> {
        self.advance(store, into)?;
        self.decoder.finish(end);
        collect_into(into, self.decoder.drain());
        Ok(())
    }
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
    run_decoder(&Decoding(protocol), &setup, |_| Ok(batch.take()), window.end, Batching::Whole)
}
