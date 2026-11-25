//! The Node-API module that loads the engine into Electron's engine process (doc 03, ADR 0002).
//! Commands run on libuv's worker threads and resolve a Promise, so the host's event loop never
//! waits on the engine. Answers and events cross as JSON text; the host passes them on unchanged.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Task};
use napi_derive::napi;
use serde_json::{Value, json};
use tokio::sync::broadcast::{self, error::TryRecvError};

use logica_engine::{Actor, Engine, EngineConfig, Event};
use logica_mcp::http::HttpServer;
use logica_mcp::{Shared, lock};

/// The app's MCP endpoint (doc 07, ADR 0007): served on 127.0.0.1 and announced in the lock
/// file. Plain Rust, so it is tested without Node.
pub struct McpEndpoint {
    runtime: tokio::runtime::Runtime,
    running: Mutex<Option<(HttpServer, std::path::PathBuf, lock::Lock)>>,
}

impl McpEndpoint {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("the engine's async runtime starts");
        McpEndpoint { runtime, running: Mutex::new(None) }
    }

    /// Starts serving and writes the lock file in `dir`; returns the port. Starting again keeps the
    /// first endpoint. Fails when another engine already serves this desktop.
    pub fn start(&self, shared: &Arc<Shared>, dir: &std::path::Path) -> Result<u16, String> {
        let mut slot = self.running.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((s, _, _)) = slot.as_ref() {
            return Ok(s.addr.port());
        }
        let token = lock::new_token().map_err(|e| e.to_string())?;
        let server = self
            .runtime
            .block_on(logica_mcp::http::serve(shared.clone(), token))
            .map_err(|e| format!("MCP could not start: {e}"))?;
        let me = lock::Lock {
            pid: std::process::id(),
            port: server.addr.port(),
            token: server.token.clone(),
            api_version: logica_engine::contract::api_version(),
            headless: false,
        };
        if let Err(e) = lock::write(dir, &me) {
            self.runtime.block_on(server.stop());
            let why = if e.kind() == std::io::ErrorKind::AlreadyExists {
                "another Logica engine already serves agents on this desktop".to_string()
            } else {
                e.to_string()
            };
            return Err(format!("MCP was not started: {why}."));
        }
        let port = me.port;
        *slot = Some((server, dir.to_path_buf(), me));
        Ok(port)
    }

    /// Stops serving and removes the lock file.
    pub fn stop(&self) {
        let taken = self.running.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let Some((server, dir, me)) = taken {
            lock::remove_if_ours(&dir, &me);
            self.runtime.block_on(server.stop());
        }
    }
}

impl Default for McpEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

/// The engine of the app.
#[napi]
pub struct LogicaEngine {
    shared: Arc<Shared>,
    events: Arc<Mutex<broadcast::Receiver<Event>>>,
    mcp: McpEndpoint,
}

#[napi]
impl LogicaEngine {
    /// `workDir`: where working captures go (the data folder, doc 03); `cacheMb`: the chunk cache
    /// (doc 04; the settings' memory for capture data), the engine's default when absent.
    #[napi(constructor)]
    pub fn new(work_dir: String, cache_mb: Option<u32>) -> Self {
        let mut cfg = EngineConfig {
            data_dir: Some(work_dir.into()),
            vla_addresses: EngineConfig::vla_from_env(),
            ..EngineConfig::default()
        };
        if let Some(mb) = cache_mb.filter(|mb| *mb >= 16) {
            cfg.cache_budget = usize::try_from(mb).unwrap_or(usize::MAX).saturating_mul(1 << 20);
        }
        let engine = Engine::new(cfg);
        // outputs need the person's confirmation in the window; until that exists (M3) they are refused
        let shared = Shared::new(engine, false);
        let events = Arc::new(Mutex::new(shared.events()));
        LogicaEngine { shared, events, mcp: McpEndpoint::new() }
    }

    /// Serves MCP on 127.0.0.1 and announces it in the lock file in `dataDir`. Returns the port.
    #[napi]
    pub fn start_mcp(&self, data_dir: String) -> napi::Result<u32> {
        self.mcp
            .start(&self.shared, std::path::Path::new(&data_dir))
            .map(u32::from)
            .map_err(napi::Error::from_reason)
    }

    /// Stops serving MCP and removes the lock file.
    #[napi]
    pub fn stop_mcp(&self) {
        self.mcp.stop();
    }

    /// The command API version, "major.minor".
    #[napi]
    pub fn api_version(&self) -> String {
        logica_engine::contract::api_version()
    }

    /// Runs a command. Resolves to `{"ok":true,"output":…}` or `{"ok":false,"error":{code,message}}`
    /// as JSON text; it never rejects for a command error.
    #[napi]
    pub fn execute(&self, actor: String, command: String, input: String) -> AsyncTask<Execute> {
        AsyncTask::new(Execute { shared: self.shared.clone(), actor, command, input })
    }

    /// Waits up to `timeoutMs` for engine events and resolves to a JSON array of them (empty when
    /// none came).
    #[napi]
    pub fn next_events(&self, timeout_ms: u32) -> AsyncTask<NextEvents> {
        AsyncTask::new(NextEvents {
            events: self.events.clone(),
            timeout: Duration::from_millis(timeout_ms.into()),
        })
    }
}

pub struct Execute {
    shared: Arc<Shared>,
    actor: String,
    command: String,
    input: String,
}

fn envelope(r: Result<Value, (String, String)>) -> String {
    match r {
        Ok(output) => json!({ "ok": true, "output": output }),
        Err((code, message)) => json!({ "ok": false, "error": { "code": code, "message": message } }),
    }
    .to_string()
}

/// Runs one command and wraps the answer (also used by the tests, without Node).
pub fn run(shared: &Shared, actor: &str, command: &str, input: &str) -> String {
    let actor: Actor = match serde_json::from_value(Value::String(actor.into())) {
        Ok(a) => a,
        Err(_) => {
            return envelope(Err((
                "invalid-input".into(),
                format!("\"{actor}\" is not an actor: user, cli or agent:<name>."),
            )));
        }
    };
    let input: Value = match serde_json::from_str(if input.trim().is_empty() { "{}" } else { input }) {
        Ok(v) => v,
        Err(e) => {
            return envelope(Err((
                "invalid-input".into(),
                format!("The input of {command} is not JSON: {e}."),
            )));
        }
    };
    envelope(shared.run(&actor, command, input).map_err(|e| {
        let code =
            serde_json::to_value(e.code).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        (code, e.message)
    }))
}

impl Task for Execute {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<String> {
        Ok(run(&self.shared, &self.actor, &self.command, &self.input))
    }

    fn resolve(&mut self, _: Env, output: String) -> napi::Result<String> {
        Ok(output)
    }
}

pub struct NextEvents {
    events: Arc<Mutex<broadcast::Receiver<Event>>>,
    timeout: Duration,
}

/// Collects the events that arrive within `timeout` (returns early once some arrived).
pub fn wait_events(rx: &Mutex<broadcast::Receiver<Event>>, timeout: Duration) -> String {
    let until = Instant::now() + timeout;
    let mut rx = rx.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(e) => out.push(serde_json::to_value(&e).unwrap_or(Value::Null)),
            Err(TryRecvError::Lagged(n)) => out.push(json!({ "event": "lagged", "missed": n })),
            Err(TryRecvError::Empty) => {
                if !out.is_empty() || Instant::now() >= until {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(TryRecvError::Closed) => break,
        }
    }
    Value::Array(out).to_string()
}

impl Task for NextEvents {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<String> {
        Ok(wait_events(&self.events, self.timeout))
    }

    fn resolve(&mut self, _: Env, output: String) -> napi::Result<String> {
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_serves_mcp_once_and_announces_it() {
        let dir = std::env::temp_dir().join(format!("logica-node-mcp-{}", std::process::id()));
        let _ = std::fs::remove_file(lock::path(&dir));
        let (s, mcp) = (shared(), McpEndpoint::new());
        let port = mcp.start(&s, &dir).unwrap();
        assert_eq!(mcp.start(&s, &dir).unwrap(), port, "a second start keeps the first");
        let l = lock::running(&dir).expect("the lock names a live endpoint");
        assert_eq!((l.port, l.headless), (port, false));
        // a second app on the same desktop is told why it does not serve
        let why = McpEndpoint::new().start(&shared(), &dir).unwrap_err();
        assert!(why.contains("another Logica engine already serves agents"), "{why}");
        mcp.stop();
        assert!(!lock::path(&dir).exists());
        mcp.stop();
        let _ = std::fs::remove_dir(&dir);
    }

    use super::*;

    fn shared() -> Arc<Shared> {
        Shared::new(Engine::new(EngineConfig::default()), false)
    }

    #[test]
    fn answers_are_envelopes_and_errors_never_throw() {
        let s = shared();
        let v: Value = serde_json::from_str(&run(&s, "user", "session.get", "")).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["output"]["apiVersion"], "1.0");
        let v: Value =
            serde_json::from_str(&run(&s, "user", "capture.close", r#"{"capture":"c9"}"#)).unwrap();
        assert_eq!((v["ok"].as_bool(), v["error"]["code"].as_str()), (Some(false), Some("not-found")));
        let v: Value = serde_json::from_str(&run(&s, "robot", "session.get", "{}")).unwrap();
        assert!(v["error"]["message"].as_str().unwrap().contains("is not an actor"));
        let v: Value = serde_json::from_str(&run(&s, "user", "session.get", "{nope")).unwrap();
        assert!(v["error"]["message"].as_str().unwrap().contains("is not JSON"));
    }

    #[test]
    fn events_arrive_or_the_wait_ends_empty() {
        let s = shared();
        let rx = Mutex::new(s.events());
        let t = Instant::now();
        assert_eq!(wait_events(&rx, Duration::from_millis(30)), "[]");
        assert!(t.elapsed() >= Duration::from_millis(30));
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/captures/1_UART_easy.demo");
        run(&s, "user", "file.open", &json!({ "path": path }).to_string());
        let v: Value = serde_json::from_str(&wait_events(&rx, Duration::from_secs(2))).unwrap();
        let kinds: Vec<&str> = v.as_array().unwrap().iter().filter_map(|e| e["event"].as_str()).collect();
        assert!(kinds.contains(&"command.executed") && kinds.contains(&"state.changed"), "{kinds:?}");
    }
}
