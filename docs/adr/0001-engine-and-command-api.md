# ADR 0001: One engine with a typed command API; the window and MCP are clients

Status: accepted

## Context
Agent control is a core differentiator. If the window holds state and logic, the MCP server ends up
with a smaller, drifting feature set, and tests have to go through the window.

## Decision
All state and behaviour live in an engine with a typed command API and an event stream. The
desktop window, the MCP server, the CLI and the tests are clients. Every command records its actor
(`user`, `agent:<name>`, `cli`), defines its inverse for undo or says it cannot be undone, and says
whether it needs a device lease (ADR 0008).

## Consequences
- Every feature needs a command first; slightly more upfront design per feature.
- MCP coverage is complete by construction (through `run_command`, ADR 0006); shared sessions are
  natural.
- Testing the product mostly means testing commands, without starting the window.
