//! PDM and sigma-delta bitstreams (doc 06, "Virtual analog channels"): a data line read on every
//! clock edge, filtered back into values by a sinc³ CIC filter with decimation ratio R.
//!
//! The filter works on whole numbers (wrapping, as CIC filters do), so it is exact and the same
//! in any batching. With bits of 0 and 1 the output over R³ is the share of ones: the fraction of
//! full scale. A value comes every R bits, at the clock edge that completes it; the first three
//! fill the filter and are left out.

use serde::{Deserialize, Serialize};

use logica_core::Level;

use super::{Source, SourceOutput};
use crate::format;
use crate::stream::{DecoderSetup, Edges, Machine, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::{RoleSpec, SettingSpec, Settings, SettingsExt};

/// Filter stages (sinc³).
const ORDER: usize = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cic {
    ratio: u64,
    integrators: [u64; ORDER],
    delays: [u64; ORDER],
    /// Bits since the last output.
    count: u64,
    /// Outputs so far, including the ones that fill the filter.
    outputs: u64,
}

impl Cic {
    fn new(ratio: u64) -> Self {
        Cic { ratio, integrators: [0; ORDER], delays: [0; ORDER], count: 0, outputs: 0 }
    }

    /// Takes one bit; every `ratio` bits returns the filtered sum, from 0 to ratio³.
    fn push(&mut self, bit: bool) -> Option<u64> {
        let mut x = u64::from(bit);
        for i in &mut self.integrators {
            *i = i.wrapping_add(x);
            x = *i;
        }
        self.count += 1;
        if self.count < self.ratio {
            return None;
        }
        self.count = 0;
        let mut y = x;
        for d in &mut self.delays {
            let prev = *d;
            *d = y;
            y = y.wrapping_sub(prev);
        }
        self.outputs += 1;
        Some(y)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pdm {
    rate: f64,
    clock: usize,
    data: usize,
    /// The level after a sampling clock edge.
    edge: Level,
    cic: Cic,
    next: u64,
    /// The first clock edge read, for the note.
    first: Option<u64>,
    last: Option<u64>,
}

impl Pdm {
    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let Some(t) = ed.edge(self.clock, self.next)? else { return Ok(false) };
        if ed.level_after(self.clock, self.next) == self.edge {
            let bit = ed.level_at(self.data, t)? == Level::High;
            self.first.get_or_insert(t);
            self.last = Some(t);
            if let Some(y) = self.cic.push(bit)
                && self.cic.outputs > ORDER as u64
            {
                #[allow(clippy::cast_precision_loss)]
                let full = self.cic.ratio.pow(3) as f64;
                #[allow(clippy::cast_precision_loss)]
                out.point(t, y as f64 / full);
            }
        }
        self.next += 1;
        ed.release_before(t);
        Ok(true)
    }
}

impl Machine for Pdm {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }

    fn at_end(&mut self, _ed: &Edges, out: &mut Out) {
        let values = self.cic.outputs.saturating_sub(ORDER as u64);
        let r = self.cic.ratio;
        match (self.first, self.last) {
            (Some(a), Some(b)) if values > 0 && b > a => {
                #[allow(clippy::cast_precision_loss)]
                let bits = (self.cic.outputs * r + self.cic.count) as f64;
                #[allow(clippy::cast_precision_loss)]
                let clock = (bits - 1.0) / ((b - a) as f64 / self.rate);
                out.note(format!(
                    "{values} values from a {} bit clock, one every {r} bits ({}); the first {} bits fill the filter and give none.",
                    format::hz(clock),
                    format::hz(clock / r as f64),
                    format::int((ORDER as u64 * r) as f64),
                ));
            }
            _ => out.note(format!(
                "No value: a value needs {} bits after the {} that fill the filter.",
                format::int(r as f64),
                format::int((ORDER as u64 * r) as f64)
            )),
        }
    }
}

pub struct PdmSource {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
}

impl Default for PdmSource {
    fn default() -> Self {
        PdmSource {
            roles: vec![
                RoleSpec::new("clock", "Clock", "the bit clock", "Evenly spaced pulses, one per bit."),
                RoleSpec::new("data", "Data", "the bitstream", "Ones and zeros whose share is the level."),
            ],
            settings: vec![
                SettingSpec::choice("edge", "Read the bit on", "rising", &[("rising", "The clock's rising edge"), ("falling", "The clock's falling edge")])
                    .help("Stereo PDM microphones share a line: one on the rising edge, the other on the falling one."),
                SettingSpec::number("ratio", "Decimation ratio", 64.0)
                    .range(2.0, 1024.0, None, &[16.0, 32.0, 64.0, 128.0, 256.0])
                    .help("Bits per value: more gives finer values at a lower rate."),
            ],
        }
    }
}

impl Source for PdmSource {
    fn id(&self) -> &str {
        "pdm"
    }
    fn name(&self) -> &str {
        "PDM or sigma-delta"
    }
    fn help(&self) -> &str {
        "A bitstream whose share of ones is the level, as from a PDM microphone or a sigma-delta modulator, filtered back into values."
    }
    fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }
    fn settings(&self) -> &[SettingSpec] {
        &self.settings
    }
    fn output(&self, _settings: &Settings) -> SourceOutput {
        SourceOutput::fraction()
    }
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        let clock = setup.slot("clock").ok_or("Choose a channel for the bit clock.")?;
        let data = setup.slot("data").ok_or("Choose a channel for the bitstream.")?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ratio = setup.settings.num("ratio").round().clamp(2.0, 1024.0) as u64;
        let m = Pdm {
            rate: setup.sample_rate_hz,
            clock,
            data,
            edge: if setup.settings.text("edge") == "falling" { Level::Low } else { Level::High },
            cic: Cic::new(ratio),
            next: 0,
            first: None,
            last: None,
        };
        decoder(m, setup, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use logica_core::Sample;

    use super::*;
    use crate::analog::testing::run;
    use crate::types::settings;

    const RATE: f64 = 4e6;

    /// A 1 MHz clock (4 samples a bit, rising at 4k) and a first-order sigma-delta bitstream of
    /// `level(t)`, changed at the falling clock edges so it is stable when read.
    fn pdm(bits: usize, level: impl Fn(f64) -> f64) -> (Vec<(Level, Vec<Sample>)>, Sample) {
        let mut clk = Vec::new();
        let mut data = Vec::new();
        let (mut acc, mut cur) = (0.0, false);
        for k in 0..bits as u64 {
            let rise = 4 + 4 * k;
            clk.extend([rise, rise + 2]);
            #[allow(clippy::cast_precision_loss)]
            let t = rise as f64 / RATE;
            acc += level(t);
            let bit = acc >= 1.0;
            if bit {
                acc -= 1.0;
            }
            if bit != cur {
                // set up half a bit before the rising edge that reads it
                data.push(rise - 2);
                cur = bit;
            }
        }
        let end = 4 + 4 * bits as u64 + 4;
        (vec![(Level::Low, clk), (Level::Low, data)], end)
    }

    #[test]
    fn the_filter_gives_the_share_of_ones() {
        let mut c = Cic::new(4);
        let outs: Vec<u64> = (0..40).filter_map(|k| c.push(k % 4 == 0)).collect();
        // a quarter of ones: 64 × 0.25 = 16 once the three stages are full
        assert_eq!(outs[3..], [16; 7]);
        let mut c = Cic::new(8);
        let ones: Vec<u64> = (0..64).filter_map(|_| c.push(true)).collect();
        assert_eq!(*ones.last().unwrap(), 512, "all ones is full scale");
    }

    #[test]
    fn a_steady_level_reads_back() {
        let (ch, end) = pdm(4096, |_| 0.3);
        let out = run(&PdmSource::default(), RATE, &ch, end, &settings(&[("ratio", 64.0.into())]));
        // 4096 bits at 64 a value: 64 values, the first three fill the filter
        assert_eq!(out.points.len(), 61);
        for p in &out.points {
            assert!((p.x.unwrap() - 0.3).abs() < 0.01, "{p:?}");
        }
        assert!(
            out.notes[0].starts_with("61 values from a 1 MHz bit clock, one every 64 bits (15.6 kHz)"),
            "{:?}",
            out.notes
        );
    }

    #[test]
    fn a_sine_reads_back_with_the_filters_delay() {
        // 1 kHz, 10 % to 90 %; the sinc³ filter delays by about 1.5 values (3 × 63 / 2 bits)
        let level = |t: f64| 0.5 + 0.4 * (std::f64::consts::TAU * 1000.0 * t).sin();
        let (ch, end) = pdm(8192, level);
        let out = run(&PdmSource::default(), RATE, &ch, end, &settings(&[("ratio", 64.0.into())]));
        let delay = 1.5 * 63.0 * 4.0 / RATE;
        for p in &out.points {
            #[allow(clippy::cast_precision_loss)]
            let t = p.sample as f64 / RATE - delay;
            assert!((p.x.unwrap() - level(t)).abs() < 0.03, "at {t}: {:?} against {}", p.x, level(t));
        }
    }

    #[test]
    fn the_falling_edge_and_too_short_a_stream() {
        let (ch, end) = pdm(512, |_| 0.75);
        let fall = run(
            &PdmSource::default(),
            RATE,
            &ch,
            end,
            &settings(&[("edge", "falling".into()), ("ratio", 32.0.into())]),
        );
        assert_eq!(fall.points.len(), 13);
        assert!(fall.points.iter().all(|p| (p.x.unwrap() - 0.75).abs() < 0.04), "{:?}", fall.points);
        let (ch, end) = pdm(150, |_| 0.5);
        let short = run(&PdmSource::default(), RATE, &ch, end, &settings(&[("ratio", 64.0.into())]));
        assert!(short.points.is_empty());
        assert_eq!(short.notes, vec!["No value: a value needs 64 bits after the 192 that fill the filter."]);
    }

    #[test]
    fn both_lines_are_needed() {
        let s = PdmSource::default();
        let mut setup = crate::analog::testing::setup(&s, RATE, &[Level::Low, Level::Low], &Settings::new());
        setup.channels.retain(|c| c.role != "data");
        assert_eq!(s.create(&setup, None).err().unwrap(), "Choose a channel for the bitstream.");
        setup.channels.clear();
        assert_eq!(s.create(&setup, None).err().unwrap(), "Choose a channel for the bit clock.");
    }
}
