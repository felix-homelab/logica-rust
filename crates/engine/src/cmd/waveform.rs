//! `waveform.get`: what the window draws for a range (doc 08). Zoomed in, every edge; zoomed out,
//! level-of-detail buckets (doc 04), so the answer's size follows the width in pixels, not the
//! length of the range.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::store::Level;
use logica_protocols::format::value_as;
use logica_protocols::types::FrameKind;

use crate::contract::{CommandError, ErrorCode};
use crate::engine::{Ctx, Done, Engine, err};
use crate::state::is_data;

use super::out;

/// Most buckets (pixel columns) per request.
pub const MAX_BUCKETS: u32 = 8192;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaveformIn {
    pub capture: String,
    pub from: Sample,
    /// End sample (exclusive); limited to the capture's length.
    pub to: Sample,
    /// Pixel columns to fill (1 to 8192).
    pub buckets: u32,
    /// Physical channels; the visible ones when absent.
    pub channels: Option<Vec<u32>>,
    /// Include decoder frames (default true).
    pub decoders: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WaveTrace {
    pub channel: u32,
    /// Level at `from`.
    pub start_level: Level,
    /// Every edge in the range (first samples at a new level), when there are few enough to draw.
    pub edges: Option<Vec<Sample>>,
    /// Otherwise transitions per bucket of `bucket` samples from `from`.
    pub counts: Option<Vec<u32>>,
    /// And the level at the start of each bucket: "0" or "1" per bucket.
    pub levels: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WaveFrame {
    pub start: Sample,
    pub end: Sample,
    pub label: String,
    pub kind: FrameKind,
    /// Physical channel it is drawn under: one of the decoder's data channels (`lanes` in the
    /// session); frames on another channel (an I2C start on SCL) go under the first of them.
    pub lane: u32,
    pub problem: bool,
    /// For data frames: the value in the decoder's data format ("A", "65", "01000001").
    pub text: Option<String>,
    /// Packet it belongs to: neighbours in one packet join into a string in the text row.
    pub packet: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WaveDecoder {
    pub id: String,
    pub name: String,
    /// Frames in the range, when there are few enough to draw one by one.
    pub frames: Option<Vec<WaveFrame>>,
    /// Otherwise stretches of frames closer than a bucket, per channel drawn under, as
    /// (start, end, any problem, lane).
    pub spans: Option<Vec<(Sample, Sample, bool, u32)>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WaveformOut {
    pub from: Sample,
    pub to: Sample,
    /// Samples per bucket (at least 1).
    pub bucket: u64,
    pub traces: Vec<WaveTrace>,
    pub decoders: Vec<WaveDecoder>,
}

fn level_char(l: Level) -> char {
    if l == Level::High { '1' } else { '0' }
}

pub(crate) fn get(e: &mut Engine, _: &Ctx, i: WaveformIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    let count = c.store.sample_count();
    let to = i.to.min(count);
    if i.from >= to {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("The range is empty: choose from < to inside 0 to {count}."),
        ));
    }
    if i.buckets == 0 || i.buckets > MAX_BUCKETS {
        return Err(err(ErrorCode::InvalidInput, format!("buckets must be 1 to {MAX_BUCKETS}.")));
    }
    let span = to - i.from;
    let bucket = span.div_ceil(u64::from(i.buckets)).max(1);
    let n = usize::try_from(span.div_ceil(bucket)).unwrap_or(0);
    // edges are cheap to draw while there are about two per pixel column or fewer
    let edge_limit = u64::from(i.buckets) * 2;
    let wanted: Vec<u32> = match &i.channels {
        Some(list) => list.clone(),
        None => c.meta.channels.iter().map(|ch| ch.index).filter(|x| !c.hidden.contains(x)).collect(),
    };
    let fail = |x: logica_core::store::StoreError| err(ErrorCode::Failed, x.to_string());
    let mut traces = Vec::with_capacity(wanted.len());
    for ch in wanted {
        let slot = c.meta.channels.iter().position(|x| x.index == ch).ok_or_else(|| {
            err(ErrorCode::NotFound, format!("Channel {ch} is not in capture {}.", i.capture))
        })?;
        let start_level = c.store.level_at(slot, i.from).map_err(fail)?;
        let total = c.store.transitions_in(slot, i.from, to).map_err(fail)?;
        if total <= edge_limit {
            let edges = c.store.edges_in(slot, i.from, to).map_err(fail)?;
            traces.push(WaveTrace {
                channel: ch,
                start_level,
                edges: Some(edges),
                counts: None,
                levels: None,
            });
        } else {
            let lod = c.store.lod(slot, i.from, bucket, n).map_err(fail)?;
            let counts = lod.iter().map(|b| u32::try_from(b.transitions).unwrap_or(u32::MAX)).collect();
            let levels = lod.iter().map(|b| level_char(b.start_level)).collect();
            traces.push(WaveTrace {
                channel: ch,
                start_level,
                edges: None,
                counts: Some(counts),
                levels: Some(levels),
            });
        }
    }
    let mut decoders = Vec::new();
    if i.decoders.unwrap_or(true) {
        for (id, d) in &c.decoders {
            let inside: Vec<&logica_protocols::types::Frame> =
                d.result.frames.iter().filter(|f| f.row == 0 && f.end > i.from && f.start < to).collect();
            let lane = |l: u32| match d.lanes.first() {
                Some(&first) if !d.lanes.contains(&l) => first,
                _ => l,
            };
            let (frames, spans) = if inside.len() as u64 <= u64::from(i.buckets) {
                let frames = inside
                    .iter()
                    .map(|f| WaveFrame {
                        start: f.start,
                        end: f.end,
                        label: f.label.clone(),
                        kind: f.kind,
                        lane: lane(f.lane),
                        problem: !f.problems.is_empty(),
                        text: f.value.filter(|_| is_data(f.kind)).map(|v| value_as(v, d.config.data_format)),
                        packet: f.packet,
                    })
                    .collect();
                (Some(frames), None)
            } else {
                let mut spans: Vec<(Sample, Sample, bool, u32)> = Vec::new();
                let mut sorted = inside;
                sorted.sort_by_key(|f| (lane(f.lane), f.start));
                for f in sorted {
                    let problem = !f.problems.is_empty();
                    let l = lane(f.lane);
                    match spans.last_mut() {
                        Some(last) if last.3 == l && f.start <= last.1.saturating_add(bucket) => {
                            last.1 = last.1.max(f.end);
                            last.2 |= problem;
                        }
                        _ => spans.push((f.start, f.end, problem, l)),
                    }
                }
                (None, Some(spans))
            };
            decoders.push(WaveDecoder { id: id.clone(), name: d.config.name.clone(), frames, spans });
        }
    }
    Ok(Done::query(out(&WaveformOut { from: i.from, to, bucket, traces, decoders })))
}
