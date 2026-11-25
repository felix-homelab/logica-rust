//! `logica-vla`, the virtual logic analyser: a separate program that behaves like a streaming
//! logic analyser on a TCP port and generates configurable signals (UART, I2C, SPI, PWM, clocks,
//! random noise) with random payloads and pauses. Logica's `vla` driver captures from it, so live
//! capture and live decoding can be built and tried without hardware. See `README.md` and
//! `PROTOCOL.md` next to this crate.

pub mod config;
pub mod generator;
pub mod rng;
pub mod server;

pub use config::Config;
