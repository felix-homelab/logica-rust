# 12 Decisions and risks

## Decisions

Decided by the owner in November 2026, unless marked open. Do not re-open them unless something in
practice proves one wrong.

| # | Decision | Outcome |
|---|---|---|
| D0 | What the project is | An open-source product for hobbyists and engineers, not only a personal tool |
| D1 | Licence | Apache-2.0; drivers and importers clean-room under the "facts only" rule (ADR 0009, [clean-room.md](clean-room.md)) |
| D2 | Application form | Desktop app: Electron window with React, engine in Rust (a Node-API module in the app, a standalone binary for the CLI and headless MCP); no browser version (ADR 0002) |
| D3 | Repository | One repository: a Cargo workspace for the engine, an npm workspace for the window (doc 03) |
| D4 | Hardware for testing | DLA-32 Plus, an FX2 clone, a Saleae Logic (**open:** model; USB vendor ID `0925` = original FX2-based Logic, `21a9` = newer Saleae, from memory and to be checked), ESP32 and Arduino for the rig. No DLA-16 Plus, no Mac |
| D5 | Hosting | The owner's personal GitHub |
| D6 | Windows driver for the DLA | Zadig guide plus "switch back" guide in version 1; FFI to an already installed WCH library evaluated after spike S3 |
| D7 | Name | The project is **Logica**; the repository is `logica-rust`; the command is `logica`, and the MCP server registers as `logica`. **Open:** whether `logica` is also published on npm or crates.io, decided at M6 (the npm name `logica` was taken by an unrelated package in November 2026) |
| D8 | Signing and distribution | Unsigned installers for Windows, macOS and Linux via GitHub Releases; ad-hoc signature on macOS so it runs at all; auto-update on Windows and Linux only |
| D9 | First hardware | The full analyser for recordings (M1, M2) before any hardware; then the DLA feasibility spike; DLA buffer mode before FX2 (re-decided after M0: S3 moved from after M0 to after M2) |
| D10 | Drivers in version 1 | Synthetic, DLA-32/16 Plus, FX2. Saleae automation, sigrok bridge and Pico after version 1 |
| D11 | Scope cuts | Lean UI polish (native frame, tabs, no screenshot tests, no jump lists); JSON custom protocols only; Learn tab after version 1; all importers including `.sal` best effort; register maps, layer decoders, protocol triggers and capture comparison in version 1 |
| D12 | Signal Detective | `../signal-detective` (the owner's own work, ISC licence) is the behavioural reference: its code is read for heuristics, evidence texts and protocol knowledge, not ported. Its samples (in `fixtures/captures/`), answer keys and test expectations are the acceptance tests. Learn-tab parts (identifier, calculators, waveform sketches) are left out. It stays a separate, finished tool |
| D13 | Architecture | One engine, command API, curated MCP tools over HTTP only (ADR 0001, 0006); one engine per desktop (ADR 0007); single-file `.lacap` (ADR 0005); undo history and device lease (ADR 0008); typed JSON queries; output confirmation in the app only; performance budgets checked locally, CI only catches large regressions |

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The DLA cannot be driven, or the clean-room protocol work stalls | Medium | Main differentiator gone | Spike S3 right after M2, before driver work; stop and re-plan the hardware milestones if it fails. The analyser for recordings is useful without it |
| Licence contamination: someone (or an AI agent) writes driver code from memory of GPL sigrok code | Medium | Relicensing or a rewrite | "Facts only" rule in CLAUDE.md and clean-room.md; provenance note per driver; people who read the code do not write that driver |
| FNIRSI example files published by mistake | Low | Copyright complaint | `fixtures/captures/local/` git-ignored; tests skip when absent; replace with rig recordings |
| USB throughput below the device rate | Low to medium | Streaming limited | Spike S2; buffer mode first; honest rate limits in capabilities |
| The engine module fails to build or load on one platform (Windows, macOS, Linux; x64, arm64) | Medium | No app on that platform | Spike S7; Node-API keeps the module ABI-stable across Electron upgrades; packaging smoke test on every system in CI |
| The command API contract drifts between Rust and TypeScript | Medium | Window breaks at runtime | TypeScript types generated from the contract; CI fails on stale generated files |
| Every engine feature costs more to write in Rust than in TypeScript | Certain | Slower progress | Accepted for predictable live capture and memory (ADR 0002); protocols stay small, testable modules |
| Electron is large (about 100 MB installers) and uses more memory than a native window | Certain | Download size, RAM | Accepted for a mature, accessible UI (ADR 0002); capture data stays in the engine with a memory budget |
| Unsigned installers scare people (SmartScreen, Gatekeeper quarantine) | High | Adoption | Install guide with screenshots; revisit signing if users ask |
| macOS build broken without anyone noticing (owner has no Mac) | Medium | macOS users stuck | Packaging smoke test on macOS CI; release notes say "untested on real hardware" |
| DLA protocol changes with FNIRSI firmware updates | Medium | Driver breaks | Firmware version check; replay transcripts per firmware |
| Windows driver binding (WCH vendor driver vs WinUSB) frustrates people | High | Setup friction | Step-by-step guide and undo; FFI option (D6) |
| `.sal` changes with every Logic 2 version | High | Best-effort import breaks | Only versions with fixtures; clear message to export binary instead |
| Scope creep (it is a big product) | High | Never finishes | Milestones end usable; the four differentiators (doc 01) decide what waits |
| Agents misuse hardware (outputs, wrong thresholds) | Low | Damage to the device under test | Output safety class off by default, confirmation in the app only, device lease, activity feed, stop button |
| Memory limits with huge captures | Low | Crashes | Chunk store with eviction to disk; the window holds only what it draws |
| The MCP specification evolves | Medium | Rework in `mcp` | Keep the MCP layer thin over the command API; follow the official specification |
| An agent launching the app surprises the person | Low | Confusion | The app starts minimised and says "started by an agent"; `--headless` for those who want no window |
| WSL users cannot see USB devices | High for the owner | Setup friction | Run the Windows build; Claude Code in WSL attaches via `logica.exe mcp` or mirrored networking (S8); usbipd guide as fallback |
