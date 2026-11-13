//! A protocol: its roles, settings, knowledge texts, detector and streaming decoder (doc 06), and
//! the registry of every protocol the engine knows.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use logica_core::measure::ChannelMeasurement;

use crate::buses::Bus;
use crate::stream::{DecoderSetup, StreamDecoder};
use crate::types::{Detection, Rejection, RoleSpec, SettingSpec, Settings};
use crate::window::{WinChannel, Window};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symptom {
    pub symptom: String,
    pub cause: String,
    pub action: String,
}

pub fn symptoms(list: &[(&str, &str, &str)]) -> Vec<Symptom> {
    list.iter()
        .map(|(s, c, a)| Symptom { symptom: (*s).into(), cause: (*c).into(), action: (*a).into() })
        .collect()
}

/// What the person (or agent) can learn about a protocol from the tool itself.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolInfo {
    /// What it is and where you meet it.
    pub what: String,
    /// How to confirm it on the waveform.
    pub confirm: String,
    /// What to set in the decoder.
    pub decoder_tips: String,
    /// What else looks similar and how to tell them apart.
    pub lookalikes: String,
    pub symptoms: Vec<Symptom>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    BuiltIn,
    Custom,
}

/// What a detector looks at: one bus of a detection window.
pub struct DetectContext<'a> {
    pub sample_rate_hz: f64,
    pub window: &'a Window,
    pub bus: &'a Bus,
    /// The bus's channels.
    pub channels: Vec<&'a WinChannel>,
    pub measurements: &'a BTreeMap<u32, ChannelMeasurement>,
}

impl DetectContext<'_> {
    pub fn m(&self, channel: u32) -> &ChannelMeasurement {
        &self.measurements[&channel]
    }
    /// Samples in the window (Signal Detective's "sample count").
    pub fn samples(&self) -> u64 {
        self.window.end - self.window.start
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetectorOutput {
    pub detections: Vec<Detection>,
    pub rejections: Vec<Rejection>,
}

impl DetectorOutput {
    pub fn rejected(protocol_id: &str, because: impl Into<String>) -> Self {
        DetectorOutput {
            detections: Vec::new(),
            rejections: vec![Rejection { protocol_id: protocol_id.into(), because: because.into() }],
        }
    }
}

pub trait Protocol: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    /// Grouping for symptom tables and the protocol list.
    fn group(&self) -> &str;
    fn origin(&self) -> Origin {
        Origin::BuiltIn
    }
    fn info(&self) -> &ProtocolInfo;
    /// Channels the decoder needs, in display order.
    fn roles(&self) -> &[RoleSpec];
    fn settings(&self) -> &[SettingSpec];
    /// A knowledge-only entry whose captures are detected and decoded as another protocol.
    fn alias_of(&self) -> Option<&str> {
        None
    }
    /// One-line summary of settings, such as "115,200 baud, 8N1".
    fn describe(&self, _settings: &Settings) -> String {
        String::new()
    }
    /// Title of a detection, when it depends on the settings ("UART, inverted").
    fn title(&self, _settings: &Settings) -> String {
        self.name().to_string()
    }
    fn detect(&self, _ctx: &DetectContext<'_>) -> Option<DetectorOutput> {
        None
    }
    fn has_decoder(&self) -> bool {
        false
    }
    /// A streaming decoder for `setup`, or a message the person can act on.
    fn decoder(
        &self,
        _setup: &DecoderSetup,
        _snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        Err(format!("There is no decoder for {}.", self.name()))
    }
}

/// The protocols the engine knows: built-in ones plus custom ones (a custom protocol with the
/// id of a built-in one replaces it).
#[derive(Clone)]
pub struct Registry {
    list: Vec<Arc<dyn Protocol>>,
}

impl Registry {
    pub fn new(built_in: Vec<Arc<dyn Protocol>>, custom: Vec<Arc<dyn Protocol>>) -> Self {
        let mut list = built_in;
        for c in custom {
            match list.iter().position(|p| p.id() == c.id()) {
                Some(i) => list[i] = c,
                None => list.push(c),
            }
        }
        Registry { list }
    }

    pub fn all(&self) -> &[Arc<dyn Protocol>] {
        &self.list
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Protocol>> {
        self.list.iter().find(|p| p.id() == id)
    }
}
