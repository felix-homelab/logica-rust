# Kickoff prompt

Paste the text below into a new Claude Code session started in this folder.

---

We are starting the implementation of Logica, a desktop logic analyser, in this folder (repository
`logica-rust`): an engine in Rust, an Electron window with React, and a built-in MCP server.
Everything is planned: read `CLAUDE.md`,
`README.md`, `docs/clean-room.md` and every document in `docs/` (including `docs/adr/`) before you
write code. Treat them as the specification. The decisions in `docs/12-decisions-and-risks.md` are
made; do not re-open them unless something in practice proves one wrong.

`../signal-detective` is the behavioural reference for the protocols (decision D12): read its
`README.md`, `src/core/` and `tests/` for heuristics, evidence texts and expectations. Write new
code; do not port it line by line.

The project is open source under Apache-2.0. Never open, fetch or reproduce sigrok, PulseView,
DSView or vendor source code (`docs/clean-room.md`).

## Before you start

Tell me in a few lines what you understood the architecture to be, and list anything in the docs
that is contradictory, unclear or that you think is wrong. Do not paper over problems.

## Then build milestone M0, and stop at its exit criteria

Scope (from `docs/11-roadmap.md`):

- A Cargo workspace with the crates of `docs/03-architecture.md` that M0 needs (`core`, `protocols`,
  `drivers`), `rustfmt` and `clippy` settings, a GitHub Actions workflow, and the npm workspace only
  as an empty skeleton (the window starts in M1). Keep `LICENSE` and `.gitignore`.
- The fixtures are already in `fixtures/` (see `fixtures/README.md`). Make the tests of the three
  FNIRSI files in `fixtures/captures/local/` skip with a visible message when the files are absent.
- The chunked transition store with per-chunk level of detail from `docs/04-data-model-and-storage.md`
  (ADR 0004): offsets, raw bits and quiet chunks, the cache with eviction to a backing, the level
  cursor, edge streams and batches. Captures longer than 2^32 samples must work.
- The `.demo` importer on it; measurements, bus grouping, detection, and every protocol of Signal
  Detective as a streaming decoder with its signal generator (`docs/06-protocol-engine.md`); the
  byte analyser and custom JSON protocols.
- `.lacap` read and write as specified in `docs/09-file-formats.md` (ADR 0005), including recovery
  from an interrupted save; importers for sigrok `.sr` and VCD, tested against `fixtures/formats/`,
  each with a `PROVENANCE.md`.
- The synthetic device in `crates/drivers` through the driver traits of
  `docs/05-devices-and-drivers.md` (no real USB yet).

Exit criteria:

- Every Signal Detective sample and test expectation holds (same detections, roles, settings,
  bytes), through the streaming decoders, for any batching and across snapshot and restore; the
  three local-only files pass on my machine.
- A synthetic capture of 5e9 samples on 8 channels can be created, saved, reopened and queried
  (level at a sample, edges in a window, LOD lookup) within a 1 GB memory budget, measured by a test.
- `.sr`, VCD and `.lacap` tested, including malformed and truncated files with clear messages.
- At least as fast as the baseline in `docs/benchmarks.md`; record the new numbers there.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` clean.

## How to work

- Small, reviewable steps. After the workspace builds, show me the crate layout and the core storage
  types before going further.
- Every new crate gets a sentence on why, and a licence check (permissive only).
- Keep `docs/progress.md` short: what is done, what is next, decisions made on the way.
- If something in the plan turns out to be wrong in practice, say so, propose the change, and update
  the relevant doc once I agree.
- When you finish M0, report: what is verified by tests, what you could only check by eye, what is
  not done, and anything you would change in the plan. Then stop: next is spike S3 (DLA feasibility
  on Windows), which needs me and the hardware.
