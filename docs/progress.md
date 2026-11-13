# Progress

What is done, what is next, and decisions made on the way. Newest first.

## M0 Foundations (started November 2026)

### Done

- Plan reviewed; open points settled with the owner (below) and written into docs 03 to 06 and 09
  to 11.

### Decisions on the way

- Toolchain: Rust edition 2024, pinned stable in `rust-toolchain.toml`.
- Chunk cache default 512 MB (doc 04); the scale test uses 64 MB and checks the whole process
  stays under 1 GB.
- Incoming bits are counted first, then stored as bits, offsets or quiet (doc 04).
- A saturated coarse LOD bucket is answered from the fine LOD, so LOD queries stay exact (doc 04).
- Measurements in `core`, bus grouping in `protocols`; new crate `fileio` for real files, so
  `core` stays free of I/O (doc 03).
- Signal Detective acceptance scope: behaviour only; Learn-tab suites and tests of its internals
  are out (doc 10).
- A synthetic device capture is exactly the requested length; a cut frame is incomplete (docs 05,
  06).
- `.lacap` details filled in: record layout, CRC coverage, bit order (doc 09).
- Optimised test builds so the scale test runs in plain `cargo test`; own benchmark harness
  (doc 10).
- `.sr` importer from doc 09 and our own fixtures only. ZIP reader of our own; `miniz_oxide`,
  `crc32fast`, `serde`, `serde_json` as the only expected dependencies.
- CI: Linux (fmt, clippy, tests), tests on Windows and macOS, wasm32 build of `core` and
  `protocols`.

### Next

- Cargo and npm workspaces, CI; core storage types for review.

## Planning (November 2026)

- The plan is complete: docs 01 to 12, the clean-room rule and nine ADRs.
- Fixtures copied: Signal Detective's sample captures and answer keys, and the `.sr` and VCD
  importer files (`fixtures/README.md`).
