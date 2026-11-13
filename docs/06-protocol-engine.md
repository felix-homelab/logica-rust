# 06 Protocol engine

## Reference

Signal Detective's protocol knowledge is the reference (decision D12): detectors with evidence and
"ruled out because" reasons, decoder behaviour, settings, measurements, bus grouping, custom JSON
protocols, the byte analyser, protocol knowledge texts and symptom tables. Its sample captures,
answer keys and test expectations are the acceptance tests. The question-based identifier,
calculators and waveform sketches belong to the Learn tab (after version 1).

Each protocol is one module: its settings, roles, knowledge texts, detector, streaming decoder and
signal generator. Every role has a short noun for setup messages ("Choose a channel for the data
line.").

## Streaming decoders

Live capture needs incremental decoding, so every decoder is streaming; a whole-capture decode is
one stream, and there is no separate batch decoder.

- **Create** from a setup (sample rate, roles, settings, bound channels with their levels at the
  start, start sample), optionally resuming from a snapshot.
- **Push** an edge batch: the edges of the bound channels in time order, complete up to the batch's
  `end`.
- **Finish** at an end sample: no more data; emit anything pending.
- **Drain**: the frames and complete packets since the last drain, and the decode notes.
- **Snapshot**: the whole state as serialisable data, so a decode can resume later.

The engine merges the decoder's channels into one time-ordered edge stream and pushes batches as
chunks arrive.

Design guidance: a decoder buffers its channels' edges and runs a resumable loop that reads edges
by absolute index and levels at any sample it has data for. When it needs data that has not
arrived, it stops before emitting anything and runs again after the next push; edges it no longer
needs are released, so memory follows the decoder's look-ahead, not the capture length.

Rules, each pinned by tests:

- Identical output for one batch, batches of 1 and 3 edges, and snapshot and restore between
  batches, for every generated example and the fixtures; identical output when a signal straddles
  sample 2^32.
- Decoders never truncate their output (Signal Detective capped frames at 200,000, I2S at 100,000);
  paging belongs to the annotation store (doc 04).
- SPI without chip select splits frames at clock pauses longer than 8 typical clock gaps. A stream
  cannot measure the typical gap over the whole capture as Signal Detective did, so it is learned
  from the first 4,096 clock edges (that covers every fixture).
- Changing a decoder's settings applies instantly to the visible window, then to the whole capture
  in the background.

## Detection

- Pipeline: measure channels → group them into buses → run every detector → choose the best
  non-overlapping explanation. Detectors trial-decode with the streaming decoders (statistics only).
- Runs on a window, so it stays fast on huge captures: from the end of a start-up artefact, at most
  2^26 samples (67 M). That covers every fixture whole (`mystery_D` has an edge at sample
  50,000,000, past a 50 M window). Sampling the middle and end of long captures, and combining
  measurements from separate windows, is designed with `analysis.detect` in M1.
- Available for any time range, so an agent can ask "what is happening between 2.1 s and 2.3 s?".

## Stacking

A layer decoder consumes another decoder's packets instead of edges. It names the protocols it can
sit on (`uart`, `spi`, `i2c`) and turns packets into frames and packets of its own.

First layers: Modbus RTU (on UART), SPI NOR flash commands (on SPI), I2C/SPI register maps (below),
NMEA sentences (on UART), AT command and response pairing (on UART).

## Register maps

Data files that describe common chips, so a decode says "write CTRL_REG1 = 0x57: 100 Hz, X/Y/Z on"
instead of "20 57":

```json
{
  "id": "lis3dh", "name": "LIS3DH accelerometer", "bus": ["spi", "i2c"],
  "i2cAddresses": [24, 25],
  "whoAmI": { "register": 15, "value": 51 },
  "registers": [
    { "address": 32, "name": "CTRL_REG1",
      "fields": [ { "bits": [7, 4], "name": "ODR", "values": { "5": "100 Hz" } },
                  { "bits": [2, 0], "name": "axes enable" } ] }
  ],
  "spi": { "readBit": 7, "autoIncrementBit": 6 }
}
```

Start with the chips the samples already use: 24Cxx EEPROM, DS3231, LM75/TMP102, LIS3DH, SSD1306,
BME280, MPU6050, DS18B20 (1-Wire). A chip can be recognised from its address or WHO_AM_I reply
(with evidence, as with protocols).

## Search, triggers and comparison

- Search over edges ("pulse narrower than 100 ns on CH3", "CH2 high while CH5 rises") and over
  decoded content ("I2C NACK", "SPI MOSI starts with 9F", "UART text contains ERROR", "frames with
  problems").
- Queries are a **typed JSON structure** defined in the command API contract, not a text grammar.
  The engine, the MCP `search` tool and triggers all take it; agents write it from the schema; the
  UI has a query builder that produces it:

  ```json
  { "all": [
      { "decoded": { "decoder": "i2c-1", "kind": "ack", "value": "nack" } },
      { "within": { "from": 0.1, "to": 0.5, "unit": "s" } } ] }
  ```

  A text syntax for people may come later as sugar for the same structure.
- The same queries drive **software triggers** and **protocol triggers** ("stop 10 ms after the
  first CAN frame with ID 0x7DF") on streaming devices (FX2). DLA buffer mode uses the device's own
  trigger.
- **Capture comparison:** two captures, or a capture's decoded data against expected bytes or text;
  differences with times (used by the `verify-expected` prompt, doc 07).

## Custom protocols

- Custom JSON protocols (pulse-code and presets such as DMX512) are data, not code, so sharing
  them is safe.
- No code plugins in version 1. After version 1, code-based decoders come as WebAssembly plugins,
  sandboxed with no file or network access. New built-in decoders arrive as pull requests.
- sigrok's Python decoders are not run (licence, ADR 0009).

## Signal generators

Every protocol has a generator that produces its example signal (used by the synthetic device and
the tests). A generated example always extends to its last edge; nothing is cut off at the end.
(A synthetic device capture is the requested length instead, like real hardware; doc 05.)

## Quality rules

- Every detector explains every choice in plain language and states its confidence.
- Every decoder reports problems (frame, parity, checksum, timing, incomplete) on the frame where
  they occur.
- Every protocol has a generated example and a test that detects and decodes it; real captures
  where available.
- Signal Detective's false-positive tests stay (BCD not reported as bit-reversed text, a clock not
  reported as UART 0x55).
