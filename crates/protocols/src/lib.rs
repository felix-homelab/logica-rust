//! Logica's protocol engine (doc 06): bus grouping, detectors, streaming decoders, signal
//! generators, the byte analyser and custom JSON protocols. No I/O; builds for WebAssembly.
//!
//! Signal Detective (`../signal-detective`) is the behavioural reference (decision D12): its
//! heuristics, evidence texts and knowledge are followed; its code is not ported.

pub mod analyse;
pub mod buses;
pub mod bytes;
pub mod calc;
pub mod can;
pub mod custom;
pub mod examples;
pub mod format;
pub mod i2c;
pub mod i2s;
pub mod jtag;
pub mod num;
pub mod protocol;
pub mod pulsecode;
pub mod pwm;
pub mod spi;
pub mod stream;
pub mod swd;
pub mod synth;
pub mod types;
pub mod uart;
pub mod window;

use std::sync::Arc;

pub use protocol::{Protocol, Registry};

/// Every built-in protocol, in the order of the protocol list (and of equal-confidence
/// detections). 1-Wire, DHT, WS2812, NEC and DMX512 are data (`data/*.json`).
pub fn built_in() -> Vec<Arc<dyn Protocol>> {
    let mut code: Vec<Arc<dyn Protocol>> = vec![
        Arc::new(uart::Uart::default()),
        Arc::new(uart::UartInverted::default()),
        Arc::new(i2c::I2c::default()),
        Arc::new(spi::Spi::default()),
        Arc::new(pwm::Pwm::default()),
    ];
    for id in ["onewire", "dht", "ws2812", "ir-nec"] {
        code.push(data_protocol(id, &code));
    }
    code.push(Arc::new(can::Can::default()));
    code.push(Arc::new(i2s::I2s::default()));
    code.push(Arc::new(swd::Swd::default()));
    code.push(Arc::new(jtag::Jtag::default()));
    let dmx = data_protocol("dmx512", &code);
    code.push(dmx);
    code
}

fn data_protocol(id: &str, known: &[Arc<dyn Protocol>]) -> Arc<dyn Protocol> {
    let cfg =
        custom::built_in_config(id).unwrap_or_else(|| panic!("built-in protocol data {id} does not parse"));
    let lookup = |b: &str| known.iter().find(|p| p.id() == b).cloned();
    custom::compile(cfg, &lookup, protocol::Origin::BuiltIn).unwrap_or_else(|e| panic!("{e}"))
}

/// The registry of built-in protocols.
pub fn registry() -> Registry {
    Registry::new(built_in(), Vec::new())
}

/// The registry of built-in protocols plus custom ones (a custom one with a built-in id replaces it).
pub fn registry_with(custom: Vec<Arc<dyn Protocol>>) -> Registry {
    Registry::new(built_in(), custom)
}
