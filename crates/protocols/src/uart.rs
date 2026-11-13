//! UART (and inverted UART): asynchronous serial, one line per direction, no clock.

use serde::{Deserialize, Serialize};

use logica_core::measure::{ChannelMeasurement, multiple_fit};
use logica_core::{Level, Sample};

use crate::analyse::decode_window;
use crate::bytes::modbus_crc;
use crate::calc::{STANDARD_BAUDS, nearest_baud};
use crate::format::{self, cap, char_of, hex2};
use crate::num;
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo, symptoms};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;
use crate::window::WinChannel;

pub fn uart_settings() -> Vec<SettingSpec> {
    vec![
        SettingSpec::number("baud", "Baud rate", 115_200.0)
            .range(50.0, 20_000_000.0, Some("baud"), &STANDARD_BAUDS)
            .help("Bits per second. 1 ÷ the narrowest pulse."),
        SettingSpec::choice(
            "dataBits",
            "Data bits",
            "8",
            &[("5", "5"), ("6", "6"), ("7", "7"), ("8", "8"), ("9", "9")],
        ),
        SettingSpec::choice(
            "parity",
            "Parity",
            "none",
            &[
                ("none", "None"),
                ("even", "Even"),
                ("odd", "Odd"),
                ("mark", "Mark (always 1)"),
                ("space", "Space (always 0)"),
            ],
        )
        .help("An extra bit after the data that makes the count of ones even or odd."),
        SettingSpec::choice("stopBits", "Stop bits", "1", &[("1", "1"), ("1.5", "1.5"), ("2", "2")]),
        SettingSpec::choice(
            "bitOrder",
            "Bit order",
            "lsb",
            &[("lsb", "Least significant bit first"), ("msb", "Most significant bit first")],
        ),
        SettingSpec::boolean("invert", "Inverted", false).help("Switch on when the line rests low."),
        SettingSpec::number("packetGap", "Pause that starts a new packet", 35.0)
            .range(2.0, 100_000.0, Some("bit times"), &[])
            .help("Bytes separated by a longer pause are listed as separate packets."),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UartStats {
    pub frames: u64,
    pub frame_errors: u64,
    pub parity_errors: u64,
    pub glitches: u64,
    pub breaks: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cfg {
    /// Samples per bit.
    t: f64,
    data_bits: u32,
    parity: String,
    stop_bits: f64,
    msb: bool,
    idle: Level,
    active: Level,
    packet_gap: f64,
    lane: u32,
    stream: String,
    limit: u64,
}

/// The UART decoder as a resumable loop (Signal Detective's `runUart`, made streaming).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UartMachine {
    cfg: Cfg,
    /// Next sample to look for a start bit.
    pos: Sample,
    last_end: f64,
    /// After a frame error: wait for the line to return to idle from here.
    resync: Option<Sample>,
    started: bool,
    pub stats: UartStats,
}

impl UartMachine {
    pub fn new(rate: f64, s: &Settings, lane: u32, name: &str, start: Sample, limit: u64) -> Self {
        let baud = s.num("baud");
        let invert = s.flag("invert");
        let idle = if invert { Level::Low } else { Level::High };
        let t = rate / baud;
        let named = !name.is_empty() && !is_default_name(name);
        UartMachine {
            cfg: Cfg {
                t,
                data_bits: s.text("dataBits").parse().unwrap_or(8),
                parity: s.text("parity").to_string(),
                stop_bits: s.text("stopBits").parse().unwrap_or(1.0),
                msb: s.text("bitOrder") == "msb",
                idle,
                active: !idle,
                packet_gap: s.num("packetGap") * t,
                lane,
                stream: if named { name.to_string() } else { format!("channel {lane}") },
                limit,
            },
            pos: start,
            last_end: f64::NEG_INFINITY,
            resync: None,
            started: false,
            stats: UartStats::default(),
        }
    }

    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let c = &self.cfg;
        if c.t.is_nan() || c.t < 1.0 || self.stats.frames >= c.limit {
            return Ok(false);
        }
        if let Some(from) = self.resync {
            // skip to the next return to idle
            let Some(e) = ed.next_edge(0, from)? else {
                return Ok(false);
            };
            if ed.level_at(0, e)? == c.idle {
                self.resync = None;
                self.pos = e + 1;
            } else {
                self.resync = Some(e + 1);
            }
            return Ok(true);
        }
        let Some(start) = ed.next_edge(0, self.pos)? else {
            return Ok(false);
        };
        // a start bit is a change to the active level
        if ed.level_at(0, start)? != c.active {
            self.pos = start + 1;
            return Ok(true);
        }
        if (start as f64) < self.last_end {
            self.pos = num::ceil(self.last_end);
            return Ok(true);
        }
        let t = c.t;
        let mid = |i: f64| num::floor(start as f64 + t * (i + 0.5));
        if ed.level_at(0, mid(0.0))? != c.active {
            self.stats.glitches += 1;
            out.add(
                NewFrame::new(start, (start + 1).max(mid(0.0)), c.lane, FrameKind::Info, "!")
                    .detail("A pulse too short to be a start bit")
                    .problem(problem(
                        ProblemCode::Glitch,
                        "A pulse shorter than half a bit: noise, or the baud rate is too low.",
                    )),
            );
            self.pos = mid(0.0);
            ed.release_before(self.pos);
            return Ok(true);
        }
        let mut value: u32 = 0;
        let mut ones = 0;
        for b in 0..c.data_bits {
            let bit = u32::from(ed.level_at(0, mid(f64::from(1 + b)))? == c.idle); // idle level = logic 1
            ones += bit;
            value = if c.msb { (value << 1) | bit } else { value | (bit << b) };
        }
        let parity_bits = u32::from(c.parity != "none");
        let mut problems = Vec::new();
        let mut parity_ok = true;
        let mut parity_bad = None;
        if parity_bits == 1 {
            let pb = u32::from(ed.level_at(0, mid(f64::from(1 + c.data_bits)))? == c.idle);
            let expected = match c.parity.as_str() {
                "even" => ones & 1,
                "odd" => 1 - (ones & 1),
                "mark" => 1,
                _ => 0,
            };
            if pb != expected {
                parity_ok = false;
                parity_bad = Some((pb, expected));
            }
        }
        let stop_at = f64::from(1 + c.data_bits + parity_bits);
        let mut stop_ok = ed.level_at(0, mid(stop_at))? == c.idle;
        if c.stop_bits >= 2.0 && stop_ok {
            stop_ok = ed.level_at(0, mid(stop_at + 1.0))? == c.idle;
        }
        // every read is done: commit
        if let Some((pb, expected)) = parity_bad {
            self.stats.parity_errors += 1;
            problems.push(problem(
                ProblemCode::ParityError,
                format!("Parity bit is {pb}, expected {expected} for {} parity.", c.parity),
            ));
        }
        let frame_bits = 1.0 + f64::from(c.data_bits) + f64::from(parity_bits) + c.stop_bits;
        let end = num::round(start as f64 + t * frame_bits);
        let is_break = value == 0 && !stop_ok && ones == 0;
        self.stats.frames += 1;
        if !stop_ok {
            if is_break {
                self.stats.breaks += 1;
            }
            self.stats.frame_errors += 1;
            problems.push(problem(
                ProblemCode::FrameError,
                if is_break {
                    "The line stayed active through the whole frame: a break, or the transmitter is off."
                } else {
                    "The stop bit is missing: the baud rate, data bits or parity are probably wrong."
                },
            ));
        }
        if start as f64 - self.last_end > c.packet_gap || out.packets_opened() == 0 {
            out.open_packet(start, format!("Packet at sample {start}"));
        }
        let parity_text =
            if parity_bits == 1 { if parity_ok { ", parity ok" } else { ", parity wrong" } } else { "" };
        out.add(
            NewFrame::new(
                start,
                end,
                c.lane,
                if is_break { FrameKind::Break } else { FrameKind::Data },
                if is_break { "break".to_string() } else { hex2(u64::from(value)) },
            )
            .value(f64::from(value))
            .detail(format!(
                "{} = {value} = '{}'{parity_text}",
                hex2(u64::from(value)),
                char_of(crate::bytes::low_byte(u64::from(value)))
            ))
            .problems(problems),
        );
        if !is_break {
            out.byte(&c.stream, c.lane, crate::bytes::low_byte(u64::from(value)), end);
        }
        self.last_end = start as f64 + t * (stop_at + 0.5);
        self.pos = num::ceil(self.last_end);
        if !stop_ok {
            // resynchronise on the next return to idle
            self.resync = Some(self.pos);
        }
        ed.release_before(self.pos.min(start + 1));
        Ok(true)
    }
}

fn is_default_name(name: &str) -> bool {
    name.strip_prefix("channel ").is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
}

impl Machine for UartMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        if !self.started {
            self.started = true;
            if self.cfg.t.is_nan() || self.cfg.t < 1.0 {
                out.note("The baud rate is higher than the sample rate allows: every bit would be shorter than one sample.");
            } else if self.cfg.t < 4.0 {
                out.note(format!(
                    "Each bit is only {} samples long. Decoding is unreliable below about 4 samples per bit; sample faster if you can.",
                    format::fixed(self.cfg.t, 1)
                ));
            }
        }
        run_steps(|| self.step(ed, out));
    }
}

/// Runs the UART loop over one window channel, keeping only statistics.
pub fn uart_stats(
    ch: &WinChannel,
    start: Sample,
    end: Sample,
    rate: f64,
    s: &Settings,
    limit: u64,
) -> UartStats {
    let mut m = UartMachine::new(rate, s, ch.index, &ch.name, start, limit);
    let mut ed = Edges::complete(vec![(ch.initial, ch.edges.clone())], end);
    let mut out = Out::new(false);
    m.started = true;
    run_steps(|| m.step(&mut ed, &mut out));
    m.stats
}

// ------------------------------------------------------------------ detection

struct Format {
    data_bits: &'static str,
    parity: &'static str,
    stop_bits: &'static str,
    name: &'static str,
}

/// Frame formats tried by the detector, in order of preference when they decode equally well.
const FORMATS: [Format; 7] = [
    Format { data_bits: "8", parity: "none", stop_bits: "1", name: "8N1" },
    Format { data_bits: "8", parity: "even", stop_bits: "1", name: "8E1" },
    Format { data_bits: "8", parity: "odd", stop_bits: "1", name: "8O1" },
    Format { data_bits: "7", parity: "even", stop_bits: "1", name: "7E1" },
    Format { data_bits: "7", parity: "odd", stop_bits: "1", name: "7O1" },
    Format { data_bits: "8", parity: "none", stop_bits: "2", name: "8N2" },
    Format { data_bits: "7", parity: "none", stop_bits: "1", name: "7N1" },
];

pub fn format_name(s: &Settings) -> String {
    let p = s.text("parity");
    let pc = if p == "none" { "N".to_string() } else { p[..1].to_uppercase() };
    format!("{}{}{}", s.text("dataBits"), pc, s.text("stopBits"))
}

fn settings_for(baud: f64, f: &Format, invert: bool) -> Settings {
    SettingSpec::with_defaults(
        &uart_settings(),
        &settings(&[
            ("baud", baud.into()),
            ("dataBits", f.data_bits.into()),
            ("parity", f.parity.into()),
            ("stopBits", f.stop_bits.into()),
            ("invert", invert.into()),
        ]),
    )
}

/// The smallest start-to-start spacing of back-to-back decoded bytes, in bit times.
fn back_to_back_bits(frames: &[Frame], t: f64) -> Option<u32> {
    let starts: Vec<Sample> = frames.iter().filter(|f| f.kind == FrameKind::Data).map(|f| f.start).collect();
    let mut counts = std::collections::BTreeMap::<u32, u32>::new();
    for w in starts.windows(2) {
        let bits = (w[1] - w[0]) as f64 / t;
        let r = bits.round();
        if (8.0..=13.0).contains(&r) && (bits - r).abs() < 0.2 {
            *counts.entry(u32::try_from(num::to_u64(r)).unwrap_or(0)).or_default() += 1;
        }
    }
    counts.into_iter().filter(|&(_, n)| n >= 2).map(|(r, _)| r).min()
}

fn bit_time_estimate(m: &ChannelMeasurement) -> Option<f64> {
    let u0 = m.narrowest?.samples;
    let (mut num, mut den) = (0.0, 0.0);
    for c in &m.clusters {
        let k = (c.mean / u0).round();
        if !(1.0..=12.0).contains(&k) || (c.mean - k * u0).abs() > 0.2 * u0 + 1.0 {
            continue;
        }
        num += c.mean * c.count as f64;
        den += k * c.count as f64;
    }
    Some(if den > 0.0 { num / den } else { u0 })
}

fn detect_one(ctx: &DetectContext<'_>, ch: &WinChannel, protocol: &Uart) -> Result<Detection, String> {
    let m = ctx.m(ch.index);
    let rate = ctx.sample_rate_hz;
    let name = format!("channel {}", ch.index);
    if m.edge_count < 10 {
        return Err(format!("{name} has too few edges to be a UART line"));
    }
    let pulses = m.edge_count.saturating_sub(1).max(1) as f64;
    let significant = m
        .clusters
        .iter()
        .filter(|c| c.count as f64 >= 2f64.max(pulses * 0.03) && c.mean <= m.burst_gap)
        .count();
    if m.periodic.is_some() && significant <= 2 {
        return Err(format!(
            "{name} has evenly spaced pulses of one or two widths, which is a clock or PWM. (A UART sending 0x55 over and over would look the same.)"
        ));
    }
    let t0 = bit_time_estimate(m).ok_or_else(|| format!("{name} has no repeated pulse width"))?;
    let fit = multiple_fit(&m.clusters, t0, 0.18);
    let invert = m.rest_level == Level::Low;
    let (ws, we) = (ctx.window.start, ctx.window.end);

    // the measured bit time, and half of it (the narrowest pulse might be two bits)
    struct Best {
        t: f64,
        baud: f64,
        fi: usize,
        stats: UartStats,
        score: f64,
    }
    let mut best: Option<Best> = None;
    for t in [t0, t0 / 2.0].into_iter().filter(|&t| t >= 2.0) {
        let measured = rate / t;
        let nb = nearest_baud(measured);
        let baud = if nb.within_tolerance { nb.standard } else { measured.round() };
        for (fi, f) in FORMATS.iter().enumerate() {
            let s = settings_for(baud, f, invert);
            let stats = uart_stats(ch, ws, we, rate, &s, 4000);
            if stats.frames == 0 {
                continue;
            }
            let err =
                (stats.frame_errors + stats.parity_errors + stats.glitches) as f64 / stats.frames as f64;
            // 7E1 and 8N1 are both 10 bits long; a parity bit that checks out every time over
            // enough bytes is strong evidence for the parity format.
            let parity_bonus = if err == 0.0 && f.parity != "none" && stats.frames >= 8 { 0.01 } else { 0.0 };
            let score = err + fi as f64 * 0.002 + if t == t0 { 0.0 } else { 0.05 } - parity_bonus;
            if best.as_ref().is_none_or(|b| score < b.score) {
                best = Some(Best { t, baud, fi, stats, score });
            }
        }
    }
    let Best { t, baud, fi, stats, .. } =
        best.ok_or_else(|| format!("decoding {name} as UART finds no frames"))?;
    let fmt = &FORMATS[fi];
    let err_rate = (stats.frame_errors + stats.parity_errors + stats.glitches) as f64 / stats.frames as f64;
    let measured_baud = rate / t;
    let nb = nearest_baud(measured_baud);
    let bit = format::time(t / rate);

    let mut evidence = vec![self::evidence(
        format!(
            "{} rests {} and has no clock{}.",
            cap(&name),
            if invert { "low" } else { "high" },
            if invert {
                ". A UART line normally rests high, so this one is inverted"
            } else {
                ", like a UART line"
            }
        ),
        true,
    )];
    evidence.push(self::evidence(
        format!(
            "{} % of the pulses are whole multiples of {bit}, so one bit lasts {bit}: {} baud{}.",
            (fit * 100.0).round(),
            format::int(measured_baud),
            if nb.within_tolerance {
                format!(", which is the standard rate {}", format::int(nb.standard))
            } else {
                ", which is not a standard rate".into()
            }
        ),
        fit > 0.8,
    ));
    if t != t0 {
        evidence.push(self::evidence(
            format!(
                "The narrowest pulse ({}) is two bits wide; half of it decodes better.",
                format::time(t0 / rate)
            ),
            true,
        ));
    }
    let settings = settings_for(baud, fmt, invert);
    let full = decode_window(protocol, ctx.window, rate, &roles(&[("data", Some(ch.index))]), &settings)?;
    let spacing = back_to_back_bits(&full.frames, t);
    if let Some(sp) = spacing {
        evidence.push(self::evidence(
            format!(
                "Back-to-back bytes start {sp} bit times apart (start bit, data, {}stop bit).",
                if sp >= 11 { "a parity or second stop bit, " } else { "" }
            ),
            true,
        ));
    }
    evidence.push(self::evidence(
        format!(
            "Decoding as {} gives {} byte{} with {}.",
            fmt.name,
            stats.frames,
            format::s(stats.frames),
            if err_rate == 0.0 {
                "no errors".to_string()
            } else {
                format!(
                    "{} frame error{} and {} parity error{}",
                    stats.frame_errors,
                    format::s(stats.frame_errors),
                    stats.parity_errors,
                    format::s(stats.parity_errors)
                )
            }
        ),
        err_rate < 0.1,
    ));
    let mut warnings = Vec::new();
    if m.glitches > 0 {
        warnings.push(format!(
            "{} has {} pulse{} much narrower than one bit: noise or crosstalk.",
            cap(&name),
            m.glitches,
            format::s(m.glitches)
        ));
    }
    if t < 4.0 {
        warnings.push(format!(
            "Only {} samples per bit. Sample at least 10 times the baud rate for reliable decoding.",
            format::fixed(t, 1)
        ));
    } else if t < 10.0 {
        warnings.push(format!(
            "{} samples per bit. That works, but 10 or more is safer for UART.",
            format::fixed(t, 1)
        ));
    }
    // Modbus RTU hint
    let crc_packets = full
        .packets
        .iter()
        .filter(|p| {
            let b = p.streams.first().map_or(&[][..], |s| &s.bytes[..]);
            if b.len() < 4 {
                return false;
            }
            let c = modbus_crc(&b[..b.len() - 2]);
            (c & 0xff) as u8 == b[b.len() - 2] && (c >> 8) as u8 == b[b.len() - 1]
        })
        .count();
    if crc_packets > 0 {
        evidence.push(self::evidence(
            format!(
                "{crc_packets} of {} packets end with a valid Modbus CRC-16, so this is Modbus RTU on top of UART.",
                full.packets.len()
            ),
            true,
        ));
    }
    let frame_len = 1
        + fmt.data_bits.parse::<u32>().unwrap_or(8)
        + u32::from(fmt.parity != "none")
        + fmt.stop_bits.parse::<u32>().unwrap_or(1);
    let format_confidence =
        clamp01(1.0 - err_rate * 3.0) * if spacing == Some(frame_len) { 1.0 } else { 0.8 };
    if m.periodic.is_some() {
        evidence.push(self::evidence(
            format!("{} also has evenly spaced pulses, which is more typical of a clock.", cap(&name)),
            false,
        ));
    }
    let confidence = clamp01(
        0.25 + 0.3 * (1.0 - (err_rate * 4.0).min(1.0)) * (stats.frames as f64 / 4.0).min(1.0)
            + 0.25 * fit
            + if nb.within_tolerance { 0.1 } else { 0.0 }
            + if invert { 0.0 } else { 0.05 }
            + 0.05,
    ) * if m.periodic.is_some() { 0.6 } else { 1.0 };
    let sign = if nb.error_pct >= 0.0 { "+" } else { "" };
    let notes = [
        ("role:data", note(0.9, format!("{} carries the bytes.", cap(&name)))),
        (
            "baud",
            note(
                clamp01(fit * if nb.within_tolerance { 1.0 } else { 0.7 }),
                format!(
                    "The narrowest bit is {bit}, and 1 ÷ {bit} = {} baud{}.",
                    format::int(measured_baud),
                    if nb.within_tolerance {
                        format!(
                            ", rounded to the standard {} ({sign}{} %)",
                            format::int(nb.standard),
                            format::fixed(nb.error_pct, 1)
                        )
                    } else {
                        String::new()
                    }
                ),
            ),
        ),
        (
            "dataBits",
            note(
                format_confidence,
                format!(
                    "{} decodes with the fewest errors{}.",
                    fmt.name,
                    spacing.map_or(String::new(), |sp| format!(
                        " and matches the {sp}-bit spacing of back-to-back bytes"
                    ))
                ),
            ),
        ),
        (
            "parity",
            note(
                format_confidence,
                if fmt.parity == "none" {
                    "Without a parity bit the stop bits land in the right place.".to_string()
                } else {
                    format!("With {} parity every parity bit checks out.", fmt.parity)
                },
            ),
        ),
        (
            "stopBits",
            note(
                format_confidence * 0.9,
                "Fewer stop bits are tried first; extra idle time between bytes looks the same as more stop bits.",
            ),
        ),
        (
            "bitOrder",
            note(
                0.6,
                "UART sends the least significant bit first almost everywhere. The waveform cannot prove it.",
            ),
        ),
        (
            "invert",
            note(
                0.9,
                if invert {
                    format!("{} rests low, so the signal is upside down.", cap(&name))
                } else {
                    format!("{} rests high, the normal UART idle level.", cap(&name))
                },
            ),
        ),
    ];
    Ok(Detection {
        protocol_id: "uart".into(),
        channels: vec![ch.index],
        roles: roles(&[("data", Some(ch.index))]),
        settings,
        notes: notes.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        confidence,
        evidence,
        warnings,
    })
}

fn evidence(text: String, supports: bool) -> Evidence {
    crate::types::evidence(text, supports)
}

// ------------------------------------------------------------------ the protocols

pub struct Uart {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl Default for Uart {
    fn default() -> Self {
        Uart {
            roles: vec![RoleSpec::new(
                "data",
                "Data (RX or TX)",
                "the data line",
                "The line carrying the bytes. Decode TX and RX as two separate UARTs.",
            )],
            settings: uart_settings(),
            info: ProtocolInfo {
                what: "Asynchronous serial. One line per direction, no clock.".into(),
                confirm: "Every byte starts with a low bit and ends high. Bytes sent back to back start exactly 10 bit times apart (11 with parity).".into(),
                decoder_tips: "Baud = 1 ÷ narrowest pulse. Data bits 8, parity none, 1 stop bit is the usual start. Bit order LSB first.".into(),
                lookalikes: "CAN also idles high with no clock, but never has more than 5 equal bits in a row. Two lines that each look like this are a TX and RX pair. Binary frames with silent gaps and 11-bit bytes suggest Modbus RTU.".into(),
                symptoms: symptoms(&[
                    ("Garbage bytes and many frame errors", "Wrong baud rate", "Measure the narrowest pulse and recalculate."),
                    ("Bytes mostly right, errors near the end of each byte", "Baud rate off by a few percent", "Measure across 10 bits and divide by 10."),
                    ("Frame errors on some bytes only", "A parity bit is present but the decoder is set to none", "Check for 11 bit times per byte, then set even or odd parity."),
                    ("Parity error on every byte", "Even and odd are swapped", "Switch the parity setting."),
                    ("Every byte wrong but consistent", "Signal inverted, or wrong bit order", "Check the rest level: low means inverted."),
                    ("First byte wrong, the rest fine", "Decoding started in the middle of a byte", "Start the decode from an idle stretch."),
                    ("00 with a frame error, line low for a long time", "Break condition, or the transmitter is off", "Check power and wiring on the sending side."),
                    ("Two devices talk at once on one line", "Bus collision (RS-485)", "Check direction control and addressing."),
                ]),
            },
        }
    }
}

impl Protocol for Uart {
    fn id(&self) -> &str {
        "uart"
    }
    fn name(&self) -> &str {
        "UART"
    }
    fn group(&self) -> &str {
        "UART"
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
            "{} baud, {}{}{}",
            format::int(s.num("baud")),
            format_name(s),
            if s.flag("invert") { ", inverted" } else { "" },
            if s.text("bitOrder") == "msb" { ", MSB first" } else { "" }
        )
    }
    fn title(&self, s: &Settings) -> String {
        if s.flag("invert") { "UART, inverted".into() } else { "UART".into() }
    }
    fn detect(&self, ctx: &DetectContext<'_>) -> Option<DetectorOutput> {
        let mut out = DetectorOutput::default();
        for ch in &ctx.channels {
            match detect_one(ctx, ch, self) {
                Ok(d) => out.detections.push(d),
                Err(because) => out.rejections.push(Rejection { protocol_id: "uart".into(), because }),
            }
        }
        Some(out)
    }
    fn has_decoder(&self) -> bool {
        true
    }
    fn decoder(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        let ch = &setup.channels[setup.slot("data").ok_or("Choose a channel for the data line.")?];
        let m = UartMachine::new(
            setup.sample_rate_hz,
            &setup.settings,
            ch.channel,
            &ch.name,
            setup.start,
            u64::MAX,
        );
        decoder(m, setup, snapshot)
    }
}

/// Knowledge-only entry: detected and decoded as UART with `invert` on.
pub struct UartInverted {
    info: ProtocolInfo,
}

impl Default for UartInverted {
    fn default() -> Self {
        UartInverted {
            info: ProtocolInfo {
                what: "Same framing as UART but upside down: rests low, start bit is high. Seen on raw RS-232 lines and some sensors.".into(),
                confirm: "Each byte starts with a high pulse and returns low at the end.".into(),
                decoder_tips: "Same as UART, with the invert option switched on.".into(),
                lookalikes: "WS2812 LED data also rests low, but has only two pulse widths.".into(),
                symptoms: Vec::new(),
            },
        }
    }
}

impl Protocol for UartInverted {
    fn id(&self) -> &str {
        "uart-inverted"
    }
    fn name(&self) -> &str {
        "UART, inverted"
    }
    fn group(&self) -> &str {
        "UART"
    }
    fn info(&self) -> &ProtocolInfo {
        &self.info
    }
    fn roles(&self) -> &[RoleSpec] {
        &[]
    }
    fn settings(&self) -> &[SettingSpec] {
        &[]
    }
    fn alias_of(&self) -> Option<&str> {
        Some("uart")
    }
}
