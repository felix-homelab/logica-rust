# ADR 0006: MCP over HTTP is the only external protocol; MCP tools are curated

Status: accepted

## Context
Every way into the engine needs code, tests and its own security review. And one MCP tool per
command would mean about 35 tools: every tool definition sits in the agent's context for the whole
session, and many similar tools make agents pick the wrong one.

## Decision
- Outside the app, the engine speaks only MCP over Streamable HTTP on 127.0.0.1 (token, `Origin`
  check; doc 07). There is no custom local-socket protocol.
- `logica mcp` is a stdio-to-HTTP relay to that endpoint (and starts the app when needed, ADR 0007).
- Other CLI subcommands (`logica convert`, `logica open`, `logica inspect`) run an engine
  in-process; they do not attach to the app.
- Inside the app, the window reaches the engine over a `MessagePort`. Tests call the engine
  in-process.
- The MCP surface is a hand-designed set of about 15 task-shaped tools (doc 07), plus `run_command`,
  which runs any command by name with the same validation. Coverage stays complete; the everyday
  tools stay few.

## Consequences
- One network protocol to secure and test.
- Each new feature either fits an existing tool or is reachable through `run_command` until it earns
  its own tool.
- A script that wants the full command API against a running app uses MCP (`run_command`).
- ADR 0001 still holds: every capability is a command first; only the MCP mapping is not 1:1.
