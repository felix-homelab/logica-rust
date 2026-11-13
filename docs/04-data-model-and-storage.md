# 04 Data model and storage

## Requirements

- Up to 32 digital channels per capture, up to several billion samples per channel (DLA-32 Plus
  buffer: 4 Gbit; long streaming captures exceed that), with no limit at 2^32 samples.
- Memory proportional to activity, not length: quiet lines cost almost nothing.
- Append while capturing; read random windows quickly; draw any zoom level at screen speed.
- The same structures in the app's engine and in the headless CLI.

## Time

- Inside the engine the unit is the **sample index** (`u64`). Seconds appear only at the edges (UI,
  MCP output, exports). Sample indices stay below 2^53, so they are exact as JavaScript numbers in
  the window (about 104 days at 1 GS/s).
- Each capture has a sample rate, a sample count, an optional trigger sample (time 0 in the UI) and
  segments (a capture can have gaps, for example stream restarts; Saleae v1 exports have chunks).

## Capture

| Field | Meaning |
|---|---|
| id, title | |
| source | device, file or synthetic, with a detail text |
| sample rate, sample count | the count grows while capturing |
| trigger sample | optional |
| channels | per channel: physical index, name, colour, enabled, threshold (volts, where adjustable) |
| segments | covered sample ranges |
| notes | format surprises, dropped data, artefacts; shown to the person and in MCP summaries |
| state | capturing, complete, aborted |
| artefact | a start-up artefact excluded from analysis (DLA captures) |
| saved decoders | decoder settings found in an imported file; shown for comparison, never used by detection |

## Transitions, chunked (ADR 0004)

Each channel's data is a sequence of **chunks** of 2^24 samples (16,777,216). A chunk is one of:

- **offsets:** the chunk-relative positions of its transitions, ascending `u32` in [0, 2^24);
- **bits:** the raw samples, 2 MiB, least significant bit first, with its transition count;
- **quiet:** no payload at all.

Every chunk also records its **entry level**, the level of the sample before it (for chunk 0, the
level at sample 0). An edge is the first sample at a new level, so an edge exactly at a chunk start
is offset 0, and every chunk can be read on its own.

- Offsets fit in 32 bits whatever the capture length; that removes the 2^32 limit.
- Above 2^19 transitions, offsets would take more than the 2 MiB of raw bits, so the chunk is stored
  as bits. Memory per chunk is never above the raw size. Writers switch representation on the fly.
  Incoming bits are first counted (one popcount pass over each word XOR its shifted self gives the
  transitions per bucket, without extracting positions); only above 2^19 transitions is a block
  kept as bits, otherwise its offsets are extracted or the chunk stays quiet. So a quiet line
  delivered as bits (every `.demo` block) still costs nothing.
- Per channel, a resident **summary** per chunk: kind, entry and exit level, transition count, where
  the payload lives, and its coarse LOD.
- Reading: level at a sample; a level cursor (amortised O(1) for increasing positions, across chunk
  boundaries and both kinds); edges in a range; an incremental edge stream; merged **edge batches**
  for decoders (sample, channel slot and new level per edge, plus `end`: every edge before `end` has
  been delivered); and windows of absolute edge positions for measurement and detection.

## Level of detail

Per channel and bucket: the transition count (saturating at 65,535) and the level at the bucket
start. LOD memory follows activity, not length:

- **Coarse** (buckets of 2^16 samples, 256 per chunk): resident, for every non-quiet chunk.
- **Fine** (buckets of 2^8 samples): only for dense chunks (more than 4,096 transitions), kept in the
  same evictable cache as payloads and stored in `.lacap`.
- Other bucket sizes are derived by summing; sparse chunks answer from their offsets; quiet chunks
  need nothing. A LOD query is exact for any bucket size and fast when buckets align to 2^8 or 2^16.
  A coarse bucket can hold 65,536 transitions (a line toggling on every sample), one more than its
  count can say; a saturated coarse bucket is answered from the chunk's fine LOD, which always
  exists then (the chunk has more than 4,096 transitions) but may have to be loaded.

A global fine level would cost about 2 bytes per 256 samples per channel even on idle lines (about
330 MB for 5e9 samples on 8 channels); per-chunk LOD avoids that.

Decoder annotations get a similar per-row density index, so a zoomed-out view shows where frames
are without loading them.

## Annotations and packets

- A **frame** has start and end samples, a lane (the channel it belongs under), kind, value, label,
  detail, problems, and the packet and row it belongs to. A **packet** groups frames (a UART line,
  an I2C transaction) with its bytes.
- Stored per decoder in append-only pages of 64k frames, each with a `[min start, max end]` index
  for windowed queries and paging (MCP `get_decoded` uses page cursors).
- Problems are also indexed by code, for "jump to next error".

## Storage

| Hot data | Cold data |
|---|---|
| chunk payloads and fine LODs in a least-recently-used cache with a byte budget (default 512 MB, adjustable, so a whole engine at default settings stays near 1 GB); summaries and coarse LODs always resident | a **backing**: chunks leaving the cache are written there and read back by offset |

Backings: memory (small captures, tests) and the `.lacap` file itself. A new capture is created
inside a working `.lacap` in the data folder: chunks evicted while capturing are appended as blobs,
and saving only appends the channel tables, index and footer (ADR 0005). An opened `.lacap` is read
by offset; nothing is unpacked, and nothing is written to it until the person saves.

File access is synchronous positional reads and writes behind a small interface (doc 03), so level
cursors and decoders stay plain sequential code; heavy work runs on engine threads.

The window never holds capture data, only what it is drawing.

## Ingest

- Drivers hand over raw blocks in one of three shapes: **packed samples** (1, 2 or 4 bytes per
  sample, one bit per channel), **per-channel bit blocks** (FNIRSI `.demo` style), or
  **transition lists** (synthetic device; later vendor run-length data). A sink writes them into the
  store.
- Packed samples to transitions: XOR each sample word with its predecessor; non-zero results name
  the channels that changed. Runs of unchanged words are skipped in bulk (SIMD compares).
- Runs on engine threads: one per group of channels for per-channel blocks; ordered hand-off for
  packed blocks.
- Throughput is measured in spike S2 and recorded in `docs/benchmarks.md`. Target: DLA stream mode
  at full rate (up to 200 MB/s of packed input) without loss.

## Integrity

- Every block carries its sample range. Gaps, overlaps and replays (DLA ring buffer) are detected
  where the driver can see them and recorded as notes and segment boundaries.
- A capture that lost data says so prominently in the UI and in every MCP summary.
