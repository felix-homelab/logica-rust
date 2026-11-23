//! Resources (doc 07): the session, the command list, protocol knowledge, capture summaries and
//! decoded packets, each read through a command.

use serde_json::{Value, json};

use logica_engine::Event;

pub const SESSION: &str = "logica://session";
pub const COMMANDS: &str = "logica://commands";
pub const PROTOCOLS: &str = "logica://protocols";

/// (uri, name, description) of the fixed resources.
pub const FIXED: &[(&str, &str, &str)] = &[
    (SESSION, "session", "Open captures, decoders, markers, the person's view, devices and leases."),
    (
        COMMANDS,
        "commands",
        "Every command of the command API: input and output schemas, safety class, lease, undo.",
    ),
    (PROTOCOLS, "protocols", "Every protocol Logica knows: roles, settings, whether it has a decoder."),
];

/// (uri template, name, description).
pub const TEMPLATES: &[(&str, &str, &str)] = &[
    (
        "logica://captures/{id}/summary",
        "capture summary",
        "A capture: length, rate, channels, decoders, markers, notes.",
    ),
    (
        "logica://captures/{id}/decoders/{decoderId}/packets{?from,to,cursor}",
        "decoded packets",
        "Decoded packets of a decoder as text, paged; from and to are samples.",
    ),
    (
        "logica://protocols/{id}",
        "protocol",
        "What a protocol is, how to confirm it, its settings and the symptoms of wrong ones.",
    ),
];

/// What a resource URI asks for.
#[derive(Debug, PartialEq)]
pub enum Target {
    Session,
    Commands,
    Protocols,
    Protocol(String),
    Capture(String),
    Packets { capture: String, decoder: String, from: Option<u64>, to: Option<u64>, cursor: Option<u64> },
}

pub fn parse(uri: &str) -> Option<Target> {
    let rest = uri.strip_prefix("logica://")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.split('/').collect();
    let q = |key: &str| -> Option<u64> {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.parse().ok())
    };
    let ok = |s: &str| !s.is_empty();
    Some(match parts.as_slice() {
        ["session"] => Target::Session,
        ["commands"] => Target::Commands,
        ["protocols"] => Target::Protocols,
        ["protocols", id] if ok(id) => Target::Protocol((*id).into()),
        ["captures", id, "summary"] if ok(id) => Target::Capture((*id).into()),
        ["captures", id, "decoders", d, "packets"] if ok(id) && ok(d) => Target::Packets {
            capture: (*id).into(),
            decoder: (*d).into(),
            from: q("from"),
            to: q("to"),
            cursor: q("cursor"),
        },
        _ => return None,
    })
}

/// The command and input that read a target (`None` for the command list, which is static).
pub fn command(t: &Target) -> Option<(&'static str, Value)> {
    Some(match t {
        Target::Session | Target::Capture(_) => ("session.get", json!({})),
        Target::Commands => return None,
        Target::Protocols => ("protocol.list", json!({})),
        Target::Protocol(id) => ("protocol.info", json!({ "protocol": id })),
        Target::Packets { capture, decoder, from, to, cursor } => (
            "decoder.frames",
            json!({ "capture": capture, "decoder": decoder, "packets": true, "from": from, "to": to, "cursor": cursor }),
        ),
    })
}

/// The URIs an engine event changes; a subscribed URI matches when it is one of them, or the same
/// path with other query parameters.
pub fn changed_by(e: &Event) -> Vec<String> {
    match e {
        Event::CaptureDone { capture, .. } => {
            vec![SESSION.into(), format!("logica://captures/{capture}/summary")]
        }
        Event::DecoderUpdated { capture, decoder } => vec![
            SESSION.into(),
            format!("logica://captures/{capture}/summary"),
            format!("logica://captures/{capture}/decoders/{decoder}/packets"),
        ],
        Event::StateChanged { .. } => vec![SESSION.into()],
        _ => Vec::new(),
    }
}

pub fn matches(subscribed: &str, changed: &str) -> bool {
    subscribed == changed || subscribed.split('?').next() == Some(changed)
}
