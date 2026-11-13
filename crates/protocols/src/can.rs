//! CAN on the logic side of a transceiver: one line, rests high, bit stuffing, CRC-15.
//!
//! A frame starts with a falling edge after at least 7 recessive bit times. Bits are read at the
//! sample point and the bit clock resynchronises on every edge near a bit boundary; after five
//! equal bits a stuff bit follows and is dropped. Behaviour, texts and heuristics follow Signal
//! Detective's `can.ts` (decision D12).

use serde::{Deserialize, Serialize};

use logica_core::{Level, Sample};

use crate::bytes::{can_crc15, low_byte};
use crate::format::{self, hex, hex2};
use crate::num;
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo, symptoms};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;
use crate::window::WinChannel;

/// Standard CAN bit rates (the setting's presets and the detector's candidates).
pub const CAN_RATES: [f64; 10] = [
    10_000.0,
    20_000.0,
    50_000.0,
    83_333.0,
    100_000.0,
    125_000.0,
    250_000.0,
    500_000.0,
    800_000.0,
    1_000_000.0,
];

pub fn can_settings() -> Vec<SettingSpec> {
    vec![
        SettingSpec::number("bitrate", "Bit rate", 500_000.0)
            .range(1000.0, 8_000_000.0, Some("bit/s"), &CAN_RATES)
            .help("1 ÷ the narrowest pulse."),
        SettingSpec::number("samplePoint", "Sample point", 75.0)
            .range(10.0, 95.0, Some("% of a bit"), &[])
            .help("Where inside each bit the level is read."),
    ]
}

/// "500 kbit/s" style.
fn bits_per_s(f: f64) -> String {
    format::hz(f).replace("Hz", "bit/s")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanStats {
    /// Frames read to the end (stuffing errors not counted).
    pub frames: u64,
    pub crc_ok: u64,
    pub stuff_errors: u64,
    /// Longest run of equal bits seen inside frames.
    pub max_run: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cfg {
    /// Samples per bit; 0 when the bit rate cannot be decoded at this sample rate.
    t: f64,
    /// Sample point as a fraction of a bit.
    sp: f64,
    lane: u32,
    /// Stop after this many frames (detection trial decodes).
    limit: u64,
}

/// The CAN decoder as a resumable loop (Signal Detective's `runCan`, made streaming). One step
/// looks at one candidate start-of-frame edge and, if it is one, reads the whole frame before it
/// emits anything.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CanMachine {
    cfg: Cfg,
    /// Absolute index of the next candidate edge.
    next: u64,
    /// The edge before it (the decode start before the first edge).
    prev: Sample,
    /// Why nothing can be decoded, said once at the start.
    unusable: Option<String>,
    started: bool,
    pub stats: CanStats,
}

impl CanMachine {
    pub fn new(rate: f64, s: &Settings, lane: u32, start: Sample, limit: u64) -> Self {
        let bitrate = s.num("bitrate");
        let t = rate / bitrate;
        let unusable = if !t.is_finite() || bitrate <= 0.0 {
            // not in Signal Detective: a bit rate of 0 would put infinities in the state
            Some("The bit rate must be above zero.".to_string())
        } else if t < 2.0 {
            Some("The bit rate is too high for this sample rate.".to_string())
        } else {
            None
        };
        CanMachine {
            cfg: Cfg {
                t: if unusable.is_some() { 0.0 } else { t },
                sp: s.num("samplePoint") / 100.0,
                lane,
                limit,
            },
            next: 0,
            prev: start,
            unusable,
            started: false,
            stats: CanStats::default(),
        }
    }

    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let c = &self.cfg;
        if self.unusable.is_some() || self.stats.frames >= c.limit {
            return Ok(false);
        }
        let k = self.next;
        let Some(sof) = ed.edge(0, k)? else {
            return Ok(false);
        };
        // start of frame: a recessive-to-dominant edge after at least 7 recessive bit times
        if ed.level_after(0, k) != Level::Low || ((sof - self.prev) as f64) < 7.0 * c.t {
            self.prev = sof;
            self.next = k + 1;
            ed.release_index(0, self.next);
            return Ok(true);
        }
        // every read first: the frame, then where the search continues
        let f = read_frame(ed, c.t, c.sp, sof)?;
        let resume = match &f.body {
            None => num::ceil(f.bit_start),
            Some(b) => b.end,
        };
        let next = ed.lower_bound(0, resume)?;
        let prev = match next.checked_sub(1) {
            Some(i) => ed.edge(0, i)?.unwrap_or(sof),
            None => self.prev,
        };

        // commit
        self.stats.max_run = self.stats.max_run.max(f.max_run);
        let lane = c.lane;
        let t = c.t;
        match f.body {
            None => {
                self.stats.stuff_errors += 1;
                out.open_packet(sof, "CAN frame (stuffing error)");
                out.add(
                    NewFrame::new(sof, num::round(f.bit_start), lane, FrameKind::Info, "stuff error")
                        .detail("Six equal bits in a row inside a frame")
                        .problem(problem(
                            ProblemCode::Stuffing,
                            "Six equal bits in a row: wrong bit rate, noise, or not CAN.",
                        )),
                );
            }
            Some(b) => {
                self.stats.frames += 1;
                let crc_ok = b.crc == b.expected;
                let mut problems = Vec::new();
                if crc_ok {
                    self.stats.crc_ok += 1;
                } else {
                    problems.push(problem(
                        ProblemCode::Checksum,
                        format!(
                            "CRC is {}, but the frame content gives {}.",
                            hex(b.crc, 4),
                            hex(b.expected, 4)
                        ),
                    ));
                }
                if !b.crc_delim || !b.ack_delim {
                    problems.push(problem(
                        ProblemCode::FrameError,
                        "A delimiter bit after the CRC is dominant: form error.",
                    ));
                }
                let id_text = hex(b.id, if b.ide { 8 } else { 3 });
                let stream = format!("ID {id_text}");
                out.open_packet(sof, format!("{stream}{}", if b.rtr { " remote request" } else { "" }));
                let sof_f = sof as f64;
                out.add(
                    NewFrame::new(sof, num::round(sof_f + t), lane, FrameKind::Start, "SOF")
                        .detail("Start of frame"),
                );
                out.add(
                    NewFrame::new(
                        num::round(sof_f + t),
                        num::round(b.dlc_start - if b.ide { 3.0 } else { 2.0 } * t),
                        lane,
                        FrameKind::Address,
                        format!("ID {:X}", b.id),
                    )
                    .value(b.id as f64)
                    .detail(format!(
                        "{} identifier {id_text}{}",
                        if b.ide { "Extended 29-bit" } else { "Standard 11-bit" },
                        if b.rtr { ", remote request" } else { "" }
                    )),
                );
                out.add(
                    NewFrame::new(
                        num::round(b.dlc_start),
                        num::round(b.data.first().map_or(b.crc_start, |d| d.1)),
                        lane,
                        FrameKind::Field,
                        format!("DLC {}", b.dlc),
                    )
                    .value(b.dlc as f64)
                    .detail(format!("{} data byte{}", b.dlc, format::s(b.dlc))),
                );
                for (i, &(v, start)) in b.data.iter().enumerate() {
                    let end = b.data.get(i + 1).map_or(b.crc_start, |d| d.1);
                    out.add(
                        NewFrame::new(
                            num::round(start),
                            num::round(end),
                            lane,
                            FrameKind::Data,
                            hex2(u64::from(v)),
                        )
                        .value(f64::from(v))
                        .detail(format!(
                            "Data byte {}: {} = {v}",
                            i + 1,
                            hex2(u64::from(v))
                        )),
                    );
                    out.byte(&stream, lane, v, num::round(b.crc_start));
                }
                out.add(
                    NewFrame::new(
                        num::round(b.crc_start),
                        num::round(b.ack_start - t),
                        lane,
                        FrameKind::Field,
                        if crc_ok { "CRC ok" } else { "CRC bad" },
                    )
                    .value(b.crc as f64)
                    .detail(format!("CRC {}", hex(b.crc, 4)))
                    .problems(problems),
                );
                out.add(
                    NewFrame::new(
                        num::round(b.ack_start),
                        num::round(b.ack_start + t),
                        lane,
                        if b.ack { FrameKind::Ack } else { FrameKind::Nack },
                        if b.ack { "ACK" } else { "no ACK" },
                    )
                    .detail(if b.ack {
                        "Another node acknowledged the frame."
                    } else {
                        "Nobody acknowledged: no other node on the bus, or they all saw an error."
                    }),
                );
                out.close_packet(b.end);
            }
        }
        self.next = next;
        self.prev = prev;
        ed.release_index(0, next);
        Ok(true)
    }
}

impl Machine for CanMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        if !self.started {
            self.started = true;
            if let Some(why) = &self.unusable {
                out.note(why.clone());
            }
        }
        run_steps(|| self.step(ed, out));
    }
}

// ------------------------------------------------------------------ reading one frame

/// A frame read to the end of its ACK delimiter.
struct Body {
    id: u64,
    ide: bool,
    rtr: bool,
    dlc: u64,
    /// Data bytes with the position where each starts.
    data: Vec<(u8, f64)>,
    dlc_start: f64,
    crc_start: f64,
    crc: u64,
    expected: u64,
    crc_delim: bool,
    /// Whether the ACK slot was dominant (another node acknowledged).
    ack: bool,
    ack_delim: bool,
    ack_start: f64,
    /// End of the end-of-frame field.
    end: Sample,
}

struct ReadFrame {
    /// `None` after a stuffing error.
    body: Option<Body>,
    /// Start of the next unread bit.
    bit_start: f64,
    max_run: u32,
}

/// Reads bits from a start-of-frame edge, resynchronising on every edge near a bit boundary.
struct BitReader<'a> {
    ed: &'a Edges,
    t: f64,
    sp: f64,
    bit_start: f64,
    /// Destuffed bits from SOF on.
    bits: Vec<u8>,
    run: u32,
    last: Option<u8>,
    stuff_error: bool,
    max_run: u32,
}

impl BitReader<'_> {
    /// One bit on the wire (stuff bits included); 1 = recessive.
    fn raw(&mut self) -> Res<u8> {
        let v = u8::from(self.ed.level_at(0, num::floor(self.bit_start + self.sp * self.t))? == Level::High);
        let nominal = self.bit_start + self.t;
        // the next bit starts at an edge within half a bit of where it is due, else on time
        let from = num::ceil(nominal - self.t / 2.0);
        let to = num::floor(nominal + self.t / 2.0) + 1;
        self.bit_start = match self.ed.edge_in(0, from, to)? {
            Some(e) => e as f64,
            None => nominal,
        };
        Ok(v)
    }

    /// One destuffed bit; after five equal bits the stuff bit is read and dropped.
    fn next(&mut self) -> Res<u8> {
        let v = self.raw()?;
        self.run = if self.last == Some(v) { self.run + 1 } else { 1 };
        self.last = Some(v);
        self.max_run = self.max_run.max(self.run);
        if self.run == 5 {
            let st = self.raw()?;
            if st == v {
                self.stuff_error = true;
            }
            self.last = Some(st);
            self.run = 1;
        }
        self.bits.push(v);
        Ok(v)
    }

    /// `n` bits, most significant first; stops early after a stuffing error.
    fn field(&mut self, n: u32) -> Res<u64> {
        let mut v = 0;
        for _ in 0..n {
            if self.stuff_error {
                break;
            }
            v = v * 2 + u64::from(self.next()?);
        }
        Ok(v)
    }
}

fn read_frame(ed: &Edges, t: f64, sp: f64, sof: Sample) -> Res<ReadFrame> {
    let mut r = BitReader {
        ed,
        t,
        sp,
        bit_start: sof as f64,
        bits: Vec::with_capacity(128),
        run: 0,
        last: None,
        stuff_error: false,
        max_run: 0,
    };
    r.next()?; // SOF
    let id11 = r.field(11)?;
    let rtr_or_srr = r.next()?;
    let ide = r.next()? == 1;
    let mut id = id11;
    let mut rtr = rtr_or_srr == 1;
    if ide {
        id = (id11 << 18) + r.field(18)?;
        rtr = r.next()? == 1;
        r.next()?; // r1
    }
    r.next()?; // r0
    let dlc_start = r.bit_start;
    let dlc = r.field(4)?;
    let n_data = if rtr { 0 } else { dlc.min(8) };
    let mut data = Vec::new();
    for _ in 0..n_data {
        if r.stuff_error {
            break;
        }
        let start = r.bit_start;
        data.push((low_byte(r.field(8)?), start));
    }
    let crc_bits = r.bits.len();
    let crc_start = r.bit_start;
    let crc = r.field(15)?;
    if r.stuff_error {
        return Ok(ReadFrame { body: None, bit_start: r.bit_start, max_run: r.max_run });
    }
    let expected = u64::from(can_crc15(&r.bits[..crc_bits]));
    // not stuffed: CRC delimiter, ACK slot, ACK delimiter
    let crc_delim = r.raw()? == 1;
    let ack_start = r.bit_start;
    let ack = r.raw()? == 0;
    let ack_delim = r.raw()? == 1;
    let end = num::round(r.bit_start + 7.0 * t);
    Ok(ReadFrame {
        body: Some(Body {
            id,
            ide,
            rtr,
            dlc,
            data,
            dlc_start,
            crc_start,
            crc,
            expected,
            crc_delim,
            ack,
            ack_delim,
            ack_start,
            end,
        }),
        bit_start: r.bit_start,
        max_run: r.max_run,
    })
}

/// Runs the CAN loop over one window channel, keeping only statistics.
pub fn can_stats(
    ch: &WinChannel,
    start: Sample,
    end: Sample,
    rate: f64,
    s: &Settings,
    limit: u64,
) -> CanStats {
    let mut m = CanMachine::new(rate, s, ch.index, start, limit);
    let mut ed = Edges::complete(vec![(ch.initial, ch.edges.clone())], end);
    let mut out = Out::new(false);
    m.run(&mut ed, &mut out);
    m.stats
}

// ------------------------------------------------------------------ detection

fn detect(ctx: &DetectContext<'_>, specs: &[SettingSpec]) -> DetectorOutput {
    let mut out = DetectorOutput::default();
    let reject = |because: String| Rejection { protocol_id: "can".into(), because };
    for ch in &ctx.channels {
        let i = ch.index;
        let m = ctx.m(i);
        let Some(narrowest) = m.narrowest.as_ref().filter(|_| m.edge_count >= 20) else {
            continue;
        };
        if m.rest_level != Level::High {
            out.rejections.push(reject(format!("CAN (logic side) rests high, and channel {i} rests low")));
            continue;
        }
        if m.periodic.is_some() {
            out.rejections.push(reject(format!("channel {i} has evenly spaced pulses, like a clock")));
            continue;
        }
        let rate = ctx.sample_rate_hz;
        let measured = rate / narrowest.samples;
        let std = CAN_RATES.iter().copied().find(|r| (r - measured).abs() / r < 0.05);
        let bitrate = std.unwrap_or_else(|| measured.round());
        let settings =
            SettingSpec::with_defaults(specs, &crate::types::settings(&[("bitrate", bitrate.into())]));
        let stats = can_stats(ch, ctx.window.start, ctx.window.end, rate, &settings, 500);
        if stats.frames == 0 {
            out.rejections.push(reject(format!(
                "decoding channel {i} as CAN at {} finds no complete frames",
                format::hz(bitrate)
            )));
            continue;
        }
        let ok_share = stats.crc_ok as f64 / stats.frames as f64;
        let notes = [
            ("role:rx", note(0.9, format!("Channel {i} carries the frames."))),
            (
                "bitrate",
                note(
                    if std.is_some() { 0.9 } else { 0.5 },
                    format!(
                        "The narrowest pulse is {}, so {}{}.",
                        format::time(narrowest.samples / rate),
                        bits_per_s(measured),
                        std.map_or(String::new(), |s| format!(", the standard {}", bits_per_s(s)))
                    ),
                ),
            ),
            ("samplePoint", note(0.7, "75 % is the usual sample point.")),
        ];
        out.detections.push(Detection {
            protocol_id: "can".into(),
            channels: vec![i],
            roles: roles(&[("rx", Some(i))]),
            settings,
            notes: notes.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            confidence: clamp01(
                0.2 + 0.6 * ok_share
                    + if std.is_some() { 0.1 } else { 0.0 }
                    + if stats.max_run <= 5 { 0.1 } else { 0.0 },
            ),
            evidence: vec![
                evidence(
                    format!(
                        "Channel {i} rests high with no clock, and its narrowest pulse matches {} ({}).",
                        if std.is_some() { "a standard CAN rate" } else { "no standard CAN rate" },
                        bits_per_s(measured)
                    ),
                    std.is_some(),
                ),
                evidence(
                    format!("{} of {} frames have a correct CRC-15.", stats.crc_ok, stats.frames),
                    ok_share > 0.8,
                ),
                evidence(
                    if stats.stuff_errors > 0 {
                        format!(
                            "{} places have six equal bits in a row, which CAN never sends.",
                            stats.stuff_errors
                        )
                    } else {
                        "Inside frames there are never more than 5 equal bits in a row (bit stuffing).".into()
                    },
                    stats.stuff_errors == 0,
                ),
            ],
            warnings: Vec::new(),
        });
    }
    out
}

// ------------------------------------------------------------------ the protocol

pub struct Can {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl Default for Can {
    fn default() -> Self {
        Can {
            roles: vec![RoleSpec::new(
                "rx",
                "CAN RX",
                "CAN RX",
                "The transceiver RX pin (logic level, rests high).",
            )],
            settings: can_settings(),
            info: ProtocolInfo {
                what: "Vehicle and industrial bus, seen at the transceiver RX pin. Rests high.".into(),
                confirm: "Inside a frame there are never more than 5 equal bits in a row. Frames end with a long high stretch. Bit time matches 125 k, 250 k, 500 k or 1 M bit/s (8, 4, 2 or 1 µs).".into(),
                decoder_tips: "One channel and the bit rate. The decoder shows identifier, length, data and CRC.".into(),
                lookalikes: "UART looks similar but has fixed 10- or 11-bit frames, each starting with a low bit.".into(),
                symptoms: symptoms(&[
                    (
                        "No ACK on every frame",
                        "Only one node on the bus, or the others have a different bit rate",
                        "Check that a second node is connected and set to the same rate.",
                    ),
                    (
                        "Stuffing or CRC errors",
                        "Wrong bit rate, missing 120 Ω termination, or noise",
                        "Check the bit rate and that both ends are terminated.",
                    ),
                ]),
            },
        }
    }
}

impl Protocol for Can {
    fn id(&self) -> &str {
        "can"
    }
    fn name(&self) -> &str {
        "CAN (logic side)"
    }
    fn group(&self) -> &str {
        "CAN"
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
        format!("{}, sample point {} %", bits_per_s(s.num("bitrate")), s.num("samplePoint"))
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
        let ch = &setup.channels[setup.slot("rx").ok_or("Choose a channel for CAN RX.")?];
        let m = CanMachine::new(setup.sample_rate_hz, &setup.settings, ch.channel, setup.start, u64::MAX);
        decoder(m, setup, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{BoundChannel, Output};
    use crate::synth::{Lane, can_frame};
    use logica_core::store::edges::{Edge, EdgeBatch};

    const RATE: f64 = 8e6;
    const BITRATE: f64 = 500e3;

    fn setup(rate: f64, bitrate: f64) -> DecoderSetup {
        DecoderSetup {
            sample_rate_hz: rate,
            start: 0,
            start_levels: vec![Level::High],
            channels: vec![BoundChannel { role: "rx".into(), channel: 0, name: "RX".into() }],
            settings: SettingSpec::with_defaults(&can_settings(), &settings(&[("bitrate", bitrate.into())])),
        }
    }

    /// Decodes in one batch, then one edge per batch with a snapshot and restore after each, and
    /// checks both give the same output.
    fn decode(edges: &[Sample], end: Sample, rate: f64, bitrate: f64) -> Output {
        let s = setup(rate, bitrate);
        let p = Can::default();
        let batch = |es: &[Sample], end: Sample| EdgeBatch {
            edges: es.iter().map(|&sample| Edge { sample, slot: 0, level: Level::Low }).collect(),
            end,
        };
        let mut whole = p.decoder(&s, None).unwrap();
        whole.push(&batch(edges, end));
        whole.finish(end);
        let expected = whole.drain();

        let mut d = p.decoder(&s, None).unwrap();
        let mut got = Output::default();
        for &e in edges {
            d.push(&batch(&[e], e + 1));
            let snap = d.snapshot();
            assert!(!snap.is_null(), "the state serialises");
            d = p.decoder(&s, Some(snap)).unwrap();
            let o = d.drain();
            got.frames.extend(o.frames);
            got.packets.extend(o.packets);
            got.notes.extend(o.notes);
        }
        d.finish(end);
        let o = d.drain();
        got.frames.extend(o.frames);
        got.packets.extend(o.packets);
        got.notes.extend(o.notes);
        assert_eq!(got, expected, "streamed output differs");
        expected
    }

    fn labels(o: &Output) -> Vec<&str> {
        o.frames.iter().map(|f| f.label.as_str()).collect()
    }

    fn has(o: &Output, code: ProblemCode) -> bool {
        o.frames.iter().any(|f| f.problems.iter().any(|p| p.code == code))
    }

    /// Frame bits from SOF to the end of the CRC, stuffed, then the unstuffed tail.
    fn wire(id: u32, rtr: bool, dlc: u8, data: &[u8], crc_delim: u8, ack: u8) -> Vec<u8> {
        let mut bits: Vec<u8> = vec![0];
        let push = |bits: &mut Vec<u8>, v: u64, n: u32| {
            for k in (0..n).rev() {
                bits.push(u8::from((v >> k) & 1 == 1));
            }
        };
        push(&mut bits, u64::from(id), 11);
        bits.extend([u8::from(rtr), 0, 0]);
        push(&mut bits, u64::from(dlc), 4);
        for &d in data {
            push(&mut bits, u64::from(d), 8);
        }
        let crc = can_crc15(&bits);
        push(&mut bits, u64::from(crc), 15);
        let mut out = Vec::new();
        let (mut run, mut last) = (0, 2u8);
        for &b in &bits {
            out.push(b);
            run = if b == last { run + 1 } else { 1 };
            last = b;
            if run == 5 {
                out.push(1 - b);
                last = 1 - b;
                run = 1;
            }
        }
        out.extend([crc_delim, ack, 1, 1, 1, 1, 1, 1, 1, 1]);
        out
    }

    fn lane_of(bits: &[u8], t0: f64) -> Lane {
        let mut l = Lane::high(RATE);
        for (i, &b) in bits.iter().enumerate() {
            l.set(t0 + i as f64 / BITRATE, b);
        }
        l
    }

    #[test]
    fn a_standard_frame_is_decoded_field_by_field() {
        let mut l = Lane::high(RATE);
        let end = can_frame(&mut l, 0.0001, BITRATE, 0x123, &[0x11, 0x22, 0x33], false);
        let o = decode(&l.edges, num::round(end * RATE), RATE, BITRATE);
        assert_eq!(labels(&o), ["SOF", "ID 123", "DLC 3", "11", "22", "33", "CRC ok", "ACK"]);
        assert_eq!(o.packets.len(), 1);
        let p = &o.packets[0];
        assert_eq!((p.title.as_str(), p.start), ("ID 0x123", 800));
        assert_eq!(
            (p.streams[0].label.as_str(), p.streams[0].bytes.clone()),
            ("ID 0x123", vec![0x11, 0x22, 0x33])
        );
        assert_eq!((o.frames[0].start, o.frames[0].end, o.frames[1].start), (800, 816, 816));
        assert_eq!(o.frames[1].detail.as_deref(), Some("Standard 11-bit identifier 0x123"));
        assert_eq!(o.frames[3].detail.as_deref(), Some("Data byte 1: 11 = 17"));
        assert!(o.frames.iter().all(|f| f.problems.is_empty()));
    }

    #[test]
    fn an_extended_frame_has_a_29_bit_identifier() {
        let mut l = Lane::high(RATE);
        let end = can_frame(&mut l, 0.0001, BITRATE, 0x18da_f110, &[0xde, 0xad], true);
        let o = decode(&l.edges, num::round(end * RATE), RATE, BITRATE);
        assert_eq!(labels(&o), ["SOF", "ID 18DAF110", "DLC 2", "DE", "AD", "CRC ok", "ACK"]);
        assert_eq!(o.packets[0].title, "ID 0x18DAF110");
        assert_eq!(o.frames[1].value, Some(f64::from(0x18da_f110_u32)));
    }

    #[test]
    fn a_remote_request_has_a_length_but_no_data() {
        let l = lane_of(&wire(0x123, true, 2, &[], 1, 0), 0.0001);
        let o = decode(&l.edges, 4000, RATE, BITRATE);
        assert_eq!(labels(&o), ["SOF", "ID 123", "DLC 2", "CRC ok", "ACK"]);
        assert_eq!(o.packets[0].title, "ID 0x123 remote request");
        assert!(o.packets[0].streams.is_empty());
        assert_eq!(o.frames[2].detail.as_deref(), Some("2 data bytes"));
    }

    #[test]
    fn a_missing_ack_and_a_dominant_delimiter_are_reported() {
        let l = lane_of(&wire(0x42, false, 1, &[0x07], 0, 1), 0.0001);
        let o = decode(&l.edges, 4000, RATE, BITRATE);
        let ack = o.frames.last().unwrap();
        assert_eq!((ack.kind, ack.label.as_str()), (FrameKind::Nack, "no ACK"));
        assert!(has(&o, ProblemCode::FrameError));
        assert!(!has(&o, ProblemCode::Checksum));
    }

    #[test]
    fn six_equal_bits_are_a_stuffing_error() {
        // SOF and identifier 0x000 without the stuff bit: six dominant bits in a row
        let mut bits = vec![0u8; 12];
        bits.extend([1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
        let l = lane_of(&bits, 0.0001);
        let o = decode(&l.edges, 4000, RATE, BITRATE);
        assert_eq!(o.packets[0].title, "CAN frame (stuffing error)");
        assert_eq!(labels(&o)[0], "stuff error");
        assert!(has(&o, ProblemCode::Stuffing));
    }

    #[test]
    fn a_bit_moved_by_one_bit_time_gives_a_crc_or_stuffing_error() {
        // Signal Detective's test: edge 20 lands on edge 21, so the two cancel out
        let mut l = Lane::high(RATE);
        can_frame(&mut l, 0.0001, BITRATE, 0x100, &[0x55], false);
        let mut e = l.edges.clone();
        e[20] += 16;
        assert_eq!(e[20], e[21], "a capture store refuses this, so the edges go in directly");
        let s = SettingSpec::with_defaults(&can_settings(), &settings(&[("bitrate", BITRATE.into())]));
        let mut m = CanMachine::new(RATE, &s, 0, 0, u64::MAX);
        let mut ed = Edges::complete(vec![(Level::High, e)], 8000);
        let mut out = Out::new(true);
        m.run(&mut ed, &mut out);
        out.finish();
        let o = out.drain();
        assert!(has(&o, ProblemCode::Checksum) || has(&o, ProblemCode::Stuffing), "{:?}", labels(&o));
    }

    #[test]
    fn a_frame_cut_off_by_the_capture_end_still_decodes() {
        let mut l = Lane::high(RATE);
        can_frame(&mut l, 0.0001, BITRATE, 0x123, &[0xab], false);
        let last = *l.edges.last().unwrap();
        let o = decode(&l.edges, last + 2, RATE, BITRATE);
        assert_eq!(labels(&o), ["SOF", "ID 123", "DLC 1", "AB", "CRC ok", "ACK"]);
    }

    #[test]
    fn a_frame_needs_seven_recessive_bits_before_it() {
        let mut l = Lane::high(RATE);
        can_frame(&mut l, 3.0 / BITRATE, BITRATE, 0x123, &[0xab], false);
        let o = decode(&l.edges, 4000, RATE, BITRATE);
        assert!(o.packets.is_empty());
    }

    #[test]
    fn unusable_bit_rates_give_a_note_not_a_crash() {
        let mut l = Lane::high(RATE);
        can_frame(&mut l, 0.0001, BITRATE, 0x123, &[0xab], false);
        let o = decode(&l.edges, 4000, RATE, RATE);
        assert_eq!(o.notes, ["The bit rate is too high for this sample rate."]);
        assert!(o.frames.is_empty());
        let o = decode(&l.edges, 4000, RATE, 0.0);
        assert_eq!(o.notes, ["The bit rate must be above zero."]);
        assert!(o.frames.is_empty());
    }

    #[test]
    fn statistics_count_frames_and_stop_at_the_limit() {
        let mut l = Lane::high(RATE);
        let t = can_frame(&mut l, 0.0001, BITRATE, 0x123, &[1], false);
        let t = can_frame(&mut l, t + 0.0001, BITRATE, 0x124, &[2], false);
        let t = can_frame(&mut l, t + 0.0001, BITRATE, 0x125, &[3], false);
        let end = num::round(t * RATE);
        let ch = WinChannel { index: 0, slot: 0, name: "RX".into(), initial: Level::High, edges: l.edges };
        let s = SettingSpec::with_defaults(&can_settings(), &Settings::new());
        let all = can_stats(&ch, 0, end, RATE, &s, 500);
        assert_eq!((all.frames, all.crc_ok, all.stuff_errors, all.max_run), (3, 3, 0, 5));
        assert_eq!(can_stats(&ch, 0, end, RATE, &s, 2).frames, 2);
    }

    #[test]
    fn knowledge_and_settings_are_complete() {
        let p = Can::default();
        let i = p.info();
        assert!([&i.what, &i.confirm, &i.decoder_tips, &i.lookalikes].iter().all(|t| t.len() > 10));
        assert_eq!(i.symptoms.len(), 2);
        assert_eq!(
            p.describe(&SettingSpec::with_defaults(p.settings(), &Settings::new())),
            "500 kbit/s, sample point 75 %"
        );
    }
}
