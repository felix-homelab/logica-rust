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
chunks arrive. While a capture runs, each decoder on it keeps one streaming decoder that is pushed
the edges that arrived since the last poll (`LiveDecode`); when the capture ends it is finished at
the end sample. A live decode gives exactly the frames and packets of a decode of the finished
capture (tested for every example, in uneven steps).

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
  50,000,000, past a 50 M window). The window also ends after 2^20 edges on any one channel, so a
  dense line cannot blow up memory; the analysis says so in a note when the window is shorter than
  the capture. Sampling the middle and end of long captures, and combining measurements from
  separate windows, is designed with `analysis.detect` in M1.
- Available for any time range, so an agent can ask "what is happening between 2.1 s and 2.3 s?".

## Stacking

A layer decoder consumes another decoder's packets instead of edges. It names the protocols it can
sit on (`uart`, `spi`, `i2c`) and turns packets into frames and packets of its own.

First layers: Modbus RTU (on UART), SPI NOR flash commands (on SPI), I2C/SPI register maps (below),
NMEA sentences (on UART), AT command and response pairing (on UART). The serial ADC source of
virtual analog channels (below) is a layer too, and the first one built: it runs the bus's own
streaming decoder inside itself, feeds it the edges, takes its complete packets and emits points;
the bus decoder's state is part of its snapshot, so live decoding and the batching rules hold
without any engine support. The layers above can follow the same pattern.

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

## Virtual analog channels (ADR 0010)

A virtual analog channel turns digital data back into the quantity it stands for. A **source**
emits points; the shared **scaling** in `core` turns them into values; doc 04 stores them.

- **Points.** A sample index and either a fraction of full scale (with the raw code where there is
  one) or a measured quantity with a unit (`Hz`, `s`). A source never scales.
- **Sources are streaming decoders** with the lifecycle above (`protocols::analog`): a `Source`
  describes the roles, settings and what its points mean, and creates a `StreamDecoder` whose
  output carries points, plus frames only for problems, and notes. The batching and snapshot rules
  apply to them unchanged. A new source implements `Source` and joins the list; nothing else
  changes.
- **Scaling.** Fraction: `low + fraction × (high − low) + offset`. Measured quantity:
  `quantity × gain + offset`. A fraction runs from 0 to just under 1. A code of N bits is a
  fraction of 2^N: unsigned, or two's complement, where the most negative code is 0 and the code 0
  is one half, so `low` and `high` are the ends of a bipolar range. The unit is free text, `V` by
  default.

| Source | Reads | Settings | Sample time |
|---|---|---|---|
| Serial ADC | packets of an SPI or I2C decoder (a layer, see "Stacking") | stream (MOSI, MISO, or the I2C data read), bits to skip, bit count N, bit order, encoding; I2C address and register; an optional match filter (for example "MOSI bits 6..4 = 2" or the mux field of a config register) | the start of the transfer |
| Parallel bus | edges of up to 16 channels, one per bit (D0 is the least significant) | N bits (D0 to D(N−1) each need a channel); latch on a clock or strobe edge (rising, falling, both) with an optional enable line (active low or high), or without a clock once every bit has stayed put for a settle time (default 3 samples) | the latch edge, or the start of the stable word (after its last changing bit) |
| PWM | edges of one channel | value: duty, frequency, period, or the width of the active pulse; active level; expected period (when not given, the last period measured) | the start of each period (an edge into the active level) |
| PDM / sigma-delta | a data and a clock channel | clock edge; sinc³ CIC decimation ratio R (default 64); the effective resolution is shown | the end of each output word |

- **Parallel words that do not settle** are left out without a problem: bits a few samples apart
  (skew) produce them on every change, so the note only counts them.
- **Presets.** ADCs are described as data (`crates/protocols/data/adc/presets.json`), checked
  in tests against their source's settings; `analog.sources` lists them with their source. A
  preset fills the source's settings and the scaling, and for a multiplexed ADC it names the
  settings of each input, so a client can create one channel per input:

  ```json
  { "id": "mcp3201", "name": "MCP3201 (12 bits, SPI)", "source": "spi-adc",
    "help": "Microchip MCP3201: 16 clocks a sample; …",
    "settings": { "mode": "0", "stream": "miso", "skipBits": 3, "bits": 12, "order": "msb",
                  "encoding": "unsigned" },
    "low": 0, "high": 3.3, "unit": "V" }
  ```

  First presets: MCP3201, MCP3208 (8 inputs, selected on MOSI), ADS1115 (I2C, pointer register
  0, two's complement), ADC0804 (parallel, RD as strobe, CS as enable).
- **I2C registers.** Many I2C ADCs (ADS1115) write a register pointer in one transaction and read
  it in the next, so the I2C ADC remembers the pointer last written to each address and keeps
  only reads of the chosen register.
- **PWM at the ends.** A line without an edge for more than two periods (the expected one, or the
  last measured) reads 0 % (low) or 100 % (high) from its last edge; frequency, period and pulse
  width leave a gap instead, since they have no value without edges. Exactly two periods still
  count as toggling. A period across the quiet stretch is not measured.
- **Problems.** A point that cannot be made is left out and a problem frame covers the stretch,
  with the decoders' problem codes and a message that says why: `incomplete` for a short transfer
  ("The SPI transfer has 8 bits; the sample needs bits 3 to 14"), `protocol` for a NACK,
  `incomplete` while a PDM filter fills (the first 3 × R bits), `timing` for a PWM that
  stopped ("The line stayed high for longer than two periods (200 µs), so it reads 100 %.").
  Problems are searchable and counted like decoder problems.
- **Math channels** take other virtual channels and a typed JSON expression (`add`, `sub`, `mul`,
  `div`, `scale`, `abs`, `min`, `max`, constants), with the unit given by the person. Inputs are
  combined by sample and hold: a point whenever any input has a new one, using the latest value of
  each, and none before every input has its first. Division by zero leaves a gap with a problem. A
  definition that would form a cycle is refused.
- **Detection** offers virtual channels as suggestions, with evidence, and never creates them:
  a channel whose PWM duty varies slowly ("duty moves between 12 % and 87 % over 400 periods"), a
  clock and data pair with PDM density, a parallel bus with a strobe, and a decoded SPI or I2C bus
  that matches an ADC preset (by I2C address, or by transfer length and the bits that stay zero).
- **Tests.** Every source has a signal generator that encodes a known waveform (a ramp and a sine)
  and is decoded back within one LSB (PDM: within its effective resolution), plus the batching and
  snapshot rules, the problems above, and both encodings at their edges (0, 2^N − 1, −2^(N−1)).

## Search, triggers and comparison

- Search over edges ("pulse narrower than 100 ns on CH3", "CH2 high while CH5 rises"), over
  decoded content ("I2C NACK", "SPI MOSI starts with 9F", "UART text contains ERROR", "frames with
  problems") and over virtual analog values ("ADC 1 above 2.5 V", "crosses 1.65 V rising",
  "PWM duty below 10 % for longer than 5 ms").
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
  them is safe. The built-in 1-Wire, DHT, WS2812, NEC infrared and DMX512 protocols are such data
  files themselves (`crates/protocols/data/`), loaded through the same validation as a person's
  own, so that path is exercised on every start. Learn-tab fields (fingerprint, sketch) are
  accepted and ignored.
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
