//! A parallel bus (doc 06, "Virtual analog channels"): one channel per bit, an ADC with a parallel
//! output such as the ADC0804.
//!
//! With a clock or strobe, every latch edge (while the enable line is active) reads a word at
//! that sample. Without one, a word counts once every bit has stayed put for the settle time, so
//! bits that change a few samples apart (skew) do not produce values in between.

use serde::{Deserialize, Serialize};

use logica_core::analog::{Encoding, fraction_of_code};
use logica_core::{Level, Sample};

use super::{Source, SourceOutput};
use crate::stream::{DecoderSetup, Edges, Machine, Out, Pending, Res, StreamDecoder, decoder, run_steps};
use crate::types::{RoleSpec, SettingSpec, Settings, SettingsExt};

/// The widest bus a source reads.
pub const MAX_BITS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Latch {
    Rising,
    Falling,
    Both,
    Settled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Parallel {
    /// Slots of D0, D1, … in bit order.
    data: Vec<usize>,
    clock: Option<usize>,
    enable: Option<(usize, Level)>,
    latch: Latch,
    settle: u64,
    bits: u32,
    encoding: Encoding,
    /// Clock mode: the next clock edge to read. Settled mode: the next edge of each data slot.
    next_clock: u64,
    next: Vec<u64>,
    /// Settled mode: where the current stretch of an unchanged word began.
    stretch: Sample,
    words: u64,
    short: u64,
}

impl Parallel {
    fn word(&self, ed: &Edges, s: Sample) -> Res<u64> {
        let mut v = 0;
        for (bit, &slot) in self.data.iter().enumerate() {
            if ed.level_at(slot, s)? == Level::High {
                v |= 1 << bit;
            }
        }
        Ok(v)
    }

    fn emit(&mut self, s: Sample, code: u64, out: &mut Out) {
        if let Ok(x) = fraction_of_code(code, self.bits, self.encoding) {
            out.point(s, x);
            self.words += 1;
        }
    }

    /// The next edge of any data line: received edges are complete up to the batch end, so the
    /// earliest one received is the next even while other lines wait.
    fn next_data_edge(&self, ed: &Edges) -> Res<Option<(usize, Sample)>> {
        let mut best: Option<(usize, Sample)> = None;
        let mut pending = false;
        for (k, &slot) in self.data.iter().enumerate() {
            match ed.edge(slot, self.next[k]) {
                Ok(Some(e)) if best.is_none_or(|(_, b)| e < b) => best = Some((k, e)),
                Ok(_) => {}
                Err(Pending) => pending = true,
            }
        }
        match best {
            Some(b) => Ok(Some(b)),
            None if pending => Err(Pending),
            None => Ok(None),
        }
    }

    fn step_clocked(&mut self, ed: &mut Edges, out: &mut Out, clock: usize) -> Res<bool> {
        let Some(t) = ed.edge(clock, self.next_clock)? else { return Ok(false) };
        let level = ed.level_after(clock, self.next_clock);
        let wanted = match self.latch {
            Latch::Rising => level == Level::High,
            Latch::Falling => level == Level::Low,
            Latch::Both | Latch::Settled => true,
        };
        let enabled = match self.enable {
            Some((slot, active)) => ed.level_at(slot, t)? == active,
            None => true,
        };
        if wanted && enabled {
            let code = self.word(ed, t)?;
            self.emit(t, code, out);
        }
        self.next_clock += 1;
        ed.release_before(t);
        Ok(true)
    }

    fn step_settled(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let Some((k, t)) = self.next_data_edge(ed)? else { return Ok(false) };
        if t - self.stretch >= self.settle {
            let code = self.word(ed, self.stretch)?;
            self.emit(self.stretch, code, out);
        } else if t != self.stretch {
            self.short += 1;
        }
        self.stretch = t;
        self.next[k] += 1;
        ed.release_before(t);
        Ok(true)
    }
}

impl Machine for Parallel {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        match self.clock {
            Some(c) if self.latch != Latch::Settled => run_steps(|| self.step_clocked(ed, out, c)),
            _ => run_steps(|| self.step_settled(ed, out)),
        }
    }

    fn at_end(&mut self, ed: &Edges, out: &mut Out) {
        if self.clock.is_none() || self.latch == Latch::Settled {
            let end = ed.known_end();
            if end.saturating_sub(self.stretch) >= self.settle
                && let Ok(code) = self.word(ed, self.stretch)
            {
                self.emit(self.stretch, code, out);
            }
        }
        out.note(match (self.words, self.short) {
            (0, _) => "No word was read: no latch edge, or no word stayed put for the settle time.".to_string(),
            (n, 0) => format!("{n} words read."),
            (n, s) => format!(
                "{n} words read; {s} words that lasted less than the settle time (bits changing a few samples apart) were left out."
            ),
        });
    }
}

pub struct ParallelSource {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
}

impl Default for ParallelSource {
    fn default() -> Self {
        let mut roles = vec![
            RoleSpec::new(
                "clock",
                "Clock or strobe",
                "the clock or strobe",
                "Latches the word; leave it out when the bus has none.",
            )
            .optional(),
            RoleSpec::new(
                "enable",
                "Enable",
                "the enable line",
                "Words count only while it is active (chip select, RD).",
            )
            .optional(),
        ];
        for bit in 0..MAX_BITS {
            let label = match bit {
                0 => "D0 (least significant bit)".to_string(),
                b => format!("D{b}"),
            };
            let r = RoleSpec::new(&format!("d{bit}"), &label, &format!("bit {bit}"), "One bit of the word.");
            roles.push(if bit == 0 { r } else { r.optional() });
        }
        ParallelSource {
            roles,
            settings: vec![
                SettingSpec::number("bits", "Bits", 8.0)
                    .range(1.0, 16.0, None, &[4.0, 8.0, 10.0, 12.0, 16.0])
                    .help("How many data lines, from D0 on; each needs a channel."),
                SettingSpec::choice(
                    "latch",
                    "Read the word",
                    "rising",
                    &[
                        ("rising", "On the clock's rising edge"),
                        ("falling", "On the clock's falling edge"),
                        ("both", "On both clock edges"),
                        ("settled", "Once it stays put (no clock)"),
                    ],
                )
                .help("Without a clock, a word counts once every bit has stayed put for the settle time."),
                SettingSpec::number("settleSamples", "Settle time (samples)", 3.0)
                    .range(1.0, 1_000_000.0, None, &[])
                    .help("Without a clock: how long every bit must stay put for the word to count."),
                SettingSpec::choice(
                    "enableActive",
                    "Enable is active",
                    "low",
                    &[("low", "Low (usual)"), ("high", "High")],
                ),
                SettingSpec::choice(
                    "encoding",
                    "Code",
                    "unsigned",
                    &[("unsigned", "Unsigned (straight binary)"), ("twos", "Two's complement (signed)")],
                ),
            ],
        }
    }
}

fn bits_of(settings: &Settings) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = settings.num("bits").round().clamp(1.0, 16.0) as u32;
    b
}

impl Source for ParallelSource {
    fn id(&self) -> &str {
        "parallel"
    }
    fn name(&self) -> &str {
        "Parallel bus"
    }
    fn help(&self) -> &str {
        "A word on one channel per bit, such as an ADC with a parallel output: read on a clock edge, or once it stays put."
    }
    fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }
    fn settings(&self) -> &[SettingSpec] {
        &self.settings
    }
    fn output(&self, settings: &Settings) -> SourceOutput {
        let encoding =
            if settings.text("encoding") == "twos" { Encoding::TwosComplement } else { Encoding::Unsigned };
        SourceOutput { bits: Some(bits_of(settings)), encoding, ..SourceOutput::fraction() }
    }
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        let s = &setup.settings;
        let bits = bits_of(s);
        let mut data = Vec::new();
        for bit in 0..bits {
            data.push(setup.slot(&format!("d{bit}")).ok_or_else(|| {
                format!(
                    "Choose a channel for D{bit}: the word has {bits} bits, so D0 to D{} each need one.",
                    bits - 1
                )
            })?);
        }
        let latch = match s.text("latch") {
            "falling" => Latch::Falling,
            "both" => Latch::Both,
            "settled" => Latch::Settled,
            _ => Latch::Rising,
        };
        let clock = setup.slot("clock");
        if clock.is_none() && latch != Latch::Settled {
            return Err(
                "Choose a channel for the clock or strobe, or read the word once it stays put.".into()
            );
        }
        let active = if s.text("enableActive") == "high" { Level::High } else { Level::Low };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let settle = s.num("settleSamples").round().max(1.0) as u64;
        let m = Parallel {
            next: vec![0; data.len()],
            data,
            clock,
            enable: setup.slot("enable").map(|e| (e, active)),
            latch,
            settle,
            bits,
            encoding: if s.text("encoding") == "twos" {
                Encoding::TwosComplement
            } else {
                Encoding::Unsigned
            },
            next_clock: 0,
            stretch: setup.start,
            words: 0,
            short: 0,
        };
        decoder(m, setup, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use logica_core::analog::Point;

    use super::*;
    use crate::analog::testing::run;
    use crate::types::settings;

    const RATE: f64 = 1e6;

    /// Lines for `words` written at the given samples: data bits D0.. first, then any extra lines.
    fn bus(bits: usize, words: &[(Sample, u64)], skew: u64) -> Vec<(Level, Vec<Sample>)> {
        let mut lines: Vec<(Level, Vec<Sample>)> = (0..bits).map(|_| (Level::Low, Vec::new())).collect();
        let mut cur = 0u64;
        for &(t, w) in words {
            for (b, line) in lines.iter_mut().enumerate() {
                if (w ^ cur) >> b & 1 == 1 {
                    // each bit a little later than the one before: skew
                    line.1.push(t + skew * b as u64);
                }
            }
            cur = w;
        }
        lines
    }

    fn src() -> ParallelSource {
        ParallelSource::default()
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn codes(points: &[Point], bits: u32) -> Vec<u64> {
        points.iter().map(|p| (p.x.unwrap() * f64::from(1u32 << bits)).round() as u64).collect()
    }

    /// Runs the source; the lines are in role order (clock, enable, d0, d1, …).
    fn with_roles(lines: Vec<(Level, Vec<Sample>)>, s: &Settings, end: Sample) -> crate::stream::Output {
        run(&src(), RATE, &lines, end, s)
    }

    #[test]
    fn settled_words_without_a_clock_ignore_skew() {
        // the roles go clock, enable, d0, d1, …: no clock and no enable, so two quiet lines first
        let words = [(100, 0x5A), (300, 0xFF), (500, 0x01)];
        let mut lines = vec![(Level::Low, vec![]), (Level::High, vec![])];
        lines.extend(bus(8, &words, 1));
        let s = settings(&[("latch", "settled".into()), ("settleSamples", 5.0.into())]);
        let out = with_roles(lines, &s, 800);
        // the word before the first change (0) counts too; skewed bits produce nothing between
        assert_eq!(codes(&out.points, 8), vec![0, 0x5A, 0xFF, 0x01]);
        // each word from its last changing bit on (0x5A changes bits 1, 3, 4 and 6: the last at 106)
        assert_eq!(out.points.iter().map(|p| p.sample).collect::<Vec<_>>(), vec![0, 106, 307, 507]);
        assert!(out.notes[0].contains("left out"), "{:?}", out.notes);
    }

    #[test]
    fn a_word_that_does_not_settle_before_the_end_is_left_out() {
        let mut lines = vec![(Level::Low, vec![]), (Level::High, vec![])];
        lines.extend(bus(4, &[(100, 0x3)], 0));
        let s =
            settings(&[("latch", "settled".into()), ("settleSamples", 50.0.into()), ("bits", 4.0.into())]);
        assert_eq!(codes(&with_roles(lines.clone(), &s, 120).points, 4), vec![0]);
        assert_eq!(codes(&with_roles(lines, &s, 150).points, 4), vec![0, 3]);
    }

    #[test]
    fn a_clock_latches_the_word_while_enabled() {
        // words change at 100, 300, 500; the strobe rises at 150, 350, 550; enable (low) is off
        // for the second one
        let words = [(100, 0x12), (300, 0x34), (500, 0x56)];
        let mut lines = vec![(Level::Low, vec![150, 160, 350, 360, 550, 560]), (Level::Low, vec![340, 370])];
        lines.extend(bus(8, &words, 2));
        let out = with_roles(lines.clone(), &Settings::new(), 700);
        assert_eq!(codes(&out.points, 8), vec![0x12, 0x56]);
        assert_eq!(out.points.iter().map(|p| p.sample).collect::<Vec<_>>(), vec![150, 550]);
        // the falling edge reads the same words ten samples later; both edges read each twice
        let fall = with_roles(lines.clone(), &settings(&[("latch", "falling".into())]), 700);
        assert_eq!(fall.points.iter().map(|p| p.sample).collect::<Vec<_>>(), vec![160, 560]);
        let both = with_roles(lines, &settings(&[("latch", "both".into())]), 700);
        assert_eq!(codes(&both.points, 8), vec![0x12, 0x12, 0x56, 0x56]);
    }

    #[test]
    fn twos_complement_and_fewer_bits() {
        let mut lines = vec![(Level::Low, vec![50, 60, 150, 160]), (Level::High, vec![])];
        lines.extend(bus(4, &[(10, 0x8), (110, 0x7)], 0));
        // enable stays high, so with "active high" it is always on
        let s =
            settings(&[("bits", 4.0.into()), ("encoding", "twos".into()), ("enableActive", "high".into())]);
        let out = with_roles(lines, &s, 300);
        // -8 is the bottom of the range (0), 7 just under the top
        assert_eq!(out.points.iter().map(|p| p.x.unwrap()).collect::<Vec<_>>(), vec![0.0, 15.0 / 16.0]);
        assert_eq!(
            src()
                .output(&SettingSpec::with_defaults(src().settings(), &settings(&[("bits", 4.0.into())])))
                .bits,
            Some(4)
        );
    }

    #[test]
    fn missing_lines_are_named() {
        let s = src();
        let mut setup = crate::analog::testing::setup(
            &s,
            RATE,
            &[Level::Low, Level::Low, Level::Low, Level::Low],
            &Settings::new(),
        );
        // clock, enable, d0, d1 bound; 8 bits need d0 to d7
        let e = s.create(&setup, None).err().unwrap();
        assert_eq!(e, "Choose a channel for D2: the word has 8 bits, so D0 to D7 each need one.");
        setup.settings = SettingSpec::with_defaults(s.settings(), &settings(&[("bits", 2.0.into())]));
        setup.channels.retain(|c| c.role != "clock");
        let e = s.create(&setup, None).err().unwrap();
        assert!(e.starts_with("Choose a channel for the clock or strobe"), "{e}");
    }
}
