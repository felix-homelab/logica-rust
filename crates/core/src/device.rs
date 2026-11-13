//! Device and driver traits (doc 05, ADR 0003). Drivers depend on `core` alone and hand captured
//! data to a [`BlockSink`] in one of three shapes (doc 04, "Ingest"); [`StoreSink`] writes them
//! into a capture store. Capabilities are data, so the UI and MCP can show valid choices and
//! reject impossible ones before touching hardware.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::capture::{CaptureMeta, CaptureState, ChannelInfo, Note, NoteLevel, Source, SourceKind, whole};
use crate::import::StoreTarget;
use crate::store::{CaptureStore, Level, Sample};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Stable id, such as "synthetic/mix" or a USB serial.
    pub id: String,
    /// Driver id: "synthetic", "fnirsi-dla", "fx2lafw".
    pub driver: String,
    pub model: String,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    pub channels: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureMode {
    /// Record to device memory, then download.
    Buffer,
    /// Samples arrive while capturing.
    Stream,
}

/// The fastest rate with up to `max_channels` channels enabled.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    pub max_channels: u32,
    pub max_rate_hz: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Physical channel numbers.
    pub channels: Vec<u32>,
    /// Rates offered.
    pub rates_hz: Vec<f64>,
    /// Rate limits by number of enabled channels, fewest channels first.
    pub limits: Vec<RateLimit>,
    /// Most samples per channel in one capture, where limited (buffer depth).
    pub max_samples: Option<u64>,
    pub modes: Vec<CaptureMode>,
    /// Adjustable threshold: lowest, highest and step in volts.
    pub threshold: Option<(f64, f64, f64)>,
    /// Outputs (PWM); added with the first device that has them (M3).
    pub outputs: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureConfig {
    /// Enabled physical channels, ascending.
    pub channels: Vec<u32>,
    pub sample_rate_hz: f64,
    /// Samples per channel.
    pub samples: Sample,
    pub mode: CaptureMode,
    pub threshold_volts: Option<f64>,
}

fn rate_text(hz: f64) -> String {
    if hz >= 1e6 { format!("{} MS/s", hz / 1e6) } else { format!("{} kS/s", hz / 1e3) }
}

impl Capabilities {
    /// Checks a configuration and explains what is impossible, before any hardware is touched.
    pub fn check(&self, c: &CaptureConfig) -> Result<(), DeviceError> {
        if c.channels.is_empty() {
            return Err(DeviceError::new("Enable at least one channel."));
        }
        if let Some(ch) = c.channels.iter().find(|ch| !self.channels.contains(ch)) {
            return Err(DeviceError::new(format!("This device has no channel {ch}.")));
        }
        if c.samples == 0 {
            return Err(DeviceError::new("Choose how many samples to capture."));
        }
        if let Some(max) = self.max_samples
            && c.samples > max
        {
            return Err(DeviceError::new(format!(
                "This device holds at most {max} samples per channel; choose fewer."
            )));
        }
        if !self.modes.contains(&c.mode) {
            return Err(DeviceError::new("This device cannot capture in that mode."));
        }
        if !self.rates_hz.iter().any(|&r| (r - c.sample_rate_hz).abs() <= r * 1e-9) {
            return Err(DeviceError::new(format!(
                "{} is not a rate this device offers. Choose one of: {}.",
                rate_text(c.sample_rate_hz),
                self.rates_hz.iter().map(|&r| rate_text(r)).collect::<Vec<_>>().join(", ")
            )));
        }
        let n = u32::try_from(c.channels.len()).unwrap_or(u32::MAX);
        if let Some(limit) = self.limits.iter().find(|l| n <= l.max_channels)
            && c.sample_rate_hz > limit.max_rate_hz
        {
            return Err(DeviceError::new(format!(
                "{} is not available with {n} channels; the maximum is {}. Disable channels or lower the rate.",
                rate_text(c.sample_rate_hz),
                rate_text(limit.max_rate_hz)
            )));
        }
        if let Some(v) = c.threshold_volts {
            match self.threshold {
                None => return Err(DeviceError::new("This device has no adjustable threshold.")),
                Some((lo, hi, _)) if v < lo || v > hi => {
                    return Err(DeviceError::new(format!(
                        "The threshold must be between {lo} V and {hi} V."
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// A device problem, worded for the person or agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceError(pub String);

impl DeviceError {
    pub fn new(m: impl Into<String>) -> Self {
        DeviceError(m.into())
    }
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeviceError {}

/// Captured data as a driver hands it over (doc 04, "Ingest"). Slots follow the enabled channels.
#[derive(Clone, Copy, Debug)]
pub enum Block<'a> {
    /// `unit_size` bytes per sample, little-endian; enabled channel slot k on bit `bits[k]`.
    Packed { start: Sample, unit_size: usize, data: &'a [u8], bits: &'a [u32] },
    /// `count` samples of one channel as bits, least significant first (FNIRSI style).
    Bits { slot: usize, start: Sample, words: &'a [u64], count: u64 },
    /// Level changes of one channel (synthetic device, run-length data).
    Transitions { slot: usize, edges: &'a [Sample] },
    /// Every channel is known up to `end`.
    Advance { end: Sample },
}

/// Where a device's data goes.
pub trait BlockSink {
    /// Called once before any block: the enabled channels, their levels at sample 0 (where the
    /// device knows them) and the rate.
    fn begin(
        &mut self,
        channels: &[ChannelInfo],
        initial: &[Level],
        sample_rate_hz: f64,
    ) -> Result<(), DeviceError>;
    fn block(&mut self, b: Block<'_>) -> Result<(), DeviceError>;
    /// Something worth telling the person: dropped data, a replayed stretch, a gap.
    fn note(&mut self, note: Note);
}

/// Asks a running capture to stop. Cheap to clone; shared between the engine and the driver.
#[derive(Clone, Debug, Default)]
pub struct StopSignal(Arc<AtomicBool>);

impl StopSignal {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn stop(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutcome {
    /// Samples per channel captured.
    pub samples: Sample,
    /// Whether the capture was stopped before its configured length.
    pub stopped: bool,
}

/// An opened device.
pub trait Device: Send {
    fn info(&self) -> &DeviceInfo;
    fn capabilities(&self) -> &Capabilities;
    /// Accepts a configuration or explains why it is impossible.
    fn configure(&mut self, config: &CaptureConfig) -> Result<(), DeviceError>;
    /// Captures with the last configuration until done or stopped.
    fn run(&mut self, sink: &mut dyn BlockSink, stop: &StopSignal) -> Result<RunOutcome, DeviceError>;
    fn close(&mut self);
}

/// A driver without a transport (the synthetic device).
pub trait VirtualDriver: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn enumerate(&self) -> Vec<DeviceInfo>;
    fn open(&self, id: &str) -> Result<Box<dyn Device>, DeviceError>;
}

/// USB devices a driver may handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbFilter {
    pub vendor_id: u16,
    pub product_id: Option<u16>,
}

/// A USB control request (bmRequestType, bRequest, wValue, wIndex).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRequest {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferDone {
    /// Id returned when the transfer was submitted.
    pub id: u64,
    pub data: Vec<u8>,
}

/// USB access for drivers (ADR 0003): the real library (chosen in spike S3), record and replay.
/// Drivers never use a USB library directly. Provisional: spike S3 settles the details.
pub trait Transport: Send {
    fn control_in(
        &mut self,
        req: ControlRequest,
        len: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, DeviceError>;
    fn control_out(&mut self, req: ControlRequest, data: &[u8], timeout: Duration)
    -> Result<(), DeviceError>;
    /// Queues a bulk-in transfer; several may be outstanding. Returns its id.
    fn submit_bulk_in(&mut self, endpoint: u8, len: usize) -> Result<u64, DeviceError>;
    fn submit_bulk_out(&mut self, endpoint: u8, data: Vec<u8>) -> Result<u64, DeviceError>;
    /// The next finished transfer, in submission order.
    fn reap(&mut self, timeout: Duration) -> Result<TransferDone, DeviceError>;
    fn cancel_all(&mut self);
}

/// A driver for USB devices.
pub trait UsbDriver: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn filters(&self) -> &[UsbFilter];
    /// Identifies the device behind `transport`, or says it is not ours (`None`).
    fn probe(&self, transport: &mut dyn Transport) -> Result<Option<DeviceInfo>, DeviceError>;
    fn open(&self, transport: Box<dyn Transport>) -> Result<Box<dyn Device>, DeviceError>;
}

/// Writes a device's blocks into a capture store (a working `.lacap` or memory).
pub struct StoreSink {
    target: StoreTarget,
    store: Option<CaptureStore>,
    channels: Vec<ChannelInfo>,
    rate: f64,
    notes: Vec<Note>,
}

impl StoreSink {
    pub fn new(target: StoreTarget) -> Self {
        StoreSink { target, store: None, channels: Vec::new(), rate: 0.0, notes: Vec::new() }
    }

    /// The store while capturing, for live views.
    pub fn store(&self) -> Option<&CaptureStore> {
        self.store.as_ref()
    }

    /// Ends the capture at `outcome.samples`; returns the store and its metadata.
    pub fn finish(
        mut self,
        outcome: &RunOutcome,
        source: Source,
        title: &str,
    ) -> Result<(CaptureStore, CaptureMeta), DeviceError> {
        let mut store = self.store.take().ok_or_else(|| DeviceError::new("The device delivered no data."))?;
        let end = outcome.samples.max(store.sample_count());
        store.finish(end).map_err(|e| DeviceError::new(e.to_string()))?;
        if outcome.stopped {
            self.notes.push(Note {
                level: NoteLevel::Info,
                message: format!("The capture was stopped after {end} samples."),
            });
        }
        let meta = CaptureMeta {
            id: title.to_string(),
            title: title.to_string(),
            source,
            sample_rate_hz: self.rate,
            trigger: None,
            channels: self.channels,
            segments: whole(end),
            notes: self.notes,
            state: if outcome.stopped { CaptureState::Aborted } else { CaptureState::Complete },
            artefact: None,
            saved_decoders: Vec::new(),
        };
        Ok((store, meta))
    }

    fn store_mut(&mut self) -> Result<&mut CaptureStore, DeviceError> {
        self.store
            .as_mut()
            .ok_or_else(|| DeviceError::new("The device sent data before it began the capture."))
    }
}

impl BlockSink for StoreSink {
    fn begin(
        &mut self,
        channels: &[ChannelInfo],
        initial: &[Level],
        sample_rate_hz: f64,
    ) -> Result<(), DeviceError> {
        let store = self.target.create(initial).map_err(|e| DeviceError::new(e.to_string()))?;
        self.store = Some(store);
        self.channels = channels.to_vec();
        self.rate = sample_rate_hz;
        Ok(())
    }

    fn block(&mut self, b: Block<'_>) -> Result<(), DeviceError> {
        let s = self.store_mut()?;
        let r = match b {
            Block::Packed { start, unit_size, data, bits } => s.push_packed(start, unit_size, data, bits),
            Block::Bits { slot, start, words, count } => s.push_bits(slot, start, words, count),
            Block::Transitions { slot, edges } => s.push_edges(slot, edges),
            Block::Advance { end } => s.advance_to(end),
        };
        r.map_err(|e| DeviceError::new(e.to_string()))
    }

    fn note(&mut self, note: Note) {
        self.notes.push(note);
    }
}

/// Where a capture came from, for device captures.
pub fn device_source(info: &DeviceInfo) -> Source {
    Source {
        kind: if info.driver == "synthetic" { SourceKind::Synthetic } else { SourceKind::Device },
        detail: info.model.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> Capabilities {
        Capabilities {
            channels: (0..32).collect(),
            rates_hz: vec![100e6, 250e6, 500e6, 1e9],
            limits: vec![
                RateLimit { max_channels: 8, max_rate_hz: 1e9 },
                RateLimit { max_channels: 16, max_rate_hz: 500e6 },
                RateLimit { max_channels: 32, max_rate_hz: 250e6 },
            ],
            max_samples: Some(1 << 27),
            modes: vec![CaptureMode::Buffer],
            threshold: Some((-5.0, 5.0, 0.1)),
            outputs: 0,
        }
    }
    fn cfg(n: u32, rate: f64) -> CaptureConfig {
        CaptureConfig {
            channels: (0..n).collect(),
            sample_rate_hz: rate,
            samples: 1000,
            mode: CaptureMode::Buffer,
            threshold_volts: None,
        }
    }

    #[test]
    fn impossible_configurations_are_explained() {
        let c = caps();
        assert!(c.check(&cfg(8, 1e9)).is_ok());
        assert_eq!(
            c.check(&cfg(20, 500e6)).unwrap_err().0,
            "500 MS/s is not available with 20 channels; the maximum is 250 MS/s. Disable channels or lower the rate."
        );
        assert!(c.check(&cfg(8, 300e6)).unwrap_err().0.contains("not a rate this device offers"));
        assert!(c.check(&cfg(0, 1e9)).unwrap_err().0.contains("at least one channel"));
        let mut x = cfg(4, 1e9);
        x.channels = vec![40];
        assert_eq!(c.check(&x).unwrap_err().0, "This device has no channel 40.");
        x = cfg(4, 1e9);
        x.samples = 1 << 28;
        assert!(c.check(&x).unwrap_err().0.contains("at most"));
        x = cfg(4, 1e9);
        x.mode = CaptureMode::Stream;
        assert!(c.check(&x).is_err());
        x = cfg(4, 1e9);
        x.threshold_volts = Some(7.0);
        assert!(c.check(&x).unwrap_err().0.contains("between -5 V and 5 V"));
    }

    #[test]
    fn a_store_sink_writes_blocks_into_a_store() {
        let mut sink = StoreSink::new(StoreTarget::memory());
        assert!(sink.block(Block::Advance { end: 5 }).is_err(), "data before begin");
        let ch = |i| ChannelInfo {
            index: i,
            name: format!("CH{i}"),
            colour: None,
            enabled: true,
            threshold_volts: None,
        };
        sink.begin(&[ch(0), ch(3)], &[Level::Low, Level::High], 1e6).unwrap();
        sink.block(Block::Transitions { slot: 0, edges: &[10, 20] }).unwrap();
        sink.block(Block::Packed { start: 0, unit_size: 1, data: &[], bits: &[0, 1] }).unwrap();
        sink.block(Block::Advance { end: 30 }).unwrap();
        sink.block(Block::Bits { slot: 1, start: 30, words: &[0], count: 10 }).unwrap();
        sink.block(Block::Advance { end: 40 }).unwrap();
        let (store, meta) = sink
            .finish(
                &RunOutcome { samples: 40, stopped: true },
                Source { kind: SourceKind::Synthetic, detail: "t".into() },
                "t",
            )
            .unwrap();
        assert_eq!(store.sample_count(), 40);
        assert_eq!(store.edges_in(0, 0, 40).unwrap(), vec![10, 20]);
        assert_eq!(store.edges_in(1, 0, 40).unwrap(), vec![30]);
        assert_eq!(meta.state, CaptureState::Aborted);
        assert_eq!(meta.channels[1].index, 3);
        assert!(meta.notes[0].message.contains("stopped after 40 samples"));
    }
}
