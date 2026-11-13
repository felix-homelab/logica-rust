//! Errors of the capture store, worded for the person or agent who sees them.

use std::fmt;
use std::io;

use super::Sample;

#[derive(Debug)]
pub enum StoreError {
    /// Reading or writing the backing file failed.
    Io { context: String, source: io::Error },
    /// A blob in the backing file does not match what the store expects.
    Damaged { slot: usize, chunk: u64, reason: String },
    /// Data was handed over out of order, with a gap or an overlap.
    Order {
        slot: usize,
        expected: Sample,
        got: Sample,
    },
    /// The store has no channel with this slot.
    NoSuchChannel { slot: usize, channels: usize },
    /// A read past the samples written so far.
    OutOfRange { sample: Sample, count: Sample },
    /// Writing after `finish`.
    Finished,
    /// A malformed request (for example a packed unit size of 0).
    Invalid(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io { context, source } => write!(f, "{context}: {source}"),
            StoreError::Damaged { slot, chunk, reason } => write!(
                f,
                "Chunk {chunk} of channel {slot} is damaged ({reason}); the capture cannot be read there."
            ),
            StoreError::Order { slot, expected, got } => write!(
                f,
                "Data for channel {slot} must continue at sample {expected}, but it starts at sample {got}."
            ),
            StoreError::NoSuchChannel { slot, channels } => {
                write!(
                    f,
                    "There is no channel {slot}; this capture has {channels} channel(s)."
                )
            }
            StoreError::OutOfRange { sample, count } => write!(
                f,
                "Sample {sample} is past the end of the capture, which has {count} sample(s) so far."
            ),
            StoreError::Finished => write!(f, "The capture is complete; no more data can be added."),
            StoreError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type StoreResult<T> = Result<T, StoreError>;
