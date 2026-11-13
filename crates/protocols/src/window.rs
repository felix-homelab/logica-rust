//! The detection window (doc 06): from the end of a start-up artefact, at most 2^26 samples and at
//! most [`MAX_WINDOW_EDGES`] edges per channel, so detection stays fast on huge captures.

use logica_core::capture::CaptureMeta;
use logica_core::store::{CaptureStore, StoreResult};
use logica_core::{Level, Sample};

/// Longest detection window: 2^26 samples (67 M), enough for every fixture whole.
pub const MAX_WINDOW: u64 = 1 << 26;
/// Most edges per channel in a detection window; a busier channel shortens the window.
pub const MAX_WINDOW_EDGES: usize = 1 << 20;

/// One channel inside the window.
#[derive(Clone, Debug, PartialEq)]
pub struct WinChannel {
    /// Physical channel number.
    pub index: u32,
    /// Slot in the store.
    pub slot: usize,
    pub name: String,
    /// Level just before the window.
    pub initial: Level,
    /// Edges inside the window, absolute samples.
    pub edges: Vec<Sample>,
}

impl WinChannel {
    /// Level at absolute sample `s` inside the window.
    pub fn level_at(&self, s: Sample) -> Level {
        let n = self.edges.partition_point(|&e| e <= s);
        if n % 2 == 1 { !self.initial } else { self.initial }
    }

    /// Level of pulse `i` (from edge `i` to edge `i + 1`).
    pub fn pulse_level(&self, i: usize) -> Level {
        logica_core::measure::pulse_level(self.initial, i)
    }

    /// Index of the first edge at or after `s`.
    pub fn lower_bound(&self, s: f64) -> usize {
        self.edges.partition_point(|&e| (e as f64) < s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub start: Sample,
    pub end: Sample,
    pub channels: Vec<WinChannel>,
    /// Why the window is shorter than the capture, if it is.
    pub note: Option<String>,
}

impl Window {
    /// The window of `store` starting at `from` (usually the end of a start-up artefact).
    pub fn from_store(store: &CaptureStore, meta: &CaptureMeta, from: Sample) -> StoreResult<Window> {
        let count = store.sample_count();
        let start = from.min(count);
        let mut end = count.min(start.saturating_add(MAX_WINDOW));
        let mut note = (end < count).then(|| {
            format!(
                "Detection looked at the first {} samples after the start; the capture is longer.",
                end - start
            )
        });
        // a channel with too many edges shortens the window for all
        for slot in 0..store.slots() {
            let mut it = store.edges(slot, start, end)?;
            if let Some(Ok(e)) = it.nth(MAX_WINDOW_EDGES) {
                end = e;
                note = Some(format!(
                    "Detection stopped at sample {e}, where a channel had changed level {MAX_WINDOW_EDGES} times."
                ));
            }
        }
        let mut channels = Vec::with_capacity(store.slots());
        for (slot, info) in meta.channels.iter().enumerate().take(store.slots()) {
            let initial =
                if start == 0 { store.initial_level(slot)? } else { store.level_at(slot, start - 1)? };
            channels.push(WinChannel {
                index: info.index,
                slot,
                name: info.name.clone(),
                initial,
                edges: store.edges_in(slot, start, end)?,
            });
        }
        Ok(Window { start, end, channels, note })
    }

    /// A window over channels given directly (generated examples, tests).
    pub fn from_channels(channels: Vec<WinChannel>, start: Sample, end: Sample) -> Window {
        Window { start, end, channels, note: None }
    }

    pub fn channel(&self, index: u32) -> Option<&WinChannel> {
        self.channels.iter().find(|c| c.index == index)
    }

    pub fn samples(&self) -> u64 {
        self.end - self.start
    }
}
