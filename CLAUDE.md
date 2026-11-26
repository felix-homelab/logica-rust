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

## Branches, commits and releases

[CONTRIBUTING.md](CONTRIBUTING.md) and [RELEASING.md](RELEASING.md) are the rules; this is the
summary, changed in the same PR as they are.

- Never push to `main` (a ruleset blocks it), never rewrite `main`, `release/*` or a tag, never
  force-push and never delete a remote branch. Bring a pushed branch up to date by merging `main`
  into it. Run the checks of what changed before the first push; until v1.0.0 fix what CI then
  finds with a further commit.
- Branches: `feature/<issue>-<desc>` and `bugfix/<issue>-<desc>` need a GitHub issue;
  `improvement/<desc>` (no behaviour change), `merge/<desc>`, `tmp/<desc>` (never merged).
  Lowercase, digits, `-`, `.`; at most 60 characters.
- Commits and PR titles: Conventional Commits, `<type>(<scope>)!: <subject>`, at most 72
  characters, lowercase subject without a period. Types `feat fix perf refactor docs test build ci
  chore revert`; scopes `core protocols drivers engine fileio mcp node cli desktop ui api vla ops
  adr deps release`. The body says why. Substantial AI-written content carries a
  `Co-Authored-By:` footer. PRs are merged with merge commits, so every commit must build and
  have a valid header; fold fix-ups in before merging.
- Every PR adds a `CHANGELOG.md` entry under `## [Unreleased]` (user-facing, present tense, ends
  with the PR number) or carries the `no-changelog` label. Breaking changes (to `.lacap`, the
  command API and MCP, or the CLI) get `!`, a `BREAKING CHANGE:` footer and a `**Breaking:**`
  entry.
- One logical change per PR; CI warns from 400 changed lines of production code.
- The version is one for everything and lives in the files: change it only with
  `node scripts/set-version.mjs X.Y.Z`, in a `chore(release)` PR. Tags `vX.Y.Z` and
  `vX.Y.Z-rc.N` are signed and made by the maintainer only.
- A PR that adds or renames a CI job updates the required checks in `.github/repo-settings.mjs`.

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
`npm run build`, `npm run lint`, `npm test`, `npm start` (the desktop app; the first run downloads
Electron), `npm run package` (installers for this OS), `npm run package:dir -w apps/desktop` then
`npm run smoke -w apps/desktop` (the packaged app starts and reaches the engine; prints the cold
start); `npm run test:e2e` (Playwright against the built app; `LOGICA_PERF=1` adds the doc 08
budget measurements, `LOGICA_SHOTS=<dir>` keeps screenshots); `logica mcp` (MCP over stdio:
relays to the running engine, starts the app, or becomes a windowless engine); `logica open`; `cargo run -p logica-vla` (a virtual analyser for
live capture without hardware, `tools/virtual-analyser/README.md`). After
changing a command's input or output types:
`LOGICA_UPDATE_CONTRACT=1 cargo test -p logica-engine --test contract`, then
`npm run generate -w packages/api`, and commit both generated files. The toolchain is pinned in
`rust-toolchain.toml`.

`bash .github/scripts/test-checks.sh` tests the PR and release checks;
`node .github/repo-settings.mjs` shows how the GitHub settings differ from CONTRIBUTING.md (the
maintainer applies them).

Releases (RELEASING.md): the maintainer pushes a signed tag `vX.Y.Z` (or `vX.Y.Z-rc.N`) on a
`main` commit whose files hold that version and whose `CHANGELOG.md` has the dated section; the
release workflow checks that and builds a draft GitHub Release with the changelog notes, the
unsigned installers for Windows, Linux and macOS, the `logica` binaries and `SHA256SUMS`.
