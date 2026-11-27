//! Virtual analog channels on a capture (ADR 0010): the definition a person or agent gives, the
//! value series computed from it, and the live computation that follows a running capture.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::analog::{Envelope, Scaling, ValueSeries};
use logica_core::capture::{CaptureMeta, CaptureState};
use logica_core::store::CaptureStore;
use logica_protocols::analog::{Source, SourceOutput, source};
use logica_protocols::analyse::{Batching, LiveDecode, Sourcing, decode_store_with};
use logica_protocols::types::{DecodeResult, ProblemCode, Roles, SettingSpec, Settings};

/// How a channel is drawn: the value held until the next point, or a line between points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AnalogStyle {
    #[default]
    Step,
    Line,
}

/// A virtual analog channel's definition: what `analog.add` takes, an undo restores and a
/// `.lacap` saves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogConfig {
    /// The source's id (`analog.sources` lists them).
    pub source: String,
    pub roles: Roles,
    #[serde(default)]
    pub settings: Settings,
    pub scaling: Scaling,
    pub name: String,
    /// The unit of its values, such as `V` or `%`.
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub style: AnalogStyle,
    /// Height of its row in the waveform, in pixels (40 to 800).
    #[serde(default = "default_height")]
    pub height: u32,
    /// The decoder it reads from, if any: its channels and bus settings follow that decoder's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder: Option<String>,
}

/// A row tall enough to read a curve.
pub const DEFAULT_HEIGHT: u32 = 160;
/// The rows a person can still read, and no taller than a screen.
pub const HEIGHTS: std::ops::RangeInclusive<u32> = 40..=800;

fn default_height() -> u32 {
    DEFAULT_HEIGHT
}

pub struct Analog {
    pub config: AnalogConfig,
    /// What the source's points mean.
    pub output: SourceOutput,
    pub series: ValueSeries,
    /// Problems (as frames), notes and their counts; the points move to `series`.
    pub result: DecodeResult,
    /// The computation following the capture while it is captured.
    pub live: Option<LiveDecode>,
}

fn find_source(id: &str) -> Result<Box<dyn Source>, String> {
    source(id)
        .ok_or_else(|| format!("There is no virtual analog source \"{id}\"; analog.sources lists them."))
}

/// Checks a definition: the source exists and the scaling fits what its points mean.
pub fn check(cfg: &AnalogConfig) -> Result<SourceOutput, String> {
    if !HEIGHTS.contains(&cfg.height) {
        return Err(format!(
            "A row is {} to {} pixels high; {} is not possible.",
            HEIGHTS.start(),
            HEIGHTS.end(),
            cfg.height
        ));
    }
    let src = find_source(&cfg.source)?;
    let output = src.output(&SettingSpec::with_defaults(src.settings(), &cfg.settings));
    cfg.scaling.validate(output.kind).map_err(|e| e.to_string())?;
    Ok(output)
}

impl Analog {
    /// Computes a channel over a capture as it is now: a finished capture whole, a running one up
    /// to its newest data and then live.
    pub fn compute(config: AnalogConfig, store: &CaptureStore, meta: &CaptureMeta) -> Result<Self, String> {
        let output = check(&config)?;
        let src = find_source(&config.source)?;
        let d = Sourcing(src.as_ref());
        let mut a = Analog {
            config,
            output,
            series: ValueSeries::new(),
            result: DecodeResult::default(),
            live: None,
        };
        if meta.state == CaptureState::Capturing {
            let mut live = LiveDecode::with(&d, store, meta, &a.config.roles, &a.config.settings)?;
            live.advance(store, &mut a.result)?;
            a.live = Some(live);
        } else {
            a.result =
                decode_store_with(&d, store, meta, &a.config.roles, &a.config.settings, Batching::Whole)?;
        }
        a.take_points();
        Ok(a)
    }

    /// Whether `new` needs the series computed again (anything but scaling, name, unit, style).
    pub fn needs_recompute(&self, new: &AnalogConfig) -> bool {
        (&self.config.source, &self.config.roles, &self.config.settings)
            != (&new.source, &new.roles, &new.settings)
    }

    fn take_points(&mut self) {
        for p in std::mem::take(&mut self.result.points) {
            let r = match p.x {
                Some(x) => self.series.push(p.sample, x),
                None => self.series.gap(p.sample),
            };
            if let Err(e) = r {
                self.result.notes.push(format!("A value was left out: {e}"));
            }
        }
    }

    /// Takes in what arrived since the last call. Returns whether anything changed.
    pub fn advance_live(&mut self, store: &CaptureStore) -> bool {
        let Some(live) = self.live.as_mut() else { return false };
        let changed = match live.advance(store, &mut self.result) {
            Ok(changed) => changed,
            Err(e) => {
                self.live = None;
                self.result.notes.push(format!("Live computing stopped: {e}"));
                true
            }
        };
        self.take_points();
        changed
    }

    /// The capture ended at `end`: computes the rest. Returns whether anything changed.
    pub fn finish_live(&mut self, store: &CaptureStore, end: Sample) -> bool {
        let Some(live) = self.live.take() else { return false };
        if let Err(e) = live.finish(store, end, &mut self.result) {
            self.result.notes.push(format!("Live computing stopped: {e}"));
        }
        self.take_points();
        true
    }

    /// One step of the code in the channel's unit, where the points are codes.
    pub fn step(&self) -> Option<f64> {
        self.config.scaling.step(self.output.bits?)
    }

    /// An envelope in the channel's unit.
    pub fn scaled(&self, e: &Envelope) -> Envelope {
        let s = &self.config.scaling;
        let (min, max) = match (e.min, e.max) {
            (Some(lo), Some(hi)) => {
                let (a, b) = s.apply_range(lo, hi);
                (Some(a), Some(b))
            }
            _ => (None, None),
        };
        Envelope { held: e.held.map(|x| s.apply(x)), min, max, ..*e }
    }
}

/// What `session.get` says about a virtual analog channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogSummary {
    pub id: String,
    pub config: AnalogConfig,
    pub output: SourceOutput,
    /// Entries in the series (values and gaps).
    pub points: usize,
    /// Lowest and highest value, in the channel's unit.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// One step of the code in the channel's unit, where the points are codes.
    pub step: Option<f64>,
    pub problem_counts: BTreeMap<ProblemCode, u64>,
    pub notes: Vec<String>,
    /// Still following a running capture.
    pub live: bool,
}

impl AnalogSummary {
    pub fn of(id: &str, a: &Analog) -> Self {
        let e = a.scaled(&a.series.envelope(0, Sample::MAX));
        AnalogSummary {
            id: id.into(),
            config: a.config.clone(),
            output: a.output.clone(),
            points: a.series.len(),
            min: e.min,
            max: e.max,
            step: a.step(),
            problem_counts: a.result.problem_counts.clone(),
            notes: a.result.notes.clone(),
            live: a.live.is_some(),
        }
    }
}
