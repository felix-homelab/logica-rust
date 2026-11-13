//! PWM or a plain clock: a repeating pulse that carries a level, not bytes.

use serde::{Deserialize, Serialize};

use logica_core::{Level, Sample};

use crate::format::{self, pct};
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PwmMachine {
    rate: f64,
    active: Level,
    lane: u32,
    next: u64,
    prev: Option<Sample>,
    active_end: Option<Sample>,
    n: u64,
    min_duty: Option<f64>,
    max_duty: Option<f64>,
    sum_period: f64,
}

impl PwmMachine {
    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let Some(t) = ed.edge(0, self.next)? else { return Ok(false) };
        let level = ed.level_after(0, self.next);
        self.next += 1;
        ed.release_index(0, self.next);
        if level != self.active {
            self.active_end = Some(t);
            return Ok(true);
        }
        // a cycle runs from one active-going edge to the next
        if let (Some(prev), Some(active_end)) = (self.prev, self.active_end)
            && active_end > prev
        {
            let period = (t - prev) as f64;
            let high = (active_end - prev) as f64;
            let duty = high / period;
            self.n += 1;
            self.sum_period += period;
            self.min_duty = Some(self.min_duty.map_or(duty, |m| m.min(duty)));
            self.max_duty = Some(self.max_duty.map_or(duty, |m| m.max(duty)));
            out.add(NewFrame::new(prev, t, self.lane, FrameKind::Cycle, pct(duty, 1)).value(duty).detail(
                format!(
                    "Period {} ({}), {} for {}",
                    format::time(period / self.rate),
                    format::hz(self.rate / period),
                    if self.active == Level::High { "high" } else { "low" },
                    format::time(high / self.rate)
                ),
            ));
        }
        self.prev = Some(t);
        Ok(true)
    }
}

impl Machine for PwmMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }
    fn at_end(&mut self, _ed: &Edges, out: &mut Out) {
        if self.n == 0 {
            out.note("No complete cycles found.");
            return;
        }
        let p = self.sum_period / self.n as f64;
        let (lo, hi) = (self.min_duty.unwrap_or(0.0), self.max_duty.unwrap_or(0.0));
        let duty =
            if (hi - lo).abs() < 0.001 { pct(lo, 1) } else { format!("{} to {}", pct(lo, 1), pct(hi, 1)) };
        out.note(format!(
            "{} cycles, average period {} ({}), duty {duty}. PWM carries a level, not bytes, so there is nothing for the byte analyser.",
            format::int(self.n as f64),
            format::time(p / self.rate),
            format::hz(self.rate / p)
        ));
    }
}

fn detect(ctx: &DetectContext<'_>, specs: &[SettingSpec]) -> DetectorOutput {
    let mut out = DetectorOutput::default();
    for ch in &ctx.channels {
        let reject = |because: String| Rejection { protocol_id: "pwm".into(), because };
        let Some(p) = &ctx.m(ch.index).periodic else {
            out.rejections.push(reject(format!("the pulses on channel {} are not evenly spaced", ch.index)));
            continue;
        };
        let pulses: u64 = p.pulses_per_burst.iter().sum();
        let long_runs = p.continuous || pulses as f64 / p.pulses_per_burst.len() as f64 >= 20.0;
        if !long_runs {
            out.rejections.push(reject(format!(
                "channel {} pulses in short bursts, like a bus clock rather than PWM",
                ch.index
            )));
            continue;
        }
        let rate = ctx.sample_rate_hz;
        let f = rate / p.period_samples;
        let (period_s, high_s) = (p.period_samples / rate, p.high_samples / rate);
        let servo = period_s > 0.018 && period_s < 0.022 && high_s > 0.0005 && high_s < 0.0025;
        let alone = ctx.bus.channels.len() == 1;
        let mut ev = vec![
            evidence(
                format!(
                    "Channel {} repeats every {} ({}) and is high for {} each time: {} duty.",
                    ch.index,
                    format::time(period_s),
                    format::hz(f),
                    format::time(high_s),
                    pct(p.duty_cycle, 1)
                ),
                true,
            ),
            evidence(
                format!(
                    "{} % of periods are the same length{}.",
                    (p.regularity * 100.0).round(),
                    if p.continuous { ", and it never pauses" } else { "" }
                ),
                p.regularity > 0.9,
            ),
        ];
        if alone {
            ev.push(evidence("No other channel is busy with it, so it is not a bus clock.", true));
        } else {
            ev.push(evidence(
                "Other channels are busy at the same moments, so this may be the clock of a bus.",
                false,
            ));
        }
        if servo {
            ev.push(evidence(
                "A 20 ms period with a 1 to 2 ms pulse is a hobby servo signal; 1.5 ms is centre.",
                true,
            ));
        } else if (p.duty_cycle - 0.5).abs() < 0.03 {
            ev.push(evidence("With 50 % duty it may simply be a clock.", true));
        }
        out.detections.push(Detection {
            protocol_id: "pwm".into(),
            channels: vec![ch.index],
            roles: roles(&[("signal", Some(ch.index))]),
            settings: SettingSpec::with_defaults(specs, &Settings::new()),
            notes: [
                (
                    "role:signal".to_string(),
                    note(0.9, format!("Channel {} is the repeating pulse.", ch.index)),
                ),
                (
                    "activeLevel".to_string(),
                    note(0.7, "Duty is normally the high time divided by the period."),
                ),
            ]
            .into_iter()
            .collect(),
            confidence: clamp01(
                if alone { 0.65 } else { 0.35 } + 0.25 * p.regularity + if p.continuous { 0.1 } else { 0.0 },
            ),
            evidence: ev,
            warnings: if p.period_samples < 20.0 {
                vec![format!(
                    "Only {} samples per period, so the duty reading is coarse.",
                    format::fixed(p.period_samples, 1)
                )]
            } else {
                vec![]
            },
        });
    }
    out
}

pub struct Pwm {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl Default for Pwm {
    fn default() -> Self {
        Pwm {
            roles: vec![RoleSpec::new("signal", "Signal", "the signal", "The repeating pulse.")],
            settings: vec![SettingSpec::choice("activeLevel", "Duty counts the time spent", "high", &[("high", "High"), ("low", "Low")])],
            info: ProtocolInfo {
                what: "A repeating pulse. It carries a level (brightness, speed, position), not bytes.".into(),
                confirm: "Period and high time stay the same from pulse to pulse, or drift slowly.".into(),
                decoder_tips: "Measure the period and the high time: frequency = 1 ÷ period, duty = high time ÷ period. A 20 ms period with 1 to 2 ms pulses is a servo signal.".into(),
                lookalikes: "If it shares its timing with other lines, it may be a bus clock.".into(),
                symptoms: Vec::new(),
            },
        }
    }
}

impl Protocol for Pwm {
    fn id(&self) -> &str {
        "pwm"
    }
    fn name(&self) -> &str {
        "PWM or a plain clock"
    }
    fn group(&self) -> &str {
        "PWM"
    }
    fn info(&self) -> &ProtocolInfo {
        &self.info
    }
    fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }
    fn settings(&self) -> &[SettingSpec] {
        &self.settings
    }
    fn describe(&self, s: &Settings) -> String {
        format!("duty from {} time", s.text("activeLevel"))
    }
    fn detect(&self, ctx: &DetectContext<'_>) -> Option<DetectorOutput> {
        Some(detect(ctx, &self.settings))
    }
    fn has_decoder(&self) -> bool {
        true
    }
    fn decoder(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        let slot = setup.slot("signal").ok_or("Choose a channel for the signal.")?;
        let m = PwmMachine {
            rate: setup.sample_rate_hz,
            active: if setup.settings.text("activeLevel") == "low" { Level::Low } else { Level::High },
            lane: setup.channels[slot].channel,
            next: 0,
            prev: None,
            active_end: None,
            n: 0,
            min_duty: None,
            max_duty: None,
            sum_period: 0.0,
        };
        decoder(m, setup, snapshot)
    }
}
