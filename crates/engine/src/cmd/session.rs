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
}

pub(crate) fn get(e: &mut Engine, _: &Ctx, _: Nothing) -> Result<Done, CommandError> {
    let devices = super::devices::entries(e);
    let s = SessionOut {
        api_version: crate::contract::api_version(),
        revision: e.revision(),
        active: e.active.clone(),
        captures: e.captures.iter().map(|(id, c)| CaptureSummary::of(id, c)).collect(),
        devices,
        running: e
            .running
            .iter()
            .map(|r| RunningEntry { device: r.device.clone(), capture: r.capture.clone() })
            .collect(),
        recent: e.history.iter().rev().take(20).cloned().collect(),
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
