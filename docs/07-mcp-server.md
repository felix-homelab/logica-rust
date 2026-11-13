# 07 MCP server

The MCP server lets an agent use the analyser the way a person does: pick a device, capture, look,
decode, search, measure, and show the person what it found. It is a client of the command API
(doc 03), so anything the window can do, an agent can do, with the same results.

## Transports and setup

MCP is the only protocol outside the app (ADR 0006).

- **Streamable HTTP**: the engine serves `http://127.0.0.1:<port>/mcp` while the app runs (on by
  default, switchable in settings). Several agents can attach at once; all share the session with
  the window.
- **stdio**: `logica mcp` speaks MCP on stdin/stdout and relays to the app's HTTP endpoint (found
  through the lock file with port and token). If the app is not running and a display is available,
  it starts the app minimised first (ADR 0007). With `--headless`, or without a display (CI), it
  runs a headless engine instead. For Claude Code: `claude mcp add logica -- logica mcp`
  (the installer puts `logica` on the PATH).
- **WSL with the app on Windows:** `claude mcp add logica -- /mnt/c/…/logica.exe mcp`
  (stdio through WSL interop; starts the Windows app if needed), or HTTP with mirrored networking
  (doc 02, spike S8).
- The settings window shows connected agents and a copyable setup command.

## Security

- Bind to 127.0.0.1 only. Binding elsewhere needs an explicit `--listen` flag and prints a warning.
- Validate the `Origin` header on HTTP requests (required by the MCP specification against DNS
  rebinding).
- A random bearer token per engine start, stored in the lock file (user-only permissions). The
  stdio relay reads it; HTTP clients must send it.
- Safety classes: `read` (analysis), `capture` (hardware inputs), `output` (drives pins, such as
  PWM). `output` is off until the person enables it in the app for the session. An agent's request
  opens a dialog **in the app** (with an OS notification if the window is in the background). MCP
  elicitation is not used for this: it is answered in the agent's client, which may auto-accept. A
  headless engine has no one to ask, so it refuses `output` commands unless started with
  `--allow-outputs`.
- Hardware commands need the device lease (ADR 0008); a busy device returns who holds it.
- The activity feed shows every command with its actor; the person can undo agents' actions and
  stop them.
- Hard limits on tool output size; no tool returns unbounded raw sample data.

## Tools

A small, hand-designed set (ADR 0006): every tool definition costs context in every agent session,
and few task-shaped tools are chosen more reliably. Everything else is reachable through
`run_command`.

Every tool returns a compact text summary first (what an agent needs for its next step) and
structured content second. Times are given in seconds with sensible precision **and** as sample
indices, so follow-up calls are exact.

| Tool | Safety class | Does |
|---|---|---|
| `list_devices` | read | Connected and available devices: driver, model, state, lease holder, capabilities, setup hints for devices that need permission or a driver |
| `device` | capture | `connect` or `disconnect` a device; connecting takes the lease and returns its capabilities |
| `capture` | capture | `start` with the configuration inline (channels and names, sample rate, duration or sample count, threshold, buffer or stream mode, trigger as a query, doc 06); validates against capabilities and explains rejections. `wait: true` (default) blocks with progress notifications and can be cancelled. `stop` ends a running capture |
| `session` | read | Open captures and their state (samples so far, data-loss notes), decoders with problem counts, markers, the person's view per tab (range, cursors, selection), lease holders, recent activity |
| `open_file` | read | Import a file from a path on the machine running the engine (doc 09) |
| `save_capture` | read | Save as `.lacap`, or export as `.sr`, VCD, CSV, Saleae binary, or decoded packets (CSV, JSON) |
| `summarize` | read | Per-channel measurements, bus groups, detected protocols with confidence and evidence, problems; for the whole capture or a time range; bounded length |
| `measure` | read | `stats` (frequency, period, duty, pulse widths, edge counts for a channel and range), `edges` (edges of chosen channels in a range, capped and paged), `between` (time between two edges) |
| `decoder` | read | `add` (protocol, roles, settings, layers, register map; or from a detection), `update` (returns how the problem counts changed), `remove` |
| `get_decoded` | read | Packets or frames filtered by time range, kind, problem and text; paged. `explain: true` groups problems and explains likely causes from the symptom knowledge |
| `search` | read | Query search over edges and decoded content (doc 06); hits with times |
| `compare` | read | Two captures, or decoded data against expected bytes or text; differences with times |
| `render` | read | PNG image of a time range with decoder rows and markers, for agents that can see images (rendered by the engine, works headless) |
| `show_user` | read | Point the person at something: a marker or range note, optionally moving their view there and posting a short message in the activity feed; shown with an "agent" badge |
| `set_output` | output | PWM or other outputs, if the device has them and the person allowed outputs |
| `run_command` | per command | Any command of the command API by name (for example `history.undo`, `analysis.bytes`, `capture.close`), validated by its contract; the command's own safety class and lease rules apply. The list is the resource `logica://commands` |

Tools change only deliberately (schema snapshot tests); a new capability first goes through
`run_command` and earns its own tool when agents need it often.

## Resources

- `logica://session`: devices, captures, decoders, leases.
- `logica://captures/{id}/summary`
- `logica://captures/{id}/decoders/{decoderId}/packets{?from,to,cursor}`
- `logica://protocols` and `logica://protocols/{id}`: the knowledge texts (what it is, how to
  confirm, settings, symptoms), so an agent can learn the domain from the tool itself.
- `logica://commands`: every command with its input schema, safety class and whether it is
  undoable.

Subscriptions: `capture.done` and `decoder.updated` notify subscribed clients.

## Prompts

- `diagnose-bus` (channels): capture, detect, decode, explain problems, mark them for the person.
- `verify-expected` (protocol, expected bytes or text): capture and compare against what the
  firmware should have sent, and report differences with times and markers.

## Output design for language models

- Lead with the answer: "I2C on CH4 (SDA) and CH5 (SCL), 400 kHz, 3 transactions, 0 errors."
- Bounded: summaries under about 2,000 tokens; lists paged with an explicit `nextCursor`.
- Units always spelled out; sample indices included for exact follow-up.
- Errors are instructions: "Sample rate 300 MHz is not available with 20 channels; the maximum is
  250 MHz. Disable channels or lower the rate." / "The DLA-32 Plus is in use by the person; wait or
  ask them to hand it over."
- Deterministic ordering, so repeated calls are comparable.

## Testing

- In-process tests with an MCP client against a headless engine using the synthetic device and the
  sample files (no network).
- HTTP transport: token required, wrong `Origin` rejected, bound only to 127.0.0.1.
- Relay: `logica mcp` relays to a running engine; starts a headless engine with `--headless`;
  refuses `output` commands headless without `--allow-outputs`.
- Golden tests for the summaries of every fixture file (snapshots, reviewed).
- Schema snapshot tests, so tool changes are deliberate.
- Lease and undo through MCP: a second actor gets the "in use" error; `run_command history.undo`
  reverts an agent's decoder change.
- End-to-end scenario: connect the synthetic device, capture its I2C example, detect, decode,
  search for NACK, show the person a marker; every step's result is asserted.
- A manual checklist for Claude Code in `docs/manual-tests.md` (written in M1).
