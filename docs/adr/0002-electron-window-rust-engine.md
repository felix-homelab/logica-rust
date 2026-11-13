# ADR 0002: Electron window with React; engine in Rust; no browser version

Status: accepted (November 2026)

## Context
The owner wants an installed desktop application, not a page in a web browser, with a modern
interface. A TypeScript prototype showed the storage and decoding design is fast enough in any
language for files (docs/benchmarks.md). What a native engine adds is predictable behaviour during
live capture and with very large captures. What a web interface adds is a mature, accessible,
testable UI.

What Rust improves in the engine:

| Area | Improvement |
|---|---|
| Live capture | No garbage-collection pauses while streaming |
| Ingest | SIMD and threads for unpacking packed samples; headroom for USB 3 rates |
| Large captures | Predictable memory, no JavaScript heap limit, file reads without extra copies |
| Headless use | `logica mcp --headless` and the CLI as one small binary without a Node runtime |

What it does not improve: drawing the waveform (Canvas 2D or WebGL2 on the GPU), and response times
that already take milliseconds.

## Decision
- **The engine is Rust:** capture store, formats, protocols and decoders, drivers, the command API
  with undo history and leases, the MCP server and the `logica` CLI.
- **The window is Electron with React** and strict TypeScript. It reaches the engine only through
  the command API (ADR 0001).
- **In the app, the engine is a Node-API module** loaded into an Electron `utilityProcess`. The
  window talks to that process over a `MessagePort`; large drawing data travels as transferable
  buffers.
- **Without the app, the engine is a standalone binary:** the CLI and the headless MCP server need no
  Node.
- **One contract for both languages:** command and event types are defined in Rust, a JSON Schema is
  generated from them, and the TypeScript types are generated from that schema; CI checks they are
  current.
- **No browser version:** no static page, no WebUSB, no bridge mode.

Options considered:

| Option | Verdict |
|---|---|
| All TypeScript (Electron, Node engine) | Fast enough for files, but garbage-collection pauses in long live captures and the throughput ceiling for USB 3 devices remain open risks |
| **Electron window, Rust engine** | Chosen: native speed and predictable memory where data is processed; keeps the mature UI, accessibility and Playwright tests |
| Fully native Rust window (egui, iced or GPUI on wgpu) | Lowest latency and footprint, but weaker accessibility, text input and UI test tooling; not worth it while drawing already runs on the GPU |
| Tauri (Rust with the system web view) | On Linux it uses WebKitGTK, whose canvas drawing is generally weaker than Chromium's |

## Consequences
- The Chromium version is ours, so Canvas 2D, WebGL2 and WebGPU behave the same for every user.
- Native file dialogs, menus, file associations, drag and drop, several windows.
- Lost: viewing without installing, viewing on a phone. People need the app (or `logica convert` to
  VCD for other tools).
- Cost: installers of about 100 MB; native builds of the engine module and CLI for Windows, Linux
  and macOS on x64 and arm64 in CI; unsigned builds trigger SmartScreen and Gatekeeper warnings (D8).
- Every engine feature costs more to write than in TypeScript (compile times, more ceremony for
  code that changes often). Accepted for the gains in live capture and memory.
- User-written decoders need a plugin host: JSON custom protocols in version 1, WebAssembly plugins
  after it (doc 06).
- Electron security rules apply: context isolation, sandboxed renderer, no Node in the window, only
  the typed command API exposed by the preload (doc 03).
- Crates are chosen when each layer is built, each checked for licence and maintenance (ADR 0009).
