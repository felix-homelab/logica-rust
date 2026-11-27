# Progress

What is done, what is next, and decisions made on the way. Newest first.

## M2a Virtual analog channels (November 2026, in progress)

ADR 0010; pull requests #15, #17, #18, #19, #20 (issue #16).

### Done

- The plan: ADR 0010 and docs 01, 03, 04, 06 to 12.
- `core::analog`: scaling (unsigned and two's complement codes, range and linear) and the value
  series with envelopes for any zoom.
- `protocols::analog`: the `Source` trait and the PWM level (duty, frequency, period, pulse width;
  a stopped PWM reads 0 % or 100 % with a reason).
- Engine: `analog.sources`, `analog.add`, `analog.update`, `analog.remove`, `analog.values`, undo,
  live following, saving in `.lacap`.
- Parallel bus: up to 16 bits, on a clock edge with an enable, or settled words without a clock;
  ADC0804 preset. No virtual analyser signal for it yet.
- Serial ADC: `spi-adc` and `i2c-adc` (a layer running the bus decoder inside itself), presets
  MCP3201, MCP3208 (8 inputs) and ADS1115, chosen in the dialog's settings step; the virtual
  analyser sends an MCP3201 sine on CH13 to CH15.
- Cursors: the Measurements panel reads every virtual analog channel at cursors A and B (value,
  raw code, the difference B − A, one step), through `analog.at`.
- Detection: a PWM with a changing duty is a PWM carrying a level (`DutyLevel` in the channel
  measurements), no longer UART; the analysis card offers "Add PWM level".
- Window: a row per channel (160 pixels unless resized by dragging its bottom edge or in the
  dialog; the height is part of the definition, so it is saved and undoable) with a value axis,
  steps or a line, problems outlined; the channel list's "Virtual analog" section and a dialog in
  four steps (source, settings, scaling, name and look) to add or change a channel.

### Not done yet

- The dialog does not know which scaling a source's settings need; the engine refuses a wrong one
  and says which to use.
- PDM, measurements, CSV export, search and math channels (doc 11, M2a).

## M2 Desktop app and viewer (November 2026)

### Done

- Engine: `waveform.get` (edges zoomed in, LOD buckets zoomed out, decoder frames or spans),
  `session.pauseAgents` ("stop agent actions"), working folders made on demand; the app serves
  MCP and writes the lock file; `logica mcp` starts the app minimised when no engine runs;
  `logica open` hands files to the app.
- `packages/ui`: tokens (light and dark, waveform dark in both, contrast checked in a test) and
  components on Radix (buttons, menus, dialogs, tabs, switches, fields, badges, splitter,
  virtual list, command palette).
- `apps/desktop`: one app per desktop, native menus and dialogs, file associations, drag and drop,
  `app://` for the window's files, settings window, the workbench of doc 08 (tabs, channels,
  waveform with LOD, cursors, selection and markers, analysis with evidence, decoders with
  settings, search builder, device panel, decoded table, measurements, activity feed with undo),
  start screen, command palette over every command, keyboard map, live updates from agents.
- Auto-update from GitHub Releases (Windows, Linux AppImage); a release workflow builds unsigned
  installers for all three systems on a version tag; the `logica` binary ships in the app.
- Tests: Playwright end-to-end tests with axe in both themes (also in CI on Linux and Windows),
  the doc 08 budgets measured in the app (`docs/benchmarks.md`), unit tests for the view
  arithmetic, formats, settings and tokens.
- Layout hardened at the minimum window (960 × 600): the tab bar no longer shows a scroll bar or
  hides its buttons, dialogs keep their buttons in sight and no longer slide in sideways, panel
  sizes are fitted to the window, measurements, activity, start and analysis cards are spaced
  alike; checked in `e2e/layout.spec.ts` and `test/panelsize.test.mjs`.

### Decisions on the way

- The ui-ux-pro-max skill shaped the grey scale and density; doc 08 kept the system font and
  Lucide, and the accent is indigo because green is a channel colour.
- Redesign (replaces the indigo accent): indigo sat 23° from the agents' violet and was on every
  button, tab, switch and badge, so agent actions did not stand out and colour said nothing. The
  main action is now ink, the accent is blue for focus, selection and links only, violet is the
  agents' alone (doc 08, "Colour means one thing each"; checked in `packages/ui/test/tokens.test.mjs`).
  States are a dot and a word (`Status`) instead of pills; the inspector is flat sections; the
  start screen lists recent files, devices and agents instead of three cards.
- The window handles every shortcut itself; the menu shows them without registering them.
- The person's own pans are sent to the engine throttled and not recorded; the window adopts the
  engine's view only when someone else changed it.
- Capture tabs are a navigation list rather than ARIA tabs, because they hold close buttons (axe).
- Canvas 2D meets the pan and zoom budget at 16 channels, so no WebGL ADR.
- The component gallery is reachable only in unpackaged builds.
- No .deb for now (owner's decision): Debian packages need a public maintainer name and email.
  Linux gets the AppImage, which also updates itself. The .deb returns before M3, because it
  installs the udev rules for USB analysers (doc 05).
- CI builds the real installers (NSIS, AppImage) on every pull request, not only the unpacked app,
  so a packaging failure shows up before a release.

### Reliability review (after the M2 merge)

- The engine process is supervised: it says when it is ready or why it cannot start; a crash
  restarts it (growing delay, at most three times a minute), calls waiting on it are answered
  with the reason, and the window offers to open the files again. A start failure is shown with
  its reason and a button, not retried in a loop.
- Unsaved work is protected: the engine knows which captures have unsaved changes
  (`unsaved` in the capture summary); a tab shows it, closing such a tab or quitting asks first.
  Save after "save as" now writes to that file (it refused before).
- Each part of the window reads again only on the events that concern it, so the person's own
  panning no longer resets the decoded table or reloads the activity feed.
- Error boundaries per region, a log file (`logs/logica.log` in the data folder, rotated; Help,
  Open the log folder) for the main process, the engine process and the window, a busy status for
  long commands, no duplicate opens, the window's size and place remembered, and the memory
  setting passed to the engine (it was ignored).

### Decoder groups and data as text (after the reliability review)

- A decoder's channels are grouped together under its name, in the waveform and in the channel
  list; channels no decoder uses come last. Inside a group the channels follow the protocol's role
  order (SPI: CLK, CS, MOSI, MISO; `channels` in the decoder summary), whatever wires they are on.
  A channel belongs to the first decoder that uses it; a second decoder on the same wires joins
  that group.
- Each decoder's frames are drawn under the channel its data is on (SPI: MOSI and MISO; UART: RX
  and TX), no longer all on one row; frames on other channels (an I2C start on SCL) go under the
  first data channel. The engine reports these channels (`lanes` in the decoder summary) and keeps
  zoomed-out stretches apart per channel.
- Below each row of frames, a text row writes the data in the format chosen per decoder: ASCII
  (the default), hex, decimal or binary (`dataFormat` in the decoder's configuration; set with
  `decoder.add` or `decoder.update`, so it is undoable, saved in `.lacap` and open to agents).
  One value under each frame when there is room, else the packet's values as one string. Changing
  only the format or the name does not decode again. Files saved before open as ASCII.
- The channel list's groups have a settings button that opens the decoder's settings in a dialog
  (the same form as the Decoders panel).

### Live capture and the virtual analyser (after the decoder groups)

- `logica-vla` (`tools/virtual-analyser`): a separate program that behaves like a streaming logic
  analyser on a TCP port. A JSON file configures its signals (UART, I2C, SPI, PWM, clocks, random
  noise) with random payloads and pauses; the same seed gives the same signals. It paces its data to
  real time (or any speed) and draws with Logica's own generators. Its wire protocol is Logica's
  own (`PROTOCOL.md`), implemented once in the driver crate and used by both ends.
- Driver `vla`: finds analysers at `LOGICA_VLA` (else `127.0.0.1:7171` to `7174`), streams their data in, and
  honours stop. Drivers can be `hotplug`: `device.list` asks them again; `session.get` does not.
- Engine: every device capture now goes through a bounded relay (doc 04 "Ingest"); the capture is
  in the session from its first block in the state `capturing`, grows as the engine polls (every
  50 ms while capturing, driven by the shared engine for both the app and `logica mcp`), reports
  `capture.progress`, and ends complete, stopped, or failed with what arrived kept. Decoders added
  while capturing follow the data (`LiveDecode`) and match a decode of the finished capture.
  Saving waits for the end; closing a running capture stops it (an agent may not close the
  person's).
- Window: the live view of doc 08 (follows the newest data, "Stay here", "Follow", "Stop capture"),
  "Capture live" on the start screen, "Look for devices" in the device panel. The session is read at most four times a second during a capture.
  The waveform shows answers that were overtaken by newer requests instead of dropping them, and a
  following view draws the range its data is for, so it is never empty.

- "Capture live…" (start screen, next to the tabs, File menu, palette, device panel) opens one
  dialog: pick one or several devices, then length (seconds or until stopped), rate (the same for
  all or one per device; not per channel, since a device samples all its channels at one rate),
  and per device channels (named by the device: `Capabilities.channelNames`), mode, threshold and
  tab name. Problems are explained while choosing (`captureplan.ts`, tested against the engine's
  check). Each device captures into its own tab; they are started together but share no clock.
  The device panel shows status, connect, take over and stop; its own capture form went. The `vla`
  driver looks at 127.0.0.1:7171 to 7174 by default, and `logica-vla --name` names an analyser, so
  several can run side by side.
- Several devices into one capture (`capture.start` with `with`, doc 05): channels numbered on and
  named by device, the capture as long as every device delivered. Lined up at their starts, or by
  a shared signal (`align`): the shift from its rising edges, then each clock's drift followed with
  a straight-line fit. The dialog offers "Each device into its own tab" or "All into one capture"
  and the shared channel per device. `logica-vla` has a `shared` signal (SYNC on CH10 in the
  built-in configuration, 100 Hz on the computer's clock) and `clockPpm` to try it.
- Devices on screen (start screen, device panel, "Capture live") are looked for every two seconds,
  so an analyser that starts or stops shows by itself; the `vla` driver greets its addresses at
  once. The time captured (live bar, status bar) keeps two fixed decimals.
- The synthetic device is no longer offered in the app (no "Try a demo capture"; the start screen
  lists virtual analysers or says how to start one). The engine registers it only when
  `EngineConfig::synthetic` is set, which tests, the scale test and the benchmarks do.

### Not done or not verified

- Several devices in one capture: not tried with real hardware; lining up by a steady clock needs
  the devices to start within half its period; packed and bit blocks are converted for the merger,
  not measured at high rates.
- Live capture: an agent's `view.set` on a capture that is running is not adopted by the window
  (the person steers a live view); no live protocol triggers yet; packed blocks are converted on
  the engine side, not yet measured at DLA stream rates (doc 04). The `vla` driver is for
  development; its data has perfect timing.
- The `.demo` files' own decoder presets (such as "display as decimal") are not applied when a
  file opens; they are listed by the CLI only.
- Not built yet: channel reordering by drag and the bus grouping editor, per-channel thresholds,
  the trigger editor and output confirmations (M3, with real devices), compare (M5), the
  capture folder setting, putting `logica` on the PATH from the installers (it ships in
  `resources/bin`).
- Not verified here: the budgets on the owner's reference machine, the Windows and macOS
  installers and auto-update (they need a release), the owner's S8 run with Claude Code in WSL,
  and the manual checklist (`docs/manual-tests.md`).

### Next

- M2a, virtual analog channels (ADR 0010, doc 11), decided by the owner after M2; then spike S3
  (DLA-32 Plus feasibility) and M3.

## M1 Engine, CLI and MCP (November 2026)

### Done

- Crate `engine`: the command API (39 commands), engine-wide history with per-actor undo and
  redo, device leases, events (doc 03, ADR 0008).
- The contract as JSON Schema, generated from the Rust types and checked in
  (`packages/api/schema/contract.json`); `packages/api` generates the TypeScript types, the
  command list and a typed client from it. Both ends fail CI when a generated file is stale.
- Crate `mcp`: the curated tools, resources, subscriptions and prompts over the command API,
  with `logica mcp` serving one agent over stdio from its own engine (no app yet, so it runs as
  `--headless` does). Tested through a real MCP client in-process, under both handshakes, and
  through the binary.
- MCP over HTTP on 127.0.0.1 (bearer token, `Origin` and `Host` checks), the lock file, and the
  `logica mcp` relay: the first `logica mcp` becomes the engine, later ones relay to it.
- `render`: a PNG of a range (the person's view by default) with channels from the LOD, decoder
  rows and markers, and a list naming the rows; own 5×7 font and PNG writer (zlib and CRC from
  the crates already used for `.lacap`). Checked by eye on `mystery_C.demo` and the synthetic mix.
- Golden summaries of every committed fixture (`crates/mcp/tests/snapshots/summaries`, reviewed),
  the exit scenario as a test through `logica mcp` on stdio, and `docs/manual-tests.md` for a
  real Claude Code session.
- Spike S7: `apps/desktop`, the Electron skeleton. The engine runs as a Node-API module
  (`crates/node`, built with `napi`) in a `utilityProcess`; the sandboxed window reaches it over a
  `MessagePort` through the preload's typed bridge. The packaged Linux app starts with Chromium's
  sandbox on and reaches the engine in a median of 157 ms on the development machine (WSL2,
  warm caches; budget 1.5 s); a CI job packages and smoke-tests it on Linux and Windows.
- Crate `cli`, the `logica` binary: `inspect` (summary, `--json`, `--index` for `.lacap`) and
  `convert` (any format it reads to `.lacap`), the engine in-process as the `cli` actor. Working
  files go to a temporary folder that is removed afterwards. No new dependencies (own argument
  parsing).

### Decisions on the way

- Only an agent's view changes go into the history; the person's own scrolling and zooming do
  not.
- The API version is written "major.minor" everywhere (contract and `session.get`).
- `logica open <file>` hands the file to the app (doc 09), not to an in-process engine as doc 03's
  diagram had it; it moves to M2 with the lock file (docs 03, 11; owner's decision).
- MCP through the official Rust SDK `rmcp` 3.10 (Apache-2.0) with `tokio` (MIT); owner's choice
  over our own HTTP and JSON-RPC code, for spec conformance as MCP changes. 96 crates in the
  Rust tree now, all MIT, Apache-2.0 or more permissive.
- MCP 2026-07-28 removed sessions; the server serves it and the session-based versions, and
  takes the actor from the client information either carries. Resource updates go out through
  `subscriptions/listen` (new) and `resources/subscribe` (old).
- `capture` with `wait` starts without waiting inside the engine (that would hold the engine for
  the whole capture) and waits for `capture.done` outside it; progress is estimated from the
  elapsed time until the engine reports capture progress. `compare` (M5) and `set_output` (M3)
  wait for their commands. The `verify-expected` prompt, planned for M5, is in already: it needs
  only `get_decoded`.
- The relay sends the standard headers of MCP 2026-07-28 (`Mcp-Method`, `Mcp-Name`) itself; the
  SDK's helper for them is private. New direct dependencies, all already in the tree through
  `rmcp`: `hyper`, `hyper-util`, `http`, `http-body-util`, `bytes`, `tokio-util` (MIT) and
  `getrandom` (MIT/Apache-2.0, for the token).
- S7: Electron is pinned (45.4.1; electron-builder needs an exact version) and downloaded by its
  own `install-electron` step, which `npm start` runs; npm 11 no longer runs it on install. Events
  reach the engine host by long poll rather than a thread-safe callback. The cold start is
  measured from outside the app: the main process's own clock starts after Chromium's start-up.
  The Linux executable is `logica-app`, so `logica` stays the command-line tool. New dependencies:
  `napi`, `napi-derive`, `napi-build` (MIT; the Node-API binding) in Rust; `electron`,
  `electron-vite`, `electron-builder`, `vite`, `react`, `react-dom` and their type packages (MIT)
  in npm, as doc 03 lists. Build-time only and not shipped: `lightningcss` (MPL-2.0) and
  `caniuse-lite` (CC-BY-4.0), through Vite.
- Known limit until M2: a windowless engine ends with the agent that started it; the others then
  get "cannot be reached" and must restart `logica mcp`.
- History texts use "an" where a name is spoken with a vowel ("an I2C decoder").
- TypeScript 7 (the native compiler; `tsc` as before). If M2's tooling (lint, bundler) still
  needs the TypeScript 5 programmatic API, the window packages pin that instead.
- New dependencies: `schemars` (Rust, MIT: the contract schema from the types);
  `json-schema-to-typescript` (npm, MIT, dev only: types from the schema) and `typescript`
  (Apache-2.0). The generator rewrites 2020-12 tuples (`prefixItems`) to the form that tool reads,
  and objects without fields to `Record<string, never>`.

## M0 Foundations (November 2026)

### Done

- Cargo workspace (`core`, `protocols`, `drivers`, `fileio`), npm workspace skeleton, rustfmt and
  clippy settings, GitHub Actions (Linux fmt/clippy/tests, Windows and macOS tests, wasm32 build).
- Chunked transition store with per-chunk LOD, eviction to a backing, level cursor, edge
  iterator, edge batches; captures past 2^32 samples (doc 04).
- `.lacap` read and write, save in place, save-as, recovery from an interrupted save (doc 09).
- Importers: FNIRSI `.demo`, sigrok `.sr`, VCD, each with a `PROVENANCE.md` (doc 09).
- Measurements, bus grouping, detection; every Signal Detective protocol as a streaming decoder
  with detector and generator; byte analyser; custom JSON protocols (doc 06).
- Driver traits and the synthetic device (doc 05).
- Benchmarks: every baseline row beaten (`docs/benchmarks.md`).

### Decisions on the way

- Toolchain: Rust edition 2024, pinned 1.100.0 in `rust-toolchain.toml`.
- Chunk cache default 512 MB (doc 04); the scale test uses 8 MB so chunks are evicted while
  capturing, and checks the whole process stays under 1 GB.
- Incoming bits are counted first, then stored as bits, offsets or quiet (doc 04).
- A saturated coarse LOD bucket is answered from the fine LOD, so LOD queries stay exact (doc 04).
- Edge batches never split the edges of one sample; data at sample 0 defines the initial level
  (doc 04).
- Payloads are written to the backing when evicted or saved; whoever evicts writes (doc 04).
- Measurements in `core`, bus grouping in `protocols`; new crate `fileio` for real files (doc 03).
- Decoders are resumable machines whose reads return "pending" until data arrives; snapshots are
  JSON (no infinities in state).
- A decode starts after the start-up artefact with the levels just before it, as Signal Detective
  trimmed channels.
- Detection window: at most 2^26 samples and 2^20 edges per channel (doc 06).
- The built-in single-wire protocols and DMX512 are JSON data files (doc 06).
- Number formatting rounds exact ties up, like JavaScript, so evidence texts match.
- Number settings are kept inside their documented range (a safety net under M1's validation).
- Signal Detective acceptance scope: behaviour only; Learn-tab suites and tests of its internals
  are out (doc 10). Its CAN corrupted-bit test moved an edge onto the next one; ours removes both
  edges, which is the same signal.
- A synthetic device capture is exactly the requested length; a cut frame is incomplete (docs 05,
  06).
- `.lacap` details: record layout, CRC coverage, bit order, index fields, damage versus
  interruption (doc 09).
- VCD: the last timestamp is a sample only if values change there; vector bits, scope paths for
  repeated names, shared identifiers (doc 09).
- `.sr` importer from doc 09 and our own fixtures only. ZIP reader of our own; dependencies:
  `crc32fast`, `miniz_oxide`, `serde`, `serde_json` (all MIT/Apache-2.0 or more permissive).
- CAN, I2S, SWD and JTAG were written by parallel agents in separate worktrees against the
  framework and reviewed before merging.

### Next

- M1: engine, command API, CLI and MCP, for recordings and the synthetic device; then M2, the
  desktop app. Owner's decision after M0: the full analyser for recordings first, hardware after
  (spike S3 moved to after M2; D9 and doc 11 updated).
- Spike S3 then settles the USB `Transport` trait, which is provisional in `core`.
- Packed-sample ingest is 168 MB/s, below the 200 MB/s DLA stream target: SIMD and ingest threads
  with spike S2.

## Planning (November 2026)

- The plan is complete: docs 01 to 12, the clean-room rule and nine ADRs.
- Fixtures copied: Signal Detective's sample captures and answer keys, and the `.sr` and VCD
  importer files (`fixtures/README.md`).
