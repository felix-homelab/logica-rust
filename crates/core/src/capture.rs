//! Capture metadata (doc 04, "Capture"). The transitions themselves are in [`crate::store`].

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::Sample;

/// Where a capture came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Device,
    File,
    Synthetic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub kind: SourceKind,
    /// For example "FNIRSI .demo file mystery_D.demo" or "synthetic device, program mix".
    pub detail: String,
}

/// One channel as the person sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    /// Physical channel number on the device or in the file (CH0 = 0).
    pub index: u32,
    pub name: String,
    /// `#rrggbb`, when the source has one.
    pub colour: Option<String>,
    pub enabled: bool,
    /// Threshold in volts, where adjustable.
    pub threshold_volts: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteLevel {
    Info,
    Warning,
}

/// Format surprises, dropped data, artefacts: shown to the person and in MCP summaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub level: NoteLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureState {
    Capturing,
    Complete,
    Aborted,
}

/// Decoder settings found in an imported file: shown for comparison, never used by detection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedDecoder {
    /// The file's own protocol name, normalised to lower case (`uart`, `i2c`, `spi`, …).
    pub protocol: String,
    /// Role name to physical channel; `None` where the file leaves it unassigned.
    pub channels: Vec<(String, Option<u32>)>,
    /// Option name to value, as text.
    pub options: Vec<(String, String)>,
}

/// Everything about a capture except its transitions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureMeta {
    pub id: String,
    pub title: String,
    pub source: Source,
    /// Samples per second. Not always an integer (a VCD file may step in thirds of a second).
    pub sample_rate_hz: f64,
    /// Time 0 in the UI.
    pub trigger: Option<Sample>,
    /// In store slot order.
    pub channels: Vec<ChannelInfo>,
    /// Covered sample ranges; gaps between them are stream restarts or chunks of an export.
    pub segments: Vec<Range<Sample>>,
    pub notes: Vec<Note>,
    pub state: CaptureState,
    /// A start-up artefact excluded from analysis (DLA captures).
    pub artefact: Option<Range<Sample>>,
    pub saved_decoders: Vec<SavedDecoder>,
}
