# 11 Roadmap

Every milestone ends with something usable, agents can test the project from M1 on, and the
riskiest unknowns are measured before the work that depends on them. The biggest unknown, talking
to the DLA-32 Plus at all, is tested right after M0, before any UI work.

## Spikes (small, time-boxed, one question each)

| Spike | Question | When |
|---|---|---|
| S3 | DLA-32 Plus on Windows: does the Rust USB library see it with WinUSB? Does the identification exchange (from the written description, clean-room) work? Record an identification and one buffer capture as transcripts (USBPcap of DLA_Logic for comparison). Chooses the USB library | Right after M0, before M1 |
| S7 | Electron skeleton: a packaged app on Windows and Linux loads the Rust engine module in a `utilityProcess`, the window talks to it over a `MessagePort`, cold start under 1.5 s; built with electron-vite and electron-builder | In M1 |
| S1 | Does the chunked store with per-chunk LOD meet the pan and zoom budget at 16 channels × 1e9 samples in Canvas 2D? | Before M2 |
| S8 | Can Claude Code in WSL reach the Windows app (`logica.exe mcp` through interop, which also starts the app; HTTP with mirrored networking)? | In M2 |
| S2 | What sustained bulk-in rate does the USB library reach with N outstanding transfers, and does ingest keep up (FX2 clone; DLA stream mode)? | Before M4 |

## Milestones

### M0 Foundations
- Cargo workspace and npm workspace (doc 03), lint and format, CI on GitHub Actions, `LICENSE`,
  `.gitignore` excluding `fixtures/captures/local/`.
- `core`: chunked transition store with per-chunk LOD, eviction to a backing, level cursor, edge
  streams and batches (doc 04); measurements; `.demo`, `.sr` and VCD import; `.lacap` read and
  write with recovery from an interrupted save (doc 09). `fileio`: the file interface on real files.
- `protocols`: every protocol of Signal Detective with its detector, streaming decoder and signal
  generator; bus grouping, byte analyser, custom JSON protocols (doc 06).
- `drivers`: the driver traits in use with the synthetic device.
- Tests against the fixtures in `fixtures/` and Signal Detective's expectations (doc 10).
- **Exit:** every Signal Detective sample and test expectation holds (same detections, roles,
  settings, bytes), through the streaming decoders, for any batching and across snapshot and
  restore; the three local-only FNIRSI files pass on the owner's machine; a synthetic capture of
  5e9 samples on 8 channels can be created, saved, reopened and queried within a 1 GB memory budget;
  `.sr`, VCD and `.lacap` tested including malformed and truncated files with clear messages; at
  least as fast as the baseline in `docs/benchmarks.md`.

### Spike S3 (DLA feasibility)
- **Exit:** a written yes or no with transcripts in `fixtures/usb/fnirsi-dla/` and a provenance
  note. If no: stop and re-plan before building M1 and M2.

### M1 Engine, CLI and MCP (files and synthetic device)
- `engine`: command API with its contract and generated TypeScript types, events, actors, undo
  history and device leases (ADR 0008); in-process use and the Node-API binding.
- MCP server over Streamable HTTP with the curated tools and `run_command` (doc 07); `logica mcp`
  as stdio relay and in `--headless` mode; `logica convert`, `logica open`, `logica inspect`.
- Spike S7 (Electron skeleton with an empty window attached to the engine).
- `render`; resources; the `diagnose-bus` prompt.
- **Exit:** from Claude Code, an agent opens `mystery_C.demo`, gets a summary naming I2C on CH4/CH5
  at 400 kHz, reads `CALIBRATED 2026` from the decoded EEPROM read, shows the person a marker and
  undoes a decoder change. A scenario test automates the same.

### M2 Desktop app and viewer UI
- Electron app (doc 03): engine process, security baseline, native frame and menus, native dialogs,
  file associations, drag and drop, tabs, settings window, lock file; `logica mcp` starting the app
  when it is not running (ADR 0007).
- Design system and components in `packages/ui` (doc 08), command palette.
- LOD waveform, cursors, markers, channel list, analysis panel, decoders, decoded table, search with
  query builder, activity feed with undo and "stop agent actions", lease display; shared session
  with agents.
- Unsigned installers from CI for Windows, Linux and macOS; auto-update from GitHub Releases on
  Windows and Linux.
- **Exit:** doc 08 budgets met on the reference machine; axe checks clean; an agent's `show_user`
  markers and view changes appear live in the open window; the owner runs the Windows build with
  Claude Code in WSL attached (S8).

### M3 FNIRSI DLA-32 Plus / DLA-16 Plus
- Transport with record and replay; USB hot-plug.
- `fnirsi-dla` driver (clean-room): identification, buffer capture, threshold, triggers, PWM (behind
  the output safety switch); stream mode with loss detection.
- Device panel with setup help per system; Zadig and "switch back" guides for Windows (D6).
- **Exit:** buffer captures at 250 MS/s with 32 channels and 1 GS/s with 8 channels decode the rig's
  signals correctly; replay tests in CI; the same capture through MCP `capture`; documented setup
  for Windows, Linux and WSL. DLA-16 Plus stated as untested.

### M4 FX2 / fx2lafw
- Spike S2 first.
- fx2lafw driver: firmware upload, re-enumeration, configure, stream; software and protocol triggers
  from queries (doc 06).
- **Exit:** 8 channels at the highest rate S2 showed sustainable, 10 s without loss, on the rig; a
  protocol trigger stops a capture on the rig's I2C NACK; replay tests in CI.

### M5 Formats and protocol depth
- Importers: Saleae binary export (v0, v1), CSV, raw binary, `.sal` best effort. Exports: `.sr`, VCD,
  CSV, Saleae binary v0, decoded packets.
- Layer decoders (Modbus RTU, SPI NOR flash, NMEA, AT), register maps for the eight chips (doc 06),
  capture comparison and the `verify-expected` prompt.
- **Exit:** every importer has fixtures (including malformed files with clear messages) and a
  provenance note; each layer decoder and register map decodes a rig or generated example.

### M6 Release 1.0
- Install guides with screenshots (SmartScreen, macOS quarantine, udev, Zadig), user documentation,
  the Claude Code setup guide; the standalone `logica` binary for headless use; first public release
  on GitHub.

### After version 1
Learn tab, Saleae automation driver, sigrok bridge, Pico analysers, WebAssembly decoder plugins, a
text query syntax, signing (if users need it), OS jump lists.

## Rough size

M0–M2 is the core product (viewer plus agent control) and the largest share of the work. M3 and M4
are the hardware risk; S3 front-loads the biggest part of it. Re-estimate after S3 and S1.
