//! Pulse-code protocols (doc 06): one line where bits are told apart by how long a pulse lasts.
//! NEC infrared, DHT sensors, WS2812 LEDs and 1-Wire are described this way, as data; a custom
//! protocol is the same structure written as JSON. All times are in microseconds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use logica_core::{Level, Sample};

use crate::format::{self, hex2};
use crate::num;
use crate::protocol::{DetectContext, DetectorOutput, Origin, Protocol, ProtocolInfo, Symptom};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LevelName {
    High,
    Low,
}

impl LevelName {
    pub fn level(self) -> Level {
        match self {
            LevelName::High => Level::High,
            LevelName::Low => Level::Low,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            LevelName::High => "high",
            LevelName::Low => "low",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentSpec {
    pub level: LevelName,
    pub min_us: f64,
    pub max_us: f64,
    /// Shown under the waveform, such as "leader" or "reset".
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    First,
    Second,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitSpec {
    /// Level of the first of the two segments that make up one bit.
    pub first: LevelName,
    /// Which segment's length carries the value.
    pub encoding: Encoding,
    pub zero: [f64; 2],
    pub one: [f64; 2],
    /// Allowed length of the segment that does not carry the value.
    pub other: [f64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitCount {
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PulseFrameSpec {
    pub name: String,
    pub preamble: Vec<SegmentSpec>,
    pub bit: Option<BitSpec>,
    /// How many bits a complete frame has.
    pub bits: Option<BitCount>,
    pub trailer: Option<Vec<SegmentSpec>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Signed {
    Twos,
    SignMagnitude,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub bits: u32,
    /// Multiply the raw value, for example 0.1 for tenths.
    pub scale: Option<f64>,
    pub unit: Option<String>,
    pub signed: Option<Signed>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CheckSpec {
    Sum8 { bytes: Vec<usize>, equals: usize, message: Option<String> },
    Inverse { a: usize, b: usize, message: Option<String> },
}

/// Knowledge texts of a data-defined protocol. Learn-tab fields (fingerprint, sketch, timing)
/// are accepted and ignored (decision D12).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoSpec {
    pub what: String,
    pub confirm: String,
    pub decoder_tips: String,
    pub lookalikes: String,
    #[serde(default)]
    pub symptoms: Vec<Symptom>,
}

impl InfoSpec {
    pub fn to_info(&self) -> ProtocolInfo {
        ProtocolInfo {
            what: self.what.clone(),
            confirm: self.confirm.clone(),
            decoder_tips: self.decoder_tips.clone(),
            lookalikes: self.lookalikes.clone(),
            symptoms: self.symptoms.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BitOrder {
    Msb,
    Lsb,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PulseCodeConfig {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    pub idle: LevelName,
    /// Quiet time required before a frame starts.
    pub min_idle_us: Option<f64>,
    /// Alternatives tried in order wherever the line leaves its idle level.
    pub frames: Vec<PulseFrameSpec>,
    pub bit_order: BitOrder,
    /// Bits per decoded word (default 8).
    pub word_bits: Option<u32>,
    pub fields: Option<Vec<FieldSpec>>,
    /// Repeat the field list over the whole frame (G, R, B for every LED).
    #[serde(default)]
    pub repeat_fields: bool,
    pub checks: Option<Vec<CheckSpec>>,
    /// Names for byte values (hex keys) at the given byte positions, such as 1-Wire commands.
    pub byte_labels: Option<BTreeMap<String, String>>,
    pub label_positions: Option<Vec<usize>>,
    pub info: InfoSpec,
}

// ------------------------------------------------------------------ matching

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Seg {
    start: Sample,
    end: Sample,
    label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Bit {
    start: Sample,
    end: Sample,
    value: u8,
}

#[derive(Clone, Debug)]
struct Matched {
    frame: usize,
    /// Index of the first pulse of the frame and of the pulse after it.
    first: i64,
    next: i64,
    start: Sample,
    end: Sample,
    segs: Vec<Seg>,
    bits: Vec<Bit>,
    complete: bool,
}

/// Pulses of one channel: pulse i runs from edge i to edge i + 1 (the last one to the end of the
/// capture); pulse -1 is the stretch before the first edge.
struct Pulses<'a> {
    ed: &'a Edges,
    origin: Sample,
}

impl Pulses<'_> {
    /// Whether pulse `i` exists.
    fn exists(&self, i: i64) -> Res<bool> {
        Ok(i < 0 || self.ed.edge(0, i as u64)?.is_some())
    }
    fn start(&self, i: i64) -> Res<Sample> {
        if i < 0 { Ok(self.origin) } else { Ok(self.ed.edge(0, i as u64)?.unwrap_or(self.ed.known_end())) }
    }
    fn end(&self, i: i64) -> Res<Sample> {
        Ok(self.ed.edge(0, (i + 1) as u64)?.unwrap_or(self.ed.known_end()))
    }
    /// The last pulse runs to the end of the capture.
    fn open(&self, i: i64) -> Res<bool> {
        Ok(self.ed.edge(0, (i + 1) as u64)?.is_none())
    }
    fn len(&self, i: i64) -> Res<f64> {
        Ok((self.end(i)? - self.start(i)?) as f64)
    }
    fn level(&self, i: i64) -> Level {
        if i < 0 { self.ed.start_level(0) } else { self.ed.level_after(0, i as u64) }
    }
}

fn in_range(len: f64, r: [f64; 2], per_us: f64, tol: f64) -> bool {
    len >= r[0] * per_us * (1.0 - tol) - 1.0 && len <= r[1] * per_us * (1.0 + tol) + 1.0
}

fn match_at(
    cfg: &PulseCodeConfig,
    fi: usize,
    p: &Pulses<'_>,
    i: i64,
    per_us: f64,
    tol: f64,
) -> Res<Option<Matched>> {
    let frame = &cfg.frames[fi];
    let idle = cfg.idle.level();
    let mut k = i;
    let mut segs = Vec::new();
    for s in &frame.preamble {
        if !p.exists(k)? || p.level(k) != s.level.level() {
            return Ok(None);
        }
        let open = p.open(k)?;
        let len = p.len(k)?;
        if !in_range(len, [s.min_us, s.max_us], per_us, tol) && !(open && len >= s.min_us * per_us) {
            return Ok(None);
        }
        segs.push(Seg {
            start: p.start(k)?,
            end: p.end(k)?,
            label: s.label.clone().unwrap_or_else(|| frame.name.clone()),
        });
        k += 1;
    }
    let mut bits = Vec::new();
    let max = frame.bits.map_or(0.0, |b| b.max);
    let min = frame.bits.map_or(0.0, |b| b.min);
    if let Some(b) = &frame.bit {
        let pick = |len: f64| -> Option<u8> {
            let z = in_range(len, b.zero, per_us, tol);
            let o = in_range(len, b.one, per_us, tol);
            if z && o {
                let cz = (b.zero[0] + b.zero[1]) / 2.0 * per_us;
                let co = (b.one[0] + b.one[1]) / 2.0 * per_us;
                return Some(if (len - cz).abs() <= (len - co).abs() { 0 } else { 1 });
            }
            if z {
                Some(0)
            } else if o {
                Some(1)
            } else {
                None
            }
        };
        while (bits.len() as f64) < max && p.exists(k)? {
            if p.level(k) != b.first.level() {
                break;
            }
            let a_len = p.len(k)?;
            let has_b = p.exists(k + 1)?;
            let b_len = if has_b { p.len(k + 1)? } else { f64::INFINITY };
            match b.encoding {
                Encoding::First => {
                    let Some(v) = pick(a_len) else { break };
                    let b_open = has_b && p.open(k + 1)?;
                    let other_ok = has_b && in_range(b_len, b.other, per_us, tol) && !b_open;
                    // the last bit's second half merges into the idle that follows (possibly to
                    // the end of the capture)
                    let ends_frame = !has_b
                        || (!b.first.level() == idle
                            && (b_open || b_len > b.other[1] * per_us * (1.0 + tol)));
                    if !other_ok && !ends_frame {
                        break;
                    }
                    if ends_frame {
                        bits.push(Bit { start: p.start(k)?, end: p.end(k)?, value: v });
                        k += 1;
                        break;
                    }
                    bits.push(Bit { start: p.start(k)?, end: p.end(k + 1)?, value: v });
                    k += 2;
                }
                Encoding::Second => {
                    if !in_range(a_len, b.other, per_us, tol) || !has_b {
                        break;
                    }
                    let Some(v) = pick(b_len) else { break };
                    if p.open(k + 1)? {
                        break;
                    }
                    bits.push(Bit { start: p.start(k)?, end: p.end(k + 1)?, value: v });
                    k += 2;
                }
            }
        }
    }
    let mut complete = bits.len() as f64 >= min;
    if complete && let Some(trailer) = &frame.trailer {
        for s in trailer {
            if !p.exists(k)?
                || p.level(k) != s.level.level()
                || !in_range(p.len(k)?, [s.min_us, s.max_us], per_us, tol)
            {
                complete = false;
                break;
            }
            segs.push(Seg {
                start: p.start(k)?,
                end: p.end(k)?,
                label: s.label.clone().unwrap_or_else(|| "end".into()),
            });
            k += 1;
        }
    }
    if segs.is_empty() && bits.is_empty() {
        return Ok(None);
    }
    let end = segs.last().map_or(0, |s| s.end).max(bits.last().map_or(0, |b| b.end));
    Ok(Some(Matched {
        frame: fi,
        first: i,
        next: k.max(i + 1),
        start: p.start(i)?,
        end,
        segs,
        bits,
        complete,
    }))
}

fn words_of(bits: &[u8], size: usize, msb: bool) -> Vec<u64> {
    bits.chunks_exact(size)
        .map(|c| c.iter().enumerate().map(|(j, &b)| u64::from(b) << if msb { size - 1 - j } else { j }).sum())
        .collect()
}

fn field_value(f: &FieldSpec, raw: u64) -> String {
    let half = 1u64 << (f.bits - 1).min(62);
    let v: f64 = match f.signed {
        Some(Signed::Twos) if raw >= half => raw as f64 - (half as f64) * 2.0,
        Some(Signed::SignMagnitude) if raw >= half => -((raw - half) as f64),
        _ => raw as f64,
    };
    let scaled = match f.scale {
        Some(s) => {
            let t = format::fixed(v * s, 6);
            t.parse::<f64>().map_or(t, |x| format!("{x}"))
        }
        None => format!("{v}"),
    };
    format!("{scaled}{}", f.unit.as_ref().map_or(String::new(), |u| format!(" {u}")))
}

/// Statistics of a run, for detection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PulseStats {
    /// Complete frames that have bits or a preamble.
    pub complete: u64,
    pub complete_with_bits: u64,
    /// Pulses covered by complete frames.
    pub covered: u64,
    pub checksum_failures: u64,
    /// The frame spec of the first complete frame with bits.
    pub first_with_bits: Option<usize>,
    /// Every complete frame with bits has a fixed bit count.
    pub fixed_bits: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PulseMachine {
    cfg: PulseCodeConfig,
    per_us: f64,
    tol: f64,
    msb: bool,
    lane: u32,
    origin: Sample,
    /// Next pulse to look at.
    i: i64,
    stats: PulseStats,
}

impl PulseMachine {
    fn new(cfg: PulseCodeConfig, rate: f64, s: &Settings, lane: u32, origin: Sample) -> Self {
        PulseMachine {
            per_us: rate / 1e6,
            tol: s.num("tolerance") / 100.0,
            msb: s.text("bitOrder") == "msb",
            cfg,
            lane,
            origin,
            i: 0,
            stats: PulseStats { fixed_bits: true, ..Default::default() },
        }
    }

    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let p = Pulses { ed, origin: self.origin };
        if !p.exists(self.i)? {
            return Ok(false);
        }
        let idle = self.cfg.idle.level();
        let i = self.i;
        if p.level(i) == idle {
            self.i += 1;
            return Ok(true);
        }
        if let Some(min_idle) = self.cfg.min_idle_us
            && i > 0
            && p.len(i - 1)? < min_idle * self.per_us * (1.0 - self.tol)
        {
            self.i += 1;
            return Ok(true);
        }
        let mut best: Option<Matched> = None;
        for fi in 0..self.cfg.frames.len() {
            let Some(m) = match_at(&self.cfg, fi, &p, i, self.per_us, self.tol)? else { continue };
            let usable = m.complete || (!m.segs.is_empty() && !self.cfg.frames[fi].preamble.is_empty());
            let better = best
                .as_ref()
                .is_none_or(|b| (m.complete && !b.complete) || (m.complete == b.complete && m.next > b.next));
            if usable && better {
                best = Some(m);
            }
        }
        let Some(m) = best else {
            self.i += 1;
            return Ok(true);
        };
        self.emit(&m, out);
        self.i = m.next;
        ed.release_index(0, u64::try_from(m.next - 1).unwrap_or(0));
        Ok(true)
    }

    fn emit(&mut self, m: &Matched, out: &mut Out) {
        let cfg = &self.cfg;
        let frame = &cfg.frames[m.frame];
        let lane = self.lane;
        let rate = self.per_us * 1e6;
        if m.complete && (!m.bits.is_empty() || !frame.preamble.is_empty()) {
            self.stats.complete += 1;
            self.stats.covered += u64::try_from(m.next - m.first).unwrap_or(0);
            if !m.bits.is_empty() {
                self.stats.complete_with_bits += 1;
                self.stats.first_with_bits.get_or_insert(m.frame);
                if !frame.bits.is_some_and(|b| b.min == b.max) {
                    self.stats.fixed_bits = false;
                }
            }
        }
        out.open_packet(m.start, frame.name.clone());
        for sg in &m.segs {
            let kind = match sg.label.as_str() {
                "reset" => FrameKind::Reset,
                "presence" => FrameKind::Presence,
                _ => FrameKind::Start,
            };
            out.add(NewFrame::new(sg.start, sg.end, lane, kind, sg.label.clone()).detail(format!(
                "{}: {}",
                sg.label,
                format::time((sg.end - sg.start) as f64 / rate)
            )));
        }
        for b in &m.bits {
            out.add(
                NewFrame::new(b.start, b.end, lane, FrameKind::Info, b.value.to_string())
                    .detail(format!("bit {}", b.value))
                    .row(1),
            );
        }
        let values: Vec<u8> = m.bits.iter().map(|b| b.value).collect();
        let word_bits = cfg.word_bits.unwrap_or(8).max(1) as usize;
        let words = words_of(&values, word_bits, self.msb);
        let mut problems = Vec::new();
        if !m.complete {
            let want = frame.bits.map_or("its ending".to_string(), |b| format!("{} bits", b.min));
            problems.push(problem(
                ProblemCode::Incomplete,
                format!(
                    "The {} stopped after {} bits; it should have {want}.",
                    frame.name.to_lowercase(),
                    m.bits.len()
                ),
            ));
        }
        let bytes: Vec<u64> = if word_bits == 8 { words.clone() } else { words_of(&values, 8, self.msb) };
        if m.complete {
            for c in cfg.checks.iter().flatten() {
                match c {
                    CheckSpec::Sum8 { bytes: idx, equals, message } => {
                        if idx.iter().any(|&k| k >= bytes.len()) || *equals >= bytes.len() {
                            continue;
                        }
                        let sum = idx.iter().map(|&k| bytes[k]).sum::<u64>() & 0xff;
                        if sum != bytes[*equals] {
                            problems.push(problem(
                                ProblemCode::Checksum,
                                message.clone().unwrap_or_else(|| {
                                    format!(
                                        "Checksum {} does not match the sum {}.",
                                        hex2(bytes[*equals]),
                                        hex2(sum)
                                    )
                                }),
                            ));
                        }
                    }
                    CheckSpec::Inverse { a, b, message } => {
                        let (Some(&x), Some(&y)) = (bytes.get(*a), bytes.get(*b)) else { continue };
                        if x ^ 0xff != y {
                            problems.push(problem(
                                ProblemCode::Checksum,
                                message.clone().unwrap_or_else(|| {
                                    format!("Byte {} should be the inverse of byte {}.", b + 1, a + 1)
                                }),
                            ));
                        }
                    }
                }
            }
        }
        if problems.iter().any(|p| p.code == ProblemCode::Checksum) {
            self.stats.checksum_failures +=
                problems.iter().filter(|p| p.code == ProblemCode::Checksum).count() as u64;
        }
        // annotations: fields if defined, else words
        let bit_start = |k: usize| m.bits.get(k).map_or(m.end, |b| b.start);
        let bit_end = |k: usize| m.bits.get(k).map_or(m.end, |b| b.end);
        match cfg.fields.as_ref().filter(|f| !f.is_empty()) {
            Some(fields) => {
                let mut k = 0usize;
                let mut round = 0;
                'outer: loop {
                    for f in fields {
                        let n = f.bits as usize;
                        if k + n > values.len() {
                            break 'outer;
                        }
                        let raw = words_of(&values[k..k + n], n, self.msb)[0];
                        let name = if cfg.repeat_fields {
                            format!("{}{}", f.name, round + 1)
                        } else {
                            f.name.clone()
                        };
                        let shown = if cfg.repeat_fields { &f.name } else { &name };
                        let label =
                            format!("{shown} {}", if f.bits <= 8 { hex2(raw) } else { format!("{raw:X}") });
                        out.add(
                            NewFrame::new(bit_start(k), bit_end(k + n - 1), lane, FrameKind::Field, label)
                                .value(raw as f64)
                                .detail(format!("{name} = {} (raw {raw})", field_value(f, raw))),
                        );
                        k += n;
                    }
                    round += 1;
                    if !(cfg.repeat_fields && k < values.len()) {
                        break;
                    }
                }
            }
            None => {
                for (n, &w) in words.iter().enumerate() {
                    let name = cfg
                        .label_positions
                        .as_ref()
                        .is_some_and(|l| l.contains(&n))
                        .then(|| cfg.byte_labels.as_ref().and_then(|b| b.get(&hex2(w))))
                        .flatten();
                    let (kind, label, detail) = match name {
                        Some(nm) => {
                            (FrameKind::Command, format!("{} {nm}", hex2(w)), format!("{}: {nm}", hex2(w)))
                        }
                        None => (FrameKind::Data, hex2(w), format!("{} = {w}", hex2(w))),
                    };
                    out.add(
                        NewFrame::new(
                            bit_start(n * word_bits),
                            bit_end(n * word_bits + word_bits - 1),
                            lane,
                            kind,
                            label,
                        )
                        .value(w as f64)
                        .detail(detail),
                    );
                }
            }
        }
        if !problems.is_empty() {
            let detail = problems.iter().map(|p| p.message.as_str()).collect::<Vec<_>>().join(" ");
            out.add(
                NewFrame::new(m.start, m.end, lane, FrameKind::Info, "!").detail(detail).problems(problems),
            );
        }
        for &b in &bytes {
            out.byte(&cfg.name, lane, crate::bytes::low_byte(b), m.end);
        }
        out.close_packet(m.end);
    }
}

impl Machine for PulseMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }
}

fn describe_frame(f: &PulseFrameSpec) -> String {
    let segs: Vec<String> = f
        .preamble
        .iter()
        .map(|s| format!("{} {}", format::time((s.min_us + s.max_us) / 2.0 * 1e-6), s.level.name()))
        .collect();
    let bits = match (&f.bit, f.bits) {
        (Some(b), Some(n)) => {
            let coded = match b.encoding {
                Encoding::First => b.first.name(),
                Encoding::Second => {
                    if b.first == LevelName::High {
                        "low"
                    } else {
                        "high"
                    }
                }
            };
            let short_is_zero = b.zero[0] < b.one[0];
            format!(
                "{} bits coded in the {coded} time (short = {}, long = {})",
                if n.min == n.max { format!("{}", n.min) } else { format!("{} or more", n.min) },
                u8::from(!short_is_zero),
                u8::from(short_is_zero)
            )
        }
        _ => String::new(),
    };
    let mut parts = Vec::new();
    if !segs.is_empty() {
        parts.push(segs.join(", then "));
    }
    if !bits.is_empty() {
        parts.push(bits);
    }
    parts.join(", then ")
}

/// A pulse-code protocol from its config.
pub struct PulseCode {
    cfg: PulseCodeConfig,
    origin: Origin,
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl PulseCode {
    pub fn new(cfg: PulseCodeConfig, origin: Origin) -> Self {
        let order = match cfg.bit_order {
            BitOrder::Msb => "msb",
            BitOrder::Lsb => "lsb",
        };
        PulseCode {
            roles: vec![RoleSpec::new(
                "data",
                "Data line",
                "the data line",
                "The single line carrying the pulses.",
            )],
            settings: vec![
                SettingSpec::choice(
                    "bitOrder",
                    "Bit order",
                    order,
                    &[("msb", "Most significant bit first"), ("lsb", "Least significant bit first")],
                ),
                SettingSpec::number("tolerance", "Extra timing tolerance", 0.0)
                    .range(0.0, 100.0, Some("%"), &[])
                    .help("Widen every timing window by this much, for sloppy transmitters."),
            ],
            info: cfg.info.to_info(),
            cfg,
            origin,
        }
    }

    pub fn config(&self) -> &PulseCodeConfig {
        &self.cfg
    }

    fn run_window(
        &self,
        ch: &crate::window::WinChannel,
        w: (Sample, Sample),
        rate: f64,
        s: &Settings,
    ) -> PulseStats {
        let mut ed = Edges::complete(vec![(ch.initial, ch.edges.clone())], w.1);
        let mut m = PulseMachine::new(self.cfg.clone(), rate, s, ch.index, w.0);
        let mut out = Out::new(false);
        m.run(&mut ed, &mut out);
        m.stats
    }
}

impl Protocol for PulseCode {
    fn id(&self) -> &str {
        &self.cfg.id
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn group(&self) -> &str {
        self.cfg.group.as_deref().unwrap_or("Single-wire")
    }
    fn origin(&self) -> Origin {
        self.origin
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
        let tol = s.num("tolerance");
        format!(
            "{} first{}",
            if s.text("bitOrder") == "msb" { "MSB" } else { "LSB" },
            if tol != 0.0 { format!(", {} % extra tolerance", format::sig(tol, 6)) } else { String::new() }
        )
    }
    fn detect(&self, ctx: &DetectContext<'_>) -> Option<DetectorOutput> {
        let cfg = &self.cfg;
        let idle = cfg.idle.level();
        let mut out = DetectorOutput::default();
        for ch in &ctx.channels {
            let m = ctx.m(ch.index);
            if m.edge_count < 4 {
                continue;
            }
            if m.rest_level != idle {
                out.rejections.push(Rejection {
                    protocol_id: cfg.id.clone(),
                    because: format!(
                        "{} rests {}, and channel {} rests {}",
                        cfg.name,
                        cfg.idle.name(),
                        ch.index,
                        if m.rest_level == Level::High { "high" } else { "low" }
                    ),
                });
                continue;
            }
            let s = SettingSpec::with_defaults(&self.settings, &Settings::new());
            let st = self.run_window(ch, (ctx.window.start, ctx.window.end), ctx.sample_rate_hz, &s);
            let Some(first) = st.first_with_bits else {
                out.rejections.push(Rejection {
                    protocol_id: cfg.id.clone(),
                    because: format!(
                        "channel {} never shows the {} pattern ({})",
                        ch.index,
                        cfg.name,
                        describe_frame(&cfg.frames[0])
                    ),
                });
                continue;
            };
            let coverage = (st.covered as f64 / m.edge_count.max(1) as f64).min(1.0);
            let has_checks = cfg.checks.as_ref().is_some_and(|c| !c.is_empty());
            let check_rate =
                if has_checks { 1.0 - st.checksum_failures as f64 / st.complete.max(1) as f64 } else { 0.5 };
            let mut ev = vec![
                evidence(
                    format!("Channel {} rests {}, as {} does.", ch.index, cfg.idle.name(), cfg.name),
                    true,
                ),
                evidence(
                    format!(
                        "{} frame{} on channel {} match the {} pattern: {}.",
                        st.complete,
                        format::s(st.complete),
                        ch.index,
                        cfg.name,
                        describe_frame(&cfg.frames[first])
                    ),
                    true,
                ),
                evidence(
                    format!(
                        "{} % of the channel's edges belong to those frames.",
                        (coverage * 100.0).round()
                    ),
                    coverage > 0.7,
                ),
            ];
            if has_checks {
                ev.push(evidence(
                    if st.checksum_failures > 0 {
                        format!(
                            "{} frame{} fail the built-in check.",
                            st.checksum_failures,
                            format::s(st.checksum_failures)
                        )
                    } else {
                        "Every frame passes the built-in check.".into()
                    },
                    st.checksum_failures == 0,
                ));
            }
            let order = if cfg.bit_order == BitOrder::Msb { "most" } else { "least" };
            out.detections.push(Detection {
                protocol_id: cfg.id.clone(),
                channels: vec![ch.index],
                roles: roles(&[("data", Some(ch.index))]),
                settings: s,
                notes: [
                    ("role:data".to_string(), note(0.9, format!("Channel {} carries the pulses.", ch.index))),
                    (
                        "bitOrder".to_string(),
                        note(0.7, format!("{} sends the {order} significant bit first.", cfg.name)),
                    ),
                    ("tolerance".to_string(), note(0.8, "The standard timing windows already fit.")),
                ]
                .into_iter()
                .collect(),
                // frames with a fixed number of bits are a more specific match than open-ended ones
                confidence: clamp01(
                    (0.3 + 0.45 * coverage + 0.2 * check_rate + if st.fixed_bits { 0.05 } else { 0.0 })
                        * if m.periodic.is_some() { 0.7 } else { 1.0 },
                ),
                evidence: ev,
                warnings: vec![],
            });
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
        let slot = setup.slot("data").ok_or("Choose a channel for the data line.")?;
        let m = PulseMachine::new(
            self.cfg.clone(),
            setup.sample_rate_hz,
            &setup.settings,
            setup.channels[slot].channel,
            setup.start,
        );
        decoder(m, setup, snapshot)
    }
}

/// Middle of a timing window; open-ended windows (like "up to 1 s") use a short value.
pub fn typical_us(lo: f64, hi: f64) -> f64 {
    (lo + hi.min(lo * 4.0 + 100.0)) / 2.0
}

/// Rounds a duration in samples (for generators).
pub fn samples(us: f64, rate: f64) -> Sample {
    num::round(us * rate / 1e6)
}
