//! The engine shared by every MCP client: commands run one at a time behind a lock, and engine
//! events are fanned out to async listeners.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;
use tokio::sync::broadcast;

use logica_engine::contract::Safety;
use logica_engine::{Actor, CommandError, Engine, ErrorCode, Event, Invocation};

/// Engine plus the event fan-out. Cheap to clone through `Arc`.
pub struct Shared {
    engine: Mutex<Engine>,
    events: broadcast::Sender<Event>,
    /// A headless engine has no one to ask, so it refuses `output` commands unless allowed (doc 07).
    allow_outputs: bool,
}

impl Shared {
    pub fn new(mut engine: Engine, allow_outputs: bool) -> Arc<Self> {
        let rx = engine.subscribe();
        let (events, _) = broadcast::channel(1024);
        let tx = events.clone();
        // ends when the engine (and with it the sending side) is dropped
        std::thread::spawn(move || {
            while let Ok(e) = rx.recv() {
                let _ = tx.send(e);
            }
        });
        Arc::new(Shared { engine: Mutex::new(engine), events, allow_outputs })
    }

    pub fn engine(&self) -> MutexGuard<'_, Engine> {
        // a panic inside a command leaves the engine usable; the history is append-only
        self.engine.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Runs one command, applying the output rule of this engine.
    pub fn run(&self, actor: &Actor, command: &str, input: Value) -> Result<Value, CommandError> {
        if !self.allow_outputs
            && logica_engine::commands::spec(command).is_some_and(|s| s.safety == Safety::Output)
        {
            return Err(CommandError::new(
                ErrorCode::NotAllowed,
                format!(
                    "{command} drives a device output. This engine runs without a window, so no one can confirm it; start it with --allow-outputs to permit outputs."
                ),
            ));
        }
        self.engine().execute(actor, &Invocation { command: command.into(), input })
    }

    /// Runs one command off the async runtime.
    pub async fn run_async(
        self: &Arc<Self>,
        actor: Actor,
        command: String,
        input: Value,
    ) -> Result<Value, CommandError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.run(&actor, &command, input)).await.unwrap_or_else(|e| {
            Err(CommandError::new(ErrorCode::Failed, format!("The command stopped: {e}.")))
        })
    }
}
