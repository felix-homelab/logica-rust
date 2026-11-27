//! Virtual analog channels (ADR 0010): sources, add, update, remove, and values for drawing and
//! reading.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::analog::{PointKind, Scaling, code_of_fraction};
use logica_protocols::analog::sources;
use logica_protocols::types::{RoleSpec, Roles, SettingSpec, Settings};

use crate::analog::{Analog, AnalogConfig, AnalogStyle, AnalogSummary};
use crate::contract::{CommandError, ErrorCode, Event};
use crate::engine::{Ctx, Done, Engine, err};
use crate::math::{Expr, MATH};

use super::{Nothing, inv, out};

/// The most buckets `analog.values` draws in one call, as for `waveform.get`.
pub const MAX_VALUE_BUCKETS: u32 = 8192;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogSourceEntry {
    pub id: String,
    pub name: String,
    pub help: String,
    pub roles: Vec<RoleSpec>,
    pub settings: Vec<SettingSpec>,
    /// Chips it knows (ADC presets): settings, scaling and, for several inputs, one channel each.
    pub presets: Vec<logica_protocols::analog::presets::AdcPreset>,
    /// Decoder protocols it can read from instead of its own channels (`spi`, `i2c`, `pwm`).
    pub decoders: Vec<String>,
    /// Its settings that then come from the decoder (the bus's own).
    pub decoder_settings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogSourcesOut {
    pub sources: Vec<AnalogSourceEntry>,
}

pub(crate) fn list_sources(e: &mut Engine, _: &Ctx, _: Nothing) -> Result<Done, CommandError> {
    let presets = logica_protocols::analog::presets::presets();
    let sources: Vec<AnalogSourceEntry> = sources()
        .iter()
        .map(|s| AnalogSourceEntry {
            presets: presets.iter().filter(|p| p.source == s.id()).cloned().collect(),
            decoders: ["spi", "i2c", "pwm"]
                .iter()
                .filter(|p| source_for(p) == Some(s.id()))
                .map(|p| (*p).to_string())
                .collect(),
            decoder_settings: ["spi", "i2c", "pwm"]
                .iter()
                .filter(|p| source_for(p) == Some(s.id()))
                .filter_map(|p| e.registry.get(p))
                .flat_map(|p| p.settings().iter().map(|x| x.key.clone()).collect::<Vec<_>>())
                .filter(|k| s.settings().iter().any(|x| &x.key == k))
                .collect(),
            id: s.id().into(),
            name: s.name().into(),
            help: s.help().into(),
            roles: s.roles().to_vec(),
            settings: s.settings().to_vec(),
        })
        .collect();
    let mut sources = sources;
    // math channels read other channels, not edges: the engine computes them
    sources.push(AnalogSourceEntry {
        id: MATH.into(),
        name: "Math on other channels".into(),
        help: "Computed from other virtual analog channels with a formula, such as A − B or A × 2, combining values sampled at different moments by holding each until its next.".into(),
        roles: Vec::new(),
        settings: Vec::new(),
        presets: Vec::new(),
        decoders: Vec::new(),
        decoder_settings: Vec::new(),
    });
    Ok(Done::query(out(&AnalogSourcesOut { sources })))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalogAddIn {
    pub capture: String,
    /// The source's id (`analog.sources`).
    pub source: String,
    pub roles: Roles,
    #[serde(default)]
    pub settings: Settings,
    /// For a fraction (an ADC code, a PWM duty): `{ "kind": "range", "low", "high", "offset" }`;
    /// for a measured quantity (Hz, s): `{ "kind": "linear", "gain", "offset" }`.
    pub scaling: Scaling,
    pub name: Option<String>,
    pub unit: Option<String>,
    pub style: Option<AnalogStyle>,
    /// Row height in pixels, 40 to 800 (160 unless given).
    pub height: Option<u32>,
    /// A decoder of the capture to read from (SPI and I2C for the ADC sources, PWM for the PWM
    /// level): its channels and bus settings are used instead of `roles`, and follow it when it
    /// changes.
    pub decoder: Option<String>,
    /// For the source `math`: the formula over other virtual analog channels, such as
    /// `{ "sub": [ { "channel": "analog-3" }, { "channel": "analog-4" } ] }`.
    pub expression: Option<Expr>,
    /// Chosen by the engine; redo and undo use the id it had.
    pub id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogOut {
    pub analog: AnalogSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalogRef {
    pub capture: String,
    pub analog: String,
}

fn find<'a>(e: &'a Engine, i: &AnalogRef) -> Result<&'a Analog, CommandError> {
    e.capture(&i.capture)?.analog.get(&i.analog).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("There is no virtual analog channel \"{}\" on this capture.", i.analog),
        )
    })
}

fn compute(e: &Engine, capture: &str, cfg: AnalogConfig) -> Result<Analog, CommandError> {
    let c = e.capture(capture)?;
    let bad = |m: String| err(ErrorCode::InvalidInput, m);
    if cfg.source == MATH {
        // computed from its inputs once it is in the capture (`refresh_math`)
        return Analog::empty(cfg).map_err(bad);
    }
    Analog::compute(cfg, &c.store, &c.meta).map_err(bad)
}

/// Refuses a formula that reads an unknown channel, or the channel `id` itself, naming the chain.
fn no_cycle(e: &Engine, capture: &str, id: &str, cfg: &AnalogConfig) -> Result<(), CommandError> {
    let Some(expr) = &cfg.expression else { return Ok(()) };
    let c = e.capture(capture)?;
    for input in expr.channels() {
        if !c.analog.contains_key(&input) && input != id {
            return Err(err(
                ErrorCode::NotFound,
                format!("The formula reads {input}, which is no virtual analog channel of this capture."),
            ));
        }
    }
    match crate::math::cycle(c, id, expr) {
        Some(path) => {
            let chain = if path.is_empty() { "itself".to_string() } else { path.join(", which reads ") };
            Err(err(
                ErrorCode::InvalidInput,
                format!(
                    "The formula of {id} would read {id} itself, through {chain}; a channel cannot depend on itself."
                ),
            ))
        }
        None => Ok(()),
    }
}

/// Computes the capture's math channels again after a change; tells clients about every one
/// except `but` (whose own command reports it).
pub(crate) fn refresh_math(e: &mut Engine, capture: &str, but: Option<&str>) {
    let Ok(c) = e.capture_mut(capture) else { return };
    for id in crate::math::refresh(c) {
        if Some(id.as_str()) != but {
            e.emit(Event::AnalogUpdated { capture: capture.into(), analog: id });
        }
    }
}

fn default_unit(scaling: &Scaling, output_unit: Option<&str>) -> String {
    match (scaling, output_unit) {
        (Scaling::Linear { gain, .. }, Some(u)) if *gain == 1.0 => u.into(),
        (Scaling::Range { low, high, .. }, _) if *low == 0.0 && *high == 100.0 => "%".into(),
        _ => "V".into(),
    }
}

/// The virtual analog source that reads a decoder of `protocol`.
pub fn source_for(protocol: &str) -> Option<&'static str> {
    match protocol {
        "spi" => Some("spi-adc"),
        "i2c" => Some("i2c-adc"),
        "pwm" => Some("pwm"),
        _ => None,
    }
}

/// Takes a decoder's channels and all its settings (the bus's own, defaults filled in, so a
/// setting the decoder went back to its default for is taken too) into a channel's definition.
fn take_decoder(e: &Engine, cfg: &mut AnalogConfig, d: &crate::state::DecoderConfig) {
    cfg.roles = d.roles.clone();
    let all = e
        .registry
        .get(&d.protocol)
        .map_or_else(|| d.settings.clone(), |p| SettingSpec::with_defaults(p.settings(), &d.settings));
    for (k, v) in all {
        cfg.settings.insert(k, v);
    }
}

/// After a decoder was added, changed or removed: the channels reading from it follow it. They
/// are derived data, so this is part of the decoder's command and its undo, not an entry of its own.
pub(crate) fn follow_decoder(e: &mut Engine, capture: &str, decoder: &str) {
    let Ok(c) = e.capture(capture) else { return };
    let linked: Vec<String> = c
        .analog
        .iter()
        .filter(|(_, a)| a.config.decoder.as_deref() == Some(decoder))
        .map(|(id, _)| id.clone())
        .collect();
    let dcfg = c.decoders.get(decoder).map(|d| d.config.clone());
    for id in linked {
        let Ok(c) = e.capture(capture) else { return };
        let a = &c.analog[&id];
        let next = match &dcfg {
            Some(d) => {
                let mut cfg = a.config.clone();
                take_decoder(e, &mut cfg, d);
                if cfg == a.config {
                    // back after an undo of its removal: nothing to compute, only the note goes
                    let gone = format!("Decoder {decoder} was removed;");
                    let Ok(c) = e.capture_mut(capture) else { return };
                    let a = c.analog.get_mut(&id).expect("listed above");
                    let before = a.result.notes.len();
                    a.result.notes.retain(|n| !n.starts_with(&gone));
                    if a.result.notes.len() != before {
                        e.emit(Event::AnalogUpdated { capture: capture.into(), analog: id });
                    }
                    continue;
                }
                Analog::compute(cfg, &c.store, &c.meta)
                    .map_err(|m| format!("The change of decoder {decoder} could not be followed: {m}"))
            }
            None => Err(format!(
                "Decoder {decoder} was removed; this channel keeps the channels and settings it had and follows it again if it comes back."
            )),
        };
        let Ok(c) = e.capture_mut(capture) else { return };
        match next {
            Ok(a) => {
                c.analog.insert(id.clone(), a);
            }
            Err(note) => {
                let a = c.analog.get_mut(&id).expect("listed above");
                if !a.result.notes.contains(&note) {
                    a.result.notes.push(note);
                }
            }
        }
        e.emit(Event::AnalogUpdated { capture: capture.into(), analog: id });
    }
    refresh_math(e, capture, None);
}

pub(crate) fn add(e: &mut Engine, _: &Ctx, i: AnalogAddIn) -> Result<Done, CommandError> {
    let src = logica_protocols::analog::source(&i.source);
    let name = i.name.clone().unwrap_or_else(|| {
        src.as_ref().map_or_else(
            || if i.source == MATH { "Math".into() } else { "Analog".into() },
            |s| s.name().to_string(),
        )
    });
    let output_unit = src.map(|s| s.output(&SettingSpec::with_defaults(s.settings(), &i.settings)).unit);
    // a math channel is in the unit of the first channel it reads, unless one is given
    let math_unit = i.expression.as_ref().and_then(|x| x.channels().into_iter().next()).and_then(|first| {
        e.capture(&i.capture).ok().and_then(|c| c.analog.get(&first)).map(|a| a.config.unit.clone())
    });
    let unit = i.unit.clone().unwrap_or_else(|| match (i.source == MATH, math_unit) {
        (true, Some(u)) => u,
        _ => default_unit(&i.scaling, output_unit.flatten().as_deref()),
    });
    let cfg = AnalogConfig {
        source: i.source.clone(),
        roles: i.roles.clone(),
        settings: i.settings.clone(),
        scaling: i.scaling,
        name,
        unit,
        style: i.style.unwrap_or_default(),
        height: i.height.unwrap_or(crate::analog::DEFAULT_HEIGHT),
        decoder: i.decoder.clone(),
        expression: i.expression.clone(),
    };
    let mut cfg = cfg;
    if let Some(did) = &i.decoder {
        let c = e.capture(&i.capture)?;
        let d = c.decoders.get(did).ok_or_else(|| {
            err(ErrorCode::NotFound, format!("There is no decoder \"{did}\" on this capture to read from."))
        })?;
        let fits = source_for(&d.config.protocol);
        if fits != Some(i.source.as_str()) {
            return Err(err(
                ErrorCode::InvalidInput,
                match fits {
                    Some(s) => {
                        format!("The {} decoder is read by the source {s}, not {}.", d.config.name, i.source)
                    }
                    None => format!(
                        "No virtual analog source reads a {} decoder; choose its channels instead.",
                        d.config.name
                    ),
                },
            ));
        }
        let dcfg = d.config.clone();
        take_decoder(e, &mut cfg, &dcfg);
    }
    let a = compute(e, &i.capture, cfg)?;
    let id = match &i.id {
        Some(id) => {
            e.reserve_id(id);
            id.clone()
        }
        None => e.new_id("analog-"),
    };
    no_cycle(e, &i.capture, &id, &a.config)?;
    let c = e.capture_mut(&i.capture)?;
    if c.analog.contains_key(&id) {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("There is already a virtual analog channel \"{id}\"."),
        ));
    }
    let cfg = a.config.clone();
    c.analog.insert(id.clone(), a);
    refresh_math(e, &i.capture, Some(&id));
    let summary = AnalogSummary::of(&id, &e.capture(&i.capture)?.analog[&id]);
    e.emit(Event::AnalogUpdated { capture: i.capture.clone(), analog: id.clone() });
    let canonical = AnalogAddIn {
        name: Some(cfg.name.clone()),
        unit: Some(cfg.unit.clone()),
        style: Some(cfg.style),
        height: Some(cfg.height),
        id: Some(id.clone()),
        ..i.clone()
    };
    Ok(Done::edit(
        out(&AnalogOut { analog: summary }),
        inv("analog.remove", &AnalogRef { capture: i.capture, analog: id }),
        format!("added the virtual analog channel {}", cfg.name),
    )
    .canonical(out(&canonical)))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalogUpdateIn {
    pub capture: String,
    pub analog: String,
    pub source: Option<String>,
    pub roles: Option<Roles>,
    /// Merged into the current settings, unless `replaceSettings` is set.
    pub settings: Option<Settings>,
    #[serde(default)]
    pub replace_settings: bool,
    pub scaling: Option<Scaling>,
    pub name: Option<String>,
    pub unit: Option<String>,
    pub style: Option<AnalogStyle>,
    pub height: Option<u32>,
    /// A math channel's new formula.
    pub expression: Option<Expr>,
}

pub(crate) fn update(e: &mut Engine, _: &Ctx, i: AnalogUpdateIn) -> Result<Done, CommandError> {
    let r = AnalogRef { capture: i.capture.clone(), analog: i.analog.clone() };
    let old = find(e, &r)?;
    let before = old.config.clone();
    let mut cfg = before.clone();
    if let Some(s) = &i.source {
        cfg.source = s.clone();
    }
    if let Some(r) = &i.roles {
        cfg.roles = r.clone();
    }
    if let Some(s) = &i.settings {
        if i.replace_settings {
            cfg.settings = s.clone();
        } else {
            cfg.settings.extend(s.clone());
        }
    }
    if let Some(s) = i.scaling {
        cfg.scaling = s;
    }
    if let Some(n) = &i.name {
        cfg.name = n.clone();
    }
    if let Some(u) = &i.unit {
        cfg.unit = u.clone();
    }
    if let Some(s) = i.style {
        cfg.style = s;
    }
    if let Some(h) = i.height {
        cfg.height = h;
    }
    if let Some(x) = &i.expression {
        cfg.expression = Some(x.clone());
    }
    no_cycle(e, &i.capture, &i.analog, &cfg)?;
    // a channel reading from a decoder keeps taking its channels and bus settings from it
    if let Some(did) = cfg.decoder.clone()
        && let Some(d) = e.capture(&i.capture)?.decoders.get(&did).map(|d| d.config.clone())
    {
        take_decoder(e, &mut cfg, &d);
    }
    let new = if cfg.source == MATH || old.needs_recompute(&cfg) {
        compute(e, &i.capture, cfg)?
    } else {
        // Scaling, name, unit and style apply when the series is read: nothing to compute.
        crate::analog::check(&cfg).map_err(|m| err(ErrorCode::InvalidInput, m))?;
        let c = e.capture_mut(&i.capture)?;
        let mut a = c.analog.remove(&i.analog).expect("found above");
        a.config = cfg;
        a
    };
    e.capture_mut(&i.capture)?.analog.insert(i.analog.clone(), new);
    refresh_math(e, &i.capture, Some(&i.analog));
    let summary = AnalogSummary::of(&i.analog, &e.capture(&i.capture)?.analog[&i.analog]);
    e.emit(Event::AnalogUpdated { capture: i.capture.clone(), analog: i.analog.clone() });
    let inverse = AnalogUpdateIn {
        source: Some(before.source),
        roles: Some(before.roles),
        settings: Some(before.settings),
        replace_settings: true,
        scaling: Some(before.scaling),
        name: Some(before.name),
        unit: Some(before.unit),
        style: Some(before.style),
        height: Some(before.height),
        expression: before.expression.clone(),
        ..i.clone()
    };
    Ok(Done::edit(
        out(&AnalogOut { analog: summary.clone() }),
        inv("analog.update", &inverse),
        format!("changed the virtual analog channel {}", summary.config.name),
    ))
}

pub(crate) fn remove(e: &mut Engine, _: &Ctx, i: AnalogRef) -> Result<Done, CommandError> {
    find(e, &i)?;
    let a = e.capture_mut(&i.capture)?.analog.remove(&i.analog).expect("found above");
    e.emit(Event::AnalogUpdated { capture: i.capture.clone(), analog: i.analog.clone() });
    refresh_math(e, &i.capture, None);
    let cfg = a.config;
    let restore = AnalogAddIn {
        capture: i.capture.clone(),
        source: cfg.source,
        roles: cfg.roles,
        settings: cfg.settings,
        scaling: cfg.scaling,
        name: Some(cfg.name.clone()),
        unit: Some(cfg.unit),
        style: Some(cfg.style),
        height: Some(cfg.height),
        decoder: cfg.decoder.clone(),
        expression: cfg.expression.clone(),
        id: Some(i.analog),
    };
    Ok(Done::edit(
        out(&Nothing {}),
        inv("analog.add", &restore),
        format!("removed the virtual analog channel {}", cfg.name),
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalogStatsIn {
    pub capture: String,
    pub analog: String,
    /// The range; the whole capture when absent.
    pub from: Option<Sample>,
    /// Exclusive.
    pub to: Option<Sample>,
}

/// A virtual analog channel over a range, in its unit. Mean and RMS weigh each value by how long it
/// holds; gaps do not count.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogStatsOut {
    pub analog: String,
    pub name: String,
    pub unit: String,
    pub from: Sample,
    pub to: Sample,
    /// Values that start in the range.
    pub count: u64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub peak_to_peak: Option<f64>,
    pub mean: Option<f64>,
    pub rms: Option<f64>,
    /// The share of the range that has a value.
    pub covered: f64,
    pub step: Option<f64>,
}

pub(crate) fn stats(e: &mut Engine, _: &Ctx, i: AnalogStatsIn) -> Result<Done, CommandError> {
    let count = e.capture(&i.capture)?.store.sample_count();
    let from = i.from.unwrap_or(0);
    let to = i.to.unwrap_or(count);
    if to <= from {
        return Err(err(ErrorCode::InvalidInput, "The range is empty: `to` must be after `from`."));
    }
    let a = find(e, &AnalogRef { capture: i.capture.clone(), analog: i.analog.clone() })?;
    let st = a.series.stats(from, to);
    let s = &a.config.scaling;
    let (min, max) = match (st.min, st.max) {
        (Some(lo), Some(hi)) => {
            let (lo, hi) = s.apply_range(lo, hi);
            (Some(lo), Some(hi))
        }
        _ => (None, None),
    };
    // the mean scales like a value; the RMS of a + b·x needs the mean square too
    let (a0, b) = (s.apply(0.0), s.apply(1.0) - s.apply(0.0));
    let rms = match (st.mean, st.mean_square) {
        (Some(m), Some(m2)) => Some((a0 * a0 + 2.0 * a0 * b * m + b * b * m2).max(0.0).sqrt()),
        _ => None,
    };
    #[allow(clippy::cast_precision_loss)]
    let covered = st.covered as f64 / (to - from) as f64;
    Ok(Done::query(out(&AnalogStatsOut {
        analog: i.analog.clone(),
        name: a.config.name.clone(),
        unit: a.config.unit.clone(),
        from,
        to,
        count: st.count,
        peak_to_peak: min.zip(max).map(|(lo, hi)| hi - lo),
        min,
        max,
        mean: st.mean.map(|m| s.apply(m)),
        rms,
        covered,
        step: a.step(),
    })))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalogAtIn {
    pub capture: String,
    /// Sample indices to read every virtual analog channel at (1 to 16), such as the cursors.
    pub at: Vec<Sample>,
}

/// One channel's values at the asked samples, in its unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogAtChannel {
    pub analog: String,
    pub name: String,
    pub unit: String,
    /// One step of the code in the unit, where the source reads codes.
    pub step: Option<f64>,
    /// In the order asked; `value` is absent where the channel has none (before its first value,
    /// in a gap).
    pub values: Vec<AnalogPoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogAtOut {
    pub channels: Vec<AnalogAtChannel>,
}

/// The raw code behind a fraction, where the source reads codes.
fn code_of(a: &crate::analog::Analog, x: f64) -> Option<i64> {
    match (a.output.kind, a.output.bits) {
        (PointKind::Fraction, Some(bits)) => code_of_fraction(x, bits, a.output.encoding),
        _ => None,
    }
}

pub(crate) fn at(e: &mut Engine, _: &Ctx, i: AnalogAtIn) -> Result<Done, CommandError> {
    if i.at.is_empty() || i.at.len() > 16 {
        return Err(err(ErrorCode::InvalidInput, "Give 1 to 16 samples to read the values at."));
    }
    let c = e.capture(&i.capture)?;
    let channels = c
        .analog
        .iter()
        .map(|(id, a)| AnalogAtChannel {
            analog: id.clone(),
            name: a.config.name.clone(),
            unit: a.config.unit.clone(),
            step: a.step(),
            values: i
                .at
                .iter()
                .map(|&s| {
                    let x = a.series.value_at(s);
                    AnalogPoint {
                        sample: s,
                        value: x.map(|x| a.config.scaling.apply(x)),
                        code: x.and_then(|x| code_of(a, x)),
                    }
                })
                .collect(),
        })
        .collect();
    Ok(Done::query(out(&AnalogAtOut { channels })))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalogValuesIn {
    pub capture: String,
    pub analog: String,
    pub from: Sample,
    /// Exclusive.
    pub to: Sample,
    /// How many buckets the range is drawn in (1 to 8192): points when there are at most two per
    /// bucket, else one envelope per bucket.
    pub buckets: u32,
}

/// A value in the channel's unit from `sample` on; `value` is absent in a gap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogPoint {
    pub sample: Sample,
    pub value: Option<f64>,
    /// The raw code, where the source reads codes.
    pub code: Option<i64>,
}

/// One bucket in the channel's unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogBucket {
    /// The value held at the bucket's start.
    pub held: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Values starting in the bucket.
    pub count: u64,
    /// Whether a gap starts in it.
    pub gap: bool,
}

/// A problem in the range: a stretch without a value or with a doubtful one, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogProblem {
    pub start: Sample,
    pub end: Sample,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogValuesOut {
    pub from: Sample,
    pub to: Sample,
    /// Samples per bucket.
    pub bucket: u64,
    pub unit: String,
    /// The value held at `from`, from a point before it.
    pub held: Option<f64>,
    pub points: Option<Vec<AnalogPoint>>,
    pub buckets: Option<Vec<AnalogBucket>>,
    /// At most 1,000.
    pub problems: Vec<AnalogProblem>,
}

pub(crate) fn values(e: &mut Engine, _: &Ctx, i: AnalogValuesIn) -> Result<Done, CommandError> {
    if i.to <= i.from {
        return Err(err(ErrorCode::InvalidInput, "The range is empty: `to` must be after `from`."));
    }
    if !(1..=MAX_VALUE_BUCKETS).contains(&i.buckets) {
        return Err(err(ErrorCode::InvalidInput, format!("Choose 1 to {MAX_VALUE_BUCKETS} buckets.")));
    }
    let a = find(e, &AnalogRef { capture: i.capture.clone(), analog: i.analog.clone() })?;
    let s = &a.config.scaling;
    let bucket = (i.to - i.from).div_ceil(u64::from(i.buckets));
    let limit = 2 * i.buckets as usize;
    let raw: Vec<_> = a.series.points(i.from, i.to).take(limit + 1).collect();
    let code = |x: f64| code_of(a, x);
    let (points, buckets) = if raw.len() <= limit {
        let pts = raw.iter().map(|p| AnalogPoint {
            sample: p.sample,
            value: p.x.map(|x| s.apply(x)),
            code: p.x.and_then(code),
        });
        (Some(pts.collect()), None)
    } else {
        let n = usize::try_from((i.to - i.from).div_ceil(bucket)).unwrap_or(usize::MAX);
        let bs = a.series.envelopes(i.from, bucket, n).into_iter().map(|e| {
            let e = a.scaled(&e);
            AnalogBucket { held: e.held, min: e.min, max: e.max, count: e.count, gap: e.gap }
        });
        (None, Some(bs.collect()))
    };
    let problems = a
        .result
        .frames
        .iter()
        .filter(|f| !f.problems.is_empty() && f.end >= i.from && f.start < i.to)
        .take(1000)
        .map(|f| AnalogProblem {
            start: f.start,
            end: f.end,
            message: f.problems.iter().map(|p| p.message.as_str()).collect::<Vec<_>>().join(" "),
        })
        .collect();
    Ok(Done::query(out(&AnalogValuesOut {
        from: i.from,
        to: i.to,
        bucket,
        unit: a.config.unit.clone(),
        held: a.series.held_before(i.from).map(|x| s.apply(x)),
        points,
        buckets,
        problems,
    })))
}
