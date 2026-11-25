//! Logica's engine (doc 03): sessions, the device manager and capture pipeline, decoder
//! orchestration, the command API with events, actors, undo history and device leases. Every
//! capability is a command first (ADR 0001); the window, MCP, the CLI and tests are clients.

pub mod cmd;
pub mod commands;
pub mod contract;
pub mod engine;
pub mod live;
pub mod render;
pub mod schema;
pub mod state;

pub use contract::{Actor, CommandError, ErrorCode, Event, Invocation};
pub use engine::{Engine, EngineConfig};
