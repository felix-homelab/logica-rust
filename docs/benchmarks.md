# Benchmarks

Measured numbers, newest first. The real budgets (doc 08) are checked on the owner's reference
machine at every milestone exit; numbers from other machines are indications only.

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

Not measured yet: packed-sample ingest throughput in MB/s (spike S2), pan and zoom (spike S1), cold
start (spike S7).
