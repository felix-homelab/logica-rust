//! I2S audio: a free-running bit clock, word select and serial data.
//!
//! Each half of the word-select cycle is one channel's sample slot. Data is read on rising edges of
//! the bit clock, most significant bit first; in standard I2S a word starts one clock after word
//! select changes, left-justified starts right away.

use serde::{Deserialize, Serialize};

use logica_core::{Level, Sample};

use crate::format;
use crate::num;
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;
use crate::window::WinChannel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Side {
    Left,
    Right,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::Left => "Left",
            Side::Right => "Right",
        }
    }
}

/// The word being read.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Word {
    side: Side,
    /// Bits read so far.
    bit: u32,
    value: u64,
    start: Sample,
}

/// Slots of the bound channels.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Slots {
    sck: usize,
    ws: usize,
    sd: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct I2sMachine {
    slots: Slots,
    /// Clocks between a word-select change and the first bit of the word.
    delay: u32,
    word_bits: u32,
    /// Word-select level of the left channel.
    left_level: Level,
    sd_lane: u32,
    /// Where the decode starts (the packet start when the clock never moves).
    start: Sample,
    opened: bool,
    /// Absolute index of the next bit-clock edge.
    next: u64,
    /// Word select at the previous rising clock edge.
    prev_ws: Option<Level>,
    /// Side of the next word and the clocks still to wait before it starts.
    pending: Option<(Side, u32)>,
    word: Option<Word>,
    samples: u64,
    /// All complete words ORed together (detection reads the unused low bits from it).
    or_values: u64,
}

impl I2sMachine {
    fn new(s: &Settings, slots: Slots, sd_lane: u32, start: Sample) -> Self {
        let word_bits = num::to_u64(s.num("wordBits").round()).clamp(1, 32);
        I2sMachine {
            slots,
            delay: u32::from(s.text("format") == "i2s"),
            word_bits: u32::try_from(word_bits).unwrap_or(16),
            left_level: if s.text("leftWhen") == "high" { Level::High } else { Level::Low },
            sd_lane,
            start,
            opened: false,
            next: 0,
            prev_ws: None,
            pending: None,
            word: None,
            samples: 0,
            or_values: 0,
        }
    }

    /// Emits the word being read if it is complete; it ends at `end`.
    fn finish_word(&mut self, end: Sample, out: &mut Out) {
        let Some(w) = &self.word else { return };
        if w.bit < self.word_bits {
            return;
        }
        let v = i64::try_from(w.value).unwrap_or(0);
        let signed = if v >= 1i64 << (self.word_bits - 1) { v - (1i64 << self.word_bits) } else { v };
        self.samples += 1;
        self.or_values |= w.value;
        let side = w.side.name();
        out.add(
            NewFrame::new(w.start, end, self.sd_lane, FrameKind::Sample, format!("{} {signed}", &side[..1]))
                .value(signed as f64)
                .detail(format!("{side} channel sample: {signed}")),
        );
    }

    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let sl = self.slots;
        if !self.opened {
            let first = ed.edge(sl.sck, 0)?;
            out.open_packet(first.unwrap_or(self.start), "Audio");
            self.opened = true;
            return Ok(true);
        }
        let Some(t) = ed.edge(sl.sck, self.next)? else { return Ok(false) };
        if ed.level_after(sl.sck, self.next) != Level::High {
            // falling edge: nothing is read on it
            self.next += 1;
            ed.release_index(sl.sck, self.next);
            return Ok(true);
        }
        // read everything first
        let ws = ed.level_at(sl.ws, t)?;
        let bit = u64::from(ed.level_at(sl.sd, t)?.is_high());
        // commit
        self.next += 1;
        if self.prev_ws != Some(ws) {
            if self.prev_ws.is_some() {
                let side = if ws == self.left_level { Side::Left } else { Side::Right };
                self.pending = Some((side, self.delay));
            }
            self.prev_ws = Some(ws);
        }
        match self.pending {
            Some((side, 0)) => {
                self.finish_word(t, out);
                self.word = Some(Word { side, bit: 0, value: 0, start: t });
                self.pending = None;
            }
            Some((side, n)) => self.pending = Some((side, n - 1)),
            None => {}
        }
        if let Some(w) = &mut self.word
            && w.bit < self.word_bits
        {
            w.value = w.value * 2 + bit;
            w.bit += 1;
        }
        ed.release_index(sl.sck, self.next);
        ed.release_before(t);
        Ok(true)
    }
}

impl Machine for I2sMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }
    fn at_end(&mut self, _ed: &Edges, out: &mut Out) {
        if self.samples > 0 {
            out.note(format!("{} samples decoded.", format::int(self.samples as f64)));
        }
    }
}

/// Decodes window channels with `settings` and returns the complete words ORed together.
fn trial_or(
    sck: &WinChannel,
    ws: &WinChannel,
    sd: &WinChannel,
    settings: &Settings,
    w: (Sample, Sample),
) -> u64 {
    let mut ed = Edges::complete(
        vec![
            (sck.initial, sck.edges.clone()),
            (ws.initial, ws.edges.clone()),
            (sd.initial, sd.edges.clone()),
        ],
        w.1,
    );
    let mut m = I2sMachine::new(settings, Slots { sck: 0, ws: 1, sd: 2 }, sd.index, w.0);
    let mut out = Out::new(false);
    m.run(&mut ed, &mut out);
    m.or_values
}

fn detect(ctx: &DetectContext<'_>, specs: &[SettingSpec]) -> DetectorOutput {
    let reject = |b: &str| DetectorOutput::rejected("i2s", b);
    let chs: Vec<&WinChannel> = ctx.channels.iter().copied().filter(|c| !c.edges.is_empty()).collect();
    if chs.len() < 3 {
        return reject("I2S needs three lines busy together (bit clock, word select, data)");
    }
    let mut periodic: Vec<&WinChannel> =
        chs.iter().copied().filter(|c| ctx.m(c.index).periodic.is_some()).collect();
    periodic.sort_by_key(|c| std::cmp::Reverse(c.edges.len()));
    let Some(sck) = periodic.first().copied() else {
        return reject("no line has a clock that runs all the time");
    };
    let Some(per) = ctx.m(sck.index).periodic.as_ref().filter(|p| p.continuous) else {
        return reject("no line has a clock that runs all the time");
    };
    let p = per.period_samples;
    let mut found: Option<(&WinChannel, u64)> = None;
    for &c in &periodic[1..] {
        let Some(q) = &ctx.m(c.index).periodic else { continue };
        let r = q.period_samples / p;
        let whole = r.round();
        if (r - whole).abs() < 0.05 && whole % 2.0 == 0.0 && whole >= 8.0 && (q.duty_cycle - 0.5).abs() < 0.1
        {
            found = Some((c, num::to_u64(whole)));
            break;
        }
    }
    let Some((ws, ratio)) = found else {
        return reject("no line is a square wave at a whole fraction of the clock, as word select would be");
    };
    let Some(sd) = chs.iter().copied().find(|c| c.index != sck.index && c.index != ws.index) else {
        return reject("there is no data line");
    };
    let rate = ctx.sample_rate_hz;
    let fs = rate / (p * ratio as f64);
    let audio = fs > 7000.0 && fs < 200_000.0;
    let slot_bits = ratio / 2;
    // Samples are often shorter than the slot (16 bits in a 32-bit slot): the unused low bits are always 0.
    let mut sample_bits = slot_bits.min(32);
    let trial = SettingSpec::with_defaults(specs, &settings(&[("wordBits", (sample_bits as f64).into())]));
    let or = trial_or(sck, ws, sd, &trial, (ctx.window.start, ctx.window.end));
    if or != 0 {
        let tz = u64::from(or.trailing_zeros()).min(sample_bits);
        let used = sample_bits - tz;
        sample_bits =
            [8, 16, 20, 24, 32].into_iter().find(|&b| b >= used && b <= sample_bits).unwrap_or(sample_bits);
    }
    let settings = SettingSpec::with_defaults(specs, &settings(&[("wordBits", (sample_bits as f64).into())]));
    let mut channels = vec![sck.index, ws.index, sd.index];
    channels.sort_unstable();
    let notes = [
        ("role:sck", note(0.9, format!("Channel {} is the fastest clock and never stops.", sck.index))),
        (
            "role:ws",
            note(
                0.9,
                format!("Channel {} is a square wave with exactly {ratio} clock pulses per cycle.", ws.index),
            ),
        ),
        ("role:sd", note(0.8, format!("Channel {} is the remaining line.", sd.index))),
        (
            "format",
            note(0.5, "Standard I2S is most common. If the samples look like noise, try left-justified."),
        ),
        (
            "wordBits",
            note(
                0.7,
                if sample_bits < slot_bits {
                    format!(
                        "Each slot has {slot_bits} clock pulses, but the last {} bits are always 0, so the samples are {sample_bits} bits long.",
                        slot_bits - sample_bits
                    )
                } else {
                    format!("Each half of the word-select cycle has {slot_bits} clock pulses.")
                },
            ),
        ),
        ("leftWhen", note(0.6, "Standard I2S sends the left channel while word select is low.")),
    ];
    let confidence =
        clamp01(0.5 + if ratio == 32 || ratio == 64 { 0.2 } else { 0.1 } + if audio { 0.2 } else { 0.0 });
    DetectorOutput {
        detections: vec![Detection {
            protocol_id: "i2s".into(),
            channels,
            roles: roles(&[("sck", Some(sck.index)), ("ws", Some(ws.index)), ("sd", Some(sd.index))]),
            settings,
            notes: notes.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            confidence,
            evidence: vec![
                evidence(
                    format!(
                        "Channel {} is a clock at {} that never stops: the bit clock.",
                        sck.index,
                        format::hz(rate / p)
                    ),
                    true,
                ),
                evidence(
                    format!(
                        "Channel {} is a square wave at {}, exactly {ratio} clock pulses per cycle: the word select.{}",
                        ws.index,
                        format::hz(fs),
                        if audio { " That is an audio sample rate." } else { "" }
                    ),
                    true,
                ),
                evidence(
                    format!(
                        "So each sample slot has {slot_bits} bits, and the audio sample rate is {}.",
                        format::hz(fs)
                    ),
                    true,
                ),
            ],
            warnings: vec![],
        }],
        rejections: vec![],
    }
}

pub struct I2s {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl Default for I2s {
    fn default() -> Self {
        I2s {
            roles: vec![
                RoleSpec::new("sck", "Bit clock (SCK, BCLK)", "the bit clock", "Runs all the time."),
                RoleSpec::new(
                    "ws",
                    "Word select (WS, LRCLK)",
                    "word select",
                    "Slow square wave at the sample rate.",
                ),
                RoleSpec::new("sd", "Data (SD)", "the data line", "Changes with the bit clock."),
            ],
            settings: vec![
                SettingSpec::choice(
                    "format",
                    "Format",
                    "i2s",
                    &[("i2s", "Standard I2S (1-bit delay)"), ("left", "Left-justified (no delay)")],
                )
                .help("Standard I2S starts each word one clock after word select changes. Left-justified starts right away."),
                SettingSpec::number("wordBits", "Bits per sample", 16.0).range(4.0, 32.0, None, &[16.0, 24.0, 32.0]),
                SettingSpec::choice(
                    "leftWhen",
                    "Left channel when word select is",
                    "low",
                    &[("low", "Low (standard I2S)"), ("high", "High")],
                ),
            ],
            info: ProtocolInfo {
                what: "Digital audio between chips: bit clock, word select, data.".into(),
                confirm: "The bit clock never stops. A second line is a slow square wave at the audio sample rate (44.1 or 48 kHz), exactly 32 or 64 clock pulses per cycle.".into(),
                decoder_tips: "Assign bit clock, word select and data. Word size is half the number of clock pulses per word-select cycle.".into(),
                lookalikes: "SPI clocks stop between frames; this one does not.".into(),
                symptoms: Vec::new(),
            },
        }
    }
}

impl Protocol for I2s {
    fn id(&self) -> &str {
        "i2s"
    }
    fn name(&self) -> &str {
        "I2S audio"
    }
    fn group(&self) -> &str {
        "Audio"
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
        format!(
            "{}, {}-bit samples",
            if s.text("format") == "i2s" { "I2S" } else { "left-justified" },
            s.num("wordBits")
        )
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
        let slot = |key: &str| {
            setup.slot(key).ok_or_else(|| {
                let noun = self.roles.iter().find(|r| r.key == key).map_or(key, |r| r.noun.as_str());
                format!("Choose a channel for {noun}.")
            })
        };
        let slots = Slots { sck: slot("sck")?, ws: slot("ws")?, sd: slot("sd")? };
        let m = I2sMachine::new(&setup.settings, slots, setup.channels[slots.sd].channel, setup.start);
        decoder(m, setup, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyse::{Batching, analyse_capture, decode_store};
    use crate::synth::{GenCapture, Lane, make_capture};

    const RATE: f64 = 50e6;

    /// Stereo samples `word_bits` long in `slot`-bit slots, with the word starting `delay` clocks
    /// after word select changes, which is `left` for the left channel. 48 kHz word select.
    fn stream(samples: &[(i64, i64)], word_bits: u32, slot: u32, delay: usize, left: u8) -> GenCapture {
        let (mut sck, mut ws, mut sd) =
            (Lane::low(RATE), Lane::new(RATE, Level::from(left == 1)), Lane::low(RATE));
        let tb = 1.0 / (48000.0 * 2.0 * f64::from(slot));
        let mut bits: Vec<u8> = Vec::new();
        for &(l, r) in samples {
            for v in [l, r] {
                let u = if v < 0 { v + (1i64 << word_bits) } else { v };
                for k in 0..slot {
                    bits.push(if k < word_bits { u8::from((u >> (word_bits - 1 - k)) & 1 == 1) } else { 0 });
                }
            }
        }
        let slot = slot as usize;
        for i in 0..bits.len() + 2 {
            let at = 1e-5 + i as f64 * tb;
            sck.set(at, 0u8);
            ws.set(at, u8::from((i / slot) % 2 == 1) ^ left);
            if i >= delay {
                sd.set(at, bits.get(i - delay).copied().unwrap_or(0));
            }
            sck.set(at + tb / 2.0, 1u8);
        }
        make_capture("t", RATE, 0.0, vec![(0, "BCLK", sck), (1, "WS", ws), (2, "SD", sd)])
    }

    fn short() -> Vec<(i64, i64)> {
        vec![(0, 0), (1, -1), (300, -300), (-32768, 32767), (5, 6)]
    }

    /// 48 stereo samples using every bit of a `bits`-bit sample.
    fn many(bits: u32) -> Vec<(i64, i64)> {
        let top = (1i64 << (bits - 1)) - 1;
        (0..48).map(|i| ((i * 7919) % top - top / 2, top - (i * 104_729) % top)).collect()
    }

    fn decode(g: &GenCapture, s: &[(&str, SettingValue)]) -> DecodeResult {
        let (store, meta) = (g.store().unwrap(), g.meta());
        let r = roles(&[("sck", Some(0)), ("ws", Some(1)), ("sd", Some(2))]);
        let reg = crate::registry();
        let whole = decode_store(&reg, &store, &meta, "i2s", &r, &settings(s), Batching::Whole).unwrap();
        for b in [Batching::Edges(1), Batching::Edges(5), Batching::WithSnapshots(4)] {
            let other = decode_store(&reg, &store, &meta, "i2s", &r, &settings(s), b).unwrap();
            assert_eq!(other, whole, "{b:?}");
        }
        whole
    }

    fn labels(r: &DecodeResult) -> Vec<&str> {
        r.frames.iter().map(|f| f.label.as_str()).collect()
    }

    fn detect_one(g: &GenCapture) -> Detection {
        let a = analyse_capture(&g.store().unwrap(), &g.meta(), &crate::registry()).unwrap();
        let d: Vec<&Detection> = a.chosen().collect();
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].protocol_id, "i2s");
        d[0].clone()
    }

    #[test]
    fn standard_i2s_decodes_signed_samples_with_sides_and_a_note() {
        let r = decode(&stream(&short(), 16, 32, 1, 0), &[]);
        // the first word is skipped: word select has not changed yet
        assert_eq!(
            labels(&r),
            ["R 0", "L 1", "R -1", "L 300", "R -300", "L -32768", "R 32767", "L 5", "R 6"]
        );
        assert_eq!(r.frames[3].detail.as_deref(), Some("Left channel sample: 300"));
        assert_eq!(r.frames[5].value, Some(-32768.0));
        assert!(r.frames.iter().all(|f| f.kind == FrameKind::Sample && f.lane == 2 && f.start < f.end));
        assert!(r.frames.windows(2).all(|w| w[0].end == w[1].start));
        assert_eq!(r.packets.len(), 1);
        assert_eq!(r.packets[0].title, "Audio");
        assert_eq!(r.notes, ["9 samples decoded."]);
    }

    #[test]
    fn left_justified_and_left_when_high() {
        let g = stream(&short(), 16, 32, 0, 1);
        let r = decode(&g, &[("format", "left".into()), ("leftWhen", "high".into())]);
        assert_eq!(labels(&r)[..4], ["R 0", "L 1", "R -1", "L 300"]);
        // the wrong word-select level swaps the sides
        let r = decode(&g, &[("format", "left".into())]);
        assert_eq!(labels(&r)[..4], ["L 0", "R 1", "L -1", "R 300"]);
        // reading left-justified data as standard I2S drops the top bit and shifts the rest up
        let r = decode(&g, &[("leftWhen", "high".into())]);
        assert_eq!(labels(&r)[1..4], ["L 2", "R -2", "L 600"]);
    }

    #[test]
    fn words_longer_than_the_slot_give_no_samples() {
        let g = stream(&short(), 16, 16, 1, 0);
        assert_eq!(labels(&decode(&g, &[]))[..3], ["R 0", "L 1", "R -1"]);
        let r = decode(&g, &[("wordBits", 24.0.into())]);
        assert!(r.frames.is_empty());
        assert!(r.notes.is_empty());
        assert_eq!(r.packets.len(), 1);
    }

    #[test]
    fn no_clock_still_gives_an_empty_packet() {
        let lanes =
            vec![(0, "BCLK", Lane::low(RATE)), (1, "WS", Lane::low(RATE)), (2, "SD", Lane::low(RATE))];
        let r = decode(&make_capture("t", RATE, 0.0001, lanes), &[]);
        assert!(r.frames.is_empty());
        assert_eq!((r.packets.len(), r.packets[0].start), (1, 0));
    }

    #[test]
    fn detection_finds_the_sample_length_inside_the_slot() {
        let d = detect_one(&stream(&many(24), 24, 32, 1, 0));
        assert_eq!(d.settings.num("wordBits"), 24.0);
        assert_eq!(
            d.notes["wordBits"].reason,
            "Each slot has 32 clock pulses, but the last 8 bits are always 0, so the samples are 24 bits long."
        );
        assert_eq!(d.roles, roles(&[("sck", Some(0)), ("ws", Some(1)), ("sd", Some(2))]));
        assert_eq!(d.channels, [0, 1, 2]);
        // 32 or 64 clocks per cycle and an audio sample rate: 0.5 + 0.2 + 0.2
        assert!((d.confidence - 0.9).abs() < 1e-9, "{}", d.confidence);
        assert_eq!(d.evidence[0].text, "Channel 0 is a clock at 3.072 MHz that never stops: the bit clock.");
        assert_eq!(
            d.evidence[1].text,
            "Channel 1 is a square wave at 48 kHz, exactly 64 clock pulses per cycle: the word select. That is an audio sample rate."
        );
        assert_eq!(
            d.evidence[2].text,
            "So each sample slot has 32 bits, and the audio sample rate is 48 kHz."
        );
        let d = detect_one(&stream(&many(32), 32, 32, 1, 0));
        assert_eq!(d.settings.num("wordBits"), 32.0);
        assert_eq!(d.notes["wordBits"].reason, "Each half of the word-select cycle has 32 clock pulses.");
        assert_eq!(crate::registry().get("i2s").unwrap().describe(&d.settings), "I2S, 32-bit samples");
        // 16-bit slots: 32 clock pulses per word-select cycle
        let d = detect_one(&stream(&many(16), 16, 16, 1, 0));
        assert_eq!(d.settings.num("wordBits"), 16.0);
        assert!((d.confidence - 0.9).abs() < 1e-9, "{}", d.confidence);
    }

    #[test]
    fn two_lines_are_rejected() {
        let mut g = stream(&many(16), 16, 32, 1, 0);
        g.channels.truncate(2);
        let a = analyse_capture(&g.store().unwrap(), &g.meta(), &crate::registry()).unwrap();
        assert!(a.chosen().all(|d| d.protocol_id != "i2s"));
        let because: Vec<&str> = a
            .buses
            .iter()
            .flat_map(|b| &b.rejections)
            .filter(|r| r.protocol_id == "i2s")
            .map(|r| r.because.as_str())
            .collect();
        assert!(!because.is_empty());
        assert!(because.iter().all(|b| b.starts_with("I2S needs three lines")), "{because:?}");
    }

    #[test]
    fn a_missing_role_is_a_setup_error() {
        let g = stream(&short(), 16, 32, 1, 0);
        let e = decode_store(
            &crate::registry(),
            &g.store().unwrap(),
            &g.meta(),
            "i2s",
            &roles(&[("sck", Some(0)), ("ws", None), ("sd", Some(2))]),
            &Settings::new(),
            Batching::Whole,
        )
        .unwrap_err();
        assert_eq!(e, "Choose a channel for word select.");
    }
}
