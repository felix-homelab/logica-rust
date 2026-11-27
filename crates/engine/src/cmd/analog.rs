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
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogSourcesOut {
    pub sources: Vec<AnalogSourceEntry>,
}

pub(crate) fn list_sources(_: &mut Engine, _: &Ctx, _: Nothing) -> Result<Done, CommandError> {
    let presets = logica_protocols::analog::presets::presets();
    let sources = sources()
        .iter()
        .map(|s| AnalogSourceEntry {
            presets: presets.iter().filter(|p| p.source == s.id()).cloned().collect(),
            id: s.id().into(),
            name: s.name().into(),
            help: s.help().into(),
            roles: s.roles().to_vec(),
            settings: s.settings().to_vec(),
        })
        .collect();
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
    Analog::compute(cfg, &c.store, &c.meta).map_err(|m| err(ErrorCode::InvalidInput, m))
}

fn default_unit(scaling: &Scaling, output_unit: Option<&str>) -> String {
    match (scaling, output_unit) {
        (Scaling::Linear { gain, .. }, Some(u)) if *gain == 1.0 => u.into(),
        (Scaling::Range { low, high, .. }, _) if *low == 0.0 && *high == 100.0 => "%".into(),
        _ => "V".into(),
    }
}

pub(crate) fn add(e: &mut Engine, _: &Ctx, i: AnalogAddIn) -> Result<Done, CommandError> {
    let src = logica_protocols::analog::source(&i.source);
    let name =
        i.name.clone().unwrap_or_else(|| src.as_ref().map_or("Analog".into(), |s| s.name().to_string()));
    let output_unit = src.map(|s| s.output(&SettingSpec::with_defaults(s.settings(), &i.settings)).unit);
    let unit = i.unit.clone().unwrap_or_else(|| default_unit(&i.scaling, output_unit.flatten().as_deref()));
    let cfg = AnalogConfig {
        source: i.source.clone(),
        roles: i.roles.clone(),
        settings: i.settings.clone(),
        scaling: i.scaling,
        name,
        unit,
        style: i.style.unwrap_or_default(),
        height: i.height.unwrap_or(crate::analog::DEFAULT_HEIGHT),
    };
    let a = compute(e, &i.capture, cfg)?;
    let id = match &i.id {
        Some(id) => {
            e.reserve_id(id);
            id.clone()
        }
        None => e.new_id("analog-"),
    };
    let c = e.capture_mut(&i.capture)?;
    if c.analog.contains_key(&id) {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("There is already a virtual analog channel \"{id}\"."),
        ));
    }
    let summary = AnalogSummary::of(&id, &a);
    let cfg = a.config.clone();
    c.analog.insert(id.clone(), a);
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
    let new = if old.needs_recompute(&cfg) {
        compute(e, &i.capture, cfg)?
    } else {
        // Scaling, name, unit and style apply when the series is read: nothing to compute.
        crate::analog::check(&cfg).map_err(|m| err(ErrorCode::InvalidInput, m))?;
        let c = e.capture_mut(&i.capture)?;
        let mut a = c.analog.remove(&i.analog).expect("found above");
        a.config = cfg;
        a
    };
    let summary = AnalogSummary::of(&i.analog, &new);
    e.capture_mut(&i.capture)?.analog.insert(i.analog.clone(), new);
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
