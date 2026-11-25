//! Live capture (doc 04 "Ingest", doc 08 "live view while capturing"). A device runs on its own
//! thread and hands its blocks to a [`RelaySink`], which passes owned copies over a bounded
//! channel. The engine applies them to the capture's store whenever it polls, so the capture is
//! in the session, readable and decodable from its first block, and the store keeps one owner.
//! The bound holds memory steady: a device faster than the engine waits.

use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use logica_core::capture::{ChannelInfo, Note};
use logica_core::device::{Block, BlockSink, DeviceError};
use logica_core::store::{CaptureStore, StoreResult};
use logica_core::{Level, Sample};

/// Deliveries in flight between a device and the engine.
const QUEUE: usize = 256;

/// What a device delivered, owned so it can cross threads.
pub enum Delivery {
    Begin { channels: Vec<ChannelInfo>, initial: Vec<Level>, sample_rate_hz: f64 },
    Packed { start: Sample, unit_size: usize, data: Vec<u8>, bits: Vec<u32> },
    Bits { slot: usize, start: Sample, words: Vec<u64>, count: u64 },
    Transitions { slot: usize, edges: Vec<Sample> },
    Advance { end: Sample },
    Note(Note),
}

impl Delivery {
    /// Writes a data delivery into `store` (not `Begin` or `Note`).
    pub fn apply(self, store: &mut CaptureStore) -> StoreResult<()> {
        match self {
            Delivery::Packed { start, unit_size, data, bits } => {
                store.push_packed(start, unit_size, &data, &bits)
            }
            Delivery::Bits { slot, start, words, count } => store.push_bits(slot, start, &words, count),
            Delivery::Transitions { slot, edges } => store.push_edges(slot, &edges),
            Delivery::Advance { end } => store.advance_to(end),
            Delivery::Begin { .. } | Delivery::Note(_) => Ok(()),
        }
    }
}

/// The device side of the relay.
pub struct RelaySink {
    tx: SyncSender<Delivery>,
}

pub fn relay() -> (RelaySink, Receiver<Delivery>) {
    let (tx, rx) = sync_channel(QUEUE);
    (RelaySink { tx }, rx)
}

impl RelaySink {
    fn send(&self, d: Delivery) -> Result<(), DeviceError> {
        // a full queue waits for the engine; a closed one means the capture is gone
        match self.tx.try_send(d) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(d)) => self
                .tx
                .send(d)
                .map_err(|_| DeviceError::new("The capture was closed while the device was sending.")),
            Err(TrySendError::Disconnected(_)) => {
                Err(DeviceError::new("The capture was closed while the device was sending."))
            }
        }
    }
}

impl BlockSink for RelaySink {
    fn begin(
        &mut self,
        channels: &[ChannelInfo],
        initial: &[Level],
        sample_rate_hz: f64,
    ) -> Result<(), DeviceError> {
        self.send(Delivery::Begin { channels: channels.to_vec(), initial: initial.to_vec(), sample_rate_hz })
    }

    fn block(&mut self, b: Block<'_>) -> Result<(), DeviceError> {
        self.send(match b {
            Block::Packed { start, unit_size, data, bits } => {
                Delivery::Packed { start, unit_size, data: data.to_vec(), bits: bits.to_vec() }
            }
            Block::Bits { slot, start, words, count } => {
                Delivery::Bits { slot, start, words: words.to_vec(), count }
            }
            // a tile without edges on a channel needs no message: the next advance covers it
            Block::Transitions { edges: [], .. } => return Ok(()),
            Block::Transitions { slot, edges } => Delivery::Transitions { slot, edges: edges.to_vec() },
            Block::Advance { end } => Delivery::Advance { end },
        })
    }

    fn note(&mut self, note: Note) {
        let _ = self.send(Delivery::Note(note));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logica_core::capture::NoteLevel;
    use std::sync::Arc;

    #[test]
    fn deliveries_rebuild_the_device_data() {
        let (mut sink, rx) = relay();
        let ch = |i| ChannelInfo {
            index: i,
            name: format!("CH{i}"),
            colour: None,
            enabled: true,
            threshold_volts: None,
        };
        sink.begin(&[ch(0), ch(2)], &[Level::Low, Level::High], 1e6).unwrap();
        sink.block(Block::Transitions { slot: 0, edges: &[3, 8] }).unwrap();
        sink.block(Block::Transitions { slot: 1, edges: &[] }).unwrap();
        sink.block(Block::Advance { end: 10 }).unwrap();
        sink.block(Block::Bits { slot: 1, start: 10, words: &[0], count: 6 }).unwrap();
        sink.block(Block::Packed { start: 10, unit_size: 1, data: &[1; 6], bits: &[0, 1] }).unwrap();
        sink.note(Note { level: NoteLevel::Info, message: "n".into() });
        drop(sink);
        let all: Vec<Delivery> = rx.iter().collect();
        assert!(matches!(&all[0], Delivery::Begin { channels, .. } if channels.len() == 2));
        assert_eq!(all.len(), 6, "the empty edge list is not sent");
        let mut store = CaptureStore::new(
            &[Level::Low, Level::High],
            Arc::new(logica_core::io::MemFile::new()),
            16,
            1 << 20,
        );
        let mut it = all.into_iter().skip(1);
        for d in it.by_ref().take(3) {
            d.apply(&mut store).unwrap();
        }
        store.advance_to(16).unwrap();
        assert_eq!(store.edges_in(0, 0, 16).unwrap(), vec![3, 8]);
        assert_eq!(store.edges_in(1, 0, 16).unwrap(), vec![10], "high, then six low samples");
    }

    #[test]
    fn a_closed_capture_stops_the_device() {
        let (mut sink, rx) = relay();
        drop(rx);
        let e = sink.block(Block::Advance { end: 1 }).unwrap_err();
        assert!(e.0.contains("closed"), "{e}");
    }
}
