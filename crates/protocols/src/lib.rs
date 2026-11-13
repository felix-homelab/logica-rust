//! Logica's protocol engine (doc 06): bus grouping, detectors, streaming decoders, signal
//! generators, the byte analyser and custom JSON protocols. No I/O; builds for WebAssembly.
//!
//! Signal Detective (`../signal-detective`) is the behavioural reference (decision D12): its
//! heuristics, evidence texts and knowledge are followed; its code is not ported.

pub mod analyse;
pub mod buses;
pub mod bytes;
pub mod calc;
pub mod format;
pub mod i2c;
pub mod num;
pub mod protocol;
pub mod pwm;
pub mod spi;
pub mod stream;
pub mod types;
pub mod uart;
pub mod window;

use std::sync::Arc;

pub use protocol::{Protocol, Registry};

/// Every built-in protocol, in the order of the protocol list.
pub fn built_in() -> Vec<Arc<dyn Protocol>> {
    vec![
        Arc::new(uart::Uart::default()),
        Arc::new(uart::UartInverted::default()),
        Arc::new(i2c::I2c::default()),
        Arc::new(spi::Spi::default()),
        Arc::new(pwm::Pwm::default()),
    ]
}

/// The registry of built-in protocols.
pub fn registry() -> Registry {
    Registry::new(built_in(), Vec::new())
}
