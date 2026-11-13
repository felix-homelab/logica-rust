# Working on this repository

Read `README.md` and the docs it lists before changing anything. The docs are the plan; if the code
needs to differ from them, update the doc in the same change and say why.

## Rules

- The engine is Rust; the window is Electron with React and strict TypeScript (ADR 0002).
- `crates/core` and `crates/protocols` do no I/O of their own (no file system, network, threads or
  clock); what they need is passed in. They must build for `wasm32-unknown-unknown`.
- The renderer is sandboxed with context isolation and no Node; it reaches the engine only through
  the command API exposed by the preload script.
- Every capability is a command in the engine API first (ADR 0001). UI and MCP are clients. Outside
  the app, MCP over HTTP is the only protocol (ADR 0006).
- The command API contract (commands, inputs, outputs, events) is defined once; the TypeScript
  types are generated from it, never written by hand (doc 03).
- Every mutating command defines its inverse for the undo history; hardware commands need the
  device lease (ADR 0008).
- Time inside the engine is the sample index; seconds only at the edges.
- No new dependency (crate or npm package) without a sentence in the change explaining why.
- Copy is plain, sentence case, and explains; detections and problems always say why.
- Hardware outputs (PWM and similar) are safety class `output`: off by default, confirmed by the
  person in the app (never through MCP elicitation).
- Licence is permissive (ADR 0009). Never copy, translate or port code from GPL projects (libsigrok,
  sigrok decoders, PulseView, DSView) or vendor software (DLA_Logic, Logic 2). Do not open or fetch
  their source files. Drivers and format importers follow the "facts only" rule in
  `docs/clean-room.md` and get a `PROVENANCE.md`.
- FNIRSI's example captures (`I2C.demo`, `SPI.demo`, `UART.demo`) are local-only fixtures and
  must never be committed (doc 10).

## Definition of done

- Rust: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` clean.
- TypeScript: `npm run build`, `npm run lint` and `npm test` clean.
- Tests for new or changed behaviour, including error and edge cases.
- Report what was verified by tests, what only by eye, and what could not be run (for example
  "needs real hardware").

## Commands

Available: `cargo build`, `cargo test`, `cargo bench` (numbers for `docs/benchmarks.md`),
`cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo build -p logica-core -p logica-protocols --target wasm32-unknown-unknown`; `npm install`,
`npm run build`, `npm run lint`, `npm test` (no packages until M1; they say so and succeed). The
toolchain is pinned in `rust-toolchain.toml`.

Planned: `npm run test:e2e` (Playwright against Electron),
`npm start` (desktop app in development), `npm run package` (installers for this OS);
`logica mcp` (MCP over stdio; relays to the running app, starts it if needed, or runs headless with
`--headless`).
