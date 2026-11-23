//! What a session holds: open captures with their decoders, markers, view and channel settings.
//! The `…Summary` types are what `session.get` returns.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::capture::{CaptureMeta, CaptureState, Note, SavedDecoder, Source};
use logica_core::store::CaptureStore;
use logica_protocols::types::{DecodeResult, ProblemCode, Roles, Settings};

use crate::contract::Actor;

/// A decoder's configuration: what `decoder.add` takes and an undo restores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecoderConfig {
    pub protocol: String,
    pub roles: Roles,
    pub settings: Settings,
    pub name: String,
}

pub struct Decoder {
    pub config: DecoderConfig,
    /// The whole-capture decode (paged by `decoder.frames`).
    pub result: DecodeResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub id: String,
    /// Sample of the marker, or the start of a range note.
    pub at: Sample,
    /// End of a range note.
    pub end: Option<Sample>,
    pub label: String,
    pub note: Option<String>,
    /// Who placed it; agents' markers carry an "agent" badge in the window.
    pub by: Actor,
}

/// A tab's view (doc 08): the visible range, cursors and selection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub from: Sample,
    pub to: Sample,
    pub cursor_a: Option<Sample>,
    pub cursor_b: Option<Sample>,
    /// Selected range, start and end.
    pub selection: Option<(Sample, Sample)>,
}

pub struct Capture {
    pub meta: CaptureMeta,
    pub store: CaptureStore,
    /// The file it was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// Whether the store reads from `path` itself (an opened or saved `.lacap`).
    pub backed_by_path: bool,
    pub decoders: BTreeMap<String, Decoder>,
    pub markers: BTreeMap<String, Marker>,
    pub view: View,
    /// Bus groups: name to physical channels.
    pub groups: BTreeMap<String, Vec<u32>>,
    pub hidden: BTreeSet<u32>,
    /// Changes not in any file yet: a capture from a device, or edits since the last open or save.
    pub unsaved: bool,
}

impl Capture {
    pub fn seconds(&self, s: Sample) -> f64 {
        s as f64 / self.meta.sample_rate_hz
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelEntry {
    pub index: u32,
    pub name: String,
    pub colour: Option<String>,
    pub hidden: bool,
    pub group: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecoderSummary {
    pub id: String,
    pub config: DecoderConfig,
    pub frames: usize,
    pub packets: usize,
    pub problem_counts: BTreeMap<ProblemCode, u64>,
    pub notes: Vec<String>,
}

impl DecoderSummary {
    pub fn of(id: &str, d: &Decoder) -> Self {
        DecoderSummary {
            id: id.into(),
            config: d.config.clone(),
            frames: d.result.frames.len(),
            packets: d.result.packets.len(),
            problem_counts: d.result.problem_counts.clone(),
            notes: d.result.notes.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSummary {
    pub id: String,
    pub title: String,
    pub path: Option<String>,
    pub source: Source,
    pub sample_rate_hz: f64,
    pub sample_count: Sample,
    pub seconds: f64,
    pub state: CaptureState,
    pub trigger: Option<Sample>,
    /// Samples left out of analysis (a start-up artefact).
    pub artefact: Option<(Sample, Sample)>,
    pub notes: Vec<Note>,
    pub channels: Vec<ChannelEntry>,
    pub decoders: Vec<DecoderSummary>,
    pub markers: Vec<Marker>,
    pub view: View,
    /// Decoder settings found in the file: for comparison, never used by detection.
    pub saved_decoders: Vec<SavedDecoder>,
    /// Closing it loses something: a capture from a device that was never saved, or decoders,
    /// markers, names or groups changed since the file was opened or saved.
    pub unsaved: bool,
}

impl CaptureSummary {
    pub fn of(id: &str, c: &Capture) -> Self {
        let group_of = |ch: u32| c.groups.iter().find(|(_, list)| list.contains(&ch)).map(|(g, _)| g.clone());
        CaptureSummary {
            id: id.into(),
            title: c.meta.title.clone(),
            path: c.path.as_ref().map(|p| p.display().to_string()),
            source: c.meta.source.clone(),
            sample_rate_hz: c.meta.sample_rate_hz,
            sample_count: c.store.sample_count(),
            seconds: c.seconds(c.store.sample_count()),
            state: c.meta.state,
            trigger: c.meta.trigger,
            artefact: c.meta.artefact.as_ref().map(|a| (a.start, a.end)),
            notes: c.meta.notes.clone(),
            channels: c
                .meta
                .channels
                .iter()
                .map(|ch| ChannelEntry {
                    index: ch.index,
                    name: ch.name.clone(),
                    colour: ch.colour.clone(),
                    hidden: c.hidden.contains(&ch.index),
                    group: group_of(ch.index),
                })
                .collect(),
            decoders: c.decoders.iter().map(|(id, d)| DecoderSummary::of(id, d)).collect(),
            markers: c.markers.values().cloned().collect(),
            view: c.view.clone(),
            saved_decoders: c.meta.saved_decoders.clone(),
            unsaved: c.unsaved,
        }
    }
}

/// What is saved with a capture in the `.lacap` index (doc 09, "session state").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedSession {
    pub decoders: BTreeMap<String, DecoderConfig>,
    pub markers: Vec<Marker>,
    pub groups: BTreeMap<String, Vec<u32>>,
    pub hidden: Vec<u32>,
}
