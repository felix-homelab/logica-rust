//! SWD: the ARM serial wire debug port, a clock and one bidirectional data line.
//!
//! Every bit is read on the rising edge of SWCLK. A transfer is: request (8 bits: start, APnDP,
//! RnW, A2, A3, parity, stop, park), turnaround, 3-bit acknowledge, then 32 data bits and parity
//! (with a turnaround on the side where the direction changes). 50 or more clocks with SWDIO high
//! are a line reset.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use logica_core::Sample;

use crate::format;
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo, symptoms};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;
use crate::window::WinChannel;

const CLK: usize = 0;
const IO: usize = 1;

/// Clocks with SWDIO high that make a line reset.
const RESET_ONES: usize = 50;

const DP_READ: [&str; 4] = ["IDCODE", "CTRL/STAT", "SELECT", "RDBUFF"];
const DP_WRITE: [&str; 4] = ["ABORT", "CTRL/STAT", "SELECT", "RDBUFF"];
const AP_NAMES: [&str; 4] = ["CSW", "TAR", "AP 0x8", "DRW"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwdStats {
    pub requests: u64,
    pub ok_requests: u64,
    pub bad_parity: u64,
    pub resets: u64,
}

/// One bit: SWDIO at a rising edge of SWCLK.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Bit {
    v: u8,
    t: Sample,
}

/// A line reset being read: its first bit, how many ones so far, and the last one's time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ResetRun {
    start: Sample,
    ones: u64,
    last: Sample,
}

struct Data {
    start: Sample,
    end: Sample,
    /// Time of the parity bit (the end of the packet's bytes).
    last: Sample,
    value: u32,
    parity_ok: bool,
}

/// A request with whatever followed it, read in full before anything is emitted.
struct Transfer {
    start: Sample,
    end: Sample,
    ap: bool,
    read: bool,
    a: u8,
    parity_ok: bool,
    /// The acknowledge `(start, end, value)`; `None` when the capture ends first.
    ack: Option<(Sample, Sample, u8)>,
    data: Option<Data>,
    /// Packet end and bits used; `None` when the capture ends inside the transfer.
    close: Option<(Sample, usize)>,
}

/// What the bits at the front of the queue mean.
enum Act {
    /// At least this many bits are needed to decide.
    Need(usize),
    /// Not a request: skip one bit.
    Skip,
    /// 50 or more ones: a line reset starts.
    StartReset,
    /// One more one in the current line reset.
    ResetOne,
    /// The line reset ends at this sample.
    EndReset(Sample),
    Transfer(Box<Transfer>),
    /// The stream has ended and every bit is used.
    Finished,
}

/// The SWD decoder (Signal Detective's `runSwd`, made streaming). Bits are queued as they arrive;
/// a transfer is decided once its last bit (at most 47 after the start bit) is in the queue, or
/// the capture has ended.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SwdMachine {
    lane: u32,
    next_clk: u64,
    bits: VecDeque<Bit>,
    reset: Option<ResetRun>,
    pub stats: SwdStats,
}

impl SwdMachine {
    pub fn new(lane: u32) -> Self {
        SwdMachine { lane, next_clk: 0, bits: VecDeque::new(), reset: None, stats: SwdStats::default() }
    }

    /// Bit `n` of the queue; `Ok(None)` past the end of an ended stream, `Err(n + 1)` (bits
    /// needed) while more may come.
    fn at(&self, n: usize, ended: bool) -> Result<Option<Bit>, usize> {
        match self.bits.get(n) {
            Some(&b) => Ok(Some(b)),
            None if ended => Ok(None),
            None => Err(n + 1),
        }
    }

    /// Time of bit `n`, or of the last bit when the stream ended before it.
    fn time_at(&self, n: usize, ended: bool) -> Result<Sample, usize> {
        match self.at(n, ended)? {
            Some(b) => Ok(b.t),
            None => Ok(self.bits.back().map_or(0, |b| b.t)),
        }
    }

    fn plan(&self, ended: bool) -> Act {
        match self.decide(ended) {
            Ok(a) => a,
            Err(n) => Act::Need(n),
        }
    }

    fn decide(&self, ended: bool) -> Result<Act, usize> {
        if let Some(r) = self.reset {
            return Ok(match self.at(0, ended)? {
                Some(b) if b.v == 1 => Act::ResetOne,
                Some(b) => Act::EndReset(b.t),
                None => Act::EndReset(r.last),
            });
        }
        let Some(first) = self.at(0, ended)? else { return Ok(Act::Finished) };
        // line reset: 50 or more ones
        let mut ones = 0;
        while let Some(b) = self.at(ones, ended)? {
            if b.v != 1 {
                break;
            }
            ones += 1;
            if ones >= RESET_ONES {
                return Ok(Act::StartReset);
            }
        }
        if first.v != 1 || self.at(7, ended)?.is_none() {
            return Ok(Act::Skip);
        }
        let b = |n: usize| self.bits[n].v;
        if !(b(6) == 0 && b(7) == 1) {
            return Ok(Act::Skip);
        }
        let (ap, read) = (b(1) == 1, b(2) == 1);
        let a = b(3) | (b(4) << 1);
        let parity_ok = (b(1) + b(2) + b(3) + b(4)) & 1 == b(5);
        let mut t = Transfer {
            start: first.t,
            end: self.time_at(8, ended)?,
            ap,
            read,
            a,
            parity_ok,
            ack: None,
            data: None,
            close: None,
        };
        // turnaround, then the acknowledge
        let ack_at = 9;
        if self.at(ack_at + 2, ended)?.is_none() {
            return Ok(Act::Transfer(Box::new(t)));
        }
        let ack = b(ack_at) | (b(ack_at + 1) << 1) | (b(ack_at + 2) << 2);
        t.ack = Some((self.bits[ack_at].t, self.time_at(ack_at + 3, ended)?, ack));
        let mut next = ack_at + 3;
        if ack == 1 {
            let data_at = if read { next } else { next + 1 };
            let Some(parity) = self.at(data_at + 32, ended)? else { return Ok(Act::Transfer(Box::new(t))) };
            let mut value = 0u32;
            let mut ones32 = 0u8;
            for n in 0..32 {
                let bit = b(data_at + n);
                ones32 += bit;
                value |= u32::from(bit) << n;
            }
            t.data = Some(Data {
                start: self.bits[data_at].t,
                end: self.time_at(data_at + 33, ended)?,
                last: parity.t,
                value,
                parity_ok: ones32 & 1 == parity.v,
            });
            next = data_at + 33 + usize::from(read);
        } else {
            next += 1;
        }
        t.close = Some((self.time_at(next, ended)?, next));
        Ok(Act::Transfer(Box::new(t)))
    }

    /// Reads the next clock edge; a rising edge adds a bit. `Ok(false)` once the stream has ended.
    fn pull(&mut self, ed: &mut Edges) -> Res<bool> {
        let Some(t) = ed.edge(CLK, self.next_clk)? else { return Ok(false) };
        let rising = ed.level_after(CLK, self.next_clk).is_high();
        let v = if rising { Some(ed.level_at(IO, t)?) } else { None };
        self.next_clk += 1;
        ed.release_before(t);
        if let Some(v) = v {
            self.bits.push_back(Bit { v: u8::from(v.is_high()), t });
        }
        Ok(true)
    }

    /// Carries out `act`; `false` once there is nothing left.
    fn apply(&mut self, act: Act, out: &mut Out) -> bool {
        match act {
            Act::Need(_) => {}
            Act::Finished => return false,
            Act::Skip => {
                self.bits.pop_front();
            }
            Act::StartReset => {
                let start = self.bits[0].t;
                let mut run = ResetRun { start, ones: 0, last: start };
                while let Some(b) = self.bits.front().copied().filter(|b| b.v == 1) {
                    run.ones += 1;
                    run.last = b.t;
                    self.bits.pop_front();
                }
                self.reset = Some(run);
            }
            Act::ResetOne => {
                if let (Some(r), Some(b)) = (&mut self.reset, self.bits.pop_front()) {
                    r.ones += 1;
                    r.last = b.t;
                }
            }
            Act::EndReset(end) => {
                if let Some(r) = self.reset.take() {
                    self.stats.resets += 1;
                    out.open_packet(r.start, "Line reset");
                    out.add(
                        NewFrame::new(r.start, end, self.lane, FrameKind::Reset, "line reset")
                            .detail(format!("{} clocks with SWDIO high: line reset", r.ones)),
                    );
                }
            }
            Act::Transfer(t) => self.transfer(*t, out),
        }
        true
    }

    fn transfer(&mut self, t: Transfer, out: &mut Out) {
        let lane = self.lane;
        if !t.parity_ok {
            self.stats.bad_parity += 1;
        }
        self.stats.requests += 1;
        let reg = if t.ap {
            AP_NAMES[usize::from(t.a)]
        } else if t.read {
            DP_READ[usize::from(t.a)]
        } else {
            DP_WRITE[usize::from(t.a)]
        };
        let port = if t.ap { "AP" } else { "DP" };
        out.open_packet(t.start, format!("{port} {} {reg}", if t.read { "read" } else { "write" }));
        out.add(
            NewFrame::new(
                t.start,
                t.end,
                lane,
                FrameKind::Command,
                format!("{port} {} {reg}", if t.read { "R" } else { "W" }),
            )
            .value(f64::from(t.a * 4))
            .detail(format!(
                "Request: {} {}, register {reg} (address 0x{:x})",
                if t.ap { "access port" } else { "debug port" },
                if t.read { "read" } else { "write" },
                t.a * 4
            ))
            .problems(if t.parity_ok {
                Vec::new()
            } else {
                vec![problem(ProblemCode::ParityError, "Request parity is wrong.")]
            }),
        );
        if let Some((start, end, ack)) = t.ack {
            let (label, detail) = match ack {
                1 => ("OK", "Target accepted the request."),
                2 => ("WAIT", "Target asks to retry later."),
                4 => ("FAULT", "Target reports a fault."),
                _ => ("no answer", "No valid acknowledge: the target is not answering."),
            };
            out.add(
                NewFrame::new(
                    start,
                    end,
                    lane,
                    if ack == 1 { FrameKind::Ack } else { FrameKind::Nack },
                    label,
                )
                .detail(detail),
            );
            if ack == 1 {
                self.stats.ok_requests += 1;
            }
        }
        if let Some(d) = t.data {
            let hex = format!("{:08X}", d.value);
            out.add(
                NewFrame::new(d.start, d.end, lane, FrameKind::Data, hex.as_str())
                    .value(f64::from(d.value))
                    .detail(format!("{} 0x{hex}", if t.read { "Read" } else { "Wrote" }))
                    .problems(if d.parity_ok {
                        Vec::new()
                    } else {
                        vec![problem(ProblemCode::ParityError, "Data parity is wrong.")]
                    }),
            );
            for byte in d.value.to_le_bytes() {
                out.byte(if t.read { "read" } else { "write" }, lane, byte, d.last);
            }
        }
        match t.close {
            Some((end, used)) => {
                out.close_packet(end);
                self.bits.drain(..used.min(self.bits.len()));
            }
            // the capture ended inside this transfer: nothing more to read
            None => self.bits.clear(),
        }
    }

    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        if let Act::Need(n) = self.plan(false) {
            while self.bits.len() < n {
                if !self.pull(ed)? {
                    let act = self.plan(true);
                    return Ok(self.apply(act, out));
                }
            }
        }
        let act = self.plan(false);
        Ok(self.apply(act, out))
    }
}

impl Machine for SwdMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }
}

/// Runs the decoder over two window channels, keeping only statistics.
pub fn swd_stats(clk: &WinChannel, io: &WinChannel, end: Sample) -> SwdStats {
    let mut m = SwdMachine::new(io.index);
    let mut ed = Edges::complete(vec![(clk.initial, clk.edges.clone()), (io.initial, io.edges.clone())], end);
    let mut out = Out::new(false);
    run_steps(|| m.step(&mut ed, &mut out));
    m.stats
}

fn detect(ctx: &DetectContext<'_>) -> DetectorOutput {
    let chs: Vec<&WinChannel> = ctx.channels.iter().copied().filter(|c| !c.edges.is_empty()).collect();
    if chs.len() != 2 {
        return DetectorOutput::rejected("swd", "SWD uses exactly two lines");
    }
    let mut out = DetectorOutput::default();
    for (clk, io) in [(chs[0], chs[1]), (chs[1], chs[0])] {
        let Some(per) = &ctx.m(clk.index).periodic else { continue };
        let stats = swd_stats(clk, io, ctx.window.end);
        if stats.requests < 1 {
            continue;
        }
        let requests = stats.requests as f64;
        let ok_share = stats.ok_requests as f64 / requests;
        let par_share = 1.0 - stats.bad_parity as f64 / requests;
        let mut channels = vec![clk.index, io.index];
        channels.sort_unstable();
        out.detections.push(Detection {
            protocol_id: "swd".into(),
            channels,
            roles: roles(&[("swclk", Some(clk.index)), ("swdio", Some(io.index))]),
            settings: Settings::new(),
            notes: [
                ("role:swclk", note(0.85, format!("Channel {} has the clock pulses.", clk.index))),
                (
                    "role:swdio",
                    note(0.85, format!("Channel {} carries requests with valid parity.", io.index)),
                ),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            confidence: clamp01(
                0.2 + 0.35 * ok_share + 0.25 * par_share + if stats.resets > 0 { 0.15 } else { 0.0 },
            ),
            evidence: vec![
                evidence(
                    format!(
                        "Channel {} is a clock in bursts at {}; channel {} carries the data.",
                        clk.index,
                        format::hz(ctx.sample_rate_hz / per.period_samples),
                        io.index
                    ),
                    true,
                ),
                evidence(
                    format!(
                        "{} request{} found with the start, stop and park bits in place; {} got an OK answer.",
                        stats.requests,
                        format::s(stats.requests),
                        stats.ok_requests
                    ),
                    ok_share > 0.5,
                ),
                evidence(
                    match stats.resets {
                        0 => "No line reset (50 clocks with data high) was seen.".to_string(),
                        1 => "There is a line reset (50 or more clocks with data high).".to_string(),
                        n => format!("There are {n} line resets (50 or more clocks with data high)."),
                    },
                    stats.resets > 0,
                ),
            ],
            warnings: Vec::new(),
        });
    }
    if out.detections.is_empty() {
        out.rejections.push(Rejection {
            protocol_id: "swd".into(),
            because: "no SWD requests (start 1, stop 0, park 1, with parity) were found".into(),
        });
    }
    out
}

pub struct Swd {
    roles: Vec<RoleSpec>,
    info: ProtocolInfo,
}

impl Default for Swd {
    fn default() -> Self {
        Swd {
            roles: vec![
                RoleSpec::new("swclk", "SWCLK", "SWCLK", "The clock from the debug probe."),
                RoleSpec::new("swdio", "SWDIO", "SWDIO", "The data line, driven by both sides in turn."),
            ],
            info: ProtocolInfo {
                what: "Two-wire debug link between a probe and an ARM microcontroller.".into(),
                confirm: "One clock and one data line, no start or stop marks, long runs of 50 or more clocks with data high (line reset).".into(),
                decoder_tips: "Assign SWCLK and SWDIO. It only appears while a debugger is attached.".into(),
                lookalikes: "I2C has groups of 9 clocks and both lines rest high.".into(),
                symptoms: symptoms(&[(
                    "Every request gets no answer",
                    "Target not powered, held in reset, or SWD disabled",
                    "Check power and reset; some chips need the JTAG-to-SWD switch sequence first.",
                )]),
            },
        }
    }
}

impl Protocol for Swd {
    fn id(&self) -> &str {
        "swd"
    }
    fn name(&self) -> &str {
        "SWD debug port"
    }
    fn group(&self) -> &str {
        "Debug"
    }
    fn info(&self) -> &ProtocolInfo {
        &self.info
    }
    fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }
    fn settings(&self) -> &[SettingSpec] {
        &[]
    }
    fn detect(&self, ctx: &DetectContext<'_>) -> Option<DetectorOutput> {
        Some(detect(ctx))
    }
    fn has_decoder(&self) -> bool {
        true
    }
    fn decoder(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        let (Some(clk), Some(io)) = (setup.slot("swclk"), setup.slot("swdio")) else {
            return Err("Choose channels for SWCLK and SWDIO.".into());
        };
        debug_assert_eq!((clk, io), (CLK, IO));
        decoder(SwdMachine::new(setup.channels[io].channel), setup, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use logica_core::Level;
    use logica_core::measure::measure_channel;
    use logica_core::store::edges::{Edge, EdgeBatch};

    use super::*;
    use crate::analyse::{Batching, decode_store};
    use crate::buses::Bus;
    use crate::examples::example;
    use crate::synth::{GenCapture, Lane, make_capture, swd_line_reset};
    use crate::window::Window;

    const RATE: f64 = 20e6;
    const HZ: f64 = 1e6;

    /// Clocks `bits` out on SWDIO, one per SWCLK period (data set before the rising edge).
    fn send(clk: &mut Lane, io: &mut Lane, mut t: f64, bits: &[u8]) -> f64 {
        let tb = 1.0 / HZ;
        for &v in bits {
            io.set(t, v);
            clk.set(t + tb / 2.0, 1u8);
            clk.set(t + tb, 0u8);
            t += tb;
        }
        t
    }

    fn word(v: u32) -> Vec<u8> {
        (0..32).map(|k| u8::from((v >> k) & 1 == 1)).collect()
    }

    fn capture(clk: Lane, io: Lane) -> GenCapture {
        make_capture("SWD test", RATE, 0.0, vec![(0, "SWCLK", clk), (1, "SWDIO", io)])
    }

    /// Decodes in every batching and checks they agree.
    fn decode(g: &GenCapture) -> DecodeResult {
        let (store, meta) = (g.store().unwrap(), g.meta());
        let reg = crate::registry();
        let r = roles(&[("swclk", Some(0)), ("swdio", Some(1))]);
        let s = Settings::new();
        let whole = decode_store(&reg, &store, &meta, "swd", &r, &s, Batching::Whole).unwrap();
        for b in [Batching::Edges(1), Batching::Edges(2), Batching::Edges(5), Batching::WithSnapshots(3)] {
            assert_eq!(decode_store(&reg, &store, &meta, "swd", &r, &s, b).unwrap(), whole, "{b:?}");
        }
        whole
    }

    fn labels(r: &DecodeResult) -> Vec<&str> {
        r.frames.iter().map(|f| f.label.as_str()).collect()
    }

    fn detect_lanes(lanes: Vec<Lane>) -> DetectorOutput {
        let end = lanes.iter().filter_map(|l| l.edges.last()).max().map_or(1000, |&e| e + 100);
        let channels: Vec<WinChannel> = lanes
            .into_iter()
            .enumerate()
            .map(|(i, l)| WinChannel {
                index: u32::try_from(i).unwrap(),
                slot: i,
                name: format!("D{i}"),
                initial: l.initial,
                edges: l.edges,
            })
            .collect();
        let measurements: BTreeMap<_, _> = channels
            .iter()
            .map(|c| (c.index, measure_channel(c.index, &c.edges, c.initial, 0, end)))
            .collect();
        let window = Window { start: 0, end, channels, note: None };
        let bus = Bus {
            id: "b".into(),
            channels: window.channels.iter().map(|c| c.index).collect(),
            reason: String::new(),
        };
        let ctx = DetectContext {
            sample_rate_hz: RATE,
            window: &window,
            bus: &bus,
            channels: window.channels.iter().collect(),
            measurements: &measurements,
        };
        detect(&ctx)
    }

    #[test]
    fn errors_answers_and_a_capture_cut_inside_a_transfer() {
        let (mut clk, mut io) = (Lane::low(RATE), Lane::low(RATE));
        let mut t = swd_line_reset(&mut clk, &mut io, 1e-5, HZ, 60);
        // DP read IDCODE, OK, data with the wrong parity (1 is right), turnaround, idle
        let mut bits = vec![1, 0, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0];
        bits.extend(word(0x1234_5678));
        bits.extend([0, 0, 0, 0]);
        t = send(&mut clk, &mut io, t + 5e-6, &bits);
        // AP write TAR with the wrong request parity, FAULT
        t = send(&mut clk, &mut io, t + 5e-6, &[1, 1, 0, 1, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
        // DP write SELECT, nobody drives the acknowledge (pulled high)
        t = send(&mut clk, &mut io, t + 5e-6, &[1, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 0, 0]);
        // AP read DRW, OK, and the capture ends after 10 data bits
        let mut bits = vec![1, 1, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0];
        bits.extend(&word(0xffff_ffff)[..10]);
        send(&mut clk, &mut io, t + 5e-6, &bits);
        let r = decode(&capture(clk, io));
        assert_eq!(
            labels(&r),
            [
                "line reset",
                "DP R IDCODE",
                "OK",
                "12345678",
                "AP W TAR",
                "FAULT",
                "DP W SELECT",
                "no answer",
                "AP R DRW",
                "OK"
            ]
        );
        assert_eq!(r.problem_counts.get(&ProblemCode::ParityError), Some(&2));
        let problems: Vec<(&str, &str)> = r
            .frames
            .iter()
            .flat_map(|f| f.problems.iter().map(move |p| (f.label.as_str(), p.message.as_str())))
            .collect();
        assert_eq!(
            problems,
            [("12345678", "Data parity is wrong."), ("AP W TAR", "Request parity is wrong.")]
        );
        let detail = |l: &str| r.frames.iter().find(|f| f.label == l).and_then(|f| f.detail.clone()).unwrap();
        assert_eq!(detail("line reset"), "60 clocks with SWDIO high: line reset");
        assert_eq!(detail("AP W TAR"), "Request: access port write, register TAR (address 0x4)");
        assert_eq!(detail("12345678"), "Read 0x12345678");
        assert_eq!(detail("no answer"), "No valid acknowledge: the target is not answering.");
        assert_eq!((r.frames[2].kind, r.frames[5].kind), (FrameKind::Ack, FrameKind::Nack));
        assert_eq!(
            r.packets.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(),
            ["Line reset", "DP read IDCODE", "AP write TAR", "DP write SELECT", "AP read DRW"]
        );
        assert_eq!(r.packets[1].stream("read").unwrap().bytes, [0x78, 0x56, 0x34, 0x12]);
        assert!(r.packets.iter().all(|p| p.end >= p.start));
    }

    #[test]
    fn a_line_reset_at_the_end_of_the_capture_ends_at_its_last_clock() {
        let (mut clk, mut io) = (Lane::low(RATE), Lane::low(RATE));
        send(&mut clk, &mut io, 1e-5, &[1; 55]);
        let rises: Vec<Sample> = clk.edges.iter().step_by(2).copied().collect();
        let r = decode(&capture(clk, io));
        assert_eq!(labels(&r), ["line reset"]);
        let f = &r.frames[0];
        assert_eq!((f.start, f.end), (rises[0], rises[54]));
        assert_eq!(f.detail.as_deref(), Some("55 clocks with SWDIO high: line reset"));
    }

    #[test]
    fn short_runs_of_ones_and_cut_requests_are_not_decoded() {
        let (mut clk, mut io) = (Lane::low(RATE), Lane::low(RATE));
        // 49 ones is not a line reset; 8 zeros after it, so no 8-bit window ends in stop 0, park 1
        let t = send(&mut clk, &mut io, 1e-5, &[[1; 49].as_slice(), &[0; 8]].concat());
        // a request start with only 5 bits before the capture ends
        send(&mut clk, &mut io, t + 5e-6, &[1, 0, 1, 0, 0]);
        let r = decode(&capture(clk, io));
        assert!(r.frames.is_empty(), "{:?}", labels(&r));
    }

    #[test]
    fn a_long_line_reset_keeps_memory_bounded() {
        let (mut clk, mut io) = (Lane::low(RATE), Lane::low(RATE));
        send(&mut clk, &mut io, 1e-5, &[1; 5000]);
        let mut edges: Vec<Edge> = Vec::new();
        for (slot, lane) in [&clk, &io].into_iter().enumerate() {
            let mut level = lane.initial;
            for &s in &lane.edges {
                level = !level;
                edges.push(Edge { sample: s, slot: u8::try_from(slot).unwrap(), level });
            }
        }
        edges.sort_by_key(|e| (e.sample, e.slot));
        let last = edges.last().unwrap().sample;
        let mut m = SwdMachine::new(1);
        let mut ed = Edges::new(&[Level::Low, Level::Low], 0);
        let mut out = Out::new(true);
        ed.push(&EdgeBatch { edges, end: last + 1 });
        m.run(&mut ed, &mut out);
        assert!(m.bits.len() < 60, "{} bits queued", m.bits.len());
        assert!(ed.buffered() < 4, "{} edges held", ed.buffered());
        assert!(out.drain().frames.is_empty(), "the reset is still running");
        ed.finish(last + 10);
        m.run(&mut ed, &mut out);
        out.finish();
        let o = out.drain();
        assert_eq!(o.frames.len(), 1);
        assert_eq!(o.frames[0].detail.as_deref(), Some("5000 clocks with SWDIO high: line reset"));
        assert_eq!(m.stats.resets, 1);
    }

    #[test]
    fn the_example_is_detected_with_the_clock_on_channel_0() {
        let g = (example("swd").unwrap().build)();
        let out = detect_lanes(g.channels.into_iter().map(|(_, _, l)| l).collect());
        let d: Vec<&Detection> = out.detections.iter().filter(|d| d.roles["swclk"] == Some(0)).collect();
        assert_eq!(d.len(), 1);
        let d = d[0];
        assert_eq!((d.roles["swdio"], d.channels.as_slice()), (Some(1), [0, 1].as_slice()));
        assert!((d.confidence - (0.2 + 0.35 * 0.75 + 0.25 + 0.15)).abs() < 1e-9, "{}", d.confidence);
        let ev: Vec<(&str, bool)> = d.evidence.iter().map(|e| (e.text.as_str(), e.supports)).collect();
        assert_eq!(
            ev,
            [
                ("Channel 0 is a clock in bursts at 1 MHz; channel 1 carries the data.", true),
                ("4 requests found with the start, stop and park bits in place; 3 got an OK answer.", true),
                ("There is a line reset (50 or more clocks with data high).", true),
            ]
        );
        assert!(out.rejections.is_empty());
    }

    #[test]
    fn the_detector_rejects_other_line_counts_and_lines_without_requests() {
        let mut one = Lane::low(RATE);
        for k in 0..100u32 {
            one.set(f64::from(k) * 1e-6, u8::from(k % 2 == 0));
        }
        let out = detect_lanes(vec![one]);
        assert!(out.detections.is_empty());
        assert_eq!(out.rejections[0].because, "SWD uses exactly two lines");

        let (mut clk, mut io) = (Lane::low(RATE), Lane::low(RATE));
        let pattern: Vec<u8> = (0..300).map(|k| u8::from(k % 3 != 2)).collect();
        send(&mut clk, &mut io, 1e-5, &pattern);
        let out = detect_lanes(vec![clk, io]);
        assert!(out.detections.is_empty(), "{:?}", out.detections);
        assert_eq!(
            out.rejections[0].because,
            "no SWD requests (start 1, stop 0, park 1, with parity) were found"
        );
    }

    #[test]
    fn a_decoder_needs_both_roles() {
        let setup = DecoderSetup {
            sample_rate_hz: RATE,
            start: 0,
            start_levels: vec![Level::Low],
            channels: vec![crate::stream::BoundChannel {
                role: "swclk".into(),
                channel: 0,
                name: String::new(),
            }],
            settings: Settings::new(),
        };
        let err = Swd::default().decoder(&setup, None).err();
        assert_eq!(err.as_deref(), Some("Choose channels for SWCLK and SWDIO."));
    }
}
