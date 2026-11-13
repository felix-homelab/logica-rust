//! Logica's engine (doc 03): sessions, the device manager and capture pipeline, decoder
//! orchestration, the command API with events, actors, undo history and device leases. Every
//! capability is a command first (ADR 0001); the window, MCP, the CLI and tests are clients.

pub mod commands;
pub mod contract;
