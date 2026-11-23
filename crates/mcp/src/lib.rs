//! Logica's MCP server (doc 07, ADR 0006): a small set of task-shaped tools, `run_command` for
//! every other command, resources, subscriptions and prompts, all as a client of the command API.
//! The transports (stdio here; Streamable HTTP with its lock file) only carry it.

pub mod app;
pub mod http;
pub mod lock;
pub mod prompts;
pub mod relay;
pub mod resources;
pub mod server;
pub mod shared;
pub mod text;
pub mod tools;

use std::sync::Arc;

use rmcp::ServiceExt;
use serde_json::Value;

pub use server::Logica;
pub use shared::Shared;

/// The text that goes with a rendered picture: what its rows show.
fn render_note(out: &Value) -> String {
    let rows: Vec<&str> =
        out["rows"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    format!(
        "A {}×{} pixel picture of samples {} to {} follows. Rows from the top: {}.",
        out["width"],
        out["height"],
        out["from"],
        out["to"],
        rows.join("; ")
    )
}

/// Serves one client over stdin and stdout until it disconnects (`logica mcp --headless`).
pub async fn serve_stdio(shared: Arc<Shared>) -> std::io::Result<()> {
    let service = Logica::new(shared)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| std::io::Error::other(format!("The MCP connection could not start: {e}")))?;
    service
        .waiting()
        .await
        .map_err(|e| std::io::Error::other(format!("The MCP connection ended badly: {e}")))?;
    Ok(())
}

/// How `logica mcp` runs.
pub struct Options {
    /// Where the lock file is (the data folder, doc 03).
    pub data_dir: std::path::PathBuf,
    /// Where an engine started here keeps its working files.
    pub work_dir: std::path::PathBuf,
    pub allow_outputs: bool,
    /// Never start the app; become a windowless engine when none runs.
    pub headless: bool,
}

/// `logica mcp`: relays to the running engine if there is one (ADR 0007: one engine per
/// desktop); otherwise becomes that engine without a window, serving this agent over stdio and
/// others over HTTP, with a lock file so they find it.
pub async fn stdio_entry(o: Options) -> std::io::Result<()> {
    if let Some(l) = lock::running(&o.data_dir) {
        return relay::relay(tokio::io::stdin(), tokio::io::stdout(), l.port, &l.token).await;
    }
    // no engine yet: on a desktop, start the app (minimised) and relay to it (ADR 0007)
    if !o.headless
        && app::display_available()
        && let Some(installed) = app::installed(&o.data_dir)
    {
        app::launch(&installed, &[std::ffi::OsStr::new(app::STARTED_BY_AGENT)])?;
        let dir = o.data_dir.clone();
        let found = tokio::task::spawn_blocking(move || {
            app::wait_for_engine(&dir, std::time::Duration::from_secs(20))
        })
        .await
        .map_err(std::io::Error::other)?;
        return match found {
            Some(l) => relay::relay(tokio::io::stdin(), tokio::io::stdout(), l.port, &l.token).await,
            None => Err(std::io::Error::other(format!(
                "Logica ({}) was started but did not offer MCP within 20 s. Check that MCP is on in its settings, or run logica mcp --headless.",
                installed.executable.display()
            ))),
        };
    }
    let engine = logica_engine::Engine::new(logica_engine::EngineConfig {
        data_dir: Some(o.work_dir.clone()),
        ..logica_engine::EngineConfig::default()
    });
    let shared = Shared::new(engine, o.allow_outputs);
    let server = http::serve(shared.clone(), lock::new_token()?).await?;
    let me = lock::Lock {
        pid: std::process::id(),
        port: server.addr.port(),
        token: server.token.clone(),
        api_version: logica_engine::contract::api_version(),
        headless: true,
    };
    match lock::write(&o.data_dir, &me) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // another engine started at the same moment: use that one
            server.stop().await;
            if let Some(l) = lock::running(&o.data_dir) {
                return relay::relay(tokio::io::stdin(), tokio::io::stdout(), l.port, &l.token).await;
            }
            return Err(e);
        }
        Err(e) => eprintln!(
            "logica: the lock file in {} could not be written ({e}); other agents will not find this engine.",
            o.data_dir.display()
        ),
    }
    let result = serve_stdio(shared).await;
    lock::remove_if_ours(&o.data_dir, &me);
    server.stop().await;
    result
}
