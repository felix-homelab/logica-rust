//! Detection, the byte analyser, measurements, search and protocol knowledge.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::measure::measure_channel;
use logica_core::{Level, Sample};
use logica_protocols::analyse::{analyse_window, analysis_start};
use logica_protocols::bytes::{analyse_bytes, parse_hex_bytes};
use logica_protocols::protocol::{Origin, ProtocolInfo};
use logica_protocols::types::{FrameKind, ProblemCode, RoleSpec, SettingSpec};
use logica_protocols::window::{MAX_WINDOW_EDGES, Window};

use crate::contract::{CommandError, ErrorCode};
use crate::engine::{Ctx, Done, Engine, err};
use crate::state::Capture;

use super::{Nothing, inv, out};

fn store_err(e: impl std::fmt::Display) -> CommandError {
    err(ErrorCode::Failed, e.to_string())
}

fn slot_of(c: &Capture, channel: u32) -> Result<usize, CommandError> {
    c.meta
        .channels
        .iter()
        .position(|ch| ch.index == channel)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("This capture has no channel {channel}.")))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeIn {
    pub capture: String,
    /// First sample; after the start-up artefact when absent.
    pub from: Option<Sample>,
    /// End sample (exclusive); the end of the detection window when absent.
    pub to: Option<Sample>,
}

pub(crate) fn detect(e: &mut Engine, _: &Ctx, i: RangeIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    let from = i.from.unwrap_or_else(|| analysis_start(&c.meta));
    let w =
        Window::from_store_range(&c.store, &c.meta, from, i.to.unwrap_or(Sample::MAX)).map_err(store_err)?;
    let a = analyse_window(&w, c.meta.sample_rate_hz, &e.registry);
    Ok(Done::query(out(&a)))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytesIn {
    pub bytes: Option<Vec<u8>>,
    /// Hex as typed by a person: "48 65 6C", "0x48,0x65".
    pub hex: Option<String>,
}

pub(crate) fn bytes(_: &mut Engine, _: &Ctx, i: BytesIn) -> Result<Done, CommandError> {
    let b = match (i.bytes, i.hex) {
        (Some(b), None) => b,
        (None, Some(h)) => parse_hex_bytes(&h).map_err(|m| err(ErrorCode::InvalidInput, m))?,
        _ => return Err(err(ErrorCode::InvalidInput, "Give either bytes or hex.")),
    };
    Ok(Done::query(out(&analyse_bytes(&b))))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasureStatsIn {
    pub capture: String,
    pub channel: u32,
    pub from: Option<Sample>,
    pub to: Option<Sample>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureStatsOut {
    pub channel: u32,
    pub from: Sample,
    pub to: Sample,
    pub seconds: f64,
    /// Level changes in the range (exact).
    pub edges: u64,
    pub rest_level: Level,
    pub frequency_hz: Option<f64>,
    pub period_seconds: Option<f64>,
    pub high_seconds: Option<f64>,
    pub duty_cycle: Option<f64>,
    /// Narrowest pulse that occurs repeatedly.
    pub narrowest_pulse_seconds: Option<f64>,
    /// Pulses well under half the narrowest repeated width.
    pub glitches: u64,
    /// Set when only part of the range was measured.
    pub note: Option<String>,
}

pub(crate) fn stats(e: &mut Engine, _: &Ctx, i: MeasureStatsIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    let slot = slot_of(c, i.channel)?;
    let n = c.store.sample_count();
    let from = i.from.unwrap_or(0).min(n);
    let to = i.to.unwrap_or(n).min(n).max(from);
    let rate = c.meta.sample_rate_hz;
    let initial = if from == 0 { c.store.initial_level(slot) } else { c.store.level_at(slot, from - 1) }
        .map_err(store_err)?;
    let mut edges = Vec::new();
    let mut end = to;
    let mut note = None;
    for x in c.store.edges(slot, from, to).map_err(store_err)? {
        let x = x.map_err(store_err)?;
        if edges.len() >= MAX_WINDOW_EDGES {
            end = x;
            note = Some(format!(
                "The pulse statistics cover samples {from} to {x}; the channel changes too often to measure all of the range."
            ));
            break;
        }
        edges.push(x);
    }
    let m = measure_channel(i.channel, &edges, initial, from, end);
    let p = m.periodic.as_ref();
    let o = MeasureStatsOut {
        channel: i.channel,
        from,
        to,
        seconds: (to - from) as f64 / rate,
        edges: c.store.transitions_in(slot, from, to).map_err(store_err)?,
        rest_level: m.rest_level,
        frequency_hz: p.map(|p| rate / p.period_samples),
        period_seconds: p.map(|p| p.period_samples / rate),
        high_seconds: p.map(|p| p.high_samples / rate),
        duty_cycle: p.map(|p| p.duty_cycle),
        narrowest_pulse_seconds: m.narrowest.map(|x| x.samples / rate),
        glitches: m.glitches,
        note,
    };
    Ok(Done::query(out(&o)))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasureEdgesIn {
    pub capture: String,
    pub channels: Vec<u32>,
    pub from: Option<Sample>,
    pub to: Option<Sample>,
    /// From the previous page's `nextCursor`.
    pub cursor: Option<Sample>,
    /// Edges per page (default 1,000, at least 64, at most 10,000).
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EdgeOut {
    pub channel: u32,
    pub sample: Sample,
    pub seconds: f64,
    /// Level from this sample on.
    pub level: Level,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureEdgesOut {
    pub edges: Vec<EdgeOut>,
    pub next_cursor: Option<Sample>,
}

pub(crate) fn edges(e: &mut Engine, _: &Ctx, i: MeasureEdgesIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    if i.channels.is_empty() {
        return Err(err(ErrorCode::InvalidInput, "Choose at least one channel."));
    }
    let slots: Vec<usize> = i.channels.iter().map(|&ch| slot_of(c, ch)).collect::<Result<_, _>>()?;
    let n = c.store.sample_count();
    let start = i.cursor.or(i.from).unwrap_or(0).min(n);
    let to = i.to.unwrap_or(n).min(n);
    let limit = i.limit.unwrap_or(1000).clamp(64, 10_000);
    let mut o = MeasureEdgesOut { edges: Vec::new(), next_cursor: None };
    if start < to {
        // edges from `start` on: the merger delivers edges after its own start sample
        let mut m = c.store.merger(&slots, start.saturating_sub(1), to).map_err(store_err)?;
        if let Some(b) = m.next_batch(limit + 1, to).map_err(store_err)? {
            let mut list: Vec<_> = b.edges.into_iter().filter(|x| x.sample >= start).collect();
            if list.len() > limit {
                let cut = list[limit].sample;
                list.retain(|x| x.sample < cut);
                o.next_cursor = Some(cut);
            }
            o.edges = list
                .into_iter()
                .map(|x| EdgeOut {
                    channel: i.channels[usize::from(x.slot)],
                    sample: x.sample,
                    seconds: c.seconds(x.sample),
                    level: x.level,
                })
                .collect();
        }
    }
    Ok(Done::query(out(&o)))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasureBetweenIn {
    pub capture: String,
    pub from: Sample,
    pub to: Sample,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureBetweenOut {
    pub samples: i128,
    pub seconds: f64,
    /// 1 ÷ the time, when it is not zero.
    pub frequency_hz: Option<f64>,
}

pub(crate) fn between(e: &mut Engine, _: &Ctx, i: MeasureBetweenIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    let samples = i128::from(i.to) - i128::from(i.from);
    let seconds = samples as f64 / c.meta.sample_rate_hz;
    let o =
        MeasureBetweenOut { samples, seconds, frequency_hz: (seconds != 0.0).then(|| 1.0 / seconds.abs()) };
    Ok(Done::query(out(&o)))
}

// ------------------------------------------------------------------ search

/// A search (doc 06): a typed structure, not a text grammar. `decoded`, `edge` and `pulse` produce
/// hits; `within`, `level`, `not` and further producers filter them inside `all`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Query {
    All(Vec<Query>),
    Any(Vec<Query>),
    Not(Box<Query>),
    Decoded(DecodedQuery),
    Edge(EdgeQuery),
    Pulse(PulseQuery),
    Level(LevelQuery),
    Within(WithinQuery),
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecodedQuery {
    /// A decoder id on the capture.
    pub decoder: String,
    pub kind: Option<FrameKind>,
    /// Exact label, such as "NACK" or "4F".
    pub label: Option<String>,
    /// Contained in the label or detail (any case).
    pub text: Option<String>,
    pub value: Option<f64>,
    /// Only frames with this problem.
    pub problem: Option<ProblemCode>,
    /// Only frames with any problem.
    #[serde(default)]
    pub has_problem: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Rising,
    Falling,
    Any,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EdgeQuery {
    pub channel: u32,
    pub direction: Direction,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PulseQuery {
    pub channel: u32,
    pub level: Level,
    /// Seconds.
    pub shorter_than: Option<f64>,
    /// Seconds.
    pub longer_than: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LevelQuery {
    pub channel: u32,
    pub level: Level,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    S,
    Ms,
    Us,
    Ns,
    Samples,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WithinQuery {
    pub from: f64,
    pub to: f64,
    /// Seconds by default.
    pub unit: Option<Unit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchIn {
    pub capture: String,
    pub query: Query,
    /// Most hits (default 100, at most 1,000).
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub start: Sample,
    pub end: Sample,
    pub seconds: f64,
    /// What was found: a frame label, "rising edge on CH5", "high pulse of 80 ns on CH3".
    pub what: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchOut {
    pub hits: Vec<Hit>,
    /// More hits exist than were returned.
    pub truncated: bool,
}

/// Edges scanned at most by one producer, to keep a search bounded.
const SCAN_LIMIT: usize = 50_000_000;

struct Searcher<'a> {
    c: &'a Capture,
}

impl Searcher<'_> {
    fn range(&self, w: &WithinQuery) -> (Sample, Sample) {
        let k = match w.unit.unwrap_or(Unit::S) {
            Unit::S => self.c.meta.sample_rate_hz,
            Unit::Ms => self.c.meta.sample_rate_hz / 1e3,
            Unit::Us => self.c.meta.sample_rate_hz / 1e6,
            Unit::Ns => self.c.meta.sample_rate_hz / 1e9,
            Unit::Samples => 1.0,
        };
        (crate_num::floor(w.from * k), crate_num::ceil(w.to * k))
    }

    fn is_producer(q: &Query) -> bool {
        matches!(q, Query::Decoded(_) | Query::Edge(_) | Query::Pulse(_) | Query::Any(_))
    }

    /// Hits of a producer inside `[from, to)`, at most `limit + 1`.
    fn produce(&self, q: &Query, from: Sample, to: Sample, limit: usize) -> Result<Vec<Hit>, CommandError> {
        let c = self.c;
        let rate = c.meta.sample_rate_hz;
        let hit = |start, end, what: String| Hit { start, end, seconds: start as f64 / rate, what };
        let mut out = Vec::new();
        match q {
            Query::Decoded(d) => {
                let dec = c.decoders.get(&d.decoder).ok_or_else(|| {
                    err(
                        ErrorCode::NotFound,
                        format!("There is no decoder \"{}\" on this capture.", d.decoder),
                    )
                })?;
                let text = d.text.as_ref().map(|t| t.to_lowercase());
                for f in &dec.result.frames {
                    if f.end <= from || f.start >= to {
                        continue;
                    }
                    let ok = d.kind.is_none_or(|k| f.kind == k)
                        && d.label.as_ref().is_none_or(|l| *l == f.label)
                        && d.value.is_none_or(|v| f.value == Some(v))
                        && d.problem.is_none_or(|p| f.problems.iter().any(|x| x.code == p))
                        && (!d.has_problem || !f.problems.is_empty())
                        && text.as_ref().is_none_or(|t| {
                            f.label.to_lowercase().contains(t)
                                || f.detail.as_ref().is_some_and(|x| x.to_lowercase().contains(t))
                        });
                    if ok {
                        out.push(hit(f.start, f.end, f.label.clone()));
                        if out.len() > limit {
                            break;
                        }
                    }
                }
            }
            Query::Edge(q) => {
                let slot = slot_of(c, q.channel)?;
                for (k, x) in c.store.edges(slot, from, to).map_err(store_err)?.enumerate() {
                    let s = x.map_err(store_err)?;
                    if k > SCAN_LIMIT {
                        break;
                    }
                    let rising = c.store.level_at(slot, s).map_err(store_err)? == Level::High;
                    let ok = match q.direction {
                        Direction::Any => true,
                        Direction::Rising => rising,
                        Direction::Falling => !rising,
                    };
                    if ok {
                        out.push(hit(
                            s,
                            s + 1,
                            format!("{} edge on CH{}", if rising { "rising" } else { "falling" }, q.channel),
                        ));
                        if out.len() > limit {
                            break;
                        }
                    }
                }
            }
            Query::Pulse(q) => {
                let slot = slot_of(c, q.channel)?;
                let short = q.shorter_than.map(|s| s * rate);
                let long = q.longer_than.map(|s| s * rate);
                let mut prev: Option<(Sample, Level)> = None;
                for (k, x) in c.store.edges(slot, from, to).map_err(store_err)?.enumerate() {
                    let s = x.map_err(store_err)?;
                    if k > SCAN_LIMIT {
                        break;
                    }
                    if let Some((p, level)) = prev {
                        let w = (s - p) as f64;
                        if level == q.level && short.is_none_or(|v| w < v) && long.is_none_or(|v| w > v) {
                            let name = if level == Level::High { "high" } else { "low" };
                            out.push(hit(
                                p,
                                s,
                                format!(
                                    "{name} pulse of {} on CH{}",
                                    logica_protocols::format::time(w / rate),
                                    q.channel
                                ),
                            ));
                            if out.len() > limit {
                                break;
                            }
                        }
                    }
                    prev = Some((s, c.store.level_at(slot, s).map_err(store_err)?));
                }
            }
            Query::Any(list) => {
                for q in list {
                    if !Self::is_producer(q) {
                        return Err(err(ErrorCode::InvalidInput, "Inside any, use decoded, edge or pulse."));
                    }
                    out.extend(self.produce(q, from, to, limit)?);
                }
                out.sort_by_key(|h| (h.start, h.end));
                out.dedup();
            }
            _ => {
                return Err(err(
                    ErrorCode::InvalidInput,
                    "A search needs decoded, edge or pulse to find something.",
                ));
            }
        }
        Ok(out)
    }

    /// Whether `q` holds for a hit.
    fn holds(&self, q: &Query, h: &Hit) -> Result<bool, CommandError> {
        Ok(match q {
            Query::Within(w) => {
                let (a, b) = self.range(w);
                h.start < b && h.end > a
            }
            Query::Level(l) => {
                let slot = slot_of(self.c, l.channel)?;
                self.c.store.level_at(slot, h.start).map_err(store_err)? == l.level
            }
            Query::Not(x) => !self.holds(x, h)?,
            Query::All(xs) => {
                for x in xs {
                    if !self.holds(x, h)? {
                        return Ok(false);
                    }
                }
                true
            }
            p => !self.produce(p, h.start, h.end.max(h.start + 1), 0)?.is_empty(),
        })
    }

    fn run(&self, q: &Query, limit: usize) -> Result<Vec<Hit>, CommandError> {
        let n = self.c.store.sample_count();
        match q {
            Query::All(xs) => {
                let first = xs.iter().find(|x| Self::is_producer(x)).ok_or_else(|| {
                    err(ErrorCode::InvalidInput, "A search needs decoded, edge or pulse to find something.")
                })?;
                // `within` limits where the producer looks
                let (mut from, mut to) = (0, n);
                for x in xs {
                    if let Query::Within(w) = x {
                        let (a, b) = self.range(w);
                        from = from.max(a);
                        to = to.min(b);
                    }
                }
                let mut hits = Vec::new();
                let mut cursor = from;
                // produce in rounds, so filters that reject many hits still fill the page
                while cursor < to && hits.len() <= limit {
                    let batch = self.produce(first, cursor, to, 4 * limit + 16)?;
                    let Some(last) = batch.last() else { break };
                    let next = last.start + 1;
                    for h in batch {
                        let mut ok = true;
                        for x in xs.iter().filter(|x| !std::ptr::eq(*x, first)) {
                            if !self.holds(x, &h)? {
                                ok = false;
                                break;
                            }
                        }
                        if ok && !hits.contains(&h) {
                            hits.push(h);
                        }
                    }
                    if next <= cursor {
                        break;
                    }
                    cursor = next;
                }
                Ok(hits)
            }
            q if Self::is_producer(q) => self.produce(q, 0, n, limit),
            _ => {
                Err(err(ErrorCode::InvalidInput, "A search needs decoded, edge or pulse to find something."))
            }
        }
    }
}

mod crate_num {
    pub use logica_protocols::num::{ceil, floor};
}

pub(crate) fn search(e: &mut Engine, _: &Ctx, i: SearchIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    let limit = i.limit.unwrap_or(100).clamp(1, 1000);
    let mut hits = Searcher { c }.run(&i.query, limit)?;
    let truncated = hits.len() > limit;
    hits.truncate(limit);
    Ok(Done::query(out(&SearchOut { hits, truncated })))
}

// ------------------------------------------------------------------ protocols

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolEntry {
    pub id: String,
    pub name: String,
    pub group: String,
    pub origin: Origin,
    pub roles: Vec<RoleSpec>,
    pub settings: Vec<SettingSpec>,
    pub has_decoder: bool,
    pub alias_of: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolListOut {
    pub protocols: Vec<ProtocolEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolRef {
    pub protocol: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolInfoOut {
    pub protocol: ProtocolEntry,
    pub info: ProtocolInfo,
}

fn entry(p: &dyn logica_protocols::Protocol) -> ProtocolEntry {
    ProtocolEntry {
        id: p.id().into(),
        name: p.name().into(),
        group: p.group().into(),
        origin: p.origin(),
        roles: p.roles().to_vec(),
        settings: p.settings().to_vec(),
        has_decoder: p.has_decoder(),
        alias_of: p.alias_of().map(String::from),
    }
}

pub(crate) fn protocol_list(e: &mut Engine, _: &Ctx, _: Nothing) -> Result<Done, CommandError> {
    let protocols = e.registry.all().iter().map(|p| entry(p.as_ref())).collect();
    Ok(Done::query(out(&ProtocolListOut { protocols })))
}

pub(crate) fn protocol_info(e: &mut Engine, _: &Ctx, i: ProtocolRef) -> Result<Done, CommandError> {
    let p = e.registry.get(&i.protocol).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("There is no protocol \"{}\". protocol.list shows them all.", i.protocol),
        )
    })?;
    Ok(Done::query(out(&ProtocolInfoOut { protocol: entry(p.as_ref()), info: p.info().clone() })))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddCustomIn {
    /// A custom protocol as JSON: the "pulse-code" or "preset" family (doc 06).
    pub config: serde_json::Value,
}

pub(crate) fn add_custom(e: &mut Engine, _: &Ctx, i: AddCustomIn) -> Result<Done, CommandError> {
    let built_in: Vec<String> = logica_protocols::built_in().iter().map(|p| p.id().to_string()).collect();
    let ids: Vec<&str> = built_in.iter().map(String::as_str).collect();
    let cfg = logica_protocols::custom::validate(&i.config, &ids).map_err(|errors| {
        err(ErrorCode::InvalidInput, format!("The custom protocol has problems: {}", errors.join(" ")))
    })?;
    if e.custom.iter().any(|p| p.id() == cfg.id()) {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("A custom protocol \"{}\" exists already; remove it first.", cfg.id()),
        ));
    }
    let reg = e.registry.clone();
    let lookup = |id: &str| reg.get(id).cloned();
    let p: Arc<dyn logica_protocols::Protocol> =
        logica_protocols::custom::compile(cfg, &lookup, Origin::Custom)
            .map_err(|m| err(ErrorCode::InvalidInput, m))?;
    let o = entry(p.as_ref());
    e.custom.push(p);
    e.registry = logica_protocols::registry_with(e.custom.clone());
    let back = ProtocolRef { protocol: o.id.clone() };
    let _ = inv("protocol.removeCustom", &back);
    Ok(Done::never(out(&o), format!("added the custom protocol {}", o.name)))
}

pub(crate) fn remove_custom(e: &mut Engine, _: &Ctx, i: ProtocolRef) -> Result<Done, CommandError> {
    let k = e.custom.iter().position(|p| p.id() == i.protocol).ok_or_else(|| {
        err(ErrorCode::NotFound, format!("There is no custom protocol \"{}\".", i.protocol))
    })?;
    let p = e.custom.remove(k);
    e.registry = logica_protocols::registry_with(e.custom.clone());
    Ok(Done::never(out(&Nothing {}), format!("removed the custom protocol {}", p.name())))
}

// ------------------------------------------------------------------ render

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderIn {
    pub capture: String,
    pub from: Option<Sample>,
    pub to: Option<Sample>,
    /// Pixels (default 1200).
    pub width: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenderOut {
    /// The PNG, base64-encoded.
    pub png_base64: String,
    pub width: u32,
    pub height: u32,
}

pub(crate) fn render(_: &mut Engine, _: &Ctx, _: RenderIn) -> Result<Done, CommandError> {
    Err(err(ErrorCode::Failed, "render is not built yet; it comes with the MCP server (M1 step 4)."))
}
