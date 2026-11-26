//! `session.get` and the history listing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contract::{CommandError, ErrorCode, HistoryEntry};
use crate::engine::{Ctx, Done, Engine, err};
use crate::state::CaptureSummary;

use super::{DeviceEntry, Nothing, out};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunningEntry {
    pub device: String,
    pub capture: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionOut {
    /// "major.minor" (doc 03).
    pub api_version: String,
    /// Grows with every change; `state.changed` events carry it.
    pub revision: u64,
    /// The capture whose tab is in front.
    pub active: Option<String>,
    pub captures: Vec<CaptureSummary>,
    pub devices: Vec<DeviceEntry>,
    pub running: Vec<RunningEntry>,
    /// The latest history entries, newest first.
    pub recent: Vec<HistoryEntry>,
    /// Agents' capture and output commands are refused (the person's "stop agent actions").
    pub agents_paused: bool,
}

pub(crate) fn get(e: &mut Engine, _: &Ctx, _: Nothing) -> Result<Done, CommandError> {
    let devices = super::devices::known_entries(e);
    let s = SessionOut {
        api_version: crate::contract::api_version(),
        revision: e.revision(),
        active: e.active.clone(),
        captures: e.captures.iter().map(|(id, c)| CaptureSummary::of(id, c)).collect(),
        devices,
        running: e
            .running
            .iter()
            .flat_map(|r| {
                r.devices.iter().map(|d| RunningEntry { device: d.clone(), capture: r.capture.clone() })
            })
            .collect(),
        recent: e.history.iter().rev().take(20).cloned().collect(),
        agents_paused: e.agents_paused,
    };
    Ok(Done::query(out(&s)))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryListIn {
    /// Most entries to return (default 50).
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct HistoryListOut {
    /// Newest first.
    pub entries: Vec<HistoryEntry>,
}

pub(crate) fn history_list(e: &mut Engine, _: &Ctx, i: HistoryListIn) -> Result<Done, CommandError> {
    let entries = e.history.iter().rev().take(i.limit.unwrap_or(50)).cloned().collect();
    Ok(Done::query(out(&HistoryListOut { entries })))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryPick {
    /// A history entry by id; without it, the actor's own last action.
    pub entry: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct HistoryOut {
    pub entry: HistoryEntry,
}

/// Undo and redo are run by the engine itself; this is never reached.
pub(crate) fn history_pick(_: &mut Engine, _: &Ctx, _: HistoryPick) -> Result<Done, CommandError> {
    Err(err(ErrorCode::Failed, "Undo and redo are run through the engine's history."))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PauseAgentsIn {
    /// true stops agents' capture and output commands; false lets them run again.
    pub paused: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PauseAgentsOut {
    pub agents_paused: bool,
    /// Captures started by agents that were stopped by this switch.
    pub stopped: Vec<String>,
}

pub(crate) fn pause_agents(e: &mut Engine, ctx: &Ctx, i: PauseAgentsIn) -> Result<Done, CommandError> {
    if !matches!(ctx.actor, crate::contract::Actor::User) {
        return Err(err(
            ErrorCode::NotAllowed,
            "Only the person, in the window, can stop or resume agent actions.",
        ));
    }
    e.agents_paused = i.paused;
    // a stop also ends captures that agents are running
    let mut stopped = Vec::new();
    if i.paused {
        for r in &e.running {
            if r.devices
                .iter()
                .any(|d| e.leases.get(d).is_some_and(|a| matches!(a, crate::contract::Actor::Agent(_))))
            {
                r.stop.stop();
                stopped.push(r.capture.clone());
            }
        }
    }
    let summary = if i.paused { "stopped agent actions" } else { "let agents act again" };
    Ok(Done::never(out(&PauseAgentsOut { agents_paused: i.paused, stopped }), summary))
}
