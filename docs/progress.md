# Progress

What is done, what is next, and decisions made on the way. Newest first.

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

- Spike S3 (DLA feasibility on Windows) with the owner and the hardware; it also settles the USB
  `Transport` trait, which is provisional in `core`.
- Then M1: engine, command API, CLI and MCP.
- Packed-sample ingest is 168 MB/s, below the 200 MB/s DLA stream target: SIMD and ingest threads
  with spike S2.

## Planning (November 2026)

- The plan is complete: docs 01 to 12, the clean-room rule and nine ADRs.
- Fixtures copied: Signal Detective's sample captures and answer keys, and the `.sr` and VCD
  importer files (`fixtures/README.md`).
