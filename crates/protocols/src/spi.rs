//! SPI: a clocked bus with a chip select, a clock and one or two data lines.

use serde::{Deserialize, Serialize};

use logica_core::{Level, Sample};

use crate::calc::{ClockEdge, spi_mode, spi_mode_parts};
use crate::format::{self, char_of, hex2};
use crate::num;
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo, symptoms};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;
use crate::window::WinChannel;

/// Clock edges learned from before the typical clock gap is fixed (no chip select, doc 06).
pub const TYPICAL_GAP_EDGES: u64 = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Line {
    slot: usize,
    lane: u32,
    label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpiStats {
    /// First word of each frame, per data line.
    pub first_words: Vec<Vec<u64>>,
    /// Sampling clock edges per frame.
    pub pulses_per_frame: Vec<usize>,
    pub leftovers: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpiMachine {
    /// Level after a sampling clock edge (high for rising).
    sample_level: Level,
    word: u32,
    msb: bool,
    cs_active: Level,
    clk_lane: u32,
    cs: Option<usize>,
    lines: Vec<Line>,
    start: Sample,
    stats_lists: bool,
    // with chip select
    cs_next: u64,
    window_start: Option<Sample>,
    // without chip select
    typical: Option<u64>,
    burst_first: u64,
    scan: u64,
    pub stats: SpiStats,
}

impl SpiMachine {
    #[allow(clippy::too_many_arguments)]
    fn new(
        s: &Settings,
        clk_lane: u32,
        cs: Option<(usize, Level)>,
        lines: Vec<Line>,
        start: Sample,
        stats_lists: bool,
    ) -> Self {
        let mode = s.text("mode").parse::<u8>().unwrap_or(0);
        let parts = spi_mode_parts(mode);
        let cs_active = if s.text("csPolarity") == "high" { Level::High } else { Level::Low };
        let word = num::to_u64(s.num("wordSize").round()).clamp(1, 32);
        let n = lines.len();
        SpiMachine {
            sample_level: if parts.sample_edge == ClockEdge::Rising { Level::High } else { Level::Low },
            word: u32::try_from(word).unwrap_or(8),
            msb: s.text("bitOrder") == "msb",
            cs_active,
            clk_lane,
            cs: cs.map(|(slot, _)| slot),
            lines,
            start,
            stats_lists,
            cs_next: 0,
            window_start: cs.and_then(|(_, level)| (level == cs_active).then_some(start)),
            typical: None,
            burst_first: 0,
            scan: 0,
            stats: SpiStats { first_words: vec![Vec::new(); n], ..Default::default() },
        }
    }

    /// Decodes one frame: the sampling clock edges `s` inside `[w0, w1]`.
    fn frame(&mut self, ed: &Edges, w0: Sample, w1: Sample, s: &[Sample], out: &mut Out) -> Res<()> {
        let word = self.word as usize;
        // read everything first
        let mut words: Vec<Vec<(u64, u32)>> = Vec::with_capacity(self.lines.len());
        for line in &self.lines {
            let mut list = Vec::new();
            let mut j = 0;
            while j + word <= s.len() {
                let (mut v, mut racy) = (0u64, 0u32);
                for b in 0..word {
                    let at = s[j + b];
                    let bit = u64::from(ed.level_at(line.slot, at)?.is_high());
                    // a data edge on the very sample of the clock edge: the bit could be read either way
                    if ed.edge_in(line.slot, at, at + 1)?.is_some() {
                        racy += 1;
                    }
                    v = if self.msb { v * 2 + bit } else { v + (bit << b) };
                }
                list.push((v, racy));
                j += word;
            }
            words.push(list);
        }
        // commit
        if self.stats_lists {
            self.stats.pulses_per_frame.push(s.len());
        }
        if s.is_empty() {
            return Ok(());
        }
        out.open_packet(w0, if self.cs.is_some() { "Chip-select frame" } else { "Clock burst" });
        let spacing = if s.len() > 1 { (s[s.len() - 1] - s[0]) as f64 / (s.len() - 1) as f64 } else { 4.0 };
        for (wi, j) in (0..).zip((0..).step_by(word).take_while(|j| j + word <= s.len())) {
            let start = if j == 0 {
                (w0 as f64).max(s[0] as f64 - spacing / 2.0)
            } else {
                (s[j - 1] + s[j]) as f64 / 2.0
            };
            let last = s[j + word - 1] as f64;
            let end = if j + word < s.len() {
                (last + s[j + word] as f64) / 2.0
            } else {
                (w1 as f64).min(last + spacing / 2.0)
            };
            for (li, line) in self.lines.iter().enumerate() {
                let (v, racy) = words[li][wi];
                if j == 0 && self.stats_lists {
                    self.stats.first_words[li].push(v);
                }
                let h = if self.word <= 8 {
                    hex2(v)
                } else {
                    format!("{v:0width$X}", width = self.word.div_ceil(4) as usize)
                };
                let mut f =
                    NewFrame::new(num::round(start), num::round(end), line.lane, FrameKind::Data, h.clone())
                        .value(v as f64)
                        .detail(format!(
                            "{} {h} = {v}{}",
                            line.label,
                            if self.word <= 8 {
                                format!(" = '{}'", char_of(crate::bytes::low_byte(v)))
                            } else {
                                String::new()
                            }
                        ));
                if racy > 0 {
                    f = f.problem(problem(
                        ProblemCode::Timing,
                        format!(
                            "{racy} bit{} changed on the same sample as the clock edge, so {} could be read either way. Sample faster to be sure.",
                            format::s(u64::from(racy)),
                            if racy == 1 { "it" } else { "they" }
                        ),
                    ));
                }
                out.add(f);
                if self.word <= 8 {
                    out.byte(&line.label, line.lane, crate::bytes::low_byte(v), num::round(end));
                } else {
                    for b in (0..self.word.div_ceil(8)).rev() {
                        out.byte(
                            &line.label,
                            line.lane,
                            crate::bytes::low_byte(v >> (8 * b)),
                            num::round(end),
                        );
                    }
                }
            }
        }
        let rest = s.len() % word;
        if rest > 0 {
            self.stats.leftovers += 1;
            out.add(
                NewFrame::new(
                    num::round(s[s.len() - rest] as f64 - spacing / 2.0),
                    num::round((w1 as f64).min(s[s.len() - 1] as f64 + spacing / 2.0)),
                    self.clk_lane,
                    FrameKind::Info,
                    format!("{rest} bits"),
                )
                .detail("Clock pulses left over at the end of the frame")
                .problem(problem(
                    ProblemCode::Incomplete,
                    format!(
                        "{} clock pulses in this frame, which is not a multiple of {}.",
                        s.len(),
                        self.word
                    ),
                )),
            );
        }
        out.close_packet(w1);
        Ok(())
    }

    /// Sampling clock edges with samples in `[w0, w1]`.
    fn sampling_edges(&self, ed: &Edges, w0: Sample, w1: Sample) -> Res<Vec<Sample>> {
        let mut k = ed.lower_bound(0, w0)?;
        let mut s = Vec::new();
        while let Some(e) = ed.edge(0, k)? {
            if e > w1 {
                break;
            }
            if ed.level_after(0, k) == self.sample_level {
                s.push(e);
            }
            k += 1;
        }
        Ok(s)
    }

    fn step_cs(&mut self, cs: usize, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let Some(w0) = self.window_start else {
            let Some(e) = ed.edge(cs, self.cs_next)? else { return Ok(false) };
            if ed.level_after(cs, self.cs_next) == self.cs_active {
                self.window_start = Some(e);
            }
            self.cs_next += 1;
            return Ok(true);
        };
        let closing = ed.edge(cs, self.cs_next)?;
        let w1 = closing.unwrap_or(ed.known_end());
        // every edge up to and including the closing sample must be in: a clock edge on the very
        // sample where chip select ends still belongs to the frame
        ed.level_at(0, w1)?;
        let s = self.sampling_edges(ed, w0, w1)?;
        self.frame(ed, w0, w1, &s, out)?;
        self.window_start = None;
        if closing.is_none() {
            return Ok(false);
        }
        self.cs_next += 1;
        ed.release_before(w1);
        Ok(true)
    }

    fn step_bursts(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let typical = match self.typical {
            Some(t) => t,
            None => {
                // learn the typical gap from the first clock edges
                let mut e = Vec::new();
                for i in 0..TYPICAL_GAP_EDGES {
                    match ed.edge(0, i)? {
                        Some(x) => e.push(x),
                        None => break,
                    }
                }
                if e.len() < 2 {
                    return Ok(false);
                }
                let mut gaps: Vec<u64> = e.windows(2).map(|w| w[1] - w[0]).collect();
                gaps.sort_unstable();
                let t = gaps[gaps.len() >> 2].max(1);
                self.typical = Some(t);
                t
            }
        };
        let limit = (8 * typical).max(4);
        let Some(first) = ed.edge(0, self.burst_first)? else { return Ok(false) };
        if self.scan < self.burst_first {
            self.scan = self.burst_first;
        }
        // find where the burst ends: the next gap longer than the limit
        let last = loop {
            let cur = ed.edge(0, self.scan)?.unwrap_or(first);
            match ed.edge(0, self.scan + 1)? {
                Some(next) if next - cur <= limit => self.scan += 1,
                _ => break cur,
            }
        };
        let w0 = first.saturating_sub(typical).max(self.start);
        let w1 = last + typical;
        let s = self.sampling_edges(ed, first, last)?;
        self.frame(ed, w0, w1, &s, out)?;
        self.burst_first = self.scan + 1;
        ed.release_index(0, self.burst_first);
        ed.release_before(last + 1);
        Ok(true)
    }
}

impl Machine for SpiMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        match self.cs {
            Some(cs) => run_steps(|| self.step_cs(cs, ed, out)),
            None => run_steps(|| self.step_bursts(ed, out)),
        }
    }
}

/// Notes about a whole decode, added at its end.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SpiDecode {
    m: SpiMachine,
    mode: u8,
}

impl Machine for SpiDecode {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        self.m.run(ed, out);
    }
    fn at_end(&mut self, ed: &Edges, out: &mut Out) {
        if self.m.cs.is_none() {
            out.note("No chip select: frames are split wherever the clock pauses.");
        }
        let cpol = spi_mode_parts(self.mode).cpol;
        if ed.count(0) > 0 {
            let rest = u8::from(ed.start_level(0).is_high());
            if rest != cpol {
                out.note(format!(
                    "The clock rests {}, but mode {} expects it to rest {}. Modes {} match this clock.",
                    if rest == 1 { "high" } else { "low" },
                    self.mode,
                    if cpol == 1 { "high" } else { "low" },
                    if rest == 1 { "2 and 3" } else { "0 and 1" }
                ));
            }
        }
    }
}

/// Runs the decoder over window channels (clock, chip select, data lines), keeping statistics.
fn spi_stats(
    clk: &WinChannel,
    cs: Option<&WinChannel>,
    data: &[&WinChannel],
    s: &Settings,
    w: (Sample, Sample),
) -> SpiStats {
    let mut chans = vec![(clk.initial, clk.edges.clone())];
    let cs_slot = cs.map(|c| {
        chans.push((c.initial, c.edges.clone()));
        (1, c.initial)
    });
    let lines = data
        .iter()
        .map(|d| {
            chans.push((d.initial, d.edges.clone()));
            Line { slot: chans.len() - 1, lane: d.index, label: "MOSI".into() }
        })
        .collect();
    let mut ed = Edges::complete(chans, w.1);
    let mut m = SpiMachine::new(s, clk.index, cs_slot, lines, w.0, true);
    let mut out = Out::new(false);
    m.run(&mut ed, &mut out);
    m.stats
}

fn is_idle_word(v: u64, bits: u32) -> bool {
    v == 0 || v == (1u64 << bits) - 1
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn detect(ctx: &DetectContext<'_>, specs: &[SettingSpec]) -> DetectorOutput {
    let reject = |b: &str| DetectorOutput::rejected("spi", b);
    let chs: Vec<&WinChannel> = ctx.channels.iter().copied().filter(|c| !c.edges.is_empty()).collect();
    if chs.len() < 2 {
        return reject(
            "SPI needs at least a clock and a data line busy together, and this group has one line",
        );
    }
    let mut periodic: Vec<&WinChannel> =
        chs.iter().copied().filter(|c| ctx.m(c.index).periodic.is_some()).collect();
    if periodic.is_empty() {
        return reject("no line in this group has evenly spaced clock pulses");
    }
    periodic.sort_by_key(|c| std::cmp::Reverse(c.edges.len()));
    let clk = periodic[0];
    let mc = ctx.m(clk.index);
    let Some(per) = &mc.periodic else { return reject("no clock") };
    let p = per.period_samples;
    let mut ev = Vec::new();
    let mut warnings = Vec::new();

    // clock bursts straight from the clock edges
    let mut bursts: Vec<(Sample, Sample)> = Vec::new();
    {
        let e = &clk.edges;
        let mut s = e[0];
        for i in 1..e.len() {
            if (e[i] - e[i - 1]) as f64 > 3.0 * p {
                bursts.push((s, e[i - 1]));
                s = e[i];
            }
        }
        bursts.push((s, e[e.len() - 1]));
    }

    // chip select: no edges inside any burst, and away from its rest level during every burst
    let mut cs: Option<&WinChannel> = None;
    let mut cs_fit = 0.0;
    for &c in &chs {
        if c.index == clk.index {
            continue;
        }
        let m = ctx.m(c.index);
        // allow chip select to change on the same sample as the first or last clock edge (or one
        // sample inside) when the clock is barely sampled
        let tol = 1f64.max((p / 2.0).ceil());
        let mut fit = 0;
        for &(b0, b1) in &bursts {
            let inside = if (b1 - b0) as f64 > 2.0 * tol {
                c.lower_bound(b1 as f64 - tol) - c.lower_bound(b0 as f64 + tol)
            } else {
                0
            };
            if inside == 0 && c.level_at((b0 + b1) >> 1) != m.rest_level {
                fit += 1;
            }
        }
        let f = f64::from(fit) / bursts.len() as f64;
        if f > cs_fit || (f == cs_fit && cs.is_some_and(|x| c.edges.len() < x.edges.len())) {
            cs_fit = f;
            cs = Some(c);
        }
    }
    if cs_fit < 0.9 {
        cs = None;
    }
    let data: Vec<&WinChannel> = chs
        .iter()
        .copied()
        .filter(|c| c.index != clk.index && cs.is_none_or(|x| x.index != c.index))
        .collect();
    if data.is_empty() {
        return reject("after finding the clock and chip select there is no line left to carry data");
    }

    let cpol = u8::from(mc.rest_level.is_high());
    // which clock edge do the data lines change on?
    let (mut after_rise, mut after_fall, mut setup_before_first) = (0u64, 0u64, 0u64);
    for d in &data {
        for &(b0, b1) in &bursts {
            let from = d.lower_bound(b0 as f64 - 2.0 * p);
            let to = d.lower_bound((b1 + 1) as f64);
            for &t in &d.edges[from..to.max(from)] {
                // last clock edge at or before t
                let k = clk.edges.partition_point(|&x| x <= t);
                if k == 0 || clk.edges[k - 1] < b0 {
                    setup_before_first += 1;
                    continue;
                }
                if clk.pulse_level(k - 1) == Level::High {
                    after_rise += 1;
                } else {
                    after_fall += 1;
                }
            }
        }
    }
    let changes = after_rise + after_fall;
    let (changes_on, cpha_conf) = if changes >= 2 {
        (
            if after_rise > after_fall { ClockEdge::Rising } else { ClockEdge::Falling },
            after_rise.max(after_fall) as f64 / changes as f64,
        )
    } else {
        // no data changes inside bursts: assume data is read on the leading edge (CPHA 0)
        (if cpol == 1 { ClockEdge::Rising } else { ClockEdge::Falling }, 0.3)
    };
    let mode = spi_mode(cpol, changes_on);

    let cs_polarity =
        if cs.is_some_and(|c| ctx.m(c.index).rest_level == Level::Low) { "high" } else { "low" };
    let mut settings = SettingSpec::with_defaults(
        specs,
        &crate::types::settings(&[
            ("mode", mode.mode.to_string().into()),
            ("csPolarity", cs_polarity.into()),
        ]),
    );
    let w = (ctx.window.start, ctx.window.end);
    let two: Vec<&WinChannel> = data.iter().copied().take(2).collect();
    let trial = spi_stats(clk, cs, &two, &settings, w);
    let counts: Vec<usize> = trial.pulses_per_frame.iter().copied().filter(|&n| n > 0).collect();
    let g = counts.iter().fold(0, |a, &b| gcd(a, b));
    let (word_size, word_conf) = if g % 8 == 0 && g > 0 {
        (8u32, 0.9)
    } else if (4..=32).contains(&g) {
        (u32::try_from(g).unwrap_or(8), 0.6)
    } else {
        (8, 0.3)
    };
    settings.insert("wordSize".into(), SettingValue::Number(f64::from(word_size)));
    let mult8 = counts.iter().filter(|&&n| n % 8 == 0).count() as f64 / counts.len().max(1) as f64;

    // which data line is MOSI?
    let first_idle_share = |ch: &WinChannel| -> f64 {
        let r = spi_stats(clk, cs, &[ch], &settings, w);
        let fw = &r.first_words[0];
        if fw.is_empty() {
            0.0
        } else {
            fw.iter().filter(|&&v| is_idle_word(v, word_size)).count() as f64 / fw.len() as f64
        }
    };
    let mut r =
        roles(&[("clk", Some(clk.index)), ("cs", cs.map(|c| c.index)), ("mosi", None), ("miso", None)]);
    let role_conf;
    let role_reason;
    if data.len() >= 2 {
        let (a, b) = (data[0], data[1]);
        let (ia, ib) = (first_idle_share(a), first_idle_share(b));
        let (mut mosi, mut miso) = (a, b);
        if ia > ib
            || (ia == ib
                && b.edges.first().copied().unwrap_or(u64::MAX)
                    < a.edges.first().copied().unwrap_or(u64::MAX))
        {
            (mosi, miso) = (b, a);
        }
        r.insert("mosi".into(), Some(mosi.index));
        r.insert("miso".into(), Some(miso.index));
        role_conf = if ia == ib { 0.5 } else { 0.85 };
        role_reason = format!(
            "Channel {} is busy in the first byte of each frame, so it carries the command (MOSI). Channel {} is quiet then and answers afterwards (MISO).",
            mosi.index, miso.index
        );
        if data.len() > 2 {
            warnings.push(format!(
                "There are {} data-like lines; only channels {} and {} are used.",
                data.len(),
                mosi.index,
                miso.index
            ));
        }
    } else {
        let d = data[0];
        let as_miso = first_idle_share(d) > 0.5;
        r.insert(if as_miso { "miso" } else { "mosi" }.into(), Some(d.index));
        role_conf = 0.35;
        role_reason = format!(
            "Channel {} is the only data line. {}, but with one line this is a guess. Check which pin it is wired to.",
            d.index,
            if as_miso {
                "Its first byte is usually idle, as a reply line (MISO) would be"
            } else {
                "It is busy from the first byte, as a command line (MOSI) would be"
            }
        );
    }

    let f = ctx.sample_rate_hz / p;
    let mut per_burst: Vec<usize> = Vec::new();
    for &n in &counts {
        if !per_burst.contains(&n) {
            per_burst.push(n);
        }
    }
    per_burst.truncate(4);
    let per_text = per_burst.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
    ev.push(evidence(
        format!(
            "Channel {} has bursts of evenly spaced pulses ({per_text}{} per frame) at {}, so it is the clock.",
            clk.index,
            if counts.len() > 4 { ", …" } else { "" },
            format::hz(f)
        ),
        true,
    ));
    if let Some(c) = cs {
        let active = if ctx.m(c.index).rest_level == Level::High { "low" } else { "high" };
        ev.push(evidence(
            format!(
                "Channel {} goes {active} before every clock burst and stays {active} until it ends, so it is chip select (active {active}).",
                c.index
            ),
            true,
        ));
    } else {
        ev.push(evidence(
            "No line frames the clock bursts the way a chip select would. SPI usually has one; JTAG and two-wire buses do not.",
            false,
        ));
    }
    if changes >= 2 {
        ev.push(evidence(
            format!(
                "The clock rests {} (CPOL {cpol}) and {} % of data changes follow a {} edge, so data is read on the {} edge: mode {}.",
                if cpol == 1 { "high" } else { "low" },
                (cpha_conf * 100.0).round(),
                changes_on.name(),
                mode.sample_edge.name(),
                mode.mode
            ),
            cpha_conf > 0.7,
        ));
    }
    if setup_before_first > 0 && mode.cpha == 0 {
        ev.push(evidence("The data lines already hold the first bit before the first clock edge, which is what CPHA 0 expects.", true));
    }
    ev.push(evidence(
        format!("{} % of frames have a multiple of 8 clock pulses.", (mult8 * 100.0).round()),
        mult8 > 0.8,
    ));
    if per.continuous {
        ev.push(evidence(
            "The clock never stops, which is unusual for SPI (I2S has a free-running clock).",
            false,
        ));
    }
    if p < 4.0 {
        warnings.push(format!(
            "The clock has only {} samples per period ({} at {}). The guide asks for at least 4; edges this close can land on the wrong sample, so check the bytes against what you expect and sample faster if you can.",
            format::fixed(p, 1),
            format::hz(f),
            format::hz(ctx.sample_rate_hz)
        ));
    }
    if cpha_conf < 0.7 && changes >= 2 {
        warnings.push(
            "The data changes are split between rising and falling edges, so the mode is uncertain. Try the other modes and see which gives sensible bytes."
                .into(),
        );
    }
    warnings.push(format!(
        "Mode {} reads on the same edge as mode {}, so it gives the same bytes. Only the clock rest level tells them apart.",
        3 - mode.mode,
        mode.mode
    ));
    if trial.leftovers > 0 {
        warnings.push(format!(
            "{} frame{} clock pulses left over after whole words.",
            trial.leftovers,
            if trial.leftovers == 1 { " has" } else { "s have" }
        ));
    }
    // Without a chip select, SPI is only one of several clock-plus-data possibilities.
    let continuous = per.continuous;
    let confidence = if cs.is_some() {
        clamp01(0.3 + 0.25 * cs_fit + 0.2 * mult8 + 0.15 * cpha_conf + if continuous { 0.0 } else { 0.1 })
    } else {
        clamp01(0.3 + 0.15 * mult8 + 0.1 * cpha_conf + if continuous { 0.0 } else { 0.05 })
    };
    let mut channels: Vec<u32> = vec![clk.index];
    channels.extend(cs.map(|c| c.index));
    channels.extend(data.iter().take(2).map(|d| d.index));
    channels.sort_unstable();
    let mut notes = vec![
        ("role:clk".to_string(), note(0.9, format!("Channel {} has the most regular pulses.", clk.index))),
        ("role:mosi".to_string(), note(role_conf, role_reason.clone())),
        ("role:miso".to_string(), note(role_conf, role_reason)),
        (
            "mode".to_string(),
            note(
                clamp01(cpha_conf * 0.9),
                format!(
                    "Clock rests {}; data changes on the {} edge, so it is read on the {} edge.",
                    if cpol == 1 { "high" } else { "low" },
                    changes_on.name(),
                    mode.sample_edge.name()
                ),
            ),
        ),
        (
            "bitOrder".to_string(),
            note(
                0.6,
                "Almost all SPI devices send the most significant bit first. The waveform cannot prove it.",
            ),
        ),
        (
            "wordSize".to_string(),
            note(
                word_conf,
                if g > 0 {
                    format!("The frames have {per_text} clock pulses.")
                } else {
                    "No complete frames to count.".into()
                },
            ),
        ),
        (
            "csPolarity".to_string(),
            note(
                if cs.is_some() { 0.9 } else { 0.3 },
                match cs {
                    Some(c) => format!(
                        "Chip select rests {} between frames.",
                        if ctx.m(c.index).rest_level == Level::High { "high" } else { "low" }
                    ),
                    None => "No chip select found.".into(),
                },
            ),
        ),
    ];
    if let Some(c) = cs {
        notes.push((
            "role:cs".to_string(),
            note(
                cs_fit,
                format!(
                    "Channel {} holds steady through every clock burst and has the fewest edges.",
                    c.index
                ),
            ),
        ));
    }
    DetectorOutput {
        detections: vec![Detection {
            protocol_id: "spi".into(),
            channels,
            roles: r,
            settings,
            notes: notes.into_iter().collect(),
            confidence,
            evidence: ev,
            warnings,
        }],
        rejections: vec![],
    }
}

pub struct Spi {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl Default for Spi {
    fn default() -> Self {
        Spi {
            roles: vec![
                RoleSpec::new("clk", "CLK (clock)", "the clock (CLK)", "Bursts of evenly spaced pulses."),
                RoleSpec::new("cs", "CS (chip select)", "chip select (CS)", "Low for the whole frame. Fewest edges.").optional(),
                RoleSpec::new("mosi", "MOSI (master out)", "MOSI", "Carries the command; busy from the first byte.").optional(),
                RoleSpec::new("miso", "MISO (master in)", "MISO", "Carries the reply; usually idle in the first byte.").optional(),
            ],
            settings: vec![
                SettingSpec::choice(
                    "mode",
                    "Mode",
                    "0",
                    &[
                        ("0", "Mode 0: rests low, reads rising"),
                        ("1", "Mode 1: rests low, reads falling"),
                        ("2", "Mode 2: rests high, reads falling"),
                        ("3", "Mode 3: rests high, reads rising"),
                    ],
                )
                .help("Mode = clock rest level (CPOL) and which clock edge reads the data (CPHA)."),
                SettingSpec::choice("bitOrder", "Bit order", "msb", &[("msb", "Most significant bit first"), ("lsb", "Least significant bit first")]),
                SettingSpec::number("wordSize", "Bits per word", 8.0).range(1.0, 32.0, None, &[8.0, 16.0, 24.0, 32.0]),
                SettingSpec::choice("csPolarity", "Chip select", "low", &[("low", "Active low (usual)"), ("high", "Active high")]),
            ],
            info: ProtocolInfo {
                what: "Clocked bus with a chip select, a clock, and one or two data lines.".into(),
                confirm: "Clock pulses come in multiples of 8 while chip select is low. The line that is active first is MOSI; the reply on MISO follows.".into(),
                decoder_tips: "Assign CLK, CS, MOSI and MISO. Then set the mode from the clock rest level and the sampling edge. Bit order MSB first, 8 bits per word.".into(),
                lookalikes: "JTAG has four lines too, but none of them frames the bursts like a chip select. I2S has a clock that never stops.".into(),
                symptoms: symptoms(&[
                    ("Values look shifted by one bit (9F reads as 4F, 06 as 03)", "Sampling on the edge where data changes", "Change CPHA."),
                    ("Values look bit-reversed (01 reads as 80)", "Wrong bit order", "Switch between MSB first and LSB first."),
                    ("MISO is always FF or always 00", "Slave not selected, not responding, or MISO not connected", "Check chip select and wiring."),
                    ("Clock count in a frame is not a multiple of 8", "Glitch on the clock, or sample rate too low", "Raise the sample rate to at least 4 times the clock."),
                    ("Frames split in odd places", "Chip select on the wrong channel, or wrong polarity", "Chip select is the line with the fewest edges; it is normally active low."),
                    ("Reply appears before the command", "MOSI and MISO are swapped", "Swap the two data channels."),
                ]),
            },
        }
    }
}

impl Protocol for Spi {
    fn id(&self) -> &str {
        "spi"
    }
    fn name(&self) -> &str {
        "SPI"
    }
    fn group(&self) -> &str {
        "SPI"
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
            "mode {}, {}-bit words, {} first",
            s.text("mode"),
            s.num("wordSize"),
            if s.text("bitOrder") == "msb" { "MSB" } else { "LSB" }
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
        let clk = setup.slot("clk").ok_or("Choose a channel for the clock (CLK).")?;
        debug_assert_eq!(clk, 0);
        let mut lines = Vec::new();
        for (role, label) in [("mosi", "MOSI"), ("miso", "MISO")] {
            if let Some(slot) = setup.slot(role) {
                lines.push(Line { slot, lane: setup.channels[slot].channel, label: label.into() });
            }
        }
        if lines.is_empty() {
            return Err("Choose a channel for MOSI or MISO (or both).".into());
        }
        let cs = setup.slot("cs").map(|s| (s, setup.start_levels[s]));
        let m = SpiMachine::new(&setup.settings, setup.channels[clk].channel, cs, lines, setup.start, false);
        let mode = setup.settings.text("mode").parse().unwrap_or(0);
        decoder(SpiDecode { m, mode }, setup, snapshot)
    }
}
