//! Sources of virtual analog channels (ADR 0010, doc 06): streaming decoders that emit points
//! instead of frames. Scaling and the value series are in `logica_core::analog`.
//!
//! A source is described by a [`Source`] (its roles, settings and what its points mean) and
//! creates a [`StreamDecoder`], so live capture, snapshots and the batching rules work for it as
//! for any decoder. A new source implements [`Source`] and is added to [`sources`]; nothing else
//! changes.

pub mod parallel;
pub mod presets;
pub mod pwm;
pub mod serial;

use logica_core::analog::{Encoding, PointKind};
use serde::{Deserialize, Serialize};

use crate::stream::{DecoderSetup, StreamDecoder};
use crate::types::{RoleSpec, SettingSpec, Settings};

/// What the points of a source mean with given settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceOutput {
    pub kind: PointKind,
    /// Bits of the code behind a fraction, for the raw code and the step size; `None` when the
    /// fraction is not a code (a PWM duty).
    pub bits: Option<u32>,
    pub encoding: Encoding,
    /// The unit of a measured quantity (`Hz`, `s`); `None` for fractions.
    pub unit: Option<String>,
}

impl SourceOutput {
    pub fn fraction() -> Self {
        SourceOutput { kind: PointKind::Fraction, bits: None, encoding: Encoding::Unsigned, unit: None }
    }

    pub fn quantity(unit: &str) -> Self {
        SourceOutput {
            kind: PointKind::Quantity,
            bits: None,
            encoding: Encoding::Unsigned,
            unit: Some(unit.into()),
        }
    }
}

/// A kind of virtual analog source.
pub trait Source: Send + Sync {
    /// Stable id, used in definitions and saved files: `pwm`.
    fn id(&self) -> &str;
    /// Name for people: "PWM level".
    fn name(&self) -> &str;
    /// One sentence on what it reads and when to use it.
    fn help(&self) -> &str;
    fn roles(&self) -> &[RoleSpec];
    fn settings(&self) -> &[SettingSpec];
    /// What the points mean with `settings` (defaults applied).
    fn output(&self, settings: &Settings) -> SourceOutput;
    /// A streaming decoder that emits points, new or restored from a snapshot.
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String>;
}

/// Every built-in source.
pub fn sources() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(pwm::PwmSource::default()),
        Box::new(serial::SerialAdc::new(serial::Bus::Spi)),
        Box::new(serial::SerialAdc::new(serial::Bus::I2c)),
        Box::new(parallel::ParallelSource::default()),
    ]
}

/// The source with `id`.
pub fn source(id: &str) -> Option<Box<dyn Source>> {
    sources().into_iter().find(|s| s.id() == id)
}

#[cfg(test)]
pub(crate) mod testing {
    //! Runs a source over edge lists in several batchings and checks they agree (doc 06).

    use logica_core::store::edges::{Edge, EdgeBatch};
    use logica_core::{Level, Sample};

    use super::Source;
    use crate::stream::{BoundChannel, DecoderSetup, Output};
    use crate::types::{SettingSpec, Settings};

    pub fn setup(src: &dyn Source, rate: f64, starts: &[Level], settings: &Settings) -> DecoderSetup {
        DecoderSetup {
            sample_rate_hz: rate,
            start: 0,
            start_levels: starts.to_vec(),
            channels: src
                .roles()
                .iter()
                .zip(0u32..)
                .take(starts.len())
                .map(|(r, i)| BoundChannel { role: r.key.clone(), channel: i, name: format!("CH{i}") })
                .collect(),
            settings: SettingSpec::with_defaults(src.settings(), settings),
        }
    }

    /// Every edge of every channel in time order (slot order at the same sample).
    fn merged(channels: &[(Level, Vec<Sample>)]) -> Vec<Edge> {
        let mut all = Vec::new();
        for (slot, (start, edges)) in channels.iter().enumerate() {
            let mut level = *start;
            for &s in edges {
                level = !level;
                all.push(Edge { sample: s, slot: u8::try_from(slot).unwrap(), level });
            }
        }
        all.sort_by_key(|e| (e.sample, e.slot));
        all
    }

    /// Runs `src` with batches of `size` edges (0: one batch), restoring from a snapshot after
    /// every batch when `restore` is set.
    pub fn run_with(
        src: &dyn Source,
        setup: &DecoderSetup,
        channels: &[(Level, Vec<Sample>)],
        end: Sample,
        size: usize,
        restore: bool,
    ) -> Output {
        let edges = merged(channels);
        let mut d = src.create(setup, None).unwrap();
        let mut out = Output::default();
        let mut take = |o: Output| {
            out.frames.extend(o.frames);
            out.points.extend(o.points);
            out.notes.extend(o.notes);
        };
        let size = if size == 0 { edges.len().max(1) } else { size };
        let mut i = 0;
        while i < edges.len() {
            let mut j = (i + size).min(edges.len());
            // A batch never splits the edges of one sample.
            while j < edges.len() && edges[j].sample == edges[j - 1].sample {
                j += 1;
            }
            let batch_end = edges.get(j).map_or(end, |e| e.sample);
            d.push(&EdgeBatch { edges: edges[i..j].to_vec(), end: batch_end });
            take(d.drain());
            if restore {
                d = src.create(setup, Some(d.snapshot())).unwrap();
            }
            i = j;
        }
        d.finish(end);
        take(d.drain());
        out
    }

    /// Runs `src` in one batch, in batches of 1 and 3 edges, and with snapshot and restore after
    /// every batch of 7; asserts all four agree and returns the output.
    pub fn run(
        src: &dyn Source,
        rate: f64,
        channels: &[(Level, Vec<Sample>)],
        end: Sample,
        settings: &Settings,
    ) -> Output {
        let starts: Vec<Level> = channels.iter().map(|c| c.0).collect();
        let setup = setup(src, rate, &starts, settings);
        let whole = run_with(src, &setup, channels, end, 0, false);
        for (size, restore) in [(1, false), (3, false), (7, true)] {
            let other = run_with(src, &setup, channels, end, size, restore);
            assert_eq!(other, whole, "{} differs in batches of {size} (restore {restore})", src.id());
        }
        whole
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_has_a_unique_id_and_explains_itself() {
        let all = sources();
        let mut ids: Vec<_> = all.iter().map(|s| s.id().to_string()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        for s in &all {
            assert!(s.help().ends_with('.'), "{}", s.id());
            assert!(!s.roles().is_empty(), "{}", s.id());
            assert!(source(s.id()).is_some());
        }
        assert!(source("nope").is_none());
    }
}
