//! Edges as decoders and detectors see them (doc 04 "Reading", doc 06 "Streaming decoders").

use super::{Level, Sample};

/// One edge of a decoder's merged edge stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    pub sample: Sample,
    /// Index of the channel in the decoder's channel list (not the physical channel).
    pub slot: u8,
    /// Level from this sample on.
    pub level: Level,
}

/// Edges of several channels in time order, complete up to `end`: every edge before `end` has
/// been delivered, in this batch or an earlier one. Edges at the same sample are ordered by slot.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EdgeBatch {
    pub edges: Vec<Edge>,
    pub end: Sample,
}
