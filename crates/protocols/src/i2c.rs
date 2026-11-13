//! I2C: two-wire bus with a clock (SCL) and a data line (SDA), both resting high.

use serde::{Deserialize, Serialize};

use logica_core::{Level, Sample};

use crate::bytes::i2c_device;
use crate::format::{self, char_of, hex, hex2};
use crate::num;
use crate::protocol::{DetectContext, DetectorOutput, Protocol, ProtocolInfo, symptoms};
use crate::stream::{DecoderSetup, Edges, Machine, NewFrame, Out, Res, StreamDecoder, decoder, run_steps};
use crate::types::*;
use crate::window::WinChannel;

const SCL: usize = 0;
const SDA: usize = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct I2cStats {
    pub starts: u64,
    pub repeated_starts: u64,
    pub stops: u64,
    pub bytes: u64,
    pub addresses: Vec<u8>,
    pub nacked_addresses: Vec<u8>,
    pub incomplete: u64,
    /// Bits per transaction (start to stop).
    pub transaction_bits: Vec<u64>,
    pub sda_edges: u64,
    pub clocks_outside: u64,
    /// Median clock period while bytes were sent, in samples.
    pub clock_period: Option<u64>,
}

/// The I2C decoder (Signal Detective's `runI2c`, made streaming). A bit is read at the clock's
/// rising edge but only counts once the clock falls again: before a stop or repeated start the
/// clock rises with no bit, and the mark follows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct I2cMachine {
    addr8: bool,
    lane: u32,
    /// Whether to keep the statistics lists (detection only).
    stats_lists: bool,
    next: [u64; 2],
    scl: Level,
    sda: Level,
    last_scl_edge: Option<Sample>,
    last_scl_fall: Option<Sample>,
    last_rise: Option<Sample>,
    in_transfer: bool,
    bits: Vec<u8>,
    byte_start: Sample,
    first_after_start: bool,
    is_read: bool,
    segment: String,
    tx_bits: u64,
    last_byte_was_read: bool,
    last_addr_nack: bool,
    periods: Vec<u64>,
    /// Titles of the current packet's transfers.
    titles: Vec<String>,
    pending: Option<(u8, Sample)>,
    last_bit_rise: Option<Sample>,
    byte8_fall: Sample,
    pub stats: I2cStats,
}

impl I2cMachine {
    pub fn new(addr8: bool, lane: u32, scl: Level, sda: Level, stats_lists: bool) -> Self {
        I2cMachine {
            addr8,
            lane,
            stats_lists,
            next: [0, 0],
            scl,
            sda,
            last_scl_edge: None,
            last_scl_fall: None,
            last_rise: None,
            in_transfer: false,
            bits: Vec::new(),
            byte_start: 0,
            first_after_start: false,
            is_read: false,
            segment: String::new(),
            tx_bits: 0,
            last_byte_was_read: false,
            last_addr_nack: false,
            periods: Vec::new(),
            titles: Vec::new(),
            pending: None,
            last_bit_rise: None,
            byte8_fall: 0,
            stats: I2cStats::default(),
        }
    }

    /// Half a clock period, for the width of start and stop marks.
    fn half(&self) -> f64 {
        match (self.last_rise, self.last_scl_fall) {
            (Some(r), Some(f)) if r > 0 && f > 0 => 2f64.max(r.abs_diff(f) as f64),
            _ => 10.0,
        }
    }

    fn flush_partial(&mut self, at: Sample, out: &mut Out) {
        if !self.bits.is_empty() && self.bits.len() < 9 {
            self.stats.incomplete += 1;
            out.add(
                NewFrame::new(
                    self.byte_start,
                    at,
                    self.lane,
                    FrameKind::Info,
                    format!("{} bits", self.bits.len()),
                )
                .detail("A byte cut short by a start or stop mark")
                .problem(problem(
                    ProblemCode::Incomplete,
                    format!("Only {} of 9 clock pulses before the start or stop.", self.bits.len()),
                )),
            );
        }
        self.bits.clear();
    }

    fn title_packet(&mut self, out: &mut Out) {
        if out.current_packet().is_some() {
            out.set_title(if self.titles.is_empty() {
                "Transaction without an address".to_string()
            } else {
                self.titles.join(", ")
            });
        }
        self.titles.clear();
    }

    fn on_scl(&mut self, level: Level, at: Sample, out: &mut Out) {
        self.scl = level;
        self.last_scl_edge = Some(at);
        if level == Level::High {
            self.last_rise = Some(at);
            if !self.in_transfer {
                self.stats.clocks_outside += 1;
                self.pending = None;
                return;
            }
            self.pending = Some((u8::from(self.sda.is_high()), at));
            return;
        }
        self.last_scl_fall = Some(at);
        let Some((bit, rise)) = self.pending.take() else { return };
        self.bits.push(bit);
        self.tx_bits += 1;
        if let Some(prev) = self.last_bit_rise
            && self.bits.len() > 1
            && self.stats_lists
        {
            self.periods.push(rise - prev);
        }
        self.last_bit_rise = Some(rise);
        if self.bits.len() == 8 {
            self.byte8_fall = at;
        }
        if self.bits.len() < 9 {
            return;
        }
        let value = self.bits[..8].iter().fold(0u8, |v, &b| (v << 1) | b);
        let ack = self.bits[8] == 0;
        let (byte_end, ack_end) = (self.byte8_fall, at);
        self.stats.bytes += 1;
        let lane = self.lane;
        if self.first_after_start {
            let addr = value >> 1;
            self.is_read = value & 1 == 1;
            if self.stats_lists {
                self.stats.addresses.push(addr);
                if !ack {
                    self.stats.nacked_addresses.push(addr);
                }
            }
            self.segment = format!(
                "{} {}",
                if self.is_read { "Read from" } else { "Write to" },
                hex(u64::from(addr), 2)
            );
            self.titles.push(if ack {
                self.segment.clone()
            } else {
                format!("{} (no answer)", self.segment)
            });
            let rw = if self.is_read { "R" } else { "W" };
            out.add(
                NewFrame::new(
                    self.byte_start,
                    byte_end,
                    lane,
                    FrameKind::Address,
                    if self.addr8 {
                        format!("{} {rw}", hex2(u64::from(value)))
                    } else {
                        format!("{} {rw}", hex2(u64::from(addr)))
                    },
                )
                .value(f64::from(value))
                .detail(format!(
                    "Address {} (byte {} on the wire), {}{}",
                    hex(u64::from(addr), 2),
                    hex(u64::from(value), 2),
                    if self.is_read { "read" } else { "write" },
                    i2c_device(addr).map_or(String::new(), |d| format!(". Often: {d}"))
                )),
            );
            self.last_addr_nack = !ack;
            self.first_after_start = false;
        } else {
            out.add(
                NewFrame::new(self.byte_start, byte_end, lane, FrameKind::Data, hex2(u64::from(value)))
                    .value(f64::from(value))
                    .detail(format!(
                        "{} = {value} = '{}' ({})",
                        hex2(u64::from(value)),
                        char_of(value),
                        if self.is_read { "from the device" } else { "to the device" }
                    )),
            );
            let label = if self.segment.is_empty() { "data".to_string() } else { self.segment.clone() };
            out.byte(&label, lane, value, byte_end);
            self.last_addr_nack = false;
        }
        self.last_byte_was_read = self.is_read;
        out.add(
            NewFrame::new(
                byte_end,
                ack_end,
                lane,
                if ack { FrameKind::Ack } else { FrameKind::Nack },
                if ack { "ACK" } else { "NACK" },
            )
            .detail(if ack {
                "Acknowledge: the receiver pulled the data line low."
            } else if self.last_addr_nack {
                "Not acknowledged: no device answered at this address."
            } else if self.last_byte_was_read {
                "Not acknowledged: the master ends the read here. This is normal."
            } else {
                "Not acknowledged: the device rejected this byte."
            }),
        );
        self.bits.clear();
        self.byte_start = ack_end;
    }

    fn on_sda(&mut self, level: Level, at: Sample, out: &mut Out) {
        self.stats.sda_edges += 1;
        self.sda = level;
        if self.scl != Level::High || self.last_scl_edge == Some(at) {
            return;
        }
        self.pending = None;
        let w = self.half() / 2.0;
        let mark_end = num::round(at as f64 + w);
        let lane = self.lane;
        if level == Level::Low {
            // START or repeated START
            if self.in_transfer {
                self.flush_partial(at, out);
                self.stats.repeated_starts += 1;
                out.add(
                    NewFrame::new(at, mark_end, lane, FrameKind::RepeatedStart, "Sr")
                        .detail("Repeated start: a new transfer without releasing the bus."),
                );
            } else {
                self.stats.starts += 1;
                self.tx_bits = 0;
                self.title_packet(out);
                out.open_packet(at, "Transaction");
                out.add(
                    NewFrame::new(at, mark_end, lane, FrameKind::Start, "S")
                        .detail("Start: data fell while the clock was high."),
                );
            }
            self.in_transfer = true;
            self.first_after_start = true;
            self.bits.clear();
            self.byte_start = at;
        } else if self.in_transfer {
            self.flush_partial(at, out);
            self.stats.stops += 1;
            if self.stats_lists {
                self.stats.transaction_bits.push(self.tx_bits);
            }
            out.add(
                NewFrame::new(at, mark_end, lane, FrameKind::Stop, "P")
                    .detail("Stop: data rose while the clock was high."),
            );
            out.close_packet(mark_end);
            self.in_transfer = false;
        }
    }

    fn step(&mut self, ed: &mut Edges, out: &mut Out) -> Res<bool> {
        let Some((slot, at)) = ed.next_merged(&self.next)? else { return Ok(false) };
        let level = ed.level_after(slot, self.next[slot]);
        self.next[slot] += 1;
        ed.release_index(slot, self.next[slot]);
        if slot == SCL {
            self.on_scl(level, at, out);
        } else {
            self.on_sda(level, at, out);
        }
        Ok(true)
    }

    fn end(&mut self, out: &mut Out) {
        if self.in_transfer && self.stats_lists {
            self.stats.transaction_bits.push(self.tx_bits);
        }
        if !self.periods.is_empty() {
            self.periods.sort_unstable();
            self.stats.clock_period = Some(self.periods[self.periods.len() >> 1]);
        }
        self.title_packet(out);
        if self.stats.starts == 0 && self.stats.bytes == 0 {
            out.note("No start mark found. If the lines are I2C, SCL and SDA may be swapped.");
        }
    }
}

impl Machine for I2cMachine {
    fn run(&mut self, ed: &mut Edges, out: &mut Out) {
        run_steps(|| self.step(ed, out));
    }
    fn at_end(&mut self, _ed: &Edges, out: &mut Out) {
        self.end(out);
    }
}

/// Runs the decoder over two window channels, keeping only statistics.
pub fn i2c_stats(scl: &WinChannel, sda: &WinChannel, end: Sample) -> I2cStats {
    let mut m = I2cMachine::new(false, sda.index, scl.initial, sda.initial, true);
    let mut ed =
        Edges::complete(vec![(scl.initial, scl.edges.clone()), (sda.initial, sda.edges.clone())], end);
    let mut out = Out::new(false);
    run_steps(|| m.step(&mut ed, &mut out));
    m.end(&mut out);
    m.stats
}

fn speed_name(f: f64) -> &'static str {
    if (f - 100e3).abs() / 100e3 < 0.25 {
        "I2C standard mode (100 kHz)"
    } else if (f - 400e3).abs() / 400e3 < 0.2 {
        "I2C fast mode (400 kHz)"
    } else if (f - 1e6).abs() / 1e6 < 0.2 {
        "I2C fast mode plus (1 MHz)"
    } else if f < 100e3 {
        "slower than standard mode"
    } else {
        "between the standard speeds"
    }
}

fn unique(v: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

fn detect(ctx: &DetectContext<'_>, settings_specs: &[SettingSpec]) -> DetectorOutput {
    let chs: Vec<&WinChannel> = ctx.channels.iter().copied().filter(|c| !c.edges.is_empty()).collect();
    if chs.len() < 2 {
        return DetectorOutput::rejected(
            "i2c",
            "I2C needs two lines that are busy together, and this group has one",
        );
    }
    if chs.len() > 4 {
        return DetectorOutput::default();
    }
    let mut best: Option<(Detection, f64)> = None;
    let mut any_high = false;
    for a in &chs {
        for b in &chs {
            if a.index == b.index {
                continue;
            }
            let (ma, mb) = (ctx.m(a.index), ctx.m(b.index));
            if ma.rest_level != Level::High || mb.rest_level != Level::High {
                continue;
            }
            any_high = true;
            let stats = i2c_stats(a, b, ctx.window.end);
            if stats.starts == 0 || stats.bytes == 0 {
                continue;
            }
            let tx: Vec<u64> = stats.transaction_bits.iter().copied().filter(|&n| n > 0).collect();
            let nines = tx.iter().filter(|&&n| n % 9 == 0).count() as f64 / tx.len().max(1) as f64;
            let conditions =
                (stats.starts + stats.repeated_starts + stats.stops) as f64 / stats.sda_edges.max(1) as f64;
            let incomplete_frac = stats.incomplete as f64 / (stats.bytes + stats.incomplete).max(1) as f64;
            let periodic = ma.periodic.as_ref().is_some_and(|p| !p.continuous);
            let conf = clamp01(
                0.25 + 0.3 * nines
                    + if periodic { 0.15 } else { 0.0 }
                    + 0.2 * (1.0 - (incomplete_frac * 3.0).min(1.0))
                    + if conditions < 0.5 { 0.1 } else { 0.0 },
            );
            let score = conf - conditions * 0.1;
            if best.as_ref().is_some_and(|(_, s)| score <= *s) {
                continue;
            }
            let rate = ctx.sample_rate_hz;
            let period = ma
                .periodic
                .as_ref()
                .map(|p| p.period_samples)
                .or(stats.clock_period.map(|p| p as f64))
                .unwrap_or(0.0);
            let f = if period > 0.0 { rate / period } else { 0.0 };
            let mut ev = vec![
                evidence("Both lines rest high, as I2C lines with pull-up resistors do.", true),
                evidence(
                    format!(
                        "Channel {} has evenly spaced pulses{}, so it is the I2C clock (SCL).",
                        a.index,
                        if tx.is_empty() {
                            String::new()
                        } else {
                            format!(", {} % of transfers in groups of 9", (nines * 100.0).round())
                        }
                    ),
                    nines > 0.5,
                ),
                evidence(
                    format!(
                        "Channel {} changes only while channel {} is low, apart from {} start and {} stop marks, so it is the data line (SDA).",
                        b.index,
                        a.index,
                        stats.starts + stats.repeated_starts,
                        stats.stops
                    ),
                    conditions < 0.5,
                ),
            ];
            if f > 0.0 {
                ev.push(evidence(format!("The clock runs at {}: {}.", format::hz(f), speed_name(f)), true));
            }
            let uniq = unique(&stats.addresses);
            if !uniq.is_empty() {
                let list: Vec<String> = uniq
                    .iter()
                    .take(4)
                    .map(|&x| {
                        format!(
                            "{}{}",
                            hex(u64::from(x), 2),
                            i2c_device(x).map_or(String::new(), |d| format!(" ({d})"))
                        )
                    })
                    .collect();
                ev.push(evidence(
                    format!(
                        "The first byte after each start is an address: {}{}.",
                        list.join(", "),
                        if uniq.len() > 4 { " and more" } else { "" }
                    ),
                    true,
                ));
            }
            let mut warnings: Vec<String> = unique(&stats.nacked_addresses)
                .iter()
                .map(|&x| {
                    format!("Address {} got no acknowledge: no device answered there.", hex(u64::from(x), 2))
                })
                .collect();
            if period > 0.0 && period < 8.0 {
                warnings.push(format!(
                    "Only {} samples per clock period. Sample at least 4 times faster than the clock ({}); 10 times is comfortable.",
                    format::fixed(period, 1),
                    format::hz(f * 4.0)
                ));
            }
            if stats.incomplete > 0 {
                warnings.push(format!(
                    "{} byte{} cut short by a start or stop mark.",
                    stats.incomplete,
                    if stats.incomplete == 1 { " was" } else { "s were" }
                ));
            }
            let mut channels = vec![a.index, b.index];
            channels.sort_unstable();
            let det = Detection {
                protocol_id: "i2c".into(),
                channels,
                roles: roles(&[("scl", Some(a.index)), ("sda", Some(b.index))]),
                settings: SettingSpec::with_defaults(settings_specs, &Settings::new()),
                notes: [
                    (
                        "role:scl",
                        note(
                            clamp01(0.5 + 0.5 * nines),
                            format!(
                                "Channel {} has the regular pulses{}.",
                                a.index,
                                if f > 0.0 {
                                    format!(" ({}, period {})", format::hz(f), format::time(period / rate))
                                } else {
                                    String::new()
                                }
                            ),
                        ),
                    ),
                    (
                        "role:sda",
                        note(clamp01(1.0 - conditions), format!("Channel {} changes while the clock is low, except at start and stop.", b.index)),
                    ),
                    ("addressFormat", note(1.0, "How the address is shown does not change the decode. Pick the form your datasheet uses.")),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                confidence: conf,
                evidence: ev,
                warnings,
            };
            best = Some((det, score));
        }
    }
    match best {
        Some((d, _)) => DetectorOutput { detections: vec![d], rejections: vec![] },
        None if !any_high => {
            DetectorOutput::rejected("i2c", "I2C lines both rest high, and no pair of lines here does")
        }
        None => DetectorOutput::rejected(
            "i2c",
            "no pair of lines shows I2C start marks (data falling while a clock line is high) followed by bytes",
        ),
    }
}

pub struct I2c {
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
    info: ProtocolInfo,
}

impl Default for I2c {
    fn default() -> Self {
        I2c {
            roles: vec![
                RoleSpec::new("scl", "SCL (clock)", "SCL", "The line with the regular pulses."),
                RoleSpec::new("sda", "SDA (data)", "SDA", "The other line."),
            ],
            settings: vec![
                SettingSpec::choice("addressFormat", "Address shown as", "7bit", &[("7bit", "7-bit address plus R/W"), ("8bit", "8-bit byte as sent")])
                    .help("Datasheets use either form. 7-bit 0x50 is the same as 8-bit 0xA0 (write) and 0xA1 (read)."),
            ],
            info: ProtocolInfo {
                what: "Two-wire bus with a clock (SCL) and a data line (SDA). Both rest high.".into(),
                confirm: "Data only changes while the clock is low, except the start (data falls while clock is high) and stop (data rises while clock is high).".into(),
                decoder_tips: "Assign SCL to the line with the regular pulses and SDA to the other. No speed setting is needed. First byte is the 7-bit address plus a read/write bit.".into(),
                lookalikes: "SWD also uses a clock and one data line, but has no groups of 9 and no start and stop marks.".into(),
                symptoms: symptoms(&[
                    ("NACK right after the address", "No device at that address, wrong address, or device busy", "Check the 7-bit versus 8-bit address form, power and wiring."),
                    ("NACK after a data byte during a write", "The device rejected the register or value", "Check the register map in the datasheet."),
                    ("NACK on the last byte of a read", "Normal: the master signals it is done", "Nothing to fix."),
                    ("Addresses counting up, nearly all NACK", "A bus scan", "The few ACKs are the devices present."),
                    ("Clock held low for a long time mid-transfer", "Clock stretching: the slave asks the master to wait", "Usually fine. If it never releases, the slave has hung."),
                    ("Data line stuck low, no activity", "A slave hung in the middle of a byte", "Send 9 clock pulses or power-cycle the device."),
                    ("Start or stop in the middle of a byte", "Noise on the data line while the clock is high", "Check pull-ups, wiring length and ground."),
                    ("Fewer than 9 clocks in a group", "Glitch on the clock, or sample rate too low", "Raise the sample rate; check the threshold."),
                    ("Read returns all FF", "The slave is not driving the line", "Check the register number and that the device is ready."),
                    ("Decoder shows nothing at all", "SCL and SDA assigned the wrong way round", "Swap the two channels in the decoder."),
                ]),
            },
        }
    }
}

impl Protocol for I2c {
    fn id(&self) -> &str {
        "i2c"
    }
    fn name(&self) -> &str {
        "I2C"
    }
    fn group(&self) -> &str {
        "I2C"
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
        if s.text("addressFormat") == "8bit" { "8-bit addresses".into() } else { "7-bit addresses".into() }
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
        let (Some(scl), Some(sda)) = (setup.slot("scl"), setup.slot("sda")) else {
            return Err("Choose channels for SCL and SDA.".into());
        };
        debug_assert_eq!((scl, sda), (SCL, SDA));
        let m = I2cMachine::new(
            setup.settings.text("addressFormat") == "8bit",
            setup.channels[sda].channel,
            setup.start_levels[scl],
            setup.start_levels[sda],
            false,
        );
        decoder(m, setup, snapshot)
    }
}
