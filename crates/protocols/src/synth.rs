//! Signal generators (doc 06): clean signals for every protocol, for the tests and the synthetic
//! device. Times are in seconds; each lane converts to samples with the capture's rate. A
//! generated capture always extends to its last edge.

use std::sync::Arc;

use logica_core::capture::{
    CaptureMeta, CaptureState, ChannelInfo, Note, NoteLevel, Source, SourceKind, whole,
};
use logica_core::io::MemFile;
use logica_core::store::{CaptureStore, StoreResult};
use logica_core::{Level, Sample};

use crate::bytes::can_crc15;
use crate::calc::spi_mode_parts;
use crate::num;
use crate::pulsecode::{Encoding, PulseCodeConfig, SegmentSpec, typical_us};

const COLOURS: [&str; 4] = ["#e54242", "#ff7f00", "#007fff", "#26ad34"];

/// One generated line.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub rate: f64,
    pub initial: Level,
    pub level: Level,
    pub edges: Vec<Sample>,
}

impl Lane {
    pub fn new(rate: f64, initial: Level) -> Self {
        Lane { rate, initial, level: initial, edges: Vec::new() }
    }

    pub fn low(rate: f64) -> Self {
        Lane::new(rate, Level::Low)
    }

    pub fn high(rate: f64) -> Self {
        Lane::new(rate, Level::High)
    }

    /// Sets the level at time `t` in seconds. Two changes on the same sample cancel out.
    pub fn set(&mut self, t: f64, level: impl Into<Bit>) {
        let lv = level.into().0;
        if lv == self.level {
            return;
        }
        let s = num::round(t * self.rate);
        match self.edges.last() {
            Some(&last) if s == last => {
                self.edges.pop();
            }
            Some(&last) if s < last => panic!("edges out of order: {s} after {last}"),
            _ => self.edges.push(s),
        }
        self.level = lv;
    }
}

/// A level given as a level, a bool or 0/1.
pub struct Bit(pub Level);
impl From<Level> for Bit {
    fn from(l: Level) -> Self {
        Bit(l)
    }
}
impl From<bool> for Bit {
    fn from(b: bool) -> Self {
        Bit(Level::from(b))
    }
}
impl From<u8> for Bit {
    fn from(b: u8) -> Self {
        Bit(Level::from(b != 0))
    }
}
impl From<u32> for Bit {
    fn from(b: u32) -> Self {
        Bit(Level::from(b != 0))
    }
}
impl From<u64> for Bit {
    fn from(b: u64) -> Self {
        Bit(Level::from(b != 0))
    }
}

/// A generated capture.
#[derive(Clone, Debug, PartialEq)]
pub struct GenCapture {
    pub title: String,
    pub rate: f64,
    pub sample_count: Sample,
    /// Physical index, name, lane; ascending by index.
    pub channels: Vec<(u32, String, Lane)>,
}

/// A capture of `seconds` at `rate` from lanes (extended to the last edge if that is later).
pub fn make_capture(title: &str, rate: f64, seconds: f64, lanes: Vec<(u32, &str, Lane)>) -> GenCapture {
    let mut channels: Vec<(u32, String, Lane)> =
        lanes.into_iter().map(|(i, n, l)| (i, n.to_string(), l)).collect();
    channels.sort_by_key(|c| c.0);
    let last = channels.iter().filter_map(|c| c.2.edges.last()).max().map_or(0, |&e| e + 1);
    GenCapture { title: title.into(), rate, sample_count: num::round(seconds * rate).max(last), channels }
}

impl GenCapture {
    pub fn meta(&self) -> CaptureMeta {
        CaptureMeta {
            id: self.title.clone(),
            title: self.title.clone(),
            source: Source { kind: SourceKind::Synthetic, detail: "generated example".into() },
            sample_rate_hz: self.rate,
            trigger: None,
            channels: self
                .channels
                .iter()
                .map(|(i, n, _)| ChannelInfo {
                    index: *i,
                    name: n.clone(),
                    colour: Some(COLOURS[*i as usize % 4].into()),
                    enabled: true,
                    threshold_volts: None,
                })
                .collect(),
            segments: whole(self.sample_count),
            notes: vec![Note {
                level: NoteLevel::Info,
                message: "This capture was generated, not recorded. Its timing is perfect.".into(),
            }],
            state: CaptureState::Complete,
            artefact: None,
            saved_decoders: Vec::new(),
        }
    }

    /// The capture in a store in memory.
    pub fn store(&self) -> StoreResult<CaptureStore> {
        let levels: Vec<Level> = self.channels.iter().map(|c| c.2.initial).collect();
        let mut s = CaptureStore::new(&levels, Arc::new(MemFile::new()), 16, 64 << 20);
        for (slot, (_, _, lane)) in self.channels.iter().enumerate() {
            s.push_edges(slot, &lane.edges)?;
        }
        s.finish(self.sample_count)?;
        Ok(s)
    }
}

// ------------------------------------------------------------------ UART

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UartOptions {
    pub baud: f64,
    pub data_bits: u32,
    /// "none", "even" or "odd".
    pub parity: &'static str,
    pub stop_bits: u32,
    pub invert: bool,
    /// Idle bits between bytes.
    pub gap_bits: f64,
}

impl UartOptions {
    pub fn baud(baud: f64) -> Self {
        UartOptions { baud, data_bits: 8, parity: "none", stop_bits: 1, invert: false, gap_bits: 0.0 }
    }
}

/// Sends bytes from time `t`; returns the time after the last stop bit.
pub fn uart_send(lane: &mut Lane, mut t: f64, bytes: &[u8], o: UartOptions) -> f64 {
    let tb = 1.0 / o.baud;
    let inv = u8::from(o.invert);
    for &b in bytes {
        let mut bits = vec![0u8];
        let mut ones = 0;
        for k in 0..o.data_bits {
            let v = (b >> k) & 1;
            ones += v;
            bits.push(v);
        }
        match o.parity {
            "even" => bits.push(ones & 1),
            "odd" => bits.push(1 - (ones & 1)),
            _ => {}
        }
        bits.extend(std::iter::repeat_n(1, o.stop_bits as usize));
        for (i, &v) in bits.iter().enumerate() {
            lane.set(t + i as f64 * tb, v ^ inv);
        }
        t += bits.len() as f64 * tb + o.gap_bits * tb;
    }
    t
}

pub fn text_bytes(s: &str) -> Vec<u8> {
    s.bytes().collect()
}

// ------------------------------------------------------------------ I2C

#[derive(Clone, Debug, PartialEq)]
pub struct I2cTransfer {
    pub address: u8,
    pub read: bool,
    pub data: Vec<u8>,
    /// The device does not answer the address.
    pub nack: bool,
}

/// One transaction of transfers joined by repeated starts; returns the end time.
pub fn i2c_transaction(
    scl: &mut Lane,
    sda: &mut Lane,
    mut t: f64,
    hz: f64,
    transfers: &[I2cTransfer],
) -> f64 {
    let q = 1.0 / hz / 4.0;
    let bit = |scl: &mut Lane, sda: &mut Lane, t: &mut f64, v: u8| {
        sda.set(*t + q, v);
        scl.set(*t + 2.0 * q, 1u8);
        scl.set(*t + 4.0 * q, 0u8);
        *t += 4.0 * q;
    };
    sda.set(t, 0u8);
    t += q * 2.0;
    scl.set(t, 0u8);
    for (i, x) in transfers.iter().enumerate() {
        if i > 0 {
            // repeated start: release SDA, raise SCL, pull SDA low
            sda.set(t + q, 1u8);
            scl.set(t + 2.0 * q, 1u8);
            sda.set(t + 3.0 * q, 0u8);
            scl.set(t + 4.0 * q, 0u8);
            t += 4.0 * q;
        }
        let first = (x.address << 1) | u8::from(x.read);
        for k in (0..8).rev() {
            bit(scl, sda, &mut t, (first >> k) & 1);
        }
        bit(scl, sda, &mut t, u8::from(x.nack));
        if x.nack {
            continue;
        }
        for (j, &d) in x.data.iter().enumerate() {
            for k in (0..8).rev() {
                bit(scl, sda, &mut t, (d >> k) & 1);
            }
            // the receiver acknowledges; on a read the master NACKs the last byte
            bit(scl, sda, &mut t, u8::from(x.read && j == x.data.len() - 1));
        }
    }
    // stop
    sda.set(t + q, 0u8);
    scl.set(t + 2.0 * q, 1u8);
    sda.set(t + 3.0 * q, 1u8);
    t + 4.0 * q
}

// ------------------------------------------------------------------ SPI

/// One chip-select frame; returns the end time.
#[allow(clippy::too_many_arguments)]
pub fn spi_frame(
    clk: &mut Lane,
    mut cs: Option<&mut Lane>,
    mut mosi: Option<&mut Lane>,
    mut miso: Option<&mut Lane>,
    mut t: f64,
    hz: f64,
    mode: u8,
    mosi_bytes: &[u8],
    miso_bytes: &[u8],
    msb_first: bool,
) -> f64 {
    let p = spi_mode_parts(mode);
    let h = 1.0 / hz / 2.0;
    if let Some(c) = cs.as_deref_mut() {
        c.set(t, 0u8);
    }
    t += h;
    let n = mosi_bytes.len().max(miso_bytes.len()) * 8;
    let bit_of = |arr: &[u8], i: usize| -> u8 {
        let byte = arr.get(i / 8).copied().unwrap_or(0);
        let k = i % 8;
        (byte >> if msb_first { 7 - k } else { k }) & 1
    };
    for i in 0..n {
        let (lead, trail) = (1 - p.cpol, p.cpol);
        if p.cpha == 0 {
            // data set up before the leading edge
            if let Some(l) = mosi.as_deref_mut() {
                l.set(t - h / 2.0, bit_of(mosi_bytes, i));
            }
            if let Some(l) = miso.as_deref_mut() {
                l.set(t - h / 2.0, bit_of(miso_bytes, i));
            }
            clk.set(t, lead);
            clk.set(t + h, trail);
        } else {
            clk.set(t, lead);
            if let Some(l) = mosi.as_deref_mut() {
                l.set(t + h / 2.0, bit_of(mosi_bytes, i));
            }
            if let Some(l) = miso.as_deref_mut() {
                l.set(t + h / 2.0, bit_of(miso_bytes, i));
            }
            clk.set(t + h, trail);
        }
        t += 2.0 * h;
    }
    if let Some(c) = cs {
        c.set(t, 1u8);
    }
    t + h
}

// ------------------------------------------------------------------ pulse-code

fn mid_us(r: [f64; 2]) -> f64 {
    typical_us(r[0], r[1]) * 1e-6
}

fn seg_mid(s: &SegmentSpec) -> f64 {
    typical_us(s.min_us, s.max_us) * 1e-6
}

/// One frame of a pulse-code protocol using the middle of every timing window.
pub fn pulse_code_send(lane: &mut Lane, mut t: f64, cfg: &PulseCodeConfig, frame: usize, bits: &[u8]) -> f64 {
    let f = &cfg.frames[frame];
    let idle = cfg.idle.level();
    for s in &f.preamble {
        lane.set(t, s.level.level());
        t += seg_mid(s);
    }
    if let Some(b) = &f.bit {
        let first = b.first.level();
        for &v in bits {
            let code = mid_us(if v != 0 { b.one } else { b.zero });
            let other = mid_us(b.other);
            lane.set(t, first);
            t += if b.encoding == Encoding::First { code } else { other };
            lane.set(t, !first);
            t += if b.encoding == Encoding::First { other } else { code };
        }
    }
    for s in f.trailer.iter().flatten() {
        lane.set(t, s.level.level());
        t += seg_mid(s);
    }
    lane.set(t, idle);
    t
}

/// The bits of bytes, most or least significant first.
pub fn bits_of(bytes: &[u8], msb: bool) -> Vec<u8> {
    bytes.iter().flat_map(|&b| (0..8).map(move |k| (b >> if msb { 7 - k } else { k }) & 1)).collect()
}

// ------------------------------------------------------------------ CAN

/// One CAN frame (data frame, standard or extended ID); returns the end time.
pub fn can_frame(lane: &mut Lane, t: f64, bitrate: f64, id: u32, data: &[u8], extended: bool) -> f64 {
    let tb = 1.0 / bitrate;
    let mut bits: Vec<u8> = vec![0];
    let push = |bits: &mut Vec<u8>, v: u64, n: u32| {
        for k in (0..n).rev() {
            bits.push(u8::from((v >> k) & 1 == 1));
        }
    };
    if extended {
        push(&mut bits, u64::from(id >> 18), 11);
        bits.extend([1, 1]); // SRR, IDE
        push(&mut bits, u64::from(id & 0x3ffff), 18);
        bits.extend([0, 0, 0]); // RTR, r1, r0
    } else {
        push(&mut bits, u64::from(id), 11);
        bits.extend([0, 0, 0]); // RTR, IDE, r0
    }
    push(&mut bits, data.len() as u64, 4);
    for &d in data {
        push(&mut bits, u64::from(d), 8);
    }
    let crc = can_crc15(&bits);
    push(&mut bits, u64::from(crc), 15);
    // stuff
    let mut stuffed = Vec::new();
    let (mut run, mut last) = (0, 2u8);
    for &b in &bits {
        stuffed.push(b);
        run = if b == last { run + 1 } else { 1 };
        last = b;
        if run == 5 {
            stuffed.push(1 - b);
            last = 1 - b;
            run = 1;
        }
    }
    stuffed.extend([1, 0, 1, 1, 1, 1, 1, 1, 1, 1]); // CRC delimiter, ACK (dominant), ACK delimiter, EOF
    for (i, &b) in stuffed.iter().enumerate() {
        lane.set(t + i as f64 * tb, b);
    }
    t + (stuffed.len() + 3) as f64 * tb
}

// ------------------------------------------------------------------ PWM

pub fn pwm_signal(lane: &mut Lane, t0: f64, t1: f64, hz: f64, duty: f64) {
    let p = 1.0 / hz;
    let mut k = 0u64;
    loop {
        let t = t0 + k as f64 * p;
        if t + p > t1 {
            break;
        }
        lane.set(t, 1u8);
        lane.set(t + p * duty, 0u8);
        k += 1;
    }
}

// ------------------------------------------------------------------ I2S

/// Stereo samples as I2S (word select leads the data by one clock); returns the end time.
#[allow(clippy::too_many_arguments)]
pub fn i2s_stream(
    sck: &mut Lane,
    ws: &mut Lane,
    sd: &mut Lane,
    t: f64,
    bclk: f64,
    slot_bits: u32,
    samples: &[(i64, i64)],
    word_bits: u32,
) -> f64 {
    let tb = 1.0 / bclk;
    let mut frames: Vec<u8> = Vec::new();
    for &(l, r) in samples {
        for v in [l, r] {
            let u = if v < 0 { v + (1i64 << word_bits) } else { v };
            for k in 0..slot_bits {
                frames.push(if k < word_bits { u8::from((u >> (word_bits - 1 - k)) & 1 == 1) } else { 0 });
            }
        }
    }
    for i in 0..=frames.len() {
        let at = t + i as f64 * tb;
        let slot = i / slot_bits as usize;
        ws.set(at, u8::from(slot % 2 == 1)); // 0 = left
        sck.set(at, 0u8);
        if i >= 1 {
            sd.set(at, frames.get(i - 1).copied().unwrap_or(0));
        }
        sck.set(at + tb / 2.0, 1u8);
    }
    t + (frames.len() + 1) as f64 * tb
}

// ------------------------------------------------------------------ SWD

/// Line reset: `n` clocks with SWDIO high, then the two idle (low) clocks before a request.
pub fn swd_line_reset(clk: &mut Lane, io: &mut Lane, mut t: f64, hz: f64, n: u32) -> f64 {
    let tb = 1.0 / hz;
    for i in 0..n + 2 {
        io.set(t, u8::from(i < n));
        clk.set(t + tb / 2.0, 1u8);
        clk.set(t + tb, 0u8);
        t += tb;
    }
    t
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwdOp {
    pub ap: bool,
    pub read: bool,
    pub reg: u8,
    pub data: u32,
    /// 1 = OK, 2 = WAIT, 4 = FAULT.
    pub ack: u8,
}

pub fn swd_transfer(clk: &mut Lane, io: &mut Lane, mut t: f64, hz: f64, op: SwdOp) -> f64 {
    let tb = 1.0 / hz;
    let a2 = (op.reg >> 2) & 1;
    let a3 = (op.reg >> 3) & 1;
    let ap = u8::from(op.ap);
    let rnw = u8::from(op.read);
    let mut bits: Vec<u8> = vec![1, ap, rnw, a2, a3, (ap + rnw + a2 + a3) & 1, 0, 1, 0];
    bits.extend([op.ack & 1, (op.ack >> 1) & 1, (op.ack >> 2) & 1]);
    if op.ack == 1 {
        if !op.read {
            bits.push(0);
        }
        let mut ones = 0u8;
        for k in 0..32 {
            let v = u8::from((op.data >> k) & 1 == 1);
            ones += v;
            bits.push(v);
        }
        bits.push(ones & 1);
        if op.read {
            bits.push(0);
        }
    } else {
        bits.push(0);
    }
    bits.extend([0, 0]); // idle
    for &v in &bits {
        io.set(t, v);
        clk.set(t + tb / 2.0, 1u8);
        clk.set(t + tb, 0u8);
        t += tb;
    }
    t
}

// ------------------------------------------------------------------ JTAG

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JtagStep {
    pub tms: u8,
    pub tdi: Option<u8>,
    pub tdo: Option<u8>,
}

pub fn jtag_clock(
    tck: &mut Lane,
    tms: &mut Lane,
    tdi: &mut Lane,
    mut tdo: Option<&mut Lane>,
    mut t: f64,
    hz: f64,
    steps: &[JtagStep],
) -> f64 {
    let tb = 1.0 / hz;
    for s in steps {
        tms.set(t, s.tms);
        if let Some(v) = s.tdi {
            tdi.set(t, v);
        }
        if let (Some(l), Some(v)) = (tdo.as_deref_mut(), s.tdo) {
            l.set(t, v);
        }
        tck.set(t + tb / 2.0, 1u8);
        tck.set(t + tb, 0u8);
        t += tb;
    }
    t
}

/// Steps that shift `bits` (LSB first) through IR or DR from Run-Test/Idle back to it.
pub fn jtag_shift(ir: bool, in_bits: &[u8], out_bits: &[u8]) -> Vec<JtagStep> {
    let step = |tms| JtagStep { tms, tdi: None, tdo: None };
    let mut steps = vec![step(1)]; // Select-DR
    if ir {
        steps.push(step(1)); // Select-IR
    }
    steps.extend([step(0), step(0)]); // Capture, then Shift
    for (i, &b) in in_bits.iter().enumerate() {
        steps.push(JtagStep {
            tms: u8::from(i == in_bits.len() - 1),
            tdi: Some(b),
            tdo: Some(out_bits.get(i).copied().unwrap_or(0)),
        });
    }
    steps.extend([step(1), step(0)]); // Update, Run-Test/Idle
    steps
}
