//! ADCs read over SPI or I2C (doc 06, "Virtual analog channels"): a layer on the bus decoder.
//!
//! The source runs the bus's own streaming decoder inside itself: it hands it the edges, takes its
//! complete packets (an SPI chip-select frame, an I2C transaction) and cuts the sample out of each
//! as a bit field. The bus decoder's state is part of the source's snapshot, so live capture and
//! the batching rules hold as for any source. The point sits at the start of the transfer, where
//! the ADC samples.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::analog::{Encoding, fraction_of_code};
use logica_core::store::edges::EdgeBatch;

use super::{Source, SourceOutput};
use crate::protocol::Protocol;
use crate::stream::{DecoderSetup, NewFrame, Out, Output, StreamDecoder};
use crate::types::{FrameKind, Packet, ProblemCode, RoleSpec, SettingSpec, Settings, SettingsExt, problem};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bus {
    Spi,
    I2c,
}

impl Bus {
    fn protocol(self) -> Box<dyn Protocol> {
        match self {
            Bus::Spi => Box::new(crate::spi::Spi::default()),
            Bus::I2c => Box::new(crate::i2c::I2c::default()),
        }
    }
}

/// The bits `skip .. skip + n` of `bytes`, first bit the most significant bit of the first byte;
/// reversed when `lsb_first`. `None` when the bytes are too short.
pub fn bit_field(bytes: &[u8], skip: usize, n: usize, lsb_first: bool) -> Option<u64> {
    if n == 0 || n > 64 || skip + n > bytes.len() * 8 {
        return None;
    }
    let mut v = 0u64;
    for i in skip..skip + n {
        v = v << 1 | u64::from(bytes[i / 8] >> (7 - i % 8) & 1);
    }
    if lsb_first {
        v = v.reverse_bits() >> (64 - n);
    }
    Some(v)
}

/// How to cut a sample out of a transfer, from the settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Field {
    stream: String,
    skip: usize,
    bits: u32,
    lsb_first: bool,
    encoding: Encoding,
    /// Only transfers whose `match_stream` has `match_value` in bits `match_skip ..` count.
    match_stream: Option<String>,
    match_skip: usize,
    match_bits: usize,
    match_value: u64,
    /// I2C: the 7-bit address and the register the sample is read from (none: any).
    address: Option<u8>,
    register: Option<u8>,
}

fn whole(x: f64) -> usize {
    // settings are checked to small ranges by their specs
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = x.max(0.0).round() as usize;
    n
}

impl Field {
    fn of(s: &Settings) -> Self {
        let upper = |k: &str| s.text(k).to_uppercase();
        let opt_u8 = |k: &str| {
            let v = s.num(k);
            (v >= 0.0).then(|| u8::try_from(whole(v)).unwrap_or(u8::MAX))
        };
        Field {
            stream: upper("stream"),
            skip: whole(s.num("skipBits")),
            bits: u32::try_from(whole(s.num("bits"))).unwrap_or(12),
            lsb_first: s.text("order") == "lsb",
            encoding: if s.text("encoding") == "twos" {
                Encoding::TwosComplement
            } else {
                Encoding::Unsigned
            },
            match_stream: (s.text("matchStream") != "none" && !s.text("matchStream").is_empty())
                .then(|| upper("matchStream")),
            match_skip: whole(s.num("matchSkip")),
            match_bits: whole(s.num("matchBits")),
            match_value: u64::try_from(whole(s.num("matchValue"))).unwrap_or(0),
            address: opt_u8("address"),
            register: opt_u8("register"),
        }
    }
}

/// What the layer remembers between packets.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    /// I2C: the register pointer last written to each device.
    pointers: BTreeMap<u8, u8>,
    samples: u64,
    skipped: u64,
}

/// A source on top of a bus decoder.
struct Layer {
    bus: Bus,
    field: Field,
    base: Box<dyn StreamDecoder>,
    state: State,
    out: Out,
    lane: u32,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    base: serde_json::Value,
    state: State,
    out: Out,
}

/// The address in an I2C segment label such as "Read from 0x48".
fn address_of(label: &str) -> Option<u8> {
    let hex = label.rsplit("0x").next()?;
    u8::from_str_radix(hex.trim(), 16).ok()
}

impl Layer {
    fn sample(&mut self, p: &Packet) {
        let f = &self.field;
        let bytes = match self.bus {
            Bus::Spi => {
                if let Some(ms) = &f.match_stream {
                    let got =
                        p.stream(ms).and_then(|s| bit_field(&s.bytes, f.match_skip, f.match_bits, false));
                    if f.match_bits > 0 && got != Some(f.match_value) {
                        self.state.skipped += 1;
                        return;
                    }
                }
                match p.stream(&f.stream) {
                    Some(s) => s.bytes.clone(),
                    None => {
                        return self.problem(
                            p,
                            ProblemCode::Incomplete,
                            format!("The transfer has no {} data.", f.stream),
                        );
                    }
                }
            }
            Bus::I2c => {
                // a write sets the register pointer of a device; a read returns that register
                for s in &p.streams {
                    if s.label.starts_with("Write to")
                        && let (Some(a), Some(&reg)) = (address_of(&s.label), s.bytes.first())
                    {
                        self.state.pointers.insert(a, reg);
                    }
                }
                let read = p.streams.iter().find(|s| {
                    s.label.starts_with("Read from")
                        && address_of(&s.label).is_some_and(|a| f.address.is_none_or(|want| want == a))
                });
                let Some(read) = read else {
                    let refused =
                        f.address.map_or("Read from".to_string(), |a| format!("Read from 0x{a:02X}"));
                    if p.title.contains(&refused) && p.title.contains("(no answer)") {
                        self.problem(
                            p,
                            ProblemCode::Protocol,
                            "The ADC did not answer the read (NACK), so there is no sample.".into(),
                        );
                    }
                    return;
                };
                let addr = address_of(&read.label).unwrap_or(0);
                if let Some(want) = f.register
                    && self.state.pointers.get(&addr) != Some(&want)
                {
                    self.state.skipped += 1;
                    return;
                }
                read.bytes.clone()
            }
        };
        let skip = f.skip;
        let n = f.bits as usize;
        let Some(code) = bit_field(&bytes, skip, n, f.lsb_first) else {
            return self.problem(
                p,
                ProblemCode::Incomplete,
                format!(
                    "The transfer has {} bits; the sample needs bits {} to {}.",
                    bytes.len() * 8,
                    skip,
                    skip + n - 1
                ),
            );
        };
        match fraction_of_code(code, f.bits, f.encoding) {
            Ok(x) => {
                self.out.point(p.start, x);
                self.state.samples += 1;
            }
            Err(e) => self.problem(p, ProblemCode::Protocol, e.to_string()),
        }
    }

    /// No sample from this transfer: a gap, and why.
    fn problem(&mut self, p: &Packet, code: ProblemCode, message: String) {
        self.out.gap(p.start);
        self.out.add(
            NewFrame::new(p.start, p.end, self.lane, FrameKind::Info, "no sample")
                .problem(problem(code, message)),
        );
    }

    fn take(&mut self, o: Output) {
        for p in &o.packets {
            self.sample(p);
        }
    }
}

impl StreamDecoder for Layer {
    fn push(&mut self, batch: &EdgeBatch) {
        self.base.push(batch);
        let o = self.base.drain();
        self.take(o);
    }

    fn finish(&mut self, end: Sample) {
        self.base.finish(end);
        let o = self.base.drain();
        self.take(o);
        let bus = if self.bus == Bus::Spi { "SPI" } else { "I2C" };
        self.out.note(match (self.state.samples, self.state.skipped) {
            (0, 0) => format!("No {bus} transfer carried a sample."),
            (n, 0) => format!("{n} samples from {bus} transfers."),
            (n, k) => format!(
                "{n} samples from {bus} transfers; {k} transfers were for something else and were left out."
            ),
        });
    }

    fn drain(&mut self) -> Output {
        self.out.drain()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(Snapshot {
            base: self.base.snapshot(),
            state: self.state.clone(),
            out: self.out.clone(),
        })
        .unwrap_or(serde_json::Value::Null)
    }
}

/// An ADC on SPI or I2C.
pub struct SerialAdc {
    bus: Bus,
    roles: Vec<RoleSpec>,
    settings: Vec<SettingSpec>,
}

impl SerialAdc {
    pub fn new(bus: Bus) -> Self {
        let base = bus.protocol();
        let mut settings: Vec<SettingSpec> = base.settings().to_vec();
        let order = [("msb", "Most significant bit first"), ("lsb", "Least significant bit first")];
        let enc = [("unsigned", "Unsigned (straight binary)"), ("twos", "Two's complement (signed)")];
        if bus == Bus::Spi {
            settings.push(
                SettingSpec::choice(
                    "stream",
                    "Sample on",
                    "miso",
                    &[("miso", "MISO (from the ADC)"), ("mosi", "MOSI")],
                )
                .help("The line the ADC sends its result on, nearly always MISO."),
            );
        } else {
            settings.push(
                SettingSpec::number("address", "Address", -1.0)
                    .range(-1.0, 127.0, None, &[])
                    .help("The ADC's 7-bit address, such as 72 for 0x48; -1 takes reads from any address."),
            );
            settings.push(
                SettingSpec::number("register", "Register", -1.0).range(-1.0, 255.0, None, &[]).help(
                    "Only reads of this register (the pointer last written to the ADC); -1 takes every read.",
                ),
            );
        }
        settings.extend([
            SettingSpec::number("skipBits", "Bits before the sample", 0.0)
                .range(0.0, 256.0, None, &[])
                .help("Bits of the transfer to skip, counted from its first bit."),
            SettingSpec::number("bits", "Resolution (bits)", 12.0).range(
                1.0,
                32.0,
                None,
                &[8.0, 10.0, 12.0, 16.0, 24.0],
            ),
            SettingSpec::choice("order", "Bit order of the sample", "msb", &order),
            SettingSpec::choice("encoding", "Code", "unsigned", &enc)
                .help("Two's complement for bipolar ADCs; the most negative code is the lowest value."),
        ]);
        if bus == Bus::Spi {
            settings.extend([
                SettingSpec::choice(
                    "matchStream",
                    "Only transfers where",
                    "none",
                    &[
                        ("none", "Every transfer counts"),
                        ("mosi", "MOSI has a value"),
                        ("miso", "MISO has a value"),
                    ],
                )
                .help("For an ADC with several inputs: keep only the transfers that select this input."),
                SettingSpec::number("matchSkip", "Bits before the value", 0.0).range(0.0, 256.0, None, &[]),
                SettingSpec::number("matchBits", "Bits of the value", 0.0).range(0.0, 32.0, None, &[]),
                SettingSpec::number("matchValue", "Value", 0.0).range(0.0, 4_294_967_295.0, None, &[]),
            ]);
        }
        SerialAdc { bus, roles: base.roles().to_vec(), settings }
    }
}

impl Source for SerialAdc {
    fn id(&self) -> &str {
        match self.bus {
            Bus::Spi => "spi-adc",
            Bus::I2c => "i2c-adc",
        }
    }
    fn name(&self) -> &str {
        match self.bus {
            Bus::Spi => "ADC on SPI",
            Bus::I2c => "ADC on I2C",
        }
    }
    fn help(&self) -> &str {
        match self.bus {
            Bus::Spi => {
                "Samples an ADC sends over SPI: a bit field of every chip-select frame, at its start."
            }
            Bus::I2c => {
                "Samples read from an ADC over I2C: a bit field of every read, at the start of the transaction."
            }
        }
    }
    fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }
    fn settings(&self) -> &[SettingSpec] {
        &self.settings
    }
    fn output(&self, settings: &Settings) -> SourceOutput {
        let f = Field::of(settings);
        SourceOutput { bits: Some(f.bits), encoding: f.encoding, ..SourceOutput::fraction() }
    }
    fn create(
        &self,
        setup: &DecoderSetup,
        snapshot: Option<serde_json::Value>,
    ) -> Result<Box<dyn StreamDecoder>, String> {
        let base = self.bus.protocol();
        let lane = setup.channels.first().map_or(0, |c| c.channel);
        let field = Field::of(&setup.settings);
        let (base_snap, state, out) = match snapshot {
            Some(v) => {
                let s: Snapshot = serde_json::from_value(v)
                    .map_err(|e| format!("The saved ADC state cannot be read ({e})."))?;
                (Some(s.base), s.state, s.out)
            }
            None => (None, State::default(), Out::new(true)),
        };
        // the bus decoder sees its own settings only
        let mut base_setup = setup.clone();
        base_setup.settings = SettingSpec::with_defaults(base.settings(), &setup.settings);
        let decoder = base.decoder(&base_setup, base_snap)?;
        Ok(Box::new(Layer { bus: self.bus, field, base: decoder, state, out, lane }))
    }
}

#[cfg(test)]
mod tests {
    use logica_core::Level;
    use logica_core::analog::Point;

    use super::*;
    use crate::analog::testing::run;
    use crate::synth::{I2cTransfer, Lane, i2c_transaction, spi_frame};
    use crate::types::settings;

    #[test]
    fn bit_fields_from_the_first_bit_on() {
        assert_eq!(bit_field(&[0b1010_0000], 0, 3, false), Some(0b101));
        assert_eq!(bit_field(&[0x0F, 0xF0], 4, 8, false), Some(0xFF));
        assert_eq!(bit_field(&[0b0000_0110], 5, 3, true), Some(0b011));
        assert_eq!(bit_field(&[0xAB, 0xCD], 0, 16, false), Some(0xABCD));
        assert_eq!(bit_field(&[0xAB], 4, 5, false), None, "past the end");
        assert_eq!(bit_field(&[0xAB], 0, 0, false), None);
    }

    /// SPI mode 0 frames on CLK 0, CS 1, MOSI 2, MISO 3: `(start, mosi, miso)` each.
    fn spi(frames: &[(f64, Vec<u8>, Vec<u8>)], rate: f64) -> (Vec<(Level, Vec<Sample>)>, Sample) {
        let mut clk = Lane::low(rate);
        let mut cs = Lane::high(rate);
        let mut mosi = Lane::low(rate);
        let mut miso = Lane::low(rate);
        let mut end = 0.0;
        for (t, out, inp) in frames {
            end = spi_frame(
                &mut clk,
                Some(&mut cs),
                Some(&mut mosi),
                Some(&mut miso),
                *t,
                100e3,
                0,
                out,
                inp,
                true,
            );
        }
        let ch = |l: Lane| (l.initial, l.edges);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let end = (end * rate) as u64 + 100;
        (vec![ch(clk), ch(cs), ch(mosi), ch(miso)], end)
    }

    fn values(points: &[Point]) -> Vec<Option<f64>> {
        points.iter().map(|p| p.x).collect()
    }

    /// An MCP3201: 16 clocks, two sampling bits, a null bit, then 12 bits MSB first.
    fn mcp3201(code: u16) -> Vec<u8> {
        let w = code << 1;
        w.to_be_bytes().to_vec()
    }

    #[test]
    fn an_mcp3201_on_spi_in_any_batching() {
        let rate = 4e6;
        let codes = [0u16, 2048, 4095, 1234];
        let frames: Vec<_> = codes
            .iter()
            .enumerate()
            .map(|(k, &c)| (1e-4 + k as f64 * 5e-4, vec![0, 0], mcp3201(c)))
            .collect();
        let (ch, end) = spi(&frames, rate);
        let s = settings(&[("skipBits", 3.0.into()), ("bits", 12.0.into())]);
        let src = SerialAdc::new(Bus::Spi);
        let out = run(&src, rate, &ch, end, &s);
        let want: Vec<Option<f64>> = codes.iter().map(|&c| Some(f64::from(c) / 4096.0)).collect();
        assert_eq!(values(&out.points), want);
        // each point at the start of its frame, in order
        assert!(out.points.windows(2).all(|w| w[0].sample < w[1].sample));
        assert_eq!(out.notes, vec!["4 samples from SPI transfers."]);
        assert_eq!(src.output(&SettingSpec::with_defaults(src.settings(), &s)).bits, Some(12));
    }

    #[test]
    fn a_multiplexed_adc_keeps_one_input() {
        // MCP3208-like: MOSI selects the input in bits 7..9, MISO answers in bits 12..23
        let rate = 4e6;
        let frame = |k: usize, input: u8, code: u16| {
            let cmd = [0x06 | (input >> 2), (input & 3) << 6, 0];
            let [hi, lo] = code.to_be_bytes();
            let reply = [0, hi & 0x0F, lo];
            (1e-4 + k as f64 * 5e-4, cmd.to_vec(), reply.to_vec())
        };
        let frames = vec![frame(0, 0, 100), frame(1, 5, 3000), frame(2, 0, 200), frame(3, 5, 3100)];
        let (ch, end) = spi(&frames, rate);
        let s = settings(&[
            ("skipBits", 12.0.into()),
            ("bits", 12.0.into()),
            ("matchStream", "mosi".into()),
            ("matchSkip", 7.0.into()),
            ("matchBits", 3.0.into()),
            ("matchValue", 5.0.into()),
        ]);
        let out = run(&SerialAdc::new(Bus::Spi), rate, &ch, end, &s);
        assert_eq!(values(&out.points), vec![Some(3000.0 / 4096.0), Some(3100.0 / 4096.0)]);
        assert_eq!(
            out.notes,
            vec!["2 samples from SPI transfers; 2 transfers were for something else and were left out."]
        );
    }

    #[test]
    fn a_frame_too_short_for_the_sample_is_a_gap_with_a_reason() {
        let rate = 4e6;
        let frames = vec![
            (1e-4, vec![0, 0], mcp3201(1000)),
            (6e-4, vec![0], vec![0x55]),
            (11e-4, vec![0, 0], mcp3201(3000)),
        ];
        let (ch, end) = spi(&frames, rate);
        let s = settings(&[("skipBits", 3.0.into()), ("bits", 12.0.into())]);
        let out = run(&SerialAdc::new(Bus::Spi), rate, &ch, end, &s);
        assert_eq!(values(&out.points), vec![Some(1000.0 / 4096.0), None, Some(3000.0 / 4096.0)]);
        assert_eq!(out.frames.len(), 1);
        assert_eq!(out.frames[0].problems[0].code, ProblemCode::Incomplete);
        assert_eq!(
            out.frames[0].problems[0].message,
            "The transfer has 8 bits; the sample needs bits 3 to 14."
        );
    }

    #[test]
    fn an_ads1115_on_i2c_through_its_register_pointer() {
        // write the pointer (0 = conversion), then read two bytes, two's complement; a config
        // read (register 1) in between is left out, and a read that is not answered is a gap
        let rate = 4e6;
        let mut scl = Lane::high(rate);
        let mut sda = Lane::high(rate);
        let w = |data: Vec<u8>| I2cTransfer { address: 0x48, read: false, data, nack: false };
        let r = |data: Vec<u8>, nack: bool| I2cTransfer { address: 0x48, read: true, data, nack };
        let mut t = 1e-4;
        for x in [
            vec![w(vec![0]), r(vec![0x40, 0x00], false)],
            vec![w(vec![1]), r(vec![0x85, 0x83], false)],
            vec![w(vec![0]), r(vec![0xC0, 0x00], false)],
            vec![r(vec![], true)],
        ] {
            t = i2c_transaction(&mut scl, &mut sda, t, 100e3, &x) + 2e-4;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let end = (t * rate) as u64;
        let ch = vec![(scl.initial, scl.edges), (sda.initial, sda.edges)];
        let s = settings(&[
            ("address", 72.0.into()),
            ("register", 0.0.into()),
            ("bits", 16.0.into()),
            ("encoding", "twos".into()),
        ]);
        let out = run(&SerialAdc::new(Bus::I2c), rate, &ch, end, &s);
        // 0x4000 is a quarter of full scale above the middle; 0xC000 a quarter below
        assert_eq!(values(&out.points), vec![Some(0.75), Some(0.25), None]);
        assert_eq!(out.frames[0].problems[0].code, ProblemCode::Protocol);
        assert!(out.frames[0].problems[0].message.contains("did not answer"), "{:?}", out.frames);
    }

    #[test]
    fn spi_and_i2c_settings_come_first_and_roles_are_the_bus_roles() {
        let spi = SerialAdc::new(Bus::Spi);
        assert_eq!(
            spi.roles().iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            ["clk", "cs", "mosi", "miso"]
        );
        assert_eq!(spi.settings()[0].key, crate::spi::Spi::default().settings()[0].key);
        let i2c = SerialAdc::new(Bus::I2c);
        assert_eq!(i2c.roles().iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["scl", "sda"]);
        assert!(i2c.settings().iter().any(|s| s.key == "register"));
        assert!(!i2c.settings().iter().any(|s| s.key == "matchStream"));
    }
}
