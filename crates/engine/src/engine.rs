//! The engine: one session with its captures, devices, leases and history. Every client (the
//! window, MCP, the CLI, tests) calls [`Engine::execute`] with an actor and an invocation; events go
//! to every subscriber.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;

use serde_json::Value;

use logica_core::capture::CaptureMeta;
use logica_core::device::{Device, RunOutcome, StopSignal, StoreSink, VirtualDriver};
use logica_core::store::{CaptureStore, DEFAULT_CACHE_BUDGET};
use logica_protocols::Registry;
use logica_protocols::protocol::Protocol;

use crate::commands::spec;
use crate::contract::{Actor, CommandError, ErrorCode, Event, HistoryEntry, Invocation, Undo};
use crate::state::Capture;

pub struct EngineConfig {
    /// Where working `.lacap` files of new captures and imports go; memory when `None`.
    pub data_dir: Option<PathBuf>,
    pub cache_budget: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig { data_dir: None, cache_budget: DEFAULT_CACHE_BUDGET }
    }
}

/// Who runs a command.
pub struct Ctx {
    pub actor: Actor,
}

/// What a command handler returns.
pub struct Done {
    pub output: Value,
    /// How to undo it (recorded for `Undo::Inverse` commands).
    pub inverse: Option<Invocation>,
    /// The command as done, with any ids it created filled in, so redo recreates the same ids.
    pub canonical: Option<Value>,
    /// The feed line, without the actor: "added an I2C decoder on CH4 and CH5".
    pub summary: String,
    /// Whether to record it in the history (agents' view changes are, the person's are not).
    pub record: bool,
    /// Why this particular call cannot be undone, when its command usually can.
    pub never_because: Option<String>,
}

impl Done {
    pub fn query(output: Value) -> Self {
        Done {
            output,
            inverse: None,
            canonical: None,
            summary: String::new(),
            record: false,
            never_because: None,
        }
    }
    pub fn edit(output: Value, inverse: Invocation, summary: impl Into<String>) -> Self {
        Done {
            output,
            inverse: Some(inverse),
            canonical: None,
            summary: summary.into(),
            record: true,
            never_because: None,
        }
    }
    pub fn never(output: Value, summary: impl Into<String>) -> Self {
        Done {
            output,
            inverse: None,
            canonical: None,
            summary: summary.into(),
            record: true,
            never_because: None,
        }
    }
    pub fn canonical(mut self, input: Value) -> Self {
        self.canonical = Some(input);
        self
    }
}

/// A device opened by `device.connect`.
pub struct OpenDevice {
    pub device: Option<Box<dyn Device>>,
    pub configured: bool,
}

/// A capture running on a worker thread.
pub struct Running {
    pub device: String,
    pub capture: String,
    pub title: String,
    pub stop: StopSignal,
    pub handle: JoinHandle<RunResult>,
}

pub type RunResult = (Box<dyn Device>, Result<(RunOutcome, StoreSink), String>);

pub struct Engine {
    pub(crate) cfg: EngineConfig,
    pub(crate) registry: Registry,
    pub(crate) custom: Vec<Arc<dyn Protocol>>,
    pub(crate) captures: BTreeMap<String, Capture>,
    pub(crate) active: Option<String>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) drivers: Vec<Box<dyn VirtualDriver>>,
    pub(crate) devices: BTreeMap<String, OpenDevice>,
    pub(crate) leases: BTreeMap<String, Actor>,
    pub(crate) running: Vec<Running>,
    /// Every device the drivers offer: driver index, info, capabilities (filled on first use).
    pub(crate) catalogue: Vec<(usize, logica_core::device::DeviceInfo, logica_core::device::Capabilities)>,
    subscribers: Vec<mpsc::Sender<Event>>,
    revision: u64,
    next_id: u64,
}

pub(crate) fn err(code: ErrorCode, message: impl Into<String>) -> CommandError {
    CommandError::new(code, message)
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Self {
        Engine {
            cfg,
            registry: logica_protocols::registry(),
            custom: Vec::new(),
            captures: BTreeMap::new(),
            active: None,
            history: Vec::new(),
            drivers: vec![Box::new(logica_drivers::synthetic::SyntheticDriver)],
            devices: BTreeMap::new(),
            leases: BTreeMap::new(),
            running: Vec::new(),
            catalogue: Vec::new(),
            subscribers: Vec::new(),
            revision: 0,
            next_id: 1,
        }
    }

    /// Events from now on; a dropped receiver unsubscribes.
    pub fn subscribe(&mut self) -> mpsc::Receiver<Event> {
        let (tx, rx) = mpsc::channel();
        self.subscribers.push(tx);
        rx
    }

    pub(crate) fn emit(&mut self, e: Event) {
        self.subscribers.retain(|s| s.send(e.clone()).is_ok());
    }

    pub(crate) fn changed(&mut self) {
        self.revision += 1;
        let revision = self.revision;
        self.emit(Event::StateChanged { revision });
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// A fresh id with a prefix: "c3", "uart-4", "m7".
    pub(crate) fn new_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// Makes sure ids given back by redo or an undo are not handed out again.
    pub(crate) fn reserve_id(&mut self, id: &str) {
        let digits: String =
            id.chars().rev().take_while(char::is_ascii_digit).collect::<String>().chars().rev().collect();
        if let Ok(n) = digits.parse::<u64>() {
            self.next_id = self.next_id.max(n + 1);
        }
    }

    pub(crate) fn capture(&self, id: &str) -> Result<&Capture, CommandError> {
        self.captures.get(id).ok_or_else(|| {
            err(
                ErrorCode::NotFound,
                format!("There is no capture \"{id}\". session.get lists the open captures."),
            )
        })
    }

    pub(crate) fn capture_mut(&mut self, id: &str) -> Result<&mut Capture, CommandError> {
        self.captures.get_mut(id).ok_or_else(|| {
            err(
                ErrorCode::NotFound,
                format!("There is no capture \"{id}\". session.get lists the open captures."),
            )
        })
    }

    pub(crate) fn add_capture(
        &mut self,
        id: String,
        meta: CaptureMeta,
        store: CaptureStore,
        path: Option<PathBuf>,
        backed: bool,
    ) {
        let to = store.sample_count();
        self.captures.insert(
            id.clone(),
            Capture {
                meta,
                store,
                path,
                backed_by_path: backed,
                decoders: BTreeMap::new(),
                markers: BTreeMap::new(),
                view: crate::state::View { from: 0, to, ..Default::default() },
                groups: BTreeMap::new(),
                hidden: Default::default(),
            },
        );
        self.active = Some(id);
    }

    /// Runs a command for `actor`: checks it exists, runs it, records it in the history (unless
    /// it only reads), and tells every subscriber.
    pub fn execute(&mut self, actor: &Actor, inv: &Invocation) -> Result<Value, CommandError> {
        self.poll();
        let s = spec(&inv.command).ok_or_else(|| {
            err(
                ErrorCode::NotFound,
                format!(
                    "There is no command \"{}\". The list is the resource logica://commands.",
                    inv.command
                ),
            )
        })?;
        match inv.command.as_str() {
            "history.undo" => return self.undo(actor, &inv.input),
            "history.redo" => return self.redo(actor, &inv.input),
            _ => {}
        }
        let ctx = Ctx { actor: actor.clone() };
        let done = crate::cmd::dispatch(self, &ctx, &inv.command, inv.input.clone())?;
        if s.undo != Undo::Query && done.record {
            // a new action by an actor discards that actor's redo entries
            for e in self.history.iter_mut().filter(|e| e.actor == *actor && e.undone) {
                e.discarded = true;
            }
            let id = self.history.last().map_or(1, |e| e.id + 1);
            let never_because = match s.undo {
                Undo::Never { reason } => Some(reason.to_string()),
                _ if done.inverse.is_none() => {
                    Some(done.never_because.clone().unwrap_or_else(|| "This change cannot be undone.".into()))
                }
                _ => None,
            };
            let entry = HistoryEntry {
                id,
                actor: actor.clone(),
                done: Invocation {
                    command: inv.command.clone(),
                    input: done.canonical.clone().unwrap_or_else(|| inv.input.clone()),
                },
                inverse: done.inverse.clone(),
                never_because,
                undone: false,
                discarded: false,
                summary: done.summary.clone(),
            };
            self.history.push(entry);
            self.emit(Event::CommandExecuted {
                entry: Some(id),
                actor: actor.clone(),
                command: inv.command.clone(),
                summary: done.summary.clone(),
            });
            self.changed();
        } else if s.undo != Undo::Query {
            // not recorded, but the session changed (the person's own view changes)
            self.changed();
        }
        Ok(done.output)
    }

    fn entry_arg(input: &Value) -> Result<Option<u64>, CommandError> {
        match input.get("entry") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_u64()
                .map(Some)
                .ok_or_else(|| err(ErrorCode::InvalidInput, "entry must be a history entry id.")),
        }
    }

    fn undo(&mut self, actor: &Actor, input: &Value) -> Result<Value, CommandError> {
        let pick = Self::entry_arg(input)?;
        let idx = match pick {
            Some(id) => self.history.iter().position(|e| e.id == id).ok_or_else(|| {
                err(
                    ErrorCode::NotFound,
                    format!("There is no history entry {id}. history.list shows the recent ones."),
                )
            })?,
            None => self
                .history
                .iter()
                .rposition(|e| e.actor == *actor && !e.undone && e.inverse.is_some())
                .ok_or_else(|| err(ErrorCode::NotFound, "There is nothing of yours left to undo."))?,
        };
        let e = self.history[idx].clone();
        if e.undone {
            return Err(err(ErrorCode::InvalidInput, format!("Entry {} is already undone.", e.id)));
        }
        let Some(inv) = &e.inverse else {
            return Err(err(
                ErrorCode::NotUndoable,
                format!(
                    "\"{}\" cannot be undone: {}",
                    e.summary,
                    e.never_because.clone().unwrap_or_default()
                ),
            ));
        };
        let ctx = Ctx { actor: actor.clone() };
        crate::cmd::dispatch(self, &ctx, &inv.command, inv.input.clone())?;
        self.history[idx].undone = true;
        let entry = self.history[idx].clone();
        self.emit(Event::CommandExecuted {
            entry: Some(entry.id),
            actor: actor.clone(),
            command: "history.undo".into(),
            summary: format!("undid: {}", entry.summary),
        });
        self.changed();
        Ok(serde_json::json!({ "entry": entry }))
    }

    fn redo(&mut self, actor: &Actor, input: &Value) -> Result<Value, CommandError> {
        let pick = Self::entry_arg(input)?;
        let idx = match pick {
            Some(id) => self.history.iter().position(|e| e.id == id).ok_or_else(|| {
                err(
                    ErrorCode::NotFound,
                    format!("There is no history entry {id}. history.list shows the recent ones."),
                )
            })?,
            None => self
                .history
                .iter()
                .rposition(|e| e.actor == *actor && e.undone && !e.discarded)
                .ok_or_else(|| err(ErrorCode::NotFound, "There is nothing of yours to redo."))?,
        };
        let e = self.history[idx].clone();
        if !e.undone {
            return Err(err(
                ErrorCode::InvalidInput,
                format!("Entry {} is not undone, so there is nothing to redo.", e.id),
            ));
        }
        if e.discarded {
            return Err(err(
                ErrorCode::NotUndoable,
                format!("Entry {} can no longer be redone: newer actions replaced it.", e.id),
            ));
        }
        let ctx = Ctx { actor: actor.clone() };
        crate::cmd::dispatch(self, &ctx, &e.done.command, e.done.input.clone())?;
        self.history[idx].undone = false;
        let entry = self.history[idx].clone();
        self.emit(Event::CommandExecuted {
            entry: Some(entry.id),
            actor: actor.clone(),
            command: "history.redo".into(),
            summary: format!("redid: {}", entry.summary),
        });
        self.changed();
        Ok(serde_json::json!({ "entry": entry }))
    }

    /// Collects finished captures from worker threads; called before every command.
    pub fn poll(&mut self) {
        let mut i = 0;
        while i < self.running.len() {
            if self.running[i].handle.is_finished() {
                let r = self.running.remove(i);
                self.finish_running(r);
            } else {
                i += 1;
            }
        }
    }

    pub(crate) fn finish_running(&mut self, r: Running) {
        let Ok((device, result)) = r.handle.join() else {
            self.emit(Event::CaptureDone { capture: r.capture, samples: 0, stopped: true });
            return;
        };
        if let Some(d) = self.devices.get_mut(&r.device) {
            d.device = Some(device);
        }
        match result {
            Ok((outcome, sink)) => {
                let info_source = self
                    .devices
                    .get(&r.device)
                    .and_then(|d| d.device.as_ref())
                    .map(|d| logica_core::device::device_source(d.info()));
                let source = info_source.unwrap_or(logica_core::capture::Source {
                    kind: logica_core::capture::SourceKind::Device,
                    detail: r.device.clone(),
                });
                match sink.finish(&outcome, source, &r.title) {
                    Ok((store, mut meta)) => {
                        meta.id = r.capture.clone();
                        let samples = store.sample_count();
                        self.add_capture(r.capture.clone(), meta, store, None, false);
                        self.emit(Event::CaptureDone {
                            capture: r.capture,
                            samples,
                            stopped: outcome.stopped,
                        });
                    }
                    Err(e) => self.capture_failed(&r.capture, &e.to_string()),
                }
            }
            Err(e) => self.capture_failed(&r.capture, &e),
        }
        self.changed();
    }

    fn capture_failed(&mut self, capture: &str, why: &str) {
        self.emit(Event::CommandExecuted {
            entry: None,
            actor: Actor::User,
            command: "capture.start".into(),
            summary: format!("the capture {capture} failed: {why}"),
        });
        self.emit(Event::CaptureDone { capture: capture.into(), samples: 0, stopped: true });
    }

    /// The registry of built-in and custom protocols.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Releases every lease an actor holds (its client went away).
    pub fn release_actor(&mut self, actor: &Actor) {
        let held: Vec<String> =
            self.leases.iter().filter(|(_, a)| *a == actor).map(|(d, _)| d.clone()).collect();
        for d in held {
            self.leases.remove(&d);
            self.emit(Event::LeaseChanged { device: d, holder: None });
        }
        self.changed();
    }
}
