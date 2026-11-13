# 03 Architecture

## One engine, many clients

Everything the instrument can do is a **command** on the **engine**. The desktop window, the MCP
server, the CLI and the tests are all clients of the same command API; nothing is reachable only
from the window (ADR 0001). The engine is Rust; the window is Electron with React (ADR 0002).

```
 ┌──────────────────────────────── desktop app (Electron) ─────────────────────────────────┐
 │                                                                                         │
 │  window (renderer, React) ◀──MessagePort──▶ engine process (utilityProcess)             │
 │     sandboxed, no Node                        Node host ─▶ Rust ENGINE (Node-API module)│
 │     preload: typed command API only             ├─ drivers ─▶ USB                       │──▶ device
 │                                                 ├─ capture store (disk)                 │
 │  main process: windows, menus, dialogs,         ├─ undo history, leases                 │
 │  file associations, updates, lock file          └─ MCP server (HTTP /mcp)               │
 └─────────────────────────────────────────────────────────┼───────────────────────────────┘
                                                           │ 127.0.0.1, token
 Claude Code ──stdio──▶ `logica mcp` (relay) ──────────────┤ starts the app if it is not running
 other agent ──Streamable HTTP─────────────────────────────┘

 headless (CI, no display, or --headless): `logica mcp --headless` ─▶ same ENGINE, no Node
 file tools: `logica convert` / `logica open` / `logica inspect` ─▶ ENGINE in-process
```

## Processes

| Process | Runs | Why there |
|---|---|---|
| **Main** (Electron) | Windows, menus, native dialogs, file associations, auto-update, the lock file | Electron requires it; kept thin so the window never stalls behind engine work |
| **Engine** (Electron `utilityProcess`) | A thin Node host that loads the Rust engine as a Node-API module. The engine runs its own threads: ingest, decoding, file I/O, the MCP HTTP server | Off the main thread; can crash and restart without taking the window down |
| **Renderer** (one per window; captures are tabs) | React UI, waveform drawing | Sandboxed, context isolation, no Node. Reaches the engine only through the command API from the preload |
| **`logica`** (standalone Rust binary) | `logica mcp`: a stdio-to-HTTP MCP relay to the app, or a headless engine. Other subcommands: the engine in-process | Agents over stdio, CI, file conversion; needs no Node |

The window and the engine process talk over a `MessagePort` created by main (`MessageChannelMain`),
so command traffic does not pass through the main process. Large drawing data (LOD buckets,
visible transitions) travels as transferable `ArrayBuffer`s, not copied.

## Shared session

The window and every agent attach to the same engine session. A capture an agent starts appears in
the window as it streams in; a cursor the person places is visible to the agent through the
`session` tool. Every command carries an **actor** (`user`, `agent:<client name>`, `cli`), so the
activity feed can say "Claude set the sample rate to 10 MHz" and the person can see, undo and stop
what an agent does.

There is one engine per desktop (ADR 0007). When the app is not running, `logica mcp` starts it
(minimised) and relays to it. A headless engine runs only with `--headless` or without a display.

## Undo history and device leases (ADR 0008)

- Every mutating command returns its inverse. The engine keeps one history with the actor of each
  entry; `history.undo` and `history.redo` work on it. Ctrl+Z undoes the person's own last action;
  the activity feed can undo any entry, including agents' actions. Captured data, hardware actions
  and file writes are not undoable and say so.
- Hardware commands (`device.configure`, `capture.start`, `output.*`) need the device's lease. The
  first actor to use a device holds it; others get a clear "in use by …" error. The person can take
  the lease at any time. Analysis state has no locks: last writer wins, visible and undoable.

## Repository layout

```
crates/
  core/        data model, chunked capture store, LOD, measurements, file import and export,
               `.lacap`, device and driver traits, ingest. No I/O of its own.
  protocols/   bus grouping, detectors, streaming decoders, stacking, register maps, custom
               protocols, queries, byte analyser, protocol knowledge, signal generators. No I/O.
  fileio/      the random-access file interface of `core` on real files (`std::fs`), for the
               engine, the CLI, tests and benchmarks
  drivers/     synthetic, fnirsi-dla, fx2lafw (each with a PROVENANCE.md)
  engine/      sessions, device manager, capture pipeline, decoder orchestration, command API,
               events, actors, undo history, leases, files and USB, PNG rendering for `render`
  mcp/         the curated MCP tools, resources and prompts, and `run_command` (ADR 0006)
  cli/         the `logica` binary
  node/        the Node-API binding that loads the engine into the Electron engine process
packages/
  api/         TypeScript types and client for the command API, generated from its contract
  ui/          React components (waveform canvas, panels, forms, tables, command palette) and
               design tokens (doc 08)
apps/
  desktop/     Electron main, preload, engine host and renderer entry; electron-vite and
               electron-builder configuration
fixtures/      capture files, format fixtures, USB replay transcripts
tools/         the known-signal rig (doc 10)
```

Rust crates form one Cargo workspace; `packages/` and `apps/` form one npm workspace.

Dependency direction: `core` ← `protocols` ← `drivers`; `core` ← `fileio`; (`core`, `protocols`,
`drivers`, `fileio`) ← `engine` ← `mcp` ← (`cli`, `node`). Drivers implement traits from `core` and
never depend on the engine. Measurements (frequency, duty, pulse widths) live in `core` because the
`measure` commands need them without protocols; bus grouping is part of detection and lives in
`protocols`. `ui` depends on `packages/api` only, never on the engine. `core` and `protocols` do no I/O
and build for WebAssembly, so they stay testable and usable anywhere.

## The command API

- Each command has a name, an input and output type, a safety class (`read`, `capture`, `output`),
  whether it needs a device lease, whether it is long-running, and how to build its inverse (or
  that it is not undoable).
- **One contract for both languages.** The command and event types are defined in Rust; a JSON
  Schema is generated from them and checked in; the TypeScript types and client in `packages/api`
  are generated from that schema. CI fails when a generated file is out of date. The engine
  validates every input against the contract, whichever client sent it; the window validates
  nothing it does not have to.
- Reached in-process (tests, the CLI, the headless engine) and over the `MessagePort` (window).
  Outside the app, only through MCP (ADR 0006): curated tools plus `run_command`, which runs any
  command by name with the same validation (doc 07).
- Command groups: `session.*`, `device.*`, `capture.*`, `file.*`, `view.*`, `analysis.*`,
  `measure.*`, `decoder.*`, `search.*`, `marker.*`, `channel.*`, `output.*`, `history.*`, `lease.*`.
- **Events** go to every attached client: `capture.progress`, `capture.done`, `decoder.updated`
  (also on undo, redo, open and close), `view.changed`, `lease.changed`, `device.disconnected`,
  `command.executed`, and `state.changed`, which tells a window to read the session again after any
  recorded command, undo, redo or lease change, so it needs no handler per event.
- Versioned: `apiVersion` in the handshake; additive changes only within a major version.
- Errors carry a code (`invalid-input`, `not-found`, `busy`, `failed`, …) and a message written
  for the person or agent, saying what to do.

## Capture pipeline

```
driver ── raw blocks (packed samples / per-channel bits / transitions) ──▶ ingest threads
   ingest: unpack → per-channel transitions → chunk store → LOD
        └─▶ streaming decoders (fed as chunks arrive) ──▶ annotation store
        └─▶ events (progress, new range available) ──▶ clients
```

- Ingest runs on engine threads, never in the window.
- Back-pressure: drivers report how far behind the reader is; the UI and MCP report dropped or
  overwritten data explicitly instead of showing a wrong waveform (the DLA ring buffer replays old
  data when the reader lags; doc 05).

## Services the engine gets from outside

`core` and `protocols` receive what they need: a random-access file interface (real files in the
engine, memory in tests), a USB `Transport` (doc 05), and the clock. Tests inject memory files and
replay transports. The data folder for working captures is passed in by the app (Electron's
user-data folder) and computed the same way by the CLI.

## Electron security baseline

- `contextIsolation: true`, `sandbox: true`, `nodeIntegration: false` for every window.
- The preload exposes one object: the typed command API client, an event subscription, and the
  few window services (native dialogs, the path of a dropped file, the platform). No
  `ipcRenderer`, no file system, no shell.
- Strict Content Security Policy; the renderer loads only packaged files through a custom `app://`
  protocol, never remote content. Navigation and `window.open` are blocked.
- Paths the person picks in native dialogs are passed to the engine as command inputs; the engine
  checks them like any other input.

## Technology choices

| Concern | Choice | Reason |
|---|---|---|
| Engine | Rust (stable), one Cargo workspace | Predictable memory and no garbage-collection pauses during live capture; SIMD and threads for ingest; a single small binary for the CLI and headless MCP (ADR 0002) |
| Window | Electron, React, TypeScript (strict) | Mature, accessible UI with predictable Chromium rendering everywhere; Playwright can test it (ADR 0002) |
| Engine in the app | Node-API module in a `utilityProcess` | ABI-stable native module; the window keeps its `MessagePort` |
| Build | Cargo for the engine; electron-vite for the app; electron-builder for installers (NSIS, AppImage, .deb, dmg) | Standard tools; electron-builder builds all targets and works with electron-updater |
| Updates | electron-updater from GitHub Releases | Works for unsigned Windows and Linux builds; macOS updates need signing |
| UI primitives | Radix UI (menus, dialogs, popovers, tabs, tooltips), Lucide icons | Accessible keyboard behaviour without imposing a look (doc 08) |
| Rendering | Canvas 2D with LOD; WebGL2 if the budgets are missed | Simple; the Chromium version is ours, so behaviour is predictable |
| Tests | `cargo test`; Vitest for TypeScript; Playwright's Electron support for the app | |
| Crates (USB, MCP, Node-API binding, compression, schema generation) | Chosen when the layer is built, each with a sentence on why and a licence check | Not fixed up front; all must be permissive (ADR 0009) |

Not in version 1: serial ports (with serial devices such as Pico analysers) and gRPC (with the
Saleae automation driver). Keep the dependency list short.
