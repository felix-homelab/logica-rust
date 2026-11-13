# 10 Testing and quality

## Definition of done (every change)

- Rust: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` clean.
- TypeScript: `npm run build` (no new warnings), `npm run lint`, `npm test` clean.
- New or changed behaviour has tests, including error cases and edge conditions (invalid input,
  boundary values, failure paths), not only the happy path.
- Anything not run or verified (for example "needs real hardware", "macOS untested") is stated
  plainly in the pull request or report.
- Drivers and third-party format importers have a `PROVENANCE.md` that lists only sources allowed by
  [clean-room.md](clean-room.md).

## Test layers

| Layer | Tool | What |
|---|---|---|
| Unit | `cargo test` | store, LOD, ingest, protocols, engine logic, importers, query validation, `.lacap`, PNG rendering |
| Fixture | `cargo test` | every capture in `fixtures/captures/` detects and decodes to its answer key and Signal Detective's test expectations |
| Generated signals | `cargo test` | every protocol's generated example round-trips; wrong settings produce the expected problems; decoder output is identical for any batching and across snapshot and restore (doc 06) |
| Scale | `cargo test` | a 5e9-sample, 8-channel synthetic capture is created, saved, reopened and queried with peak memory under 1 GB (created by the synthetic device's `mix-dense` program into a working `.lacap` on disk with an 8 MB cache budget, so chunks are evicted to the file while capturing; answers checked against the program; peak resident memory read from `/proc/self/status` in a test binary of its own, Linux only, skipped with a message elsewhere); a signal straddling sample 2^32 decodes identically. Test builds are optimised (workspace `profile.dev`) so plain `cargo test` runs it in seconds |
| Driver replay | `cargo test` | drivers against recorded USB transcripts (below) |
| Engine and API | `cargo test` | commands in-process; undo (every mutating command's inverse restores the state, redo reapplies it), leases (busy errors, take-over, release on disconnect), events; the generated TypeScript contract is up to date |
| Node binding | Vitest | the Node-API module loads, runs commands, delivers events over a `MessagePort` |
| MCP | `cargo test` with an MCP client | tool schemas (snapshot), golden summaries, HTTP security (token, `Origin`, 127.0.0.1), the stdio relay and headless mode, the end-to-end scenario (doc 07) |
| UI units | Vitest | view arithmetic (zoom, pan, ticks), formatting, layout, palette filtering |
| App | Playwright with its Electron support | the packaged app: open a recording, detect, decode, undo; capture from the synthetic device and from a replay transport; save and reopen; tabs; the output confirmation dialog; axe checks (including the component gallery in both themes); performance traces (doc 08) |
| Electron security | Playwright | window options (context isolation, sandbox, no Node), nothing but the bridge exposed, CSP present, navigation and `window.open` blocked |
| Packaging | CI on Windows, macOS, Linux | installers build; the packaged app starts, loads the engine module, lists the synthetic device, and quits cleanly |
| Hardware in the loop | manual or a local CI runner | real devices against the known-signal rig |
| Benchmarks | `cargo bench` | ingest throughput, LOD build, decode speed; results in `docs/benchmarks.md`. Own harness (`harness = false`, best of 5 with `std::time::Instant`), no benchmark crate |

Native dialogs and drag and drop from the file manager cannot be driven by Playwright; tests call
the command the dialog would trigger, and the real interaction is on the manual checklist.

## Replay testing for drivers

- A **record transport** wraps a real transport and writes every control and bulk transfer
  (direction, request, length, data, timing) to `fixtures/usb/<driver>/<scenario>.json`, with large
  payloads as separate compressed files.
- A **replay transport** plays the device side back and asserts that the driver sends the same
  requests, so drivers are tested without hardware in CI.
- Transcripts are recorded once per scenario (identify, configure, buffer capture at two rates,
  trigger, stream with forced lag) and again when firmware changes.
- Hardware the owner has: DLA-32 Plus, an FX2 clone, a Saleae Logic (model to be checked). DLA-16
  Plus support stays untested until someone with the device records transcripts; macOS USB is
  untested by the owner.

## Known-signal rig

An ESP32 (fast signals: SPI in all four modes at several MHz, WS2812-like timing with RMT, PWM, an
8-pin counter) plus an Arduino (slow, simple signals: UART at several rates and formats, I2C
transactions with a NACK, 1-Wire-like bursts), each repeating a fixed script on known pins. The
expected output is generated from the same script, so hardware tests can assert exact decoded
bytes. Firmware and scripts live in `tools/signal-rig/`. Its recordings also replace the FNIRSI
example files as public real-hardware fixtures.

## Fixtures

- `fixtures/captures/`: Signal Detective's generated captures (`1_UART_easy`, `2_UART_mystery_baud`,
  `3_I2C_bus`, `4_SPI_wrong_mode`, `mystery_A` to `mystery_D`) and both answer-key files, copied
  unchanged. The expectations come from the answer keys and Signal Detective's tests
  (`../signal-detective/tests/`). In scope is behaviour: `samples`, `protocols` (without the
  `fingerprint` check), `custom`, `bytes`, and the behavioural parts of `capture` (reading `.demo`,
  the start-up artefact, error messages). Out of scope are the Learn-tab suites (`identify`, `calc`,
  fingerprints and sketches, D12) and tests of Signal Detective's internals (the `EdgeBuilder`
  edge limit, the exact number of progress calls, its INI helper), which get equivalents on our own
  API.
- `I2C.demo`, `SPI.demo` and `UART.demo` are FNIRSI's example captures (shipped with DLA_Logic) and
  must not be published. They live in `fixtures/captures/local/`, which is git-ignored. Their tests
  run when the files are present and are skipped with a visible "skipped: FNIRSI example files not
  present" message otherwise (for example in CI). Rig recordings replace them.
- `fixtures/formats/`: `.sr` and VCD files for the importer tests, described in
  `fixtures/README.md`. Every new importer gets small fixtures made by the respective tool, with a
  note on how they were made (one per Saleae export version). Malformed and truncated variants are
  made inside the tests.

## CI (GitHub Actions)

- Rust: format, clippy, tests on Linux; tests also on Windows and macOS; `core` and `protocols`
  built for `wasm32-unknown-unknown`; from M1 the engine module and the `logica` binary built for
  Windows, macOS and Linux (x64 and arm64). The toolchain is pinned in `rust-toolchain.toml` and
  bumped deliberately.
- TypeScript: lint, type-check, Vitest; the generated command-API types checked for staleness.
- Playwright against the Electron app on Linux under a virtual display (`xvfb`).
- Packaging jobs on Windows, macOS and Linux (path handling, native module loading, USB permission
  messages). Releases go to GitHub Releases for electron-updater.
- Benchmarks and performance traces are recorded and their trend reported; CI fails only on
  regressions of more than 2× the baseline. The real budgets (doc 08) are checked on the owner's
  reference machine at every milestone exit.
