//! PWM level: one value per period of a repeating pulse (doc 06, "Virtual analog channels").
//!
//! A period runs from one edge into the active level to the next. Its point is placed at the
//! period's start. A line that stops toggling for more than two periods (the expected period, or
//! the last one measured) reads 0 % or 100 % from its last edge for the duty, and has no value
//! for frequency, period and pulse width; either way a problem says why.

use serde::{Deserialize, Serialize};

use logica_core::{Level, Sample};

use super::{Source, SourceOutput};
use crate::format;
use crate::stream::{
    DecoderSetup, Edges, Machine, NewFrame, Out, Pending, Res, StreamDecoder, decoder, run_steps,
};
use crate::types::{FrameKind, ProblemCode, RoleSpec, SettingSpec, Settings, SettingsExt, problem};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Measure {
    Duty,
    Frequency,
    Period,
    Width,
}

impl Measure {
    fn of(settings: &Settings) -> Self {
        match settings.text("measure") {
            "frequency" => Measure::Frequency,
            "period" => Measure::Period,
            "width" => Measure::Width,
            _ => Measure::Duty,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Measure::Duty => "duty",
            Measure::Frequency => "frequency",
            Measure::Period => "period",
            Measure::Width => "pulse width",
        }
    }
}

/// A stretch without edges, from the last edge on.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Quiet {
    from: Sample,
    level: Level,
    /// The longest gap between edges that still counted as toggling, in samples.
    limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PwmLevel {
    rate: f64,
    measure: Measure,
    active: Level,
    lane: u32,
    /// The expected period in samples, if the person gave one.
    expected: Option<f64>,
    /// Index of the next edge to read.
    next: u64,
    /// The last edge read and the level after it.
    last: Option<(Sample, Level)>,
    /// Start of the current period (the last edge into the active level).
    start: Option<Sample>,
    /// End of the active time in the current period.
    active_end: Option<Sample>,
    last_period: Option<f64>,
    periods: u64,
    quiet: Option<Quiet>,
    stops: u64,
}

impl PwmLevel {
    /// The longest time without an edge that still counts as toggling: two periods.
    fn limit(&self) -> Option<u64> {
        // Periods are whole numbers of samples, or a person's choice; rounding up is safe.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        self.expected.or(self.last_period).map(|p| (2.0 * p).ceil() as u64)
    }

    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let t = match ed.edge(0, self.next) {
            Ok(Some(t)) => t,
            Ok(None) => return Ok(false),
            Err(Pending) => {
                // No edge yet: has the line been quiet for long enough to say so?
                if let (None, Some((e, _)), Some(limit)) = (self.quiet, self.last, self.limit())
                    && ed.edge_in(0, e + 1, e + limit + 1)?.is_none()
                {
                    self.stop(limit, out);
                }
                return Err(Pending);
            }
        };
        let level = ed.level_after(0, self.next);
        self.next += 1;
        ed.release_index(0, self.next);
        if let (None, Some((e, _)), Some(limit)) = (self.quiet, self.last, self.limit())
            && t > e + limit
        {
            self.stop(limit, out);
        }
        if let Some(q) = self.quiet.take() {
            self.resume(q, t, out);
        }
        self.last = Some((t, level));
        if level != self.active {
            self.active_end = Some(t);
            return Ok(true);
        }
        if let (Some(start), Some(active_end)) = (self.start, self.active_end)
            && active_end > start
        {
            self.period(start, active_end, t, out);
        }
        self.start = Some(t);
        Ok(true)
    }

    /// A complete period from `start` to `end`, active until `active_end`.
    #[allow(clippy::cast_precision_loss)]
    fn period(&mut self, start: Sample, active_end: Sample, end: Sample, out: &mut Out) {
        let period = (end - start) as f64;
        let active = (active_end - start) as f64;
        out.point(
            start,
            match self.measure {
                Measure::Duty => active / period,
                Measure::Frequency => self.rate / period,
                Measure::Period => period / self.rate,
                Measure::Width => active / self.rate,
            },
        );
        self.last_period = Some(period);
        self.periods += 1;
    }

    /// The line has not toggled for more than `limit` samples since its last edge.
    fn stop(&mut self, limit: u64, out: &mut Out) {
        let Some((from, level)) = self.last else { return };
        if self.measure == Measure::Duty {
            out.point(from, if level == self.active { 1.0 } else { 0.0 });
        } else {
            out.gap(from);
        }
        self.quiet = Some(Quiet { from, level, limit });
        self.stops += 1;
    }

    /// The quiet stretch `q` ends at `to` (the next edge, or the end of the capture).
    #[allow(clippy::cast_precision_loss)]
    fn resume(&mut self, q: Quiet, to: Sample, out: &mut Out) {
        let reads = match (self.measure, q.level == self.active) {
            (Measure::Duty, true) => "it reads 100 %".to_string(),
            (Measure::Duty, false) => "it reads 0 %".to_string(),
            (m, _) => format!("there is no {} here", m.noun()),
        };
        let message = format!(
            "The line stayed {} for longer than two periods ({}), so {reads}.",
            if q.level == Level::High { "high" } else { "low" },
            format::time(q.limit as f64 / self.rate),
        );
        out.add(
            NewFrame::new(q.from, to, self.lane, FrameKind::Info, "stopped")
                .problem(problem(ProblemCode::Timing, message)),
        );
        // A period across the quiet stretch would mix two signals.
        self.start = None;
        self.active_end = None;
    }
}

impl Machine for PwmLevel {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }

    fn at_end(&mut self, ed: &Edges, out: &mut Out) {
        let end = ed.known_end();
        if let (None, Some((e, _)), Some(limit)) = (self.quiet, self.last, self.limit())
            && end > e + limit
        {
            self.stop(limit, out);
        }
        if let Some(q) = self.quiet.take() {
            self.resume(q, end, out);
        }
        out.note(match (self.periods, self.stops) {
            (0, _) => "No complete period: a PWM level needs two edges into the active level.".to_string(),
            (n, 0) => format!("{} periods measured.", format::int(n as f64)),
            (n, s) => format!(
                "{} periods measured; the line stopped toggling {} time{}.",
                format::int(n as f64),
                format::int(s as f64),
                if s == 1 { "" } else { "s" }
            ),
        });
    }
}

pub struct PwmSource {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
}

impl Default for PwmSource {
    fn default() -> Self {
        PwmSource {
            roles: vec![RoleSpec::new("signal", "Signal", "the signal", "The repeating pulse.")],
            settings: vec![
                SettingSpec::choice(
                    "measure",
                    "Value",
                    "duty",
                    &[("duty", "Duty cycle"), ("frequency", "Frequency"), ("period", "Period"), ("width", "Pulse width")],
                )
                .help("Duty is a fraction of the period; frequency, period and pulse width are measured in Hz and seconds."),
                SettingSpec::choice("activeLevel", "Active level", "high", &[("high", "High"), ("low", "Low")])
                    .help("Duty and pulse width count the time spent at this level."),
                SettingSpec::number("expectedPeriod", "Expected period", 0.0)
                    .range(0.0, 3600.0, Some("s"), &[])
                    .help("After two periods without an edge the PWM counts as stopped. 0 uses the last period measured."),
            ],
        }
    }
}

impl Source for PwmSource {
    fn id(&self) -> &str {
        "pwm"
    }
    fn name(&self) -> &str {
        "PWM level"
    }
    fn help(&self) -> &str {
        "One value per period of a repeating pulse: its duty cycle, frequency, period or pulse width."
    }
    fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }
    fn settings(&self) -> &[SettingSpec] {
        &self.settings
    }
    fn output(&self, settings: &Settings) -> SourceOutput {
        match Measure::of(settings) {
            Measure::Duty => SourceOutput::fraction(),
            Measure::Frequency => SourceOutput::quantity("Hz"),
            Measure::Period | Measure::Width => SourceOutput::quantity("s"),
        }
    }
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        let slot = setup.slot("signal").ok_or("Choose a channel for the signal.")?;
        if setup.sample_rate_hz.is_nan() || setup.sample_rate_hz <= 0.0 {
            return Err("The capture has no sample rate, so periods cannot be measured.".into());
        }
        let expected = setup.settings.num("expectedPeriod");
        let m = PwmLevel {
            rate: setup.sample_rate_hz,
            measure: Measure::of(&setup.settings),
            active: if setup.settings.text("activeLevel") == "low" { Level::Low } else { Level::High },
            lane: setup.channels[slot].channel,
            expected: (expected > 0.0).then_some(expected * setup.sample_rate_hz),
            next: 0,
            last: None,
            start: None,
            active_end: None,
            last_period: None,
            periods: 0,
            quiet: None,
            stops: 0,
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

    const RATE: f64 = 1_000_000.0;

    /// A pulse train from `t0`: each `(period, high)` in samples, starting with a rising edge.
    fn pulses(t0: Sample, cycles: &[(u64, u64)]) -> Vec<Sample> {
        let mut t = t0;
        let mut edges = Vec::new();
        for &(period, high) in cycles {
            edges.push(t);
            edges.push(t + high);
            t += period;
        }
        edges.push(t);
        edges
    }

    fn values(points: &[Point]) -> Vec<(Sample, Option<f64>)> {
        points.iter().map(|p| (p.sample, p.x)).collect()
    }

    fn src() -> PwmSource {
        PwmSource::default()
    }

    #[test]
    fn duty_per_period_at_the_period_start() {
        let edges = pulses(10, &[(100, 25), (100, 50), (100, 75)]);
        let out = run(&src(), RATE, &[(Level::Low, edges)], 320, &Settings::new());
        // The last rising edge at 310 starts a period that never completes.
        assert_eq!(values(&out.points), vec![(10, Some(0.25)), (110, Some(0.5)), (210, Some(0.75))]);
        assert!(out.frames.is_empty());
        assert_eq!(out.notes, vec!["3 periods measured."]);
    }

    #[test]
    fn frequency_period_and_width_are_measured_quantities() {
        let edges = pulses(0, &[(200, 50), (400, 100)]);
        let ch = [(Level::Low, edges)];
        let get = |m: &str| {
            let out = run(&src(), RATE, &ch, 650, &settings(&[("measure", m.into())]));
            out.points.iter().map(|p| p.x.unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(get("frequency"), vec![5000.0, 2500.0]);
        assert_eq!(get("period"), vec![0.0002, 0.0004]);
        assert_eq!(get("width"), vec![0.00005, 0.0001]);
        let s = src();
        assert_eq!(s.output(&settings(&[("measure", "frequency".into())])).unit.as_deref(), Some("Hz"));
        assert_eq!(s.output(&settings(&[("measure", "width".into())])).unit.as_deref(), Some("s"));
        assert_eq!(s.output(&Settings::new()), SourceOutput::fraction());
    }

    #[test]
    fn active_low_counts_the_low_time() {
        // Starts high; falling edges start the periods.
        let edges = vec![0, 30, 100, 110, 200];
        let out =
            run(&src(), RATE, &[(Level::High, edges)], 210, &settings(&[("activeLevel", "low".into())]));
        assert_eq!(values(&out.points), vec![(0, Some(0.3)), (100, Some(0.1))]);
    }

    #[test]
    fn a_line_stuck_high_reads_full_duty_with_a_reason() {
        // Two periods of 100, then the line goes high at 200 and stays there until 900; one more
        // period from 950.
        let mut edges = pulses(0, &[(100, 40), (100, 40)]);
        edges.extend([900, 950, 1000, 1050]);
        let out = run(&src(), RATE, &[(Level::Low, edges)], 1100, &Settings::new());
        assert_eq!(
            values(&out.points),
            vec![(0, Some(0.4)), (100, Some(0.4)), (200, Some(1.0)), (950, Some(0.5))]
        );
        assert_eq!(out.frames.len(), 1);
        let f = &out.frames[0];
        assert_eq!((f.start, f.end), (200, 900));
        assert_eq!(f.problems[0].code, ProblemCode::Timing);
        assert_eq!(
            f.problems[0].message,
            "The line stayed high for longer than two periods (200 µs), so it reads 100 %."
        );
        assert_eq!(out.notes, vec!["3 periods measured; the line stopped toggling 1 time."]);
    }

    #[test]
    fn a_line_stuck_low_until_the_end_reads_zero() {
        let edges = pulses(0, &[(100, 40), (100, 40)]);
        // The last edge rises at 200; it falls at 240 and stays low.
        let edges = [edges, vec![240]].concat();
        let out = run(&src(), RATE, &[(Level::Low, edges)], 1000, &Settings::new());
        assert_eq!(values(&out.points).last(), Some(&(240, Some(0.0))));
        assert_eq!((out.frames[0].start, out.frames[0].end), (240, 1000));
        assert!(out.frames[0].problems[0].message.ends_with("so it reads 0 %."));
    }

    #[test]
    fn a_stopped_line_leaves_a_gap_for_measured_quantities() {
        let mut edges = pulses(0, &[(100, 40), (100, 40)]);
        edges.extend([900, 950, 1000, 1050]);
        let out =
            run(&src(), RATE, &[(Level::Low, edges)], 1100, &settings(&[("measure", "frequency".into())]));
        assert_eq!(
            values(&out.points),
            vec![(0, Some(10_000.0)), (100, Some(10_000.0)), (200, None), (950, Some(10_000.0))]
        );
        assert!(out.frames[0].problems[0].message.ends_with("so there is no frequency here."));
    }

    #[test]
    fn exactly_two_periods_without_an_edge_is_still_toggling() {
        // Last period 100; the next edge comes exactly 200 after the last one.
        let edges = vec![0, 40, 100, 300, 340, 400];
        let out = run(&src(), RATE, &[(Level::Low, edges)], 400, &Settings::new());
        assert!(out.frames.is_empty(), "{:?}", out.frames);
        // And a capture that ends exactly two periods after the last edge did not stop either.
        let out = run(&src(), RATE, &[(Level::Low, vec![0, 40, 100])], 300, &Settings::new());
        assert!(out.frames.is_empty());
        let out = run(&src(), RATE, &[(Level::Low, vec![0, 40, 100])], 301, &Settings::new());
        assert_eq!(out.frames.len(), 1);
    }

    #[test]
    fn an_expected_period_detects_a_stop_before_any_period_completes() {
        // One rising edge at 0 and nothing else: with 50 µs expected, it stopped after 100 µs.
        let s = settings(&[("expectedPeriod", 0.00005.into())]);
        let out = run(&src(), RATE, &[(Level::Low, vec![0])], 500, &s);
        assert_eq!(values(&out.points), vec![(0, Some(1.0))]);
        assert!(out.frames[0].problems[0].message.contains("(100 µs)"), "{:?}", out.frames);
        // Without one, nothing can be said.
        let out = run(&src(), RATE, &[(Level::Low, vec![0])], 500, &Settings::new());
        assert!(out.points.is_empty() && out.frames.is_empty());
        assert_eq!(out.notes, vec!["No complete period: a PWM level needs two edges into the active level."]);
    }

    #[test]
    fn a_quiet_line_has_no_values() {
        let out = run(&src(), RATE, &[(Level::High, vec![])], 1000, &Settings::new());
        assert!(out.points.is_empty() && out.frames.is_empty());
    }

    #[test]
    fn a_long_varying_signal_in_any_batching() {
        // A slowly changing duty with two stops, checked against the generated values.
        let mut cycles = Vec::new();
        for k in 0..300u64 {
            cycles.push((1000, 100 + (k * 7) % 800));
        }
        let mut edges = pulses(5, &cycles);
        let t = *edges.last().unwrap();
        edges.extend([t + 10_000, t + 10_500, t + 11_000]);
        let out = run(&src(), RATE, &[(Level::Low, edges)], t + 50_000, &Settings::new());
        for (k, p) in out.points.iter().take(300).enumerate() {
            let k = k as u64;
            assert_eq!(p.sample, 5 + 1000 * k);
            assert_eq!(p.x, Some((100 + (k * 7) % 800) as f64 / 1000.0));
        }
        assert_eq!(out.frames.len(), 2);
    }

    #[test]
    fn needs_a_channel_and_a_sample_rate() {
        let s = src();
        let mut setup = crate::analog::testing::setup(&s, RATE, &[Level::Low], &Settings::new());
        setup.sample_rate_hz = 0.0;
        assert!(s.create(&setup, None).err().unwrap().contains("no sample rate"));
        setup.channels.clear();
        assert_eq!(s.create(&setup, None).err().unwrap(), "Choose a channel for the signal.");
    }
}
