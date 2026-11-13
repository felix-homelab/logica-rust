//! The contract as one JSON Schema document (doc 03): every command's spec, input and output, the
//! events and the error. Checked in as `packages/api/schema/contract.json`; the TypeScript types
//! are generated from it, and a test fails when it is out of date.

use serde_json::{Map, Value, json};

use crate::commands::spec;
use crate::contract::{API_VERSION, CommandError, Event, HistoryEntry, Invocation};

/// Where the checked-in contract lives, relative to the workspace root.
pub const CONTRACT_PATH: &str = "packages/api/schema/contract.json";

pub fn contract() -> Value {
    let mut g = schemars::generate::SchemaSettings::draft2020_12().into_generator();
    let mut commands = Map::new();
    for (name, input, output) in crate::cmd::schemas(&mut g) {
        let s = spec(name).expect("every command in the table has a spec");
        commands.insert(
            name.to_string(),
            json!({
                "summary": s.summary,
                "safety": s.safety,
                "lease": s.lease,
                "longRunning": s.long_running,
                "undo": s.undo,
                "input": input,
                "output": output,
            }),
        );
    }
    let event = g.subschema_for::<Event>();
    let error = g.subschema_for::<CommandError>();
    let invocation = g.subschema_for::<Invocation>();
    let history = g.subschema_for::<HistoryEntry>();
    let defs: Map<String, Value> = g.take_definitions(true).into_iter().collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Logica command API",
        "apiVersion": format!("{}.{}", API_VERSION.0, API_VERSION.1),
        "commands": commands,
        "event": event,
        "error": error,
        "invocation": invocation,
        "historyEntry": history,
        "$defs": defs,
    })
}

/// The contract as the checked-in text.
pub fn contract_text() -> String {
    serde_json::to_string_pretty(&contract()).unwrap_or_default() + "\n"
}
