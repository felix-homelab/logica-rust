//! Custom protocols written as JSON (doc 06): data, not code, so sharing them is safe. Two
//! families: "pulse-code" (a single line where bit values are pulse lengths, see
//! [`crate::pulsecode`]) and "preset" (an existing protocol with fixed settings and its own name,
//! such as DMX512 = UART at 250,000 baud, 8N2; detected when the base protocol is detected with
//! matching settings, and decoded by the base decoder).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::protocol::{DetectContext, DetectorOutput, Origin, Protocol, ProtocolInfo};
use crate::pulsecode::{InfoSpec, PulseCode, PulseCodeConfig};
use crate::stream::{DecoderSetup, StreamDecoder};
use crate::types::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetConfig {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    /// Id of the protocol this is a variant of, such as "uart".
    pub base: String,
    pub settings: Settings,
    /// Settings that must match for a detection to count (default: all of `settings`). Numbers
    /// match within 3 %.
    pub r#match: Option<Vec<String>>,
    pub info: InfoSpec,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "kebab-case")]
pub enum CustomConfig {
    PulseCode(PulseCodeConfig),
    Preset(PresetConfig),
}

impl CustomConfig {
    pub fn id(&self) -> &str {
        match self {
            CustomConfig::PulseCode(c) => &c.id,
            CustomConfig::Preset(c) => &c.id,
        }
    }
}

struct Checker {
    errors: Vec<String>,
}

fn is_text(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty())
}

impl Checker {
    fn text(&mut self, o: &Map<String, Value>, key: &str, path: &str, required: bool) {
        if o.get(key).is_none() && !required {
            return;
        }
        if !is_text(o.get(key)) {
            self.errors.push(format!("{path}.{key} must be a non-empty text."));
        }
    }
    fn num(&mut self, o: &Map<String, Value>, key: &str, path: &str, required: bool, min: Option<f64>) {
        if o.get(key).is_none() && !required {
            return;
        }
        let ok =
            o.get(key).and_then(Value::as_f64).is_some_and(|n| n.is_finite() && min.is_none_or(|m| n >= m));
        if !ok {
            self.errors.push(format!(
                "{path}.{key} must be a number{}.",
                min.map_or(String::new(), |m| format!(" of at least {}", crate::format::sig(m, 17)))
            ));
        }
    }
    fn one_of(&mut self, o: &Map<String, Value>, key: &str, path: &str, values: &[&str], required: bool) {
        if o.get(key).is_none() && !required {
            return;
        }
        if !o.get(key).and_then(Value::as_str).is_some_and(|v| values.contains(&v)) {
            let list = values.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join(", ");
            self.errors.push(format!("{path}.{key} must be one of: {list}."));
        }
    }
    fn range(&mut self, o: &Map<String, Value>, key: &str, path: &str) {
        let ok = o.get(key).and_then(Value::as_array).is_some_and(|r| {
            let n: Vec<Option<f64>> = r.iter().map(Value::as_f64).collect();
            r.len() == 2
                && n.iter().all(|x| x.is_some_and(|x| x.is_finite() && x >= 0.0))
                && n[0].unwrap_or(0.0) <= n[1].unwrap_or(0.0)
        });
        if !ok {
            self.errors.push(format!("{path}.{key} must be [min, max] in microseconds, with min <= max."));
        }
    }
    fn info(&mut self, info: Option<&Value>, path: &str) {
        let Some(info) = info.and_then(Value::as_object) else {
            self.errors
                .push(format!("{path} must be an object with what, confirm, decoderTips and lookalikes."));
            return;
        };
        for k in ["what", "confirm", "decoderTips", "lookalikes"] {
            self.text(info, k, path, true);
        }
    }
    fn segments(&mut self, list: Option<&Value>, path: &str) {
        let Some(list) = list.and_then(Value::as_array) else {
            self.errors.push(format!("{path} must be a list of segments."));
            return;
        };
        for (i, s) in list.iter().enumerate() {
            let p = format!("{path}[{i}]");
            let Some(s) = s.as_object() else {
                self.errors.push(format!("{p} must be an object."));
                continue;
            };
            self.one_of(s, "level", &p, &["high", "low"], true);
            self.num(s, "minUs", &p, true, Some(0.0));
            self.num(s, "maxUs", &p, true, Some(0.0));
            if let (Some(a), Some(b)) =
                (s.get("minUs").and_then(Value::as_f64), s.get("maxUs").and_then(Value::as_f64))
                && a > b
            {
                self.errors.push(format!("{p}: minUs is larger than maxUs."));
            }
            self.text(s, "label", &p, false);
        }
    }
}

/// Checks a parsed JSON value and returns a usable config, or every problem with its path.
pub fn validate(raw: &Value, taken_ids: &[&str]) -> Result<CustomConfig, Vec<String>> {
    let Some(o) = raw.as_object() else { return Err(vec!["The config must be a JSON object.".into()]) };
    let mut c = Checker { errors: Vec::new() };
    c.one_of(o, "family", "config", &["pulse-code", "preset"], true);
    c.text(o, "id", "config", true);
    c.text(o, "name", "config", true);
    c.text(o, "group", "config", false);
    if let Some(id) = o.get("id").and_then(Value::as_str) {
        let ok = id.chars().next().is_some_and(|f| f.is_ascii_lowercase() || f.is_ascii_digit())
            && id.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
        if !ok {
            c.errors.push("config.id may only use lowercase letters, digits and \"-\".".into());
        }
        if taken_ids.contains(&id) {
            c.errors
                .push(format!("config.id \"{id}\" is already used by a built-in protocol. Pick another id."));
        }
    }
    c.info(o.get("info"), "config.info");
    match o.get("family").and_then(Value::as_str) {
        Some("pulse-code") => {
            c.one_of(o, "idle", "config", &["high", "low"], true);
            c.one_of(o, "bitOrder", "config", &["msb", "lsb"], true);
            c.num(o, "minIdleUs", "config", false, Some(0.0));
            c.num(o, "wordBits", "config", false, Some(1.0));
            match o.get("frames").and_then(Value::as_array) {
                Some(frames) if !frames.is_empty() => {
                    for (i, f) in frames.iter().enumerate() {
                        let p = format!("config.frames[{i}]");
                        let Some(f) = f.as_object() else {
                            c.errors.push(format!("{p} must be an object."));
                            continue;
                        };
                        c.text(f, "name", &p, true);
                        c.segments(f.get("preamble"), &format!("{p}.preamble"));
                        if f.get("trailer").is_some() {
                            c.segments(f.get("trailer"), &format!("{p}.trailer"));
                        }
                        if let Some(bit) = f.get("bit") {
                            match bit.as_object() {
                                Some(b) => {
                                    c.one_of(b, "first", &format!("{p}.bit"), &["high", "low"], true);
                                    c.one_of(b, "encoding", &format!("{p}.bit"), &["first", "second"], true);
                                    for k in ["zero", "one", "other"] {
                                        c.range(b, k, &format!("{p}.bit"));
                                    }
                                }
                                None => c.errors.push(format!("{p}.bit must be an object.")),
                            }
                            match f.get("bits").and_then(Value::as_object) {
                                Some(n) => {
                                    c.num(n, "min", &format!("{p}.bits"), true, Some(0.0));
                                    c.num(n, "max", &format!("{p}.bits"), true, Some(1.0));
                                }
                                None => c.errors.push(format!(
                                    "{p}.bits must be {{ \"min\": n, \"max\": n }} when bit is given."
                                )),
                            }
                        }
                        if f.get("preamble").and_then(Value::as_array).is_some_and(Vec::is_empty)
                            && f.get("bit").is_none()
                        {
                            c.errors.push(format!("{p} needs a preamble or a bit definition."));
                        }
                    }
                }
                _ => c.errors.push("config.frames must list at least one frame shape.".into()),
            }
            if let Some(fields) = o.get("fields") {
                match fields.as_array() {
                    Some(list) => {
                        for (i, f) in list.iter().enumerate() {
                            let p = format!("config.fields[{i}]");
                            let Some(f) = f.as_object() else {
                                c.errors.push(format!("{p} must be an object."));
                                continue;
                            };
                            c.text(f, "name", &p, true);
                            c.num(f, "bits", &p, true, Some(1.0));
                            c.num(f, "scale", &p, false, None);
                            c.text(f, "unit", &p, false);
                            c.one_of(f, "signed", &p, &["twos", "sign-magnitude"], false);
                        }
                    }
                    None => c.errors.push("config.fields must be a list.".into()),
                }
            }
            if let Some(checks) = o.get("checks") {
                match checks.as_array() {
                    Some(list) => {
                        for (i, k) in list.iter().enumerate() {
                            let p = format!("config.checks[{i}]");
                            let Some(k) = k.as_object() else {
                                c.errors.push(format!("{p} must be an object."));
                                continue;
                            };
                            c.one_of(k, "kind", &p, &["sum8", "inverse"], true);
                            let kind = k.get("kind").and_then(Value::as_str);
                            if kind == Some("sum8")
                                && (!k.get("bytes").is_some_and(Value::is_array)
                                    || !k.get("equals").is_some_and(Value::is_number))
                            {
                                c.errors.push(format!("{p}: sum8 needs \"bytes\" (positions) and \"equals\" (position of the checksum)."));
                            }
                            if kind == Some("inverse")
                                && (!k.get("a").is_some_and(Value::is_number)
                                    || !k.get("b").is_some_and(Value::is_number))
                            {
                                c.errors.push(format!("{p}: inverse needs byte positions \"a\" and \"b\"."));
                            }
                        }
                    }
                    None => c.errors.push("config.checks must be a list.".into()),
                }
            }
        }
        Some("preset") => {
            c.text(o, "base", "config", true);
            if !o.get("settings").is_some_and(Value::is_object) {
                c.errors.push(
                    "config.settings must be an object of setting values, e.g. { \"baud\": 250000 }.".into(),
                );
            }
            if let Some(m) = o.get("match")
                && !m.as_array().is_some_and(|a| a.iter().all(Value::is_string))
            {
                c.errors.push("config.match must be a list of setting names.".into());
            }
        }
        _ => {}
    }
    if !c.errors.is_empty() {
        return Err(c.errors);
    }
    serde_json::from_value(raw.clone()).map_err(|e| vec![format!("The config cannot be read: {e}.")])
}

/// Parses and validates JSON text.
pub fn parse(text: &str, taken_ids: &[&str]) -> Result<CustomConfig, Vec<String>> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| vec![format!("The text is not valid JSON: {e}.")])?;
    validate(&v, taken_ids)
}

fn js_string(v: &SettingValue) -> String {
    match v {
        SettingValue::Number(n) => format!("{n}"),
        SettingValue::Text(t) => t.clone(),
        SettingValue::Bool(b) => b.to_string(),
    }
}

fn same_setting(a: Option<&SettingValue>, b: &SettingValue) -> bool {
    match (a, b) {
        (Some(SettingValue::Number(x)), SettingValue::Number(y)) => (x - y).abs() <= y.abs() * 0.03,
        (Some(a), b) => js_string(a) == js_string(b),
        (None, _) => false,
    }
}

/// A preset: an existing protocol with fixed settings.
pub struct Preset {
    cfg: PresetConfig,
    base: Arc<dyn Protocol>,
    origin: Origin,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl Preset {
    fn keys(&self) -> Vec<String> {
        self.cfg.r#match.clone().unwrap_or_else(|| self.cfg.settings.keys().cloned().collect())
    }
    fn key_text(&self) -> String {
        self.keys()
            .iter()
            .map(|k| format!("{k} {}", self.cfg.settings.get(k).map_or(String::new(), js_string)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Protocol for Preset {
    fn id(&self) -> &str {
        &self.cfg.id
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn group(&self) -> &str {
        self.cfg.group.as_deref().unwrap_or_else(|| self.base.group())
    }
    fn origin(&self) -> Origin {
        self.origin
    }
    fn info(&self) -> &ProtocolInfo {
        &self.info
    }
    fn roles(&self) -> &[RoleSpec] {
        self.base.roles()
    }
    fn settings(&self) -> &[SettingSpec] {
        &self.settings
    }
    fn describe(&self, s: &Settings) -> String {
        self.base.describe(s)
    }
    fn detect(&self, ctx: &DetectContext<'_>) -> Option<DetectorOutput> {
        let out = self.base.detect(ctx)?;
        let keys = self.keys();
        let detections: Vec<Detection> = out
            .detections
            .into_iter()
            .filter(|d| {
                keys.iter()
                    .all(|k| self.cfg.settings.get(k).is_some_and(|v| same_setting(d.settings.get(k), v)))
            })
            .map(|mut d| {
                let mut merged = d.settings.clone();
                merged.extend(self.cfg.settings.clone());
                d.protocol_id = self.cfg.id.clone();
                d.settings = SettingSpec::with_defaults(&self.settings, &merged);
                d.confidence = (d.confidence + 0.05).min(1.0);
                d.evidence.push(evidence(
                    format!(
                        "The {} settings match the {} preset ({}).",
                        self.base.name(),
                        self.cfg.name,
                        self.key_text()
                    ),
                    true,
                ));
                d
            })
            .collect();
        let rejections = if detections.is_empty() {
            vec![Rejection {
                protocol_id: self.cfg.id.clone(),
                because: format!(
                    "no {} line here has the {} settings ({})",
                    self.base.name(),
                    self.cfg.name,
                    self.key_text()
                ),
            }]
        } else {
            vec![]
        };
        Some(DetectorOutput { detections, rejections })
    }
    fn has_decoder(&self) -> bool {
        self.base.has_decoder()
    }
    fn decoder(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        self.base.decoder(setup, snapshot)
    }
}

/// Builds a protocol from a validated config; a preset needs its base protocol.
pub fn compile(
    cfg: CustomConfig,
    lookup: &dyn Fn(&str) -> Option<Arc<dyn Protocol>>,
    origin: Origin,
) -> Result<Arc<dyn Protocol>, String> {
    match cfg {
        CustomConfig::PulseCode(p) => Ok(Arc::new(PulseCode::new(p, origin))),
        CustomConfig::Preset(p) => {
            let base = lookup(&p.base).ok_or_else(|| {
                format!(
                    "The preset \"{}\" is based on \"{}\", which is not a known protocol.",
                    p.name, p.base
                )
            })?;
            let settings = base
                .settings()
                .iter()
                .map(|s| {
                    let mut s = s.clone();
                    if let Some(v) = p.settings.get(&s.key) {
                        match (&mut s.kind, v) {
                            (SettingKind::Number { default, .. }, SettingValue::Number(n)) => *default = *n,
                            (SettingKind::Choice { default, .. }, SettingValue::Text(t)) => {
                                *default = t.clone()
                            }
                            (SettingKind::Boolean { default }, SettingValue::Bool(b)) => *default = *b,
                            _ => {}
                        }
                    }
                    s
                })
                .collect();
            Ok(Arc::new(Preset { info: p.info.to_info(), cfg: p, base, origin, settings }))
        }
    }
}

/// The built-in protocols that are data: 1-Wire, DHT, WS2812, NEC infrared and the DMX512 preset.
pub const BUILT_IN_JSON: [(&str, &str); 5] = [
    ("onewire", include_str!("../data/onewire.json")),
    ("dht", include_str!("../data/dht.json")),
    ("ws2812", include_str!("../data/ws2812.json")),
    ("ir-nec", include_str!("../data/ir-nec.json")),
    ("dmx512", include_str!("../data/dmx512.json")),
];

/// The template offered when someone writes their own protocol.
pub const TEMPLATE_JSON: &str = include_str!("../data/custom-template.json");

/// A built-in data protocol, by id.
pub fn built_in_config(id: &str) -> Option<CustomConfig> {
    let (_, text) = BUILT_IN_JSON.iter().find(|(k, _)| *k == id)?;
    parse(text, &[]).ok()
}
