# Benchmarks

Measured numbers, newest first. The real budgets (doc 08) are checked on the owner's reference
machine at every milestone exit; numbers from other machines are indications only.

## M0: the Rust engine (November 2026)

`cargo bench` (crates/drivers/benches/baseline.rs, release build, best of 5; the large capture one
run). Same machine as the baseline: Intel Core i7-1370P laptop, WSL2, Rust 1.100.0. Single-threaded
(`core` has no threads of its own). Numbers vary by up to about 3× between runs on this laptop
(an earlier run while four other builds were running gave 6.2 ms for detection), so compare the
order of magnitude, not the last digit.

| What | Rust (M0) | TypeScript baseline | Budget or target |
|---|---|---|---|
| Import `mystery_D.demo` (50 M samples, 6 channels) into the store | 17 ms | 75 ms | under 2 s (doc 08) |
| Detection on `mystery_D` (all protocols, 6 channels) | 1.8 ms | 12 ms | — |
| Streaming SPI decode of `mystery_D` | 0.017 ms | 0.2 ms | — |
| `.lacap` save / open of `mystery_D` (in memory, 18.7 KB) | 0.021 ms / 0.009 ms | 0.3 ms / 0.1 ms | — |
| 5e9 samples × 8 channels, synthetic `mix-dense` (23.7 M transitions): create into a working `.lacap` on disk | 0.23 s | 1.1 s | — |
| … save in place / reopen / a set of queries (level at a sample, edges in a window, LOD lookup) | 7.7 ms / 0.53 ms / 0.08 ms | 0.11 s / 0.01 s / 0.12 s | — |
| … file size | 14.5 MB | 58 MB | — |
| … peak resident memory of the whole test process (scale test, cache budget 8 MB) | 21 MB | 318 MB | under 1 GB (M0 exit) |
| … chunk cache high-water mark (budget 64 MB; the whole capture's chunks are 12.5 MB) | 12.5 MB | 64 MB | the budget |
| Packed-sample ingest, 8 channels, 1 byte per sample (256 MB, a counter stepping every 16 samples) | 168 MB/s | not measured | 200 MB/s for DLA stream mode (doc 04, spike S2) |

Packed ingest is below the DLA stream-mode target. It is scalar and single-threaded today (one
level change at a time); spike S2 measures it against real USB rates, and SIMD comparison of
sample words and ingest threads (doc 04) are the planned remedies.

## Baseline to beat: the TypeScript prototype (November 2026)

The same storage and decoding design, implemented in TypeScript on Node 24. Machine: Intel Core
i7-1370P laptop, WSL2. Best of 5 runs, except the large capture (one run). The Rust engine must be
at least this fast on comparable hardware (M0 exit).

| What | Result | Budget or target |
|---|---|---|
| Import `mystery_D.demo` (50 M samples, 6 channels) into the store | 75 ms | under 2 s (doc 08) |
| Detection on `mystery_D` (all protocols, 6 channels) | 12 ms | — |
| Streaming SPI decode of `mystery_D` | 0.2 ms | — |
| `.lacap` save / open of `mystery_D` (in memory, 18.7 KB) | 0.3 ms / 0.1 ms | — |
| 5e9 samples × 8 channels, synthetic `mix` with a dense stretch (about 24 M transitions): create into a working `.lacap` on disk | 1.1 s | — |
| … save in place / reopen / a set of queries (level at a sample, edges in a window, LOD lookup) | 0.11 s / 0.01 s / 0.12 s | — |
| … file size | 58 MB | — |
| … peak resident memory of the whole test process | 318 MB | under 1 GB (M0 exit) |
| … chunk cache high-water mark (budget 64 MB) | 64 MB | the budget |

Not measured yet: pan and zoom (spike S1), cold start (spike S7), sustained USB rates (spike S2).
