//! The protocol engine's data types (doc 06): settings and roles, decoded frames and packets,
//! detections with evidence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use logica_core::Sample;

// ------------------------------------------------------------------ settings and roles

/// A setting's value: a number, a choice (text) or a switch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl From<f64> for SettingValue {
    fn from(v: f64) -> Self {
        SettingValue::Number(v)
    }
}
impl From<u32> for SettingValue {
    fn from(v: u32) -> Self {
        SettingValue::Number(f64::from(v))
    }
}
impl From<bool> for SettingValue {
    fn from(v: bool) -> Self {
        SettingValue::Bool(v)
    }
}
impl From<&str> for SettingValue {
    fn from(v: &str) -> Self {
        SettingValue::Text(v.to_string())
    }
}
impl From<String> for SettingValue {
    fn from(v: String) -> Self {
        SettingValue::Text(v)
    }
}

pub type Settings = BTreeMap<String, SettingValue>;

/// Builds settings from pairs: `settings(&[("baud", 9600.0.into())])`.
pub fn settings(pairs: &[(&str, SettingValue)]) -> Settings {
    pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

/// Typed reads of settings that went through [`SettingSpec::with_defaults`].
pub trait SettingsExt {
    fn num(&self, key: &str) -> f64;
    fn text(&self, key: &str) -> &str;
    fn flag(&self, key: &str) -> bool;
}

impl SettingsExt for Settings {
    fn num(&self, key: &str) -> f64 {
        match self.get(key) {
            Some(SettingValue::Number(n)) => *n,
            Some(SettingValue::Text(t)) => t.parse().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }
    fn text(&self, key: &str) -> &str {
        match self.get(key) {
            Some(SettingValue::Text(t)) => t,
            _ => "",
        }
    }
    fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(SettingValue::Bool(true)))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SettingKind {
    Number { default: f64, unit: Option<String>, min: Option<f64>, max: Option<f64>, presets: Vec<f64> },
    Choice { default: String, options: Vec<(String, String)> },
    Boolean { default: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingSpec {
    pub key: String,
    pub label: String,
    pub help: Option<String>,
    pub kind: SettingKind,
}

impl SettingSpec {
    pub fn number(key: &str, label: &str, default: f64) -> Self {
        SettingSpec {
            key: key.into(),
            label: label.into(),
            help: None,
            kind: SettingKind::Number { default, unit: None, min: None, max: None, presets: Vec::new() },
        }
    }
    pub fn choice(key: &str, label: &str, default: &str, options: &[(&str, &str)]) -> Self {
        SettingSpec {
            key: key.into(),
            label: label.into(),
            help: None,
            kind: SettingKind::Choice {
                default: default.into(),
                options: options.iter().map(|(v, l)| ((*v).into(), (*l).into())).collect(),
            },
        }
    }
    pub fn boolean(key: &str, label: &str, default: bool) -> Self {
        SettingSpec {
            key: key.into(),
            label: label.into(),
            help: None,
            kind: SettingKind::Boolean { default },
        }
    }
    pub fn help(mut self, h: &str) -> Self {
        self.help = Some(h.into());
        self
    }
    pub fn range(mut self, lo: f64, hi: f64, unit: Option<&str>, presets: &[f64]) -> Self {
        if let SettingKind::Number { min, max, unit: u, presets: p, .. } = &mut self.kind {
            *min = Some(lo);
            *max = Some(hi);
            *u = unit.map(String::from);
            *p = presets.to_vec();
        }
        self
    }

    /// The value of this setting in `given`, coerced to the kind, or the default.
    pub fn value(&self, given: &Settings) -> SettingValue {
        let v = given.get(&self.key);
        match &self.kind {
            SettingKind::Number { default, min, max, .. } => {
                let n = match v {
                    Some(SettingValue::Number(n)) => *n,
                    Some(SettingValue::Text(t)) => t.trim().parse().unwrap_or(f64::NAN),
                    _ => f64::NAN,
                };
                // a safety net: the command API (M1) rejects out-of-range values with a message
                let n = if n.is_finite() { n } else { *default };
                SettingValue::Number(n.max(min.unwrap_or(f64::MIN)).min(max.unwrap_or(f64::MAX)))
            }
            SettingKind::Boolean { default } => SettingValue::Bool(match v {
                Some(SettingValue::Bool(b)) => *b,
                Some(SettingValue::Text(t)) if t == "true" => true,
                Some(SettingValue::Text(t)) if t == "false" => false,
                _ => *default,
            }),
            SettingKind::Choice { default, options } => {
                let s = match v {
                    None => default.clone(),
                    Some(SettingValue::Text(t)) => t.clone(),
                    Some(SettingValue::Number(n)) => crate::format::sig(*n, 17),
                    Some(SettingValue::Bool(b)) => b.to_string(),
                };
                SettingValue::Text(if options.iter().any(|(o, _)| *o == s) { s } else { default.clone() })
            }
        }
    }

    /// Settings with defaults filled in and values coerced to their kinds; unknown keys dropped.
    pub fn with_defaults(specs: &[SettingSpec], given: &Settings) -> Settings {
        specs.iter().map(|s| (s.key.clone(), s.value(given))).collect()
    }
}

/// A channel a protocol needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoleSpec {
    pub key: String,
    pub label: String,
    /// Short noun for setup messages: "the data line" in "Choose a channel for the data line."
    pub noun: String,
    pub help: String,
    pub optional: bool,
}

impl RoleSpec {
    pub fn new(key: &str, label: &str, noun: &str, help: &str) -> Self {
        RoleSpec {
            key: key.into(),
            label: label.into(),
            noun: noun.into(),
            help: help.into(),
            optional: false,
        }
    }
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// Role key to physical channel number; `None` = not connected.
pub type Roles = BTreeMap<String, Option<u32>>;

pub fn roles(pairs: &[(&str, Option<u32>)]) -> Roles {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

// ------------------------------------------------------------------ decoded output

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameKind {
    Data,
    Address,
    Start,
    RepeatedStart,
    Stop,
    Ack,
    Nack,
    Break,
    Reset,
    Presence,
    Field,
    Command,
    Sample,
    Cycle,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProblemCode {
    FrameError,
    ParityError,
    Incomplete,
    Glitch,
    Checksum,
    Stuffing,
    Timing,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub code: ProblemCode,
    pub message: String,
}

pub fn problem(code: ProblemCode, message: impl Into<String>) -> Problem {
    Problem { code, message: message.into() }
}

/// One annotation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub start: Sample,
    pub end: Sample,
    /// Physical channel it belongs under.
    pub lane: u32,
    pub kind: FrameKind,
    pub value: Option<f64>,
    /// Short label under the waveform, such as "4F".
    pub label: String,
    /// Longer explanation for the table and tooltips.
    pub detail: Option<String>,
    pub problems: Vec<Problem>,
    /// Index of the packet it belongs to.
    pub packet: u64,
    /// Annotation row under the lane: 0 for bytes and fields, 1 for single bits.
    pub row: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PacketStream {
    /// "MOSI", "Write to 0x50", "RX", …
    pub label: String,
    pub lane: u32,
    pub bytes: Vec<u8>,
}

/// A group of frames (a UART line, an I2C transaction, a chip-select frame) with its bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Packet {
    pub index: u64,
    pub start: Sample,
    pub end: Sample,
    pub title: String,
    pub streams: Vec<PacketStream>,
}

impl Packet {
    pub fn stream(&self, label: &str) -> Option<&PacketStream> {
        self.streams.iter().find(|s| s.label == label)
    }
}

/// A whole decode, collected from a streaming decoder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DecodeResult {
    pub frames: Vec<Frame>,
    pub packets: Vec<Packet>,
    pub problem_counts: BTreeMap<ProblemCode, u64>,
    pub notes: Vec<String>,
}

// ------------------------------------------------------------------ detection

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub text: String,
    /// Whether the observation supports the protocol or counts against it.
    pub supports: bool,
}

pub fn evidence(text: impl Into<String>, supports: bool) -> Evidence {
    Evidence { text: text.into(), supports }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingNote {
    /// 0 to 1.
    pub confidence: f64,
    pub reason: String,
}

pub fn note(confidence: f64, reason: impl Into<String>) -> SettingNote {
    SettingNote { confidence, reason: reason.into() }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub protocol_id: String,
    /// Physical channels this detection explains, ascending.
    pub channels: Vec<u32>,
    pub roles: Roles,
    pub settings: Settings,
    /// Why each setting (or role, keyed "role:<key>") has its value.
    pub notes: BTreeMap<String, SettingNote>,
    /// 0 to 1.
    pub confidence: f64,
    pub evidence: Vec<Evidence>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rejection {
    pub protocol_id: String,
    pub because: String,
}

pub fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_coercion() {
        let specs = vec![
            SettingSpec::number("baud", "Baud", 115_200.0),
            SettingSpec::choice("bits", "Bits", "8", &[("7", "7"), ("8", "8")]),
            SettingSpec::boolean("invert", "Inverted", false),
        ];
        let s = SettingSpec::with_defaults(
            &specs,
            &settings(&[("baud", "9600".into()), ("bits", 7u32.into()), ("x", 1u32.into())]),
        );
        assert_eq!(s.num("baud"), 9600.0);
        assert_eq!(s.text("bits"), "7");
        assert!(!s.flag("invert"));
        assert_eq!(s.len(), 3);
        let s = SettingSpec::with_defaults(
            &specs,
            &settings(&[("baud", "fast".into()), ("bits", "9".into()), ("invert", "true".into())]),
        );
        assert_eq!((s.num("baud"), s.text("bits"), s.flag("invert")), (115_200.0, "8", true));
    }
}
