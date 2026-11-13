//! The command API contract (doc 03, ADR 0001, ADR 0008): what every command, event and error looks
//! like, whoever sends it. Defined once here; the JSON Schema and the TypeScript types are
//! generated from it (step 1 of M1, after review).

use serde::{Deserialize, Serialize};

/// Major and minor version of the contract, sent in the handshake. Additive changes only within a
/// major version.
pub const API_VERSION: (u32, u32) = (1, 0);

/// Who sent a command: the person, an agent (by its client name), or the command line.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Actor {
    User,
    Agent(String),
    Cli,
}

impl Actor {
    /// "user", "agent:<name>" or "cli".
    pub fn wire(&self) -> String {
        match self {
            Actor::User => "user".into(),
            Actor::Agent(name) => format!("agent:{name}"),
            Actor::Cli => "cli".into(),
        }
    }

    pub fn parse(s: &str) -> Option<Actor> {
        match s {
            "user" => Some(Actor::User),
            "cli" => Some(Actor::Cli),
            _ => s.strip_prefix("agent:").filter(|n| !n.is_empty()).map(|n| Actor::Agent(n.into())),
        }
    }
}

impl Serialize for Actor {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.wire())
    }
}

impl<'de> Deserialize<'de> for Actor {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Actor::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown actor \"{s}\"")))
    }
}

/// What a command may touch (doc 07).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    /// Analysis only.
    Read,
    /// Hardware inputs.
    Capture,
    /// Drives pins (PWM); off until the person allows it in the app.
    Output,
}

/// How a command is undone (ADR 0008).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Undo {
    /// Changes nothing; not recorded in the history.
    Query,
    /// Recorded with the inverse it returns.
    Inverse,
    /// Recorded, but cannot be undone; the reason is shown.
    Never { reason: &'static str },
}

/// The fixed facts about one command, also served as the resource `logica://commands`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    /// "decoder.add", "history.undo", …
    pub name: &'static str,
    /// One line for people and agents.
    pub summary: &'static str,
    pub safety: Safety,
    /// Needs the device's lease (hardware commands).
    pub lease: bool,
    /// Reports progress and can be cancelled.
    pub long_running: bool,
    pub undo: Undo,
}

/// A command call as data: what the history stores and `run_command` takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invocation {
    pub command: String,
    pub input: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    /// The input does not match the contract or is impossible ("300 MHz with 20 channels").
    InvalidInput,
    /// A capture, decoder, marker, device or history entry that does not exist.
    NotFound,
    /// The device is leased by someone else, or a capture is running.
    Busy,
    /// Outputs are switched off, or an agent's actions are paused by the person.
    NotAllowed,
    /// Asked for something this command cannot do (undo of a file write).
    NotUndoable,
    Cancelled,
    /// Anything else; the message says what happened and what to do.
    Failed,
}

/// An error worded for the person or agent: what is wrong and what to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        CommandError { code, message: message.into() }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

/// Sent to every attached client (doc 03).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename = "capture.progress")]
    CaptureProgress { capture: String, samples: u64, fraction: Option<f64> },
    #[serde(rename = "capture.done")]
    CaptureDone { capture: String, samples: u64, stopped: bool },
    /// Also after undo, redo, open and close.
    #[serde(rename = "decoder.updated")]
    DecoderUpdated { capture: String, decoder: String },
    #[serde(rename = "view.changed")]
    ViewChanged { capture: String, actor: Actor },
    #[serde(rename = "lease.changed")]
    LeaseChanged { device: String, holder: Option<Actor> },
    #[serde(rename = "device.disconnected")]
    DeviceDisconnected { device: String },
    #[serde(rename = "command.executed")]
    CommandExecuted { entry: Option<u64>, actor: Actor, command: String, summary: String },
    /// Read the session again: after any recorded command, undo, redo or lease change.
    #[serde(rename = "state.changed")]
    StateChanged { revision: u64 },
}

/// One entry of the engine-wide history (ADR 0008).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: u64,
    pub actor: Actor,
    pub done: Invocation,
    /// `None` when the command cannot be undone.
    pub inverse: Option<Invocation>,
    /// Why it cannot be undone, when it cannot.
    pub never_because: Option<String>,
    pub undone: bool,
    /// Plain-language line for the activity feed: "Claude added an I2C decoder on CH4 and CH5".
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actors_on_the_wire() {
        for (a, w) in [
            (Actor::User, "user"),
            (Actor::Cli, "cli"),
            (Actor::Agent("Claude Code".into()), "agent:Claude Code"),
        ] {
            assert_eq!(serde_json::to_value(&a).unwrap(), serde_json::json!(w));
            assert_eq!(serde_json::from_value::<Actor>(serde_json::json!(w)).unwrap(), a);
        }
        assert!(serde_json::from_value::<Actor>(serde_json::json!("agent:")).is_err());
        assert!(serde_json::from_value::<Actor>(serde_json::json!("robot")).is_err());
    }

    #[test]
    fn events_and_errors_on_the_wire() {
        let e = Event::LeaseChanged { device: "synthetic/mix".into(), holder: Some(Actor::User) };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"event": "lease.changed", "device": "synthetic/mix", "holder": "user"})
        );
        let err = CommandError::new(ErrorCode::InvalidInput, "Choose a channel for SDA.");
        assert_eq!(serde_json::to_value(&err).unwrap()["code"], "invalid-input");
    }
}
