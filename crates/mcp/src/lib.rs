//! Logica's MCP server (doc 07, ADR 0006): a small set of task-shaped tools, `run_command` for
//! every other command, resources, subscriptions and prompts, all as a client of the command API.
//! The transports (stdio here; Streamable HTTP with its lock file) only carry it.

pub mod prompts;
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

/// The text that goes with a rendered picture.
fn render_note(out: &Value) -> String {
    format!("A {}×{} pixel picture of the range follows.", out["width"], out["height"])
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
