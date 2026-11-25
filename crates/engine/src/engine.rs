//! The engine: one session with its captures, devices, leases and history. Every client (the
//! window, MCP, the CLI, tests) calls [`Engine::execute`] with an actor and an invocation; events go
//! to every subscriber.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;

use logica_core::capture::{CaptureMeta, CaptureState, ChannelInfo, Note, NoteLevel, Source, whole};
use logica_core::device::{Device, RunOutcome, StopSignal, VirtualDriver};
use logica_core::import::StoreTarget;
use logica_core::store::{CaptureStore, DEFAULT_CACHE_BUDGET};
use logica_core::{Level, Sample};
use logica_protocols::Registry;
use logica_protocols::protocol::Protocol;

use crate::commands::spec;
use crate::contract::{Actor, CommandError, ErrorCode, Event, HistoryEntry, Invocation, Undo};
use crate::live::Delivery;
use crate::state::Capture;

/// Longest a poll spends taking in device data, so commands stay responsive while capturing.
const POLL_BUDGET: Duration = Duration::from_millis(25);

pub struct EngineConfig {
    /// Where working `.lacap` files of new captures and imports go; memory when `None`.
    pub data_dir: Option<PathBuf>,
    pub cache_budget: usize,
    /// Where to look for virtual analysers (`logica-vla`); none unless given, so tests do not
    /// depend on what runs on the machine. The app passes [`EngineConfig::vla_from_env`].
    pub vla_addresses: Vec<String>,
    /// Offer the synthetic device (programs of known signals, in-process). Off in the app, where
    /// the virtual analyser is the way to capture without hardware; tests turn it on.
    pub synthetic: bool,
}

impl EngineConfig {
    /// `LOGICA_VLA` (comma-separated `host:port`), else the analyser's default address.
    pub fn vla_from_env() -> Vec<String> {
        logica_drivers::vla::addresses_from_env()
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            data_dir: None,
            cache_budget: DEFAULT_CACHE_BUDGET,
            vla_addresses: Vec::new(),
            synthetic: false,
        }
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
    /// Samples per channel of the last configuration.
    pub samples: Sample,
}

/// A capture running on a worker thread. Its data arrives on `feed` (see [`crate::live`]); the
/// capture joins the session with the first block, in the state `capturing`.
pub struct Running {
    pub device: String,
    pub capture: String,
    pub title: String,
    pub source: Source,
    pub stop: StopSignal,
    pub handle: JoinHandle<RunResult>,
    pub feed: mpsc::Receiver<Delivery>,
    /// Where the store goes, until the device begins.
    pub target: Option<StoreTarget>,
    /// Samples asked for, for the progress fraction.
    pub samples: Sample,
    /// Whether the device began, so the capture is in the session.
    pub begun: bool,
    /// Why the data could not be taken in; the capture is stopped and keeps what arrived.
    pub failed: Option<String>,
    /// Samples in the last progress event.
    pub reported: Sample,
}

pub type RunResult = (Box<dyn Device>, Result<RunOutcome, String>);

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
    /// The person's "stop agent actions" switch (doc 08): agents' capture and output commands
    /// are refused while it is on.
    pub(crate) agents_paused: bool,
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
        let mut drivers: Vec<Box<dyn VirtualDriver>> = Vec::new();
        if cfg.synthetic {
            drivers.push(Box::new(logica_drivers::synthetic::SyntheticDriver));
        }
        if !cfg.vla_addresses.is_empty() {
            drivers.push(Box::new(logica_drivers::vla::VlaDriver::new(cfg.vla_addresses.clone())));
        }
        Engine {
            cfg,
            registry: logica_protocols::registry(),
            custom: Vec::new(),
            captures: BTreeMap::new(),
            active: None,
            history: Vec::new(),
            drivers,
            devices: BTreeMap::new(),
            leases: BTreeMap::new(),
            running: Vec::new(),
            agents_paused: false,
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

    /// Marks the capture a command changed as unsaved. Views are not stored in files, and opening,
    /// saving and closing set the state themselves.
    fn touch(&mut self, inv: &Invocation) {
        if matches!(inv.command.as_str(), "view.set" | "file.open" | "file.save" | "capture.close") {
            return;
        }
        if let Some(c) =
            inv.input.get("capture").and_then(Value::as_str).and_then(|id| self.captures.get_mut(id))
        {
            c.unsaved = true;
        }
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
        let unsaved = path.is_none();
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
                // a capture from a device exists nowhere else; an opened file is on disk
                unsaved,
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
        if self.agents_paused
            && matches!(actor, Actor::Agent(_))
            && matches!(s.safety, crate::contract::Safety::Capture | crate::contract::Safety::Output)
        {
            return Err(err(
                ErrorCode::NotAllowed,
                format!(
                    "The person has stopped agent actions in the window, so {} is not run. Ask them to resume agent actions.",
                    inv.command
                ),
            ));
        }
        let ctx = Ctx { actor: actor.clone() };
        let done = crate::cmd::dispatch(self, &ctx, &inv.command, inv.input.clone())?;
        if done.record {
            self.touch(inv);
        }
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
        self.touch(inv);
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
        self.touch(&e.done);
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

    /// Takes in what running captures delivered and collects finished ones; called before every
    /// command and, while capturing, regularly by the host.
    pub fn poll(&mut self) {
        let mut i = 0;
        while i < self.running.len() {
            let mut r = self.running.remove(i);
            self.pump(&mut r, Some(Instant::now() + POLL_BUDGET));
            if r.handle.is_finished() {
                self.finish_running(r);
            } else {
                self.running.insert(i, r);
                i += 1;
            }
        }
    }

    /// Whether a capture is running (the host polls more often then).
    pub fn is_capturing(&self) -> bool {
        !self.running.is_empty()
    }

    /// Applies the deliveries waiting on `r`'s feed, until `until` if given, then brings its live
    /// decoders up to date and reports progress.
    fn pump(&mut self, r: &mut Running, until: Option<Instant>) {
        let mut any = false;
        while until.is_none_or(|t| Instant::now() < t) {
            let Ok(d) = r.feed.try_recv() else { break };
            self.deliver(r, d);
            any = true;
        }
        if any {
            self.live_update(r);
        }
    }

    fn deliver(&mut self, r: &mut Running, d: Delivery) {
        match d {
            Delivery::Begin { channels, initial, sample_rate_hz } => {
                self.live_begin(r, channels, &initial, sample_rate_hz)
            }
            Delivery::Note(n) => {
                if let Some(c) = self.captures.get_mut(&r.capture) {
                    c.meta.notes.push(n);
                }
            }
            data => {
                if r.failed.is_some() {
                    return;
                }
                let Some(c) = self.captures.get_mut(&r.capture) else {
                    r.failed = Some("The device sent data before it began the capture.".into());
                    r.stop.stop();
                    return;
                };
                if let Err(e) = data.apply(&mut c.store) {
                    r.failed = Some(e.to_string());
                    r.stop.stop();
                }
            }
        }
    }

    /// The device began: the capture joins the session and becomes the active tab.
    fn live_begin(&mut self, r: &mut Running, channels: Vec<ChannelInfo>, initial: &[Level], rate: f64) {
        let Some(target) = r.target.take() else {
            r.failed = Some("The device began the capture twice.".into());
            r.stop.stop();
            return;
        };
        let store = match target.create(initial) {
            Ok(s) => s,
            Err(e) => {
                r.failed = Some(e.to_string());
                r.stop.stop();
                return;
            }
        };
        let meta = CaptureMeta {
            id: r.capture.clone(),
            title: r.title.clone(),
            source: r.source.clone(),
            sample_rate_hz: rate,
            trigger: None,
            channels,
            segments: whole(0),
            notes: Vec::new(),
            state: CaptureState::Capturing,
            artefact: None,
            saved_decoders: Vec::new(),
        };
        self.add_capture(r.capture.clone(), meta, store, None, false);
        r.begun = true;
        self.changed();
    }

    /// Advances the live decoders of `r`'s capture and reports its progress.
    fn live_update(&mut self, r: &mut Running) {
        let Some(c) = self.captures.get_mut(&r.capture) else { return };
        let samples = c.store.sample_count();
        let mut updated = Vec::new();
        for (id, d) in &mut c.decoders {
            if d.advance_live(&c.store) {
                updated.push(id.clone());
            }
        }
        if samples != r.reported {
            r.reported = samples;
            let fraction = (r.samples > 0).then(|| (samples as f64 / r.samples as f64).min(1.0));
            self.emit(Event::CaptureProgress { capture: r.capture.clone(), samples, fraction });
        }
        for decoder in updated {
            self.emit(Event::DecoderUpdated { capture: r.capture.clone(), decoder });
        }
    }

    /// Waits for a capture's device to finish, taking in everything it still delivers, and ends
    /// the capture: complete, or aborted with the reason, keeping what arrived.
    pub(crate) fn finish_running(&mut self, mut r: Running) {
        loop {
            self.pump(&mut r, None);
            match r.feed.recv_timeout(Duration::from_millis(5)) {
                Ok(d) => self.deliver(&mut r, d),
                // the device thread dropped its sink: it is done
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        self.pump(&mut r, None);
        let Ok((device, result)) = r.handle.join() else {
            if r.begun {
                self.end_live(&r.capture, None, true, Some("The device driver stopped unexpectedly.".into()));
            } else {
                self.capture_failed(&r.capture, "the device driver stopped unexpectedly");
            }
            return;
        };
        if let Some(d) = self.devices.get_mut(&r.device) {
            d.device = Some(device);
        }
        if !r.begun {
            let why =
                r.failed.clone().or(result.err()).unwrap_or_else(|| "The device delivered no data.".into());
            self.capture_failed(&r.capture, &why);
            return;
        }
        match result {
            Ok(outcome) => {
                let failed = r.failed.clone();
                self.end_live(&r.capture, Some(outcome.samples), outcome.stopped, failed);
            }
            Err(e) => self.end_live(&r.capture, None, true, Some(r.failed.clone().unwrap_or(e))),
        }
    }

    /// Ends a capture that began: the store is finished at its end, live decoders emit what is
    /// pending, and the state says whether it is complete.
    fn end_live(&mut self, capture: &str, samples: Option<Sample>, stopped: bool, failed: Option<String>) {
        let Some(c) = self.captures.get_mut(capture) else {
            // closed while capturing
            self.emit(Event::CaptureDone { capture: capture.into(), samples: 0, stopped: true });
            self.changed();
            return;
        };
        let have = c.store.sample_count();
        let end = if failed.is_some() { have } else { samples.unwrap_or(have).max(have) };
        if let Err(e) = c.store.finish(end) {
            c.meta.notes.push(Note {
                level: NoteLevel::Warning,
                message: format!("The capture could not be finished: {e}."),
            });
        }
        if let Some(why) = &failed {
            c.meta.notes.push(Note {
                level: NoteLevel::Warning,
                message: format!(
                    "The capture failed after {end} samples: {why} What arrived until then is kept."
                ),
            });
        } else if stopped {
            c.meta.notes.push(Note {
                level: NoteLevel::Info,
                message: format!("The capture was stopped after {end} samples."),
            });
        }
        let stopped = stopped || failed.is_some();
        c.meta.state = if stopped { CaptureState::Aborted } else { CaptureState::Complete };
        c.meta.segments = whole(end);
        let mut updated = Vec::new();
        for (id, d) in &mut c.decoders {
            if d.finish_live(&c.store, end) {
                updated.push(id.clone());
            }
        }
        for decoder in updated {
            self.emit(Event::DecoderUpdated { capture: capture.into(), decoder });
        }
        self.emit(Event::CaptureDone { capture: capture.into(), samples: end, stopped });
        self.changed();
    }

    /// Stops the capture running into `capture`, if one is, and ends it.
    pub(crate) fn stop_capture_of(&mut self, capture: &str) {
        if let Some(k) = self.running.iter().position(|r| r.capture == capture) {
            let r = self.running.remove(k);
            r.stop.stop();
            self.finish_running(r);
        }
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
