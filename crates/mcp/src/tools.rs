//! The curated tools (doc 07, ADR 0006): a small, task-shaped set over the command API, plus
//! `run_command` for everything else. Each tool is data: what it maps to, and its input schema,
//! which is assembled from the commands' schemas in the contract, so it cannot drift from them.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

/// What a tool runs.
pub enum Kind {
    /// One command; the tool's input is the command's input.
    Command(&'static str),
    /// An `action` field picks the command; the other fields are its input.
    Actions(&'static [(&'static str, &'static str)]),
    /// `start`: configure (when `config` is given), start, and wait with progress; `stop`.
    Capture,
    /// Any command by name.
    RunCommand,
}

pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Only reads (no change to the session, no device use).
    pub read_only: bool,
    pub kind: Kind,
    /// Input fields of the command that the tool does not offer (internal to undo and redo).
    pub hide: &'static [&'static str],
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "session",
        title: "Session",
        description: "What is open and what the person sees: captures with their decoders and markers, the person's view per tab (range, cursors, selection), devices with lease holders, running captures, recent activity. Start here.",
        read_only: true,
        kind: Kind::Command("session.get"),
        hide: &[],
    },
    ToolDef {
        name: "list_devices",
        title: "List devices",
        description: "Connected and available logic analysers: driver, model, whether connected or capturing, who holds the lease, channels, rates and modes.",
        read_only: true,
        kind: Kind::Command("device.list"),
        hide: &[],
    },
    ToolDef {
        name: "device",
        title: "Connect or disconnect a device",
        description: "action \"connect\" opens a device, takes its lease and returns its capabilities; \"disconnect\" closes it.",
        read_only: false,
        kind: Kind::Actions(&[("connect", "device.connect"), ("disconnect", "device.disconnect")]),
        hide: &[],
    },
    ToolDef {
        name: "capture",
        title: "Capture",
        description: "action \"start\" records from a connected device: give `config` (channels, sampleRateHz, samples, mode) to configure it first; waits until the capture ends (wait: true, the default) with progress, and can be cancelled. \"stop\" ends a running capture.",
        read_only: false,
        kind: Kind::Capture,
        hide: &[],
    },
    ToolDef {
        name: "open_file",
        title: "Open a capture file",
        description: "Open a capture file from a path on the machine running Logica: .lacap, FNIRSI .demo, sigrok .sr or VCD. Returns the capture's id, length, rate and channels.",
        read_only: false,
        kind: Kind::Command("file.open"),
        hide: &["id"],
    },
    ToolDef {
        name: "save_capture",
        title: "Save a capture",
        description: "Save a capture as .lacap with its decoders and markers: in place, or to `path`. Exports to other formats come later.",
        read_only: false,
        kind: Kind::Command("file.save"),
        hide: &[],
    },
    ToolDef {
        name: "summarize",
        title: "Summarize a capture",
        description: "What is on the wire: detected protocols with channels, settings, confidence and evidence; per-channel measurements; channels nothing explains. For the whole capture or a range of samples.",
        read_only: true,
        kind: Kind::Command("analysis.detect"),
        hide: &[],
    },
    ToolDef {
        name: "measure",
        title: "Measure",
        description: "action \"stats\": frequency, period, duty cycle, pulse widths and edge count of one channel; \"edges\": the edges of channels in a range (paged); \"between\": the time between two samples.",
        read_only: true,
        kind: Kind::Actions(&[
            ("stats", "measure.stats"),
            ("edges", "measure.edges"),
            ("between", "measure.between"),
        ]),
        hide: &[],
    },
    ToolDef {
        name: "decoder",
        title: "Add, change or remove a decoder",
        description: "action \"add\" decodes a protocol on channels (roles such as {\"scl\": 5, \"sda\": 4}) with settings; \"update\" changes roles or settings and says how the problem counts changed; \"remove\" deletes it. Every change can be undone.",
        read_only: false,
        kind: Kind::Actions(&[
            ("add", "decoder.add"),
            ("update", "decoder.update"),
            ("remove", "decoder.remove"),
        ]),
        hide: &["id"],
    },
    ToolDef {
        name: "get_decoded",
        title: "Read decoded data",
        description: "Decoded packets (bytes as text per transaction or line) or frames, filtered by range, kind, problems and text; paged with nextCursor. explain: true groups the problems and names likely causes.",
        read_only: true,
        kind: Kind::Command("decoder.frames"),
        hide: &[],
    },
    ToolDef {
        name: "search",
        title: "Search",
        description: "Find edges, pulses and decoded content with a structured query (all, any, not, decoded, edge, pulse, level, within). Hits with times and samples.",
        read_only: true,
        kind: Kind::Command("search.run"),
        hide: &[],
    },
    ToolDef {
        name: "render",
        title: "Render",
        description: "A PNG picture of a range of a capture with its decoder rows and markers, for clients that can see images.",
        read_only: true,
        kind: Kind::Command("render"),
        hide: &[],
    },
    ToolDef {
        name: "show_user",
        title: "Show the person",
        description: "Point the person at something: a marker at a sample, or a range note with `end`, with a label and an optional note; moveView: true also moves their view there. Shown with an agent badge.",
        read_only: false,
        kind: Kind::Command("marker.add"),
        hide: &["id", "by"],
    },
    ToolDef {
        name: "run_command",
        title: "Run a command",
        description: "Run any command of Logica's command API by name with its input, for example history.undo, analysis.bytes, channel.rename or capture.close. The commands and their input schemas are in the resource logica://commands.",
        read_only: false,
        kind: Kind::RunCommand,
        hide: &[],
    },
];

pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The input schema of a command, with `$ref`s resolved one level and `$defs` it needs.
fn command_input(contract: &Value, command: &str) -> Map<String, Value> {
    let input = &contract["commands"][command]["input"];
    let resolved = match input.get("$ref").and_then(Value::as_str) {
        Some(r) => contract["$defs"][r.trim_start_matches("#/$defs/")].clone(),
        None => input.clone(),
    };
    resolved.as_object().cloned().unwrap_or_default()
}

/// Adds every definition `node` refers to (and theirs) to `out`.
fn collect_defs(node: &Value, defs: &Value, out: &mut Map<String, Value>) {
    match node {
        Value::Object(m) => {
            if let Some(name) =
                m.get("$ref").and_then(Value::as_str).map(|r| r.trim_start_matches("#/$defs/"))
                && !out.contains_key(name)
                && let Some(d) = defs.get(name)
            {
                out.insert(name.to_string(), d.clone());
                collect_defs(d, defs, out);
            }
            for v in m.values() {
                collect_defs(v, defs, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|v| collect_defs(v, defs, out)),
        _ => {}
    }
}

fn props(schema: &Map<String, Value>) -> Map<String, Value> {
    schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn required(schema: &Map<String, Value>) -> BTreeSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default()
}

/// One object schema from the inputs of several actions: `action` first, the fields of every
/// action after it, required only where every action requires them. (Agent APIs refuse a
/// top-level oneOf.)
fn merged(actions: &[(&str, Map<String, Value>)]) -> Map<String, Value> {
    let mut properties = Map::new();
    let names: Vec<&str> = actions.iter().map(|(a, _)| *a).collect();
    properties.insert("action".into(), json!({ "type": "string", "enum": names }));
    let mut req: Option<BTreeSet<String>> = None;
    for (action, schema) in actions {
        for (k, mut v) in props(schema) {
            if let Some(existing) = properties.get_mut(&k) {
                note_action(existing, action);
                continue;
            }
            note_action(&mut v, action);
            properties.insert(k, v);
        }
        let this = required(schema);
        req = Some(match req {
            None => this,
            Some(r) => r.intersection(&this).cloned().collect(),
        });
    }
    let mut required: Vec<Value> = vec!["action".into()];
    required.extend(req.unwrap_or_default().into_iter().map(Value::String));
    let mut s = Map::new();
    s.insert("type".into(), "object".into());
    s.insert("properties".into(), Value::Object(properties));
    s.insert("required".into(), Value::Array(required));
    s
}

/// Lists the actions a merged field is for in its description.
fn note_action(field: &mut Value, action: &str) {
    let Some(m) = field.as_object_mut() else { return };
    let list = m.remove("x-actions").and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut list: Vec<String> = list.iter().filter_map(Value::as_str).map(String::from).collect();
    list.push(action.to_string());
    m.insert("x-actions".into(), json!(list));
}

/// Turns the `x-actions` bookkeeping into description text.
fn finish_actions(schema: &mut Map<String, Value>, all: usize) {
    let Some(Value::Object(properties)) = schema.get_mut("properties") else { return };
    for v in properties.values_mut() {
        let Some(m) = v.as_object_mut() else { continue };
        let Some(list) = m.remove("x-actions") else { continue };
        let list: Vec<&str> =
            list.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if list.len() < all {
            let says = format!("For {}.", list.join(", "));
            let text = match m.get("description").and_then(Value::as_str) {
                Some(d) => format!("{d} {says}"),
                None => says,
            };
            m.insert("description".into(), text.into());
        }
    }
}

/// The input schema of a tool.
pub fn input_schema(tool: &ToolDef, contract: &Value) -> Map<String, Value> {
    let mut schema = match &tool.kind {
        Kind::Command(c) => command_input(contract, c),
        Kind::Actions(list) => {
            let actions: Vec<(&str, Map<String, Value>)> =
                list.iter().map(|(a, c)| (*a, command_input(contract, c))).collect();
            let mut s = merged(&actions);
            finish_actions(&mut s, list.len());
            s
        }
        Kind::Capture => {
            let mut start = command_input(contract, "capture.start");
            if let Some(Value::Object(p)) = start.get_mut("properties") {
                p.insert(
                    "config".into(),
                    json!({
                        "description": "Channels, sample rate, samples per channel and mode; configures the device before starting. Without it, the device's last configuration is used.",
                        "$ref": "#/$defs/CaptureConfig"
                    }),
                );
            }
            let stop = command_input(contract, "capture.stop");
            let mut s = merged(&[("start", start), ("stop", stop)]);
            finish_actions(&mut s, 2);
            s
        }
        Kind::RunCommand => {
            let names: Vec<&str> = logica_engine::commands::COMMANDS.iter().map(|c| c.name).collect();
            let mut s = Map::new();
            s.insert("type".into(), "object".into());
            s.insert(
                "properties".into(),
                json!({
                    "command": { "type": "string", "enum": names, "description": "The command's name." },
                    "input": { "type": "object", "description": "The command's input, as in its schema in logica://commands." }
                }),
            );
            s.insert("required".into(), json!(["command"]));
            s.insert("additionalProperties".into(), false.into());
            s
        }
    };
    if !tool.hide.is_empty() {
        if let Some(Value::Object(p)) = schema.get_mut("properties") {
            for h in tool.hide {
                p.remove(*h);
            }
        }
        if let Some(Value::Array(r)) = schema.get_mut("required") {
            r.retain(|x| x.as_str().is_none_or(|x| !tool.hide.contains(&x)));
        }
    }
    let mut defs = Map::new();
    collect_defs(&Value::Object(schema.clone()), &contract["$defs"], &mut defs);
    if !defs.is_empty() {
        schema.insert("$defs".into(), Value::Object(defs));
    }
    schema
}
