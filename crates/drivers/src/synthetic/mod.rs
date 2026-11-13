//! The synthetic device (doc 05): plays a program, repeated to the requested length — every
//! generated protocol example, or `mix` (UART, I2C, SPI and PWM on 8 channels at 100 MHz), or
//! `mix-dense` (the same with a dense 25 MHz stretch). It streams one tile at a time, so 5e9
//! samples run in bounded memory. Like real hardware, a capture is exactly the requested length.

use logica_core::capture::ChannelInfo;
use logica_core::device::{
    Block, BlockSink, Capabilities, CaptureConfig, CaptureMode, Device, DeviceError, DeviceInfo, RunOutcome,
    StopSignal, VirtualDriver,
};
use logica_core::{Level, Sample};
use logica_protocols::examples::EXAMPLES;
use logica_protocols::synth::*;

/// Rate of the `mix` programs.
pub const MIX_RATE: f64 = 100e6;
/// Length of one `mix` tile: 10 ms.
pub const MIX_TILE: Sample = 1_000_000;
/// The dense stretch of `mix-dense`: a 25 MHz square wave on channel 7 from 1 s for 0.45 s.
pub const DENSE_START: Sample = 100_000_000;
pub const DENSE_LEN: Sample = 45_000_000;

const COLOURS: [&str; 4] = ["#e54242", "#ff7f00", "#007fff", "#26ad34"];

/// One repetition of a program.
#[derive(Clone, Debug)]
pub struct Program {
    pub name: String,
    pub title: String,
    pub tile: GenCapture,
    /// A 25 MHz stretch on the last channel: start and length in samples.
    pub dense: Option<(Sample, Sample)>,
}

fn mix_tile() -> GenCapture {
    let r = MIX_RATE;
    let mut tx = Lane::high(r);
    uart_send(&mut tx, 0.0005, &text_bytes("logica mix\r\n"), UartOptions::baud(115_200.0));
    let (mut scl, mut sda) = (Lane::high(r), Lane::high(r));
    i2c_transaction(
        &mut scl,
        &mut sda,
        0.002,
        400e3,
        &[I2cTransfer { address: 0x50, read: false, data: vec![0x00, 0x10, 0x42], nack: false }],
    );
    let (mut clk, mut cs, mut mosi) = (Lane::low(r), Lane::high(r), Lane::low(r));
    spi_frame(
        &mut clk,
        Some(&mut cs),
        Some(&mut mosi),
        None,
        0.004,
        1e6,
        0,
        &[0x9f, 0x00, 0x00, 0x00],
        &[],
        true,
    );
    let mut pwm = Lane::low(r);
    pwm_signal(&mut pwm, 0.0, 0.01, 1000.0, 0.25);
    let mut g = make_capture(
        "mix",
        r,
        0.01,
        vec![
            (0, "TX", tx),
            (1, "SCL", scl),
            (2, "SDA", sda),
            (3, "CLK", clk),
            (4, "CS", cs),
            (5, "MOSI", mosi),
            (6, "PWM", pwm),
            (7, "FAST", Lane::low(r)),
        ],
    );
    g.sample_count = MIX_TILE;
    g
}

/// Every program: the generated examples, `mix` and `mix-dense`.
pub fn programs() -> Vec<Program> {
    let mut out = vec![
        Program {
            name: "mix".into(),
            title: "UART, I2C, SPI and PWM on 8 channels at 100 MHz".into(),
            tile: mix_tile(),
            dense: None,
        },
        Program {
            name: "mix-dense".into(),
            title: "UART, I2C, SPI and PWM at 100 MHz, with a 25 MHz stretch on channel 7".into(),
            tile: mix_tile(),
            dense: Some((DENSE_START, DENSE_LEN)),
        },
    ];
    for e in &EXAMPLES {
        out.push(Program { name: e.id.into(), title: e.title.into(), tile: (e.build)(), dense: None });
    }
    out
}

pub struct SyntheticDriver;

impl VirtualDriver for SyntheticDriver {
    fn id(&self) -> &str {
        "synthetic"
    }
    fn name(&self) -> &str {
        "Synthetic device"
    }
    fn enumerate(&self) -> Vec<DeviceInfo> {
        programs().iter().map(info).collect()
    }
    fn open(&self, id: &str) -> Result<Box<dyn Device>, DeviceError> {
        let name = id.strip_prefix("synthetic/").unwrap_or(id);
        let p = programs().into_iter().find(|p| p.name == name).ok_or_else(|| {
            DeviceError::new(format!(
                "There is no synthetic program \"{name}\". Choose one of: {}.",
                programs().iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")
            ))
        })?;
        Ok(Box::new(SyntheticDevice::new(p)))
    }
}

fn info(p: &Program) -> DeviceInfo {
    DeviceInfo {
        id: format!("synthetic/{}", p.name),
        driver: "synthetic".into(),
        model: format!("Synthetic device: {}", p.title),
        serial: None,
        firmware: None,
        channels: u32::try_from(p.tile.channels.len()).unwrap_or(u32::MAX),
    }
}

pub struct SyntheticDevice {
    program: Program,
    info: DeviceInfo,
    caps: Capabilities,
    config: Option<CaptureConfig>,
}

impl SyntheticDevice {
    pub fn new(program: Program) -> Self {
        let caps = Capabilities {
            channels: program.tile.channels.iter().map(|c| c.0).collect(),
            rates_hz: vec![program.tile.rate],
            limits: Vec::new(),
            max_samples: None,
            modes: vec![CaptureMode::Stream, CaptureMode::Buffer],
            threshold: None,
            outputs: 0,
        };
        SyntheticDevice { info: info(&program), program, caps, config: None }
    }

    /// Edges of tile `k` of channel `c` (program order), absolute samples below `end`.
    fn tile_edges(&self, c: usize, base: Sample, end: Sample, out: &mut Vec<Sample>) {
        let lane = &self.program.tile.channels[c].2;
        out.extend(lane.edges.iter().map(|&e| base + e).take_while(|&e| e < end));
        if let Some((d0, len)) = self.program.dense
            && c == self.program.tile.channels.len() - 1
        {
            let from = d0.max(base);
            let to = (d0 + len).min(end);
            let mut s = from + (from % 2);
            while s < to {
                out.push(s);
                s += 2;
            }
        }
    }
}

impl Device for SyntheticDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn configure(&mut self, config: &CaptureConfig) -> Result<(), DeviceError> {
        self.caps.check(config).map_err(|e| {
            if config.sample_rate_hz != self.program.tile.rate {
                DeviceError::new(format!(
                    "The synthetic program \"{}\" runs at {} MHz; choose that rate.",
                    self.program.name,
                    self.program.tile.rate / 1e6
                ))
            } else {
                e
            }
        })?;
        self.config = Some(config.clone());
        Ok(())
    }
    fn run(&mut self, sink: &mut dyn BlockSink, stop: &StopSignal) -> Result<RunOutcome, DeviceError> {
        let cfg =
            self.config.clone().ok_or_else(|| DeviceError::new("Configure the device before capturing."))?;
        let tile = &self.program.tile;
        let enabled: Vec<usize> = tile
            .channels
            .iter()
            .enumerate()
            .filter(|(_, c)| cfg.channels.contains(&c.0))
            .map(|(i, _)| i)
            .collect();
        let infos: Vec<ChannelInfo> = enabled
            .iter()
            .map(|&i| {
                let (index, name, _) = &tile.channels[i];
                ChannelInfo {
                    index: *index,
                    name: name.clone(),
                    colour: Some(COLOURS[*index as usize % 4].into()),
                    enabled: true,
                    threshold_volts: None,
                }
            })
            .collect();
        let initial: Vec<Level> = enabled.iter().map(|&i| tile.channels[i].2.initial).collect();
        sink.begin(&infos, &initial, tile.rate)?;
        let len = tile.sample_count.max(1);
        // level of each enabled channel where the last tile ended
        let mut level = initial.clone();
        let mut base: Sample = 0;
        let mut edges = Vec::new();
        let mut stopped = false;
        while base < cfg.samples {
            if stop.is_stopped() {
                stopped = true;
                break;
            }
            let end = (base + len).min(cfg.samples);
            for (slot, &c) in enabled.iter().enumerate() {
                edges.clear();
                // a tile starts at the program's initial level
                if base > 0 && level[slot] != initial[slot] {
                    edges.push(base);
                }
                self.tile_edges(c, base, end, &mut edges);
                // two changes on one sample cancel out
                let mut kept: Vec<Sample> = Vec::with_capacity(edges.len());
                for &e in &edges {
                    if kept.last() == Some(&e) {
                        kept.pop();
                    } else {
                        kept.push(e);
                    }
                }
                edges = kept;
                if edges.len() % 2 == 1 {
                    level[slot] = !level[slot];
                }
                sink.block(Block::Transitions { slot, edges: &edges })?;
            }
            sink.block(Block::Advance { end })?;
            base = end;
        }
        Ok(RunOutcome { samples: base, stopped })
    }
    fn close(&mut self) {
        self.config = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logica_core::capture::{Source, SourceKind};
    use logica_core::device::StoreSink;
    use logica_core::import::StoreTarget;

    fn capture(
        program: &str,
        samples: Sample,
        channels: Option<Vec<u32>>,
    ) -> (logica_core::store::CaptureStore, logica_core::capture::CaptureMeta) {
        let mut d = SyntheticDriver.open(&format!("synthetic/{program}")).unwrap();
        let chans = channels.unwrap_or_else(|| d.capabilities().channels.clone());
        let rate = d.capabilities().rates_hz[0];
        d.configure(&CaptureConfig {
            channels: chans,
            sample_rate_hz: rate,
            samples,
            mode: CaptureMode::Stream,
            threshold_volts: None,
        })
        .unwrap();
        let mut sink = StoreSink::new(StoreTarget::memory());
        let out = d.run(&mut sink, &StopSignal::new()).unwrap();
        sink.finish(&out, Source { kind: SourceKind::Synthetic, detail: "test".into() }, program).unwrap()
    }

    #[test]
    fn lists_every_program() {
        let ids: Vec<String> = SyntheticDriver.enumerate().into_iter().map(|d| d.id).collect();
        assert!(ids.contains(&"synthetic/mix".to_string()) && ids.contains(&"synthetic/can".to_string()));
        assert_eq!(ids.len(), 2 + EXAMPLES.len());
        assert!(
            SyntheticDriver
                .open("synthetic/nope")
                .err()
                .unwrap()
                .0
                .starts_with("There is no synthetic program \"nope\"")
        );
    }

    #[test]
    fn a_capture_is_exactly_the_requested_length_and_repeats_the_program() {
        let (s, meta) = capture("mix", 2 * MIX_TILE + 12_345, None);
        assert_eq!(s.sample_count(), 2 * MIX_TILE + 12_345);
        assert_eq!(meta.channels.len(), 8);
        let tile0 = s.edges_in(0, 0, MIX_TILE).unwrap();
        let tile1: Vec<Sample> =
            s.edges_in(0, MIX_TILE, 2 * MIX_TILE).unwrap().iter().map(|e| e - MIX_TILE).collect();
        assert!(!tile0.is_empty());
        assert_eq!(tile0, tile1, "the UART line repeats");
        // PWM: 1 kHz, 25 % duty from sample 0 (data at sample 0 sets the initial level)
        assert_eq!(s.initial_level(6).unwrap(), Level::High);
        assert_eq!(s.edges_in(6, 0, 300_000).unwrap(), [25_000, 100_000, 125_000, 200_000, 225_000]);
    }

    #[test]
    fn the_dense_stretch_is_stored_as_raw_bits() {
        let (s, _) = capture("mix-dense", DENSE_START + 3 * (1 << 24), Some(vec![7]));
        let kinds: Vec<_> = s.chunks(0).unwrap().iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&logica_core::store::chunk::ChunkKind::Bits), "{kinds:?}");
        assert_eq!(s.transitions_in(0, DENSE_START, DENSE_START + 1000).unwrap(), 500);
        assert_eq!(s.transitions_in(0, 0, DENSE_START).unwrap(), 0);
    }

    #[test]
    fn every_example_program_plays_back_its_example() {
        for e in &EXAMPLES {
            let g = (e.build)();
            let (s, _) = capture(e.id, g.sample_count, None);
            for (slot, (_, _, lane)) in g.channels.iter().enumerate() {
                let want: Vec<Sample> = lane.edges.iter().copied().filter(|&x| x > 0).collect();
                assert_eq!(s.edges_in(slot, 0, g.sample_count).unwrap(), want, "{} slot {slot}", e.id);
            }
        }
    }

    #[test]
    fn wrong_configurations_and_stopping() {
        let mut d = SyntheticDriver.open("synthetic/mix").unwrap();
        let bad = CaptureConfig {
            channels: vec![0],
            sample_rate_hz: 1e6,
            samples: 10,
            mode: CaptureMode::Stream,
            threshold_volts: None,
        };
        assert_eq!(
            d.configure(&bad).unwrap_err().0,
            "The synthetic program \"mix\" runs at 100 MHz; choose that rate."
        );
        let mut sink = StoreSink::new(StoreTarget::memory());
        assert!(d.run(&mut sink, &StopSignal::new()).is_err(), "not configured");
        d.configure(&CaptureConfig { sample_rate_hz: MIX_RATE, samples: 10 * MIX_TILE, ..bad }).unwrap();
        let stop = StopSignal::new();
        stop.stop();
        let out = d.run(&mut sink, &stop).unwrap();
        assert_eq!(out, RunOutcome { samples: 0, stopped: true });
    }
}
