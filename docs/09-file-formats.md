# 09 File formats

## Import

| Format | Milestone | Notes |
|---|---|---|
| FNIRSI `.demo` | M0 | ZIP; per-channel 1-bit blocks of 8,388,608 samples; a block bitmap; saved decoders (shown for comparison, never used for detection). Known surprises are documented in Signal Detective's README |
| sigrok `.sr` | M0 | ZIP: `version` (2), `metadata` (INI: samplerate as text with unit, unitsize 1 to 4, `probeN` = channel on bit N-1), `logic-1-N` files of packed little-endian samples, read through the device ingest path. Analog channels noted and ignored; more than 32 channels refused |
| VCD (IEEE 1364) | M0 | Text, read in two passes in pieces; `$timescale`, scopes, `$var`; one-bit variables become channels, vectors one channel per bit; x and z read as low (counted in a note); reals skipped; sample 0 is the first timestamp and one sample is the greatest common divisor of the time steps; the last timestamp ends the capture and is a sample of its own only if values change there (so sigrok's VCD of a 30,000-sample `.sr` has 30,000 samples too). A vector's channels are named `name[k]` after its declared range, least significant first; a name used in more than one scope gets its scope path (`top.sub.en`); declarations sharing an identifier each get a channel. Extended VCD (strengths) is not handled |
| Saleae binary export | M5 | Documented: header `<SALEAE>`, version, type; digital v0 (initial state, begin and end time, transition times as doubles) and v1 (chunks). Times are converted to sample indices at a chosen or inferred rate |
| Saleae `.sal` | M5, best effort (owner's choice) | ZIP with `meta.json` and `digital-N.bin`; undocumented and changing between versions. Only versions we have fixtures for; otherwise tell the person to export binary from Logic 2 |
| CSV | M5 | A time column plus one column per channel, or sample index; detect delimiter and header; rows only on change or every sample |
| Raw binary | M5 | Packed samples with a given unitsize and rate |

Every importer streams where possible, reports progress, records surprises as capture notes, and
gives a clear message for unsupported variants ("This .sal file is from a newer Logic 2 version than
this tool knows; export binary from Logic 2 instead"). Never a blank screen. Importers for
third-party formats follow the clean-room rule (published specifications and our own sample files
only, [clean-room.md](clean-room.md)) and have a `PROVENANCE.md`.

Opening files: menu, command palette, drag and drop onto the window, double-click in the file
manager (file associations for `.lacap`, `.demo`, `.sr`, `.vcd`), or `logica open <file>`.
`logica convert <in> <out>` converts between formats without the app.

## Export

| Format | Notes |
|---|---|
| `.lacap` (native) | Everything: data, decoders with settings, markers, notes, agent annotations |
| sigrok `.sr` | Opens in PulseView |
| VCD | Opens in GTKWave and most tools |
| CSV | Transitions only, or decoded packets |
| Saleae binary v0 | For further analysis in Saleae's ecosystem |
| Decoded data | CSV or JSON of packets per decoder |

## Native format `.lacap` (ADR 0005)

A single-file container of our own, not a ZIP: random access by offset, files larger than 4 GB, no
unpacking on open. All integers are little-endian.

```
header      "LACAP\0\r\n", u16 major (1), u16 minor (0), u32 reserved                    16 bytes
blobs …     each: u32 type, u32 flags (0), u64 payload length, payload, u32 CRC-32 of payload
              (the payload is everything after the length field: the type's own header fields
              below and its data, so the CRC protects both)
              type 1  chunk:          u32 channel, u32 kind (1 offsets, 2 bits), u64 base sample,
                                      u8 entry level, 3 reserved, u32 transitions, then the u32
                                      offsets or the 2 MiB of raw bits
              type 2  LOD (fine):     u32 channel, u32 shift (8), u64 base sample, u32 buckets,
                                      u32 reserved, u16 counts, packed start levels
              type 3  annotations:    cached decoder output (layout defined in M1)
              type 4  channel table:  u32 channel, u32 chunk count, u32 record size (40), u32
                                      reserved; per chunk a 40-byte record: u8 kind (0 quiet,
                                      1 offsets, 2 bits), u8 entry level, u8 exit level, u8 flags
                                      (bit 0: has a fine LOD), u32 transitions, u64 offset and
                                      u64 length of the chunk blob, u64 offset and u64 length of
                                      the fine LOD blob (0 and 0 where there is none); then for
                                      every non-quiet chunk, in chunk order, its coarse LOD (256
                                      u16 counts, 32 bytes of packed start levels)
index       raw-deflate JSON (camelCase keys): format "lacap", version [major, minor], chunk
            size (2^24), sample count, initial level per channel (needed when there are no
            chunks), capture metadata (channels, segments, notes, …), where each channel table
            blob is ([offset, length] per channel), and the session state saved with the capture
            (decoder instances, markers, range notes)
footer      u64 index offset, u64 index length, u32 CRC-32 of index, u32 reserved, "LACAPEND"
```

- CRC-32 is the ZIP/zlib one (polynomial 0xEDB88320). 64-bit values stay below 2^53. Opening reads
  the footer, the index and one table per channel; chunks and fine LODs are read by offset when
  needed, with their CRC checked; a bad one is reported by name ("Chunk 3 of channel 2 … is
  damaged").
- Readers refuse newer major versions with a clear message. Blobs are reached only through the
  tables, so unknown blob types are skipped by construction.
- Saving appends new or changed blobs, new channel tables, a new index and a new footer; old ones
  become dead space. "Save as" writes a compact copy.
- If saving is interrupted, the file ends in a partial append. The reader scans back for the last
  complete footer (its magic, an index that ends right before it, and the index CRC) and opens that
  state with a note; the next save cuts the partial append off before writing. The footer is
  written last, so a footer magic at the very end whose index fails its CRC is damage, not an
  interruption: the reader still falls back to an earlier save if there is one, and otherwise
  reports the damage.
- Opening checks every table against itself and the index: chunk count from the sample count,
  kinds, transition counts within the limits of their kind, exit level = entry level flipped once
  per transition, each chunk's entry = the previous chunk's exit, blob places inside the file.
- A capture being recorded or imported lives in a working `.lacap` in the data folder (doc 04);
  saving there appends the chunks still only in the cache (for a small capture, all of them), then
  tables, index and footer.
- Packed start levels: bit k of byte k / 8 (least significant bit first) is the level at the start
  of bucket k. Offsets are the chunk-relative transition positions, ascending; raw bits are least
  significant bit first, sample 0 of the chunk in bit 0 of byte 0.
- `logica inspect <file>` prints the index (the file cannot be opened with `unzip`).
