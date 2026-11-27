//! What a session holds: open captures with their decoders, markers, view and channel settings.
//! The `…Summary` types are what `session.get` returns.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::capture::{CaptureMeta, CaptureState, Note, SavedDecoder, Source};
use logica_core::store::CaptureStore;
use logica_protocols::analyse::LiveDecode;
use logica_protocols::types::{DataFormat, DecodeResult, FrameKind, ProblemCode, Roles, Settings};

use crate::contract::Actor;

/// A decoder's configuration: what `decoder.add` takes and an undo restores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecoderConfig {
    pub protocol: String,
    pub roles: Roles,
    pub settings: Settings,
    pub name: String,
    /// How its data is written in the waveform's text row (ASCII unless chosen).
    #[serde(default)]
    pub data_format: DataFormat,
}

pub struct Decoder {
    pub config: DecoderConfig,
    /// The whole-capture decode (paged by `decoder.frames`); while capturing, what is decoded so
    /// far.
    pub result: DecodeResult,
    /// Channels its data frames are drawn under, in order.
    pub lanes: Vec<u32>,
    /// Its channels in the protocol's role order (SPI: CLK, CS, MOSI, MISO).
    pub channels: Vec<u32>,
    /// The decode following the capture while it is captured.
    pub live: Option<LiveDecode>,
}

/// Frames that carry data (bytes, words, samples): what the waveform's text row writes out.
pub fn is_data(kind: FrameKind) -> bool {
    matches!(kind, FrameKind::Data | FrameKind::Sample)
}

impl Decoder {
    /// `role_order`: the protocol's role keys in display order; roles it does not name follow.
    pub fn new(config: DecoderConfig, result: DecodeResult, role_order: &[&str]) -> Self {
        let lanes: BTreeSet<u32> =
            result.frames.iter().filter(|f| f.row == 0 && is_data(f.kind)).map(|f| f.lane).collect();
        let named = role_order.iter().filter_map(|k| config.roles.get(*k).copied().flatten());
        let rest =
            config.roles.iter().filter(|(k, _)| !role_order.contains(&k.as_str())).filter_map(|(_, c)| *c);
        let mut channels: Vec<u32> = Vec::new();
        for c in named.chain(rest) {
            if !channels.contains(&c) {
                channels.push(c);
            }
        }
        Decoder { config, result, lanes: lanes.into_iter().collect(), channels, live: None }
    }

    /// Adds the lanes of frames from index `from` on.
    fn absorb(&mut self, from: usize) {
        for f in &self.result.frames[from.min(self.result.frames.len())..] {
            if f.row == 0 && is_data(f.kind) && !self.lanes.contains(&f.lane) {
                let at = self.lanes.partition_point(|&l| l < f.lane);
                self.lanes.insert(at, f.lane);
            }
        }
    }

    /// Decodes what arrived since the last call; returns whether anything new was decoded. A
    /// failing live decode ends with a note and keeps what it decoded.
    pub fn advance_live(&mut self, store: &CaptureStore) -> bool {
        let Some(live) = self.live.as_mut() else { return false };
        let from = self.result.frames.len();
        match live.advance(store, &mut self.result) {
            Ok(changed) => {
                self.absorb(from);
                changed
            }
            Err(e) => {
                self.live = None;
                self.result.notes.push(format!("Live decoding stopped: {e}"));
                true
            }
        }
    }

    /// The capture ended at `end`: the decoder emits what is pending and stops following.
    pub fn finish_live(&mut self, store: &CaptureStore, end: Sample) -> bool {
        let Some(live) = self.live.take() else { return false };
        let from = self.result.frames.len();
        if let Err(e) = live.finish(store, end, &mut self.result) {
            self.result.notes.push(format!("Live decoding stopped: {e}"));
        }
        self.absorb(from);
        true
    }
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
    /// Virtual analog channels (ADR 0010).
    pub analog: BTreeMap<String, crate::analog::Analog>,
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
    /// Channels its data appears under (SPI: MOSI and MISO; UART: RX and TX), in order.
    pub lanes: Vec<u32>,
    /// Its channels in the protocol's role order (SPI: CLK, CS, MOSI, MISO): the order they are
    /// shown in, grouped under the decoder.
    pub channels: Vec<u32>,
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
            lanes: d.lanes.clone(),
            channels: d.channels.clone(),
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
    /// Virtual analog channels (ADR 0010).
    pub analog: Vec<crate::analog::AnalogSummary>,
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
            analog: c.analog.iter().map(|(id, a)| crate::analog::AnalogSummary::of(id, a)).collect(),
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
    /// Virtual analog channel definitions; their values are computed again on open (doc 09).
    #[serde(default)]
    pub analog: BTreeMap<String, crate::analog::AnalogConfig>,
    pub markers: Vec<Marker>,
    pub groups: BTreeMap<String, Vec<u32>>,
    pub hidden: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoders_saved_before_data_formats_existed_open_as_ascii() {
        let old =
            r#"{ "protocol": "uart", "roles": { "data": 0 }, "settings": { "baud": 9600 }, "name": "UART" }"#;
        let cfg: DecoderConfig = serde_json::from_str(old).unwrap();
        assert_eq!(cfg.data_format, DataFormat::Ascii);
    }
}
