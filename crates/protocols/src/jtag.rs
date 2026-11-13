//! JTAG: test access port with TCK, TMS and, optionally, TDI and TDO.
//!
//! The decoder follows the TAP controller on every rising TCK edge, starting in Test-Logic-Reset.
//! TAP states go in annotation row 1 under TMS; each IR or DR shift is a packet with one frame in
//! row 0 (bits least significant first, as JTAG shifts them).

use serde::{Deserialize, Serialize};

use logica_core::Sample;

use crate::format;
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;
use crate::window::WinChannel;

/// The sixteen states of the TAP controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tap {
    TestLogicReset,
    RunTestIdle,
    SelectDr,
    CaptureDr,
    ShiftDr,
    Exit1Dr,
    PauseDr,
    Exit2Dr,
    UpdateDr,
    SelectIr,
    CaptureIr,
    ShiftIr,
    Exit1Ir,
    PauseIr,
    Exit2Ir,
    UpdateIr,
}

impl Tap {
    /// The state after a rising TCK edge with TMS at `tms`.
    pub fn next(self, tms: bool) -> Tap {
        use Tap::*;
        let (low, high) = match self {
            TestLogicReset => (RunTestIdle, TestLogicReset),
            RunTestIdle => (RunTestIdle, SelectDr),
            SelectDr => (CaptureDr, SelectIr),
            CaptureDr => (ShiftDr, Exit1Dr),
            ShiftDr => (ShiftDr, Exit1Dr),
            Exit1Dr => (PauseDr, UpdateDr),
            PauseDr => (PauseDr, Exit2Dr),
            Exit2Dr => (ShiftDr, UpdateDr),
            UpdateDr => (RunTestIdle, SelectDr),
            SelectIr => (CaptureIr, TestLogicReset),
            CaptureIr => (ShiftIr, Exit1Ir),
            ShiftIr => (ShiftIr, Exit1Ir),
            Exit1Ir => (PauseIr, UpdateIr),
            PauseIr => (PauseIr, Exit2Ir),
            Exit2Ir => (ShiftIr, UpdateIr),
            UpdateIr => (RunTestIdle, SelectDr),
        };
        if tms { high } else { low }
    }

    pub fn name(self) -> &'static str {
        use Tap::*;
        match self {
            TestLogicReset => "Test-Logic-Reset",
            RunTestIdle => "Run-Test/Idle",
            SelectDr => "Select-DR",
            CaptureDr => "Capture-DR",
            ShiftDr => "Shift-DR",
            Exit1Dr => "Exit1-DR",
            PauseDr => "Pause-DR",
            Exit2Dr => "Exit2-DR",
            UpdateDr => "Update-DR",
            SelectIr => "Select-IR",
            CaptureIr => "Capture-IR",
            ShiftIr => "Shift-IR",
            Exit1Ir => "Exit1-IR",
            PauseIr => "Pause-IR",
            Exit2Ir => "Exit2-IR",
            UpdateIr => "Update-IR",
        }
    }

    /// The register shifted in this state ("IR" or "DR"), if it is a shift state.
    pub fn register(self) -> Option<&'static str> {
        match self {
            Tap::ShiftIr => Some("IR"),
            Tap::ShiftDr => Some("DR"),
            _ => None,
        }
    }
}

/// Bits (least significant first) as hex, most significant digit first; "0" for none.
fn hex_lsb_first(bits: &[bool]) -> String {
    if bits.is_empty() {
        return "0".into();
    }
    bits.chunks(4)
        .rev()
        .map(|nibble| {
            let v = nibble.iter().enumerate().fold(0u32, |v, (j, &b)| v | (u32::from(b) << j));
            char::from_digit(v, 16).unwrap_or('0').to_ascii_uppercase()
        })
        .collect()
}

/// Bits (least significant first) packed into bytes, least significant bit first.
fn pack_lsb_first(bits: &[bool]) -> Vec<u8> {
    bits.chunks(8)
        .map(|byte| byte.iter().enumerate().fold(0u8, |v, (j, &b)| v | (u8::from(b) << j)))
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Line {
    slot: usize,
    lane: u32,
}

/// What a trial decode found (for detection).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JtagStats {
    /// Shift-IR and Shift-DR states left.
    pub shifts: u64,
    /// Runs of 5 clocks with TMS high (the reset sequence).
    pub resets: u64,
    pub ir_shifts: u64,
    /// Instruction shifts where TDO starts with 1, 0 (IEEE 1149.1 requires the IR capture value to
    /// end in binary 01).
    pub ir_capture01: u64,
    /// Start and end of every shift, kept only for trial decodes.
    pub shift_windows: Vec<(Sample, Sample)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JtagMachine {
    /// Slot of TMS (TCK is slot 0).
    tms: usize,
    /// Lane of the TAP state frames (TMS).
    state_lane: u32,
    /// Lane of the shift frames (TDI, else TDO, else TMS).
    data_lane: u32,
    tdi: Option<Line>,
    tdo: Option<Line>,
    keep_windows: bool,
    /// Absolute index of the next TCK edge.
    next: u64,
    state: Tap,
    /// Where the current state began; `None` before the first TCK edge.
    state_start: Option<Sample>,
    /// Rising TCK edges in a row with TMS high.
    ones_run: u64,
    shift_start: Sample,
    shift_clocks: u64,
    shift_in: Vec<bool>,
    shift_out: Vec<bool>,
    pub stats: JtagStats,
}

impl JtagMachine {
    fn new(tms: Line, tdi: Option<Line>, tdo: Option<Line>, keep_windows: bool) -> Self {
        let data_lane = tdi.as_ref().or(tdo.as_ref()).map_or(tms.lane, |l| l.lane);
        JtagMachine {
            tms: tms.slot,
            state_lane: tms.lane,
            data_lane,
            tdi,
            tdo,
            keep_windows,
            next: 0,
            state: Tap::TestLogicReset,
            state_start: None,
            ones_run: 0,
            shift_start: 0,
            shift_clocks: 0,
            shift_in: Vec::new(),
            shift_out: Vec::new(),
            stats: JtagStats::default(),
        }
    }

    /// One TCK edge. Rising edges read TMS (and TDI and TDO in a shift state) and step the TAP.
    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let k = self.next;
        let Some(t) = ed.edge(0, k)? else { return Ok(false) };
        let mut sampled = None;
        if ed.level_after(0, k).is_high() {
            let tms = ed.level_at(self.tms, t)?.is_high();
            let shifting = self.state.register().is_some();
            let read = |line: &Option<Line>| -> Res<Option<bool>> {
                match line {
                    Some(l) if shifting => Ok(Some(ed.level_at(l.slot, t)?.is_high())),
                    _ => Ok(None),
                }
            };
            let tdi = read(&self.tdi)?;
            let tdo = read(&self.tdo)?;
            sampled = Some((tms, tdi, tdo));
        }
        // every read is done: commit
        self.state_start.get_or_insert(t);
        self.next = k + 1;
        if let Some((tms, tdi, tdo)) = sampled {
            self.clock(t, tms, tdi, tdo, out);
        }
        ed.release_index(0, self.next);
        ed.release_before(t);
        Ok(true)
    }

    fn clock(&mut self, t: Sample, tms: bool, tdi: Option<bool>, tdo: Option<bool>, out: &mut Out) {
        self.ones_run = if tms { self.ones_run.saturating_add(1) } else { 0 };
        if self.ones_run == 5 {
            self.stats.resets += 1;
        }
        let register = self.state.register();
        if register.is_some() {
            self.shift_clocks += 1;
            self.shift_in.extend(tdi);
            self.shift_out.extend(tdo);
        }
        let next = self.state.next(tms);
        if next == self.state {
            return;
        }
        if let Some(reg) = register {
            self.shift(reg, t, out);
        }
        let start = self.state_start.unwrap_or(t);
        let name = self.state.name();
        out.add(
            NewFrame::new(start, t, self.state_lane, FrameKind::Info, name)
                .detail(format!("TAP state {name}"))
                .row(1),
        );
        if next.register().is_some() {
            self.shift_start = t;
            self.shift_clocks = 0;
            self.shift_in.clear();
            self.shift_out.clear();
        }
        self.state = next;
        self.state_start = Some(t);
    }

    /// A shift state ends at `t`: one packet with the bits shifted in and out.
    fn shift(&mut self, reg: &str, t: Sample, out: &mut Out) {
        self.stats.shifts += 1;
        if self.keep_windows {
            self.stats.shift_windows.push((self.shift_start, t));
        }
        if reg == "IR" {
            self.stats.ir_shifts += 1;
            if self.shift_out.first() == Some(&true) && self.shift_out.get(1) == Some(&false) {
                self.stats.ir_capture01 += 1;
            }
        }
        let n = self.shift_clocks;
        let (hex_in, hex_out) = (hex_lsb_first(&self.shift_in), hex_lsb_first(&self.shift_out));
        let mut label = vec![reg.to_string()];
        let mut values = Vec::new();
        if self.tdi.is_some() {
            label.push(format!("in {hex_in}"));
            values.push(format!("TDI 0x{hex_in}"));
        }
        if self.tdo.is_some() {
            label.push(format!("out {hex_out}"));
            values.push(format!("TDO 0x{hex_out}"));
        }
        let mut detail = format!(
            "{n} bit{} shifted into the {} register.",
            format::s(n),
            if reg == "IR" { "instruction" } else { "data" }
        );
        if !values.is_empty() {
            detail.push_str(&format!(" {} (least significant bit first)", values.join(", ")));
        }
        out.open_packet(self.shift_start, format!("Shift {reg} ({n} bit{})", format::s(n)));
        out.add(
            NewFrame::new(self.shift_start, t, self.data_lane, FrameKind::Data, label.join(" "))
                .detail(detail),
        );
        if let Some(l) = &self.tdi {
            for b in pack_lsb_first(&self.shift_in) {
                out.byte("TDI", l.lane, b, t);
            }
        }
        if let Some(l) = &self.tdo {
            for b in pack_lsb_first(&self.shift_out) {
                out.byte("TDO", l.lane, b, t);
            }
        }
        out.close_packet(t);
    }
}

impl Machine for JtagMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }
}

// ------------------------------------------------------------------ detection

/// Runs the decoder over window channels, keeping statistics only.
fn trial(
    tck: &WinChannel,
    tms: &WinChannel,
    tdi: Option<&WinChannel>,
    tdo: Option<&WinChannel>,
    end: Sample,
) -> JtagStats {
    let mut chans = vec![(tck.initial, tck.edges.clone()), (tms.initial, tms.edges.clone())];
    let mut line = |c: Option<&WinChannel>| {
        c.map(|c| {
            chans.push((c.initial, c.edges.clone()));
            Line { slot: chans.len() - 1, lane: c.index }
        })
    };
    let (tdi, tdo) = (line(tdi), line(tdo));
    let mut ed = Edges::complete(chans, end);
    let mut m = JtagMachine::new(Line { slot: 1, lane: tms.index }, tdi, tdo, true);
    m.run(&mut ed, &mut Out::new(false));
    m.stats
}

/// TDO edges more than 2 samples outside every shift (TDO is driven only while shifting).
fn tdo_outside_shift(tdo: &WinChannel, windows: &[(Sample, Sample)]) -> u64 {
    let mut w = 0;
    let mut outside = 0;
    for &t in &tdo.edges {
        while windows.get(w).is_some_and(|&(_, b)| b + 2 < t) {
            w += 1;
        }
        if windows.get(w).is_none_or(|&(a, _)| t + 2 < a) {
            outside += 1;
        }
    }
    outside
}

/// Every ordering of `xs`, in lexicographic order of positions.
fn permutations<T: Copy>(xs: &[T]) -> Vec<Vec<T>> {
    if xs.len() <= 1 {
        return vec![xs.to_vec()];
    }
    let mut all = Vec::new();
    for i in 0..xs.len() {
        let mut rest = xs.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x);
            all.push(p);
        }
    }
    all
}

fn detect(ctx: &DetectContext<'_>) -> DetectorOutput {
    let reject = |b: &str| DetectorOutput::rejected("jtag", b);
    let chs: Vec<&WinChannel> = ctx.channels.iter().copied().filter(|c| !c.edges.is_empty()).collect();
    if chs.len() < 3 || chs.len() > 4 {
        return reject("JTAG uses three or four lines (TCK, TMS, TDI, TDO)");
    }
    let mut clocks: Vec<&WinChannel> =
        chs.iter().copied().filter(|c| ctx.m(c.index).periodic.is_some()).collect();
    clocks.sort_by_key(|c| std::cmp::Reverse(c.edges.len()));
    let Some(&tck) = clocks.first() else { return reject("no line has clock pulses") };
    let Some(per) = &ctx.m(tck.index).periodic else { return reject("no line has clock pulses") };
    let others: Vec<&WinChannel> = chs.iter().copied().filter(|c| c.index != tck.index).collect();

    // A line that holds steady through every clock burst is a chip select: that is SPI, not JTAG.
    let p = per.period_samples;
    let mut bursts: Vec<(Sample, Sample)> = Vec::new();
    {
        let e = &tck.edges;
        let mut s = e[0];
        for i in 1..e.len() {
            if (e[i] - e[i - 1]) as f64 > 3.0 * p {
                bursts.push((s, e[i - 1]));
                s = e[i];
            }
        }
        bursts.push((s, e[e.len() - 1]));
    }
    let tol = 1f64.max((p / 2.0).ceil());
    let cs_like = others.iter().copied().find(|c| {
        let rest = ctx.m(c.index).rest_level;
        bursts.iter().all(|&(a, b)| {
            let inside = if (b - a) as f64 > 2.0 * tol {
                c.lower_bound(b as f64 - tol).saturating_sub(c.lower_bound(a as f64 + tol + 1.0))
            } else {
                0
            };
            inside == 0 && c.level_at((a + b) >> 1) != rest
        })
    });

    // every assignment of the other lines to TMS, TDI and TDO; the best explanation wins
    let mut best: Option<(f64, Vec<&WinChannel>, JtagStats, f64, f64)> = None;
    for perm in permutations(&others) {
        let (tms, tdi, tdo) = (perm[0], perm.get(1).copied(), perm.get(2).copied());
        let stats = trial(tck, tms, tdi, tdo, ctx.window.end);
        if stats.shifts == 0 {
            continue;
        }
        let tdo_clean = match tdo {
            Some(c) => 1.0 - tdo_outside_shift(c, &stats.shift_windows) as f64 / c.edges.len().max(1) as f64,
            None => 0.5,
        };
        let ir01 = if stats.ir_shifts > 0 { stats.ir_capture01 as f64 / stats.ir_shifts as f64 } else { 0.0 };
        let score = stats.shifts as f64 / 10.0 + stats.resets as f64 + tdo_clean + 2.0 * ir01;
        if best.as_ref().is_some_and(|b| score <= b.0) {
            continue;
        }
        best = Some((score, perm, stats, tdo_clean, ir01));
    }
    let Some((_, perm, stats, tdo_clean, ir01)) = best else {
        return reject("no line works as TMS: the TAP controller never reaches a shift state");
    };
    let (tms, tdi, tdo) = (perm[0], perm.get(1).copied(), perm.get(2).copied());
    let continuous = per.continuous;
    let confidence = clamp01(
        0.35 + if stats.resets > 0 { 0.2 } else { 0.0 }
            + (stats.shifts as f64 * 0.05).min(0.15)
            + 0.1 * tdo_clean
            + 0.2 * ir01,
    ) * if cs_like.is_some() { 0.4 } else { 1.0 }
        * if continuous { 0.5 } else { 1.0 };

    let mut channels: Vec<u32> = std::iter::once(tck).chain(perm.iter().copied()).map(|c| c.index).collect();
    channels.sort_unstable();
    let shifts = format!("{} shift{}", stats.shifts, format::s(stats.shifts));
    let notes = [
        ("role:tck", note(0.85, format!("Channel {} has the clock pulses.", tck.index))),
        (
            "role:tms",
            note(
                0.75,
                format!("Read as TMS, channel {} steps the TAP controller through {shifts}.", tms.index),
            ),
        ),
        (
            "role:tdi",
            note(
                0.6,
                match tdi {
                    Some(c) => format!(
                        "Channel {} changes outside the shift states too, as the probe's output can.",
                        c.index
                    ),
                    None => "No line left for TDI.".into(),
                },
            ),
        ),
        (
            "role:tdo",
            note(
                if ir01 > 0.5 { 0.9 } else { 0.6 },
                match tdo {
                    Some(c) if ir01 > 0.5 => format!(
                        "Channel {} shifts out 1 then 0 at the start of every instruction shift: the capture value every JTAG chip must send.",
                        c.index
                    ),
                    Some(c) => format!(
                        "Channel {} changes {} % of the time inside shift states, where the target drives it.",
                        c.index,
                        crate::num::round(tdo_clean * 100.0)
                    ),
                    None => "No line left for TDO.".into(),
                },
            ),
        ),
    ];
    let mut ev = vec![
        evidence(
            format!("Channel {} is a clock in bursts at {}.", tck.index, format::hz(ctx.sample_rate_hz / p)),
            true,
        ),
        match cs_like {
            Some(c) => evidence(
                format!(
                    "Channel {} holds steady through every clock burst like an SPI chip select, which JTAG does not have.",
                    c.index
                ),
                false,
            ),
            None => evidence("No line frames the bursts like an SPI chip select.", true),
        },
        evidence(
            format!(
                "Reading channel {} as TMS walks the TAP controller through {} Shift-IR / Shift-DR state{}.",
                tms.index,
                stats.shifts,
                format::s(stats.shifts)
            ),
            true,
        ),
        if stats.resets > 0 {
            evidence("TMS is held high for 5 or more clocks, the JTAG reset sequence.", true)
        } else {
            evidence("No reset sequence (5 clocks with TMS high) was seen.", false)
        },
    ];
    if continuous {
        ev.push(evidence("The clock never stops, which is unusual for JTAG.", false));
    }
    DetectorOutput {
        detections: vec![Detection {
            protocol_id: "jtag".into(),
            channels,
            roles: roles(&[
                ("tck", Some(tck.index)),
                ("tms", Some(tms.index)),
                ("tdi", tdi.map(|c| c.index)),
                ("tdo", tdo.map(|c| c.index)),
            ]),
            settings: Settings::new(),
            notes: notes.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            confidence,
            evidence: ev,
            warnings: vec![
                "The decoder assumes the TAP starts in Test-Logic-Reset. Decoding is reliable from the first reset sequence onwards."
                    .into(),
            ],
        }],
        rejections: vec![],
    }
}

// ------------------------------------------------------------------ protocol

pub struct Jtag {
    roles: Vec<RoleSpec>,
    info: ProtocolInfo,
}

impl Default for Jtag {
    fn default() -> Self {
        Jtag {
            roles: vec![
                RoleSpec::new("tck", "TCK (clock)", "TCK", "The clock."),
                RoleSpec::new("tms", "TMS (mode select)", "TMS", "Steps the TAP state machine."),
                RoleSpec::new("tdi", "TDI (data in)", "TDI", "From the probe to the chip.").optional(),
                RoleSpec::new("tdo", "TDO (data out)", "TDO", "From the chip to the probe.").optional(),
            ],
            info: ProtocolInfo {
                what: "Four-wire debug and programming port: TCK, TMS, TDI, TDO.".into(),
                confirm: "One clock, three data-like lines, and none of them stays low for a whole burst the way a chip select would.".into(),
                decoder_tips: "Assign TCK, TMS, TDI and TDO.".into(),
                lookalikes: "SPI has a chip select that frames every burst.".into(),
                symptoms: Vec::new(),
            },
        }
    }
}

impl Protocol for Jtag {
    fn id(&self) -> &str {
        "jtag"
    }
    fn name(&self) -> &str {
        "JTAG"
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
        let tck = setup.slot("tck").ok_or("Choose a channel for TCK.")?;
        debug_assert_eq!(tck, 0);
        let line =
            |role: &str| setup.slot(role).map(|slot| Line { slot, lane: setup.channels[slot].channel });
        let tms = line("tms").ok_or("Choose a channel for TMS.")?;
        let m = JtagMachine::new(tms, line("tdi"), line("tdo"), false);
        decoder(m, setup, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyse::{Batching, analyse_window, decode_store};
    use crate::buses::Bus;
    use crate::synth::*;
    use crate::window::Window;
    use logica_core::Level;
    use logica_core::measure::measure_channel;

    const RATE: f64 = 20e6;
    const HZ: f64 = 1e6;

    fn step(tms: u8) -> JtagStep {
        JtagStep { tms, tdi: None, tdo: None }
    }

    /// TCK, TMS, TDI, TDO lanes: reset, Run-Test/Idle, then the given steps.
    fn lanes(steps: &[JtagStep]) -> (Lane, Lane, Lane, Lane, f64) {
        let (mut tck, mut tms, mut tdi, mut tdo) =
            (Lane::low(RATE), Lane::high(RATE), Lane::low(RATE), Lane::high(RATE));
        let t = jtag_clock(&mut tck, &mut tms, &mut tdi, None, 0.0001, HZ, &[step(1); 5]);
        let t = jtag_clock(&mut tck, &mut tms, &mut tdi, None, t, HZ, &[step(0)]);
        let t = jtag_clock(&mut tck, &mut tms, &mut tdi, Some(&mut tdo), t, HZ, steps);
        (tck, tms, tdi, tdo, t)
    }

    fn capture(steps: &[JtagStep]) -> GenCapture {
        let (tck, tms, tdi, tdo, _) = lanes(steps);
        make_capture(
            "t",
            RATE,
            0.0005,
            vec![(0, "TCK", tck), (1, "TMS", tms), (2, "TDI", tdi), (3, "TDO", tdo)],
        )
    }

    /// Decodes in every batching (with snapshots too), asserting identical output.
    fn decode(g: &GenCapture, r: &Roles) -> DecodeResult {
        let (store, meta) = (g.store().unwrap(), g.meta());
        let reg = crate::registry();
        let whole = decode_store(&reg, &store, &meta, "jtag", r, &Settings::new(), Batching::Whole).unwrap();
        for b in
            [Batching::Edges(1), Batching::Edges(2), Batching::WithSnapshots(1), Batching::WithSnapshots(5)]
        {
            let other = decode_store(&reg, &store, &meta, "jtag", r, &Settings::new(), b).unwrap();
            assert_eq!(other, whole, "{b:?}");
        }
        whole
    }

    fn all_roles() -> Roles {
        roles(&[("tck", Some(0)), ("tms", Some(1)), ("tdi", Some(2)), ("tdo", Some(3))])
    }

    fn labels(r: &DecodeResult, row: u8) -> Vec<&str> {
        r.frames.iter().filter(|f| f.row == row).map(|f| f.label.as_str()).collect()
    }

    #[test]
    fn five_clocks_with_tms_high_reset_the_tap_from_any_state() {
        use Tap::*;
        let all = [
            TestLogicReset,
            RunTestIdle,
            SelectDr,
            CaptureDr,
            ShiftDr,
            Exit1Dr,
            PauseDr,
            Exit2Dr,
            UpdateDr,
            SelectIr,
            CaptureIr,
            ShiftIr,
            Exit1Ir,
            PauseIr,
            Exit2Ir,
            UpdateIr,
        ];
        for s in all {
            assert_eq!((0..5).fold(s, |s, _| s.next(true)), TestLogicReset, "{s:?}");
        }
        assert_eq!(RunTestIdle.next(true).next(true).next(false).next(false), ShiftIr);
        assert_eq!(Exit1Dr.next(false).next(true).next(false), ShiftDr);
        assert_eq!(
            (ShiftIr.register(), ShiftDr.register(), PauseDr.register()),
            (Some("IR"), Some("DR"), None)
        );
    }

    #[test]
    fn knowledge_texts_and_roles_are_complete() {
        let p = Jtag::default();
        let i = p.info();
        for text in [&i.what, &i.confirm, &i.decoder_tips, &i.lookalikes] {
            assert!(text.len() > 10, "{text}");
        }
        assert!(p.has_decoder());
        assert_eq!(
            p.roles().iter().map(|r| (r.key.as_str(), r.optional)).collect::<Vec<_>>(),
            [("tck", false), ("tms", false), ("tdi", true), ("tdo", true)]
        );
        assert!(p.roles().iter().all(|r| !r.noun.is_empty() && !r.label.is_empty()));
    }

    #[test]
    fn bits_are_shown_and_packed_least_significant_first() {
        assert_eq!(hex_lsb_first(&[]), "0");
        assert_eq!(hex_lsb_first(&[false, true, true, true]), "E");
        assert_eq!(hex_lsb_first(&[true, false, false, false, true]), "11");
        assert_eq!(pack_lsb_first(&[true; 9]), [0xff, 0x01]);
    }

    #[test]
    fn an_instruction_shift_is_one_packet_with_its_states_in_row_1() {
        let g = capture(&jtag_shift(true, &[0, 1, 1, 1], &[1, 0, 0, 0]));
        let r = decode(&g, &all_roles());
        assert_eq!(labels(&r, 0), ["IR in E out 1"]);
        assert_eq!(
            labels(&r, 1),
            [
                "Test-Logic-Reset",
                "Run-Test/Idle",
                "Select-DR",
                "Select-IR",
                "Capture-IR",
                "Shift-IR",
                "Exit1-IR",
                "Update-IR"
            ]
        );
        let shift = r.frames.iter().find(|f| f.row == 0).unwrap();
        assert_eq!((shift.lane, shift.kind), (2, FrameKind::Data));
        assert_eq!(
            shift.detail.as_deref(),
            Some(
                "4 bits shifted into the instruction register. TDI 0xE, TDO 0x1 (least significant bit first)"
            )
        );
        assert!(r.frames.iter().filter(|f| f.row == 1).all(|f| f.lane == 1 && f.kind == FrameKind::Info));
        assert_eq!(r.packets.len(), 1);
        let p = &r.packets[0];
        assert_eq!(p.title, "Shift IR (4 bits)");
        assert_eq!(
            (p.stream("TDI").unwrap().bytes.clone(), p.stream("TDO").unwrap().bytes.clone()),
            (vec![0x0e], vec![0x01])
        );
        // the shift frame spans Shift-IR exactly
        let state = r.frames.iter().find(|f| f.label == "Shift-IR").unwrap();
        assert_eq!((shift.start, shift.end), (state.start, state.end));
        assert!(r.problem_counts.is_empty());
    }

    #[test]
    fn a_pause_in_the_middle_of_a_shift_splits_it_in_two() {
        // Run-Test/Idle → Select-DR → Capture-DR → Shift-DR, 2 bits, Exit1, Pause, Exit2, Shift-DR, 2 bits
        let bit = |tms, v| JtagStep { tms, tdi: Some(v), tdo: Some(v) };
        let steps = [
            step(1),
            step(0),
            step(0),
            bit(0, 1),
            bit(1, 1),
            step(0),
            step(1),
            step(0),
            bit(0, 0),
            bit(1, 1),
            step(1),
            step(0),
        ];
        let r = decode(&capture(&steps), &all_roles());
        assert_eq!(labels(&r, 0), ["DR in 3 out 3", "DR in 2 out 2"]);
        assert!(labels(&r, 1).contains(&"Pause-DR") && labels(&r, 1).contains(&"Exit2-DR"));
        assert_eq!(r.packets.len(), 2);
    }

    #[test]
    fn without_tdi_and_tdo_the_shifts_and_bit_counts_are_still_shown() {
        let g = capture(&jtag_shift(false, &[0; 32], &[0; 32]));
        let r = decode(&g, &roles(&[("tck", Some(0)), ("tms", Some(1))]));
        assert_eq!(labels(&r, 0), ["DR"]);
        assert_eq!(r.packets[0].title, "Shift DR (32 bits)");
        assert!(r.packets[0].streams.is_empty());
        assert_eq!(r.frames.iter().find(|f| f.row == 0).unwrap().lane, 1);
        // with TDO only, the label has no gap where TDI would be
        let r = decode(
            &capture(&jtag_shift(true, &[0, 1], &[1, 0])),
            &roles(&[("tck", Some(0)), ("tms", Some(1)), ("tdo", Some(3))]),
        );
        assert_eq!(labels(&r, 0), ["IR out 1"]);
        assert_eq!(r.frames.iter().find(|f| f.row == 0).unwrap().lane, 3);
        assert_eq!(r.packets[0].title, "Shift IR (2 bits)");
    }

    #[test]
    fn a_capture_that_ends_mid_shift_emits_only_finished_states() {
        // Select-DR, Capture-DR, Shift-DR, then three bits and the capture ends
        let bit = |v| JtagStep { tms: 0, tdi: Some(v), tdo: Some(v) };
        let r = decode(&capture(&[step(1), step(0), step(0), bit(1), bit(0), bit(1)]), &all_roles());
        assert!(labels(&r, 0).is_empty());
        assert_eq!(labels(&r, 1).last(), Some(&"Capture-DR"));
        assert!(r.packets.is_empty());
    }

    #[test]
    fn missing_roles_are_setup_errors_and_no_edges_decode_to_nothing() {
        let g = capture(&[]);
        let (store, meta) = (g.store().unwrap(), g.meta());
        let reg = crate::registry();
        let e = decode_store(
            &reg,
            &store,
            &meta,
            "jtag",
            &roles(&[("tck", Some(0))]),
            &Settings::new(),
            Batching::Whole,
        );
        assert_eq!(e.unwrap_err(), "Choose a channel for TMS.");
        let quiet =
            make_capture("q", RATE, 0.001, vec![(0, "TCK", Lane::low(RATE)), (1, "TMS", Lane::high(RATE))]);
        let r = decode(&quiet, &roles(&[("tck", Some(0)), ("tms", Some(1))]));
        assert!(r.frames.is_empty() && r.packets.is_empty());
    }

    fn window(g: &GenCapture) -> Window {
        let channels = g
            .channels
            .iter()
            .enumerate()
            .map(|(slot, (index, name, lane))| WinChannel {
                index: *index,
                slot,
                name: name.clone(),
                initial: lane.initial,
                edges: lane.edges.clone(),
            })
            .collect();
        Window::from_channels(channels, 0, g.sample_count)
    }

    /// The JTAG detection (chosen or alternative) on the first bus of the full analysis.
    fn jtag_detection(g: &GenCapture) -> Option<Detection> {
        let a = analyse_window(&window(g), g.rate, &crate::registry());
        let bus = &a.buses[0];
        bus.chosen.iter().chain(&bus.alternatives).find(|d| d.protocol_id == "jtag").cloned()
    }

    /// The JTAG detector on all channels of `g` as one bus.
    fn detect_on(g: &GenCapture) -> DetectorOutput {
        let w = window(g);
        let measurements = w
            .channels
            .iter()
            .map(|c| (c.index, measure_channel(c.index, &c.edges, c.initial, w.start, w.end)))
            .collect();
        let bus = Bus {
            id: "b".into(),
            channels: w.channels.iter().map(|c| c.index).collect(),
            reason: String::new(),
        };
        let ctx = DetectContext {
            sample_rate_hz: g.rate,
            window: &w,
            bus: &bus,
            channels: w.channels.iter().collect(),
            measurements: &measurements,
        };
        detect(&ctx)
    }

    #[test]
    fn an_spi_bus_with_a_chip_select_is_not_confidently_jtag() {
        let rate = 10e6;
        let (mut clk, mut cs, mut mosi, mut miso) =
            (Lane::low(rate), Lane::high(rate), Lane::low(rate), Lane::low(rate));
        let mut t = 0.0001;
        for b in [0x9f_u8, 0x03, 0x0b, 0x05, 0x06, 0x02] {
            t = spi_frame(
                &mut clk,
                Some(&mut cs),
                Some(&mut mosi),
                Some(&mut miso),
                t,
                500e3,
                0,
                &[b, 0x00],
                &[0xff, b ^ 0x5a],
                true,
            ) + 0.00005;
        }
        let g = make_capture(
            "spi",
            rate,
            t + 0.0001,
            vec![(0, "CLK", clk), (1, "CS", cs), (2, "MOSI", mosi), (3, "MISO", miso)],
        );
        let out = detect_on(&g);
        // data lines can walk the TAP into shift states, but the chip select gives SPI away
        let d = &out.detections[0];
        assert!(d.confidence <= 0.4, "{}", d.confidence);
        assert!(d.evidence.iter().any(|e| {
            !e.supports
                && e.text
                    == "Channel 1 holds steady through every clock burst like an SPI chip select, which JTAG does not have."
        }));
    }

    #[test]
    fn too_few_lines_no_clock_or_no_shift_are_rejected_with_a_reason() {
        let because = |g: &GenCapture| {
            let out = detect_on(g);
            assert!(out.detections.is_empty());
            out.rejections[0].because.clone()
        };
        let (tck, tms, _, _, _) = lanes(&jtag_shift(false, &[0; 8], &[0; 8]));
        let g = make_capture("two", RATE, 0.0005, vec![(0, "TCK", tck), (1, "TMS", tms)]);
        assert_eq!(because(&g), "JTAG uses three or four lines (TCK, TMS, TDI, TDO)");
        // three lines with a few irregular edges each
        let mut irregular = Vec::new();
        for (i, name) in [(0u32, "A"), (1, "B"), (2, "C")] {
            let mut l = Lane::low(RATE);
            for (k, at) in [3.0, 17.0, 41.0, 44.0, 130.0].iter().enumerate() {
                l.set((at + f64::from(i)) * 1e-6, u8::from(k % 2 == 0));
            }
            irregular.push((i, name, l));
        }
        assert_eq!(because(&make_capture("noise", RATE, 0.0005, irregular)), "no line has clock pulses");
        // TMS 0, 1, 1, 1 over and over only circles Run-Test/Idle, Select-DR, Select-IR, Test-Logic-Reset
        let (mut tck, mut tms, mut x) = (Lane::low(RATE), Lane::high(RATE), Lane::high(RATE));
        let steps: Vec<JtagStep> = (0..40)
            .map(|i| {
                let v = u8::from(i % 4 != 0);
                JtagStep { tms: v, tdi: Some(v), tdo: None }
            })
            .collect();
        jtag_clock(&mut tck, &mut tms, &mut x, None, 0.0001, HZ, &steps);
        let g = make_capture("loop", RATE, 0.0005, vec![(0, "TCK", tck), (1, "TMS", tms), (2, "X", x)]);
        assert_eq!(because(&g), "no line works as TMS: the TAP controller never reaches a shift state");
    }

    #[test]
    fn the_example_is_detected_with_its_reasons() {
        let g = (crate::examples::example("jtag").unwrap().build)();
        let d = jtag_detection(&g).unwrap();
        assert_eq!(d.roles, all_roles());
        assert_eq!(d.channels, [0, 1, 2, 3]);
        assert!(d.confidence > 0.9, "{}", d.confidence);
        let texts: Vec<&str> = d.evidence.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts[0], "Channel 0 is a clock in bursts at 1 MHz.");
        assert_eq!(texts[1], "No line frames the bursts like an SPI chip select.");
        assert_eq!(
            texts[2],
            "Reading channel 1 as TMS walks the TAP controller through 2 Shift-IR / Shift-DR states."
        );
        assert!(d.notes["role:tdo"].reason.contains("shifts out 1 then 0"));
        assert!(d.evidence.iter().all(|e| e.supports));
    }

    #[test]
    fn tdo_edges_outside_every_shift_are_counted() {
        let tdo = WinChannel {
            index: 3,
            slot: 3,
            name: String::new(),
            initial: Level::High,
            edges: vec![5, 20, 23, 50, 99],
        };
        // 20 and 23 are inside or within 2 samples of [22, 40]; 5, 50 and 99 are outside
        assert_eq!(tdo_outside_shift(&tdo, &[(22, 40), (60, 70)]), 3);
        assert_eq!(tdo_outside_shift(&tdo, &[]), 5);
    }
}
