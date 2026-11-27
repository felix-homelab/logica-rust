# ADR 0010: Virtual analog channels from digital data

Status: accepted

## Context
A logic analyser often watches an analog quantity indirectly: an ADC on SPI or I2C, an ADC with a
parallel output on several pins, a PWM signal that stands for a level, or the bitstream of a PDM
microphone or a sigma-delta modulator. People want to see that quantity as a curve in volts, per
cent or their own unit, measure it and search it, not read hex values off a decoder row. Real
analog inputs (an oscilloscope) stay out of scope; these channels are computed from digital data
that is already captured.

The kinds of source differ a lot (a protocol layer, a latched bus, edge timing, a digital filter),
and more will come. The design must let a new source be added without touching scaling, storage,
the command API, MCP or the window.

## Decision
- **A virtual analog channel** is a definition in the session: a source with its channels and
  settings, a scaling, a name, a unit and a colour. Its values are derived like a decode, never
  recorded data.
- **Two stages.** A **source** turns digital data into points: a sample index and either a
  **fraction of full scale** (an ADC code over 2^N, a PWM duty, a PDM density) with the raw code
  where there is one, or a **measured quantity** with a unit (frequency in Hz, period or pulse
  width in seconds). One shared **scaling** step turns points into values:
  - fraction: `value = low + fraction × (high − low) + offset`;
  - measured quantity: `value = quantity × gain + offset`.
  `low`, `high` and `offset` are in the channel's unit; the offset is a calibration trim. Codes are
  unsigned or two's complement (offset binary is unsigned with a negative `low`). Following ADC
  datasheets, code 2^N − 1 reads one LSB below `high`. The resolution N belongs to the source.
- **First sources:** serial ADC (a layer on an SPI or I2C decoder: a bit field of the transfer, an
  optional match filter for multiplexed inputs, presets as data files), parallel bus (latched on a
  clock or strobe edge, or once the word has been stable for a settle time), PWM (duty, frequency,
  period or pulse width; a line that stops toggling reads 0 % or 100 % after two periods, with a
  note) and PDM / sigma-delta (a sinc³ CIC filter with a decimation ratio).
- **Sources are streaming decoders.** They live in `protocols` and follow the decoder lifecycle of
  doc 06 (create, push, finish, drain, snapshot), so live capture, resume and the batching tests
  apply unchanged. Scaling and the value-series store live in `core`. Neither does I/O.
- **Nothing is invented.** Input a source cannot turn into a value (a short frame, a NACK, a PWM
  that stopped) leaves a gap and a problem that says why. What every capture has (a filter
  filling at the start, words between skewed bits) is left out too, and said once in a note
  rather than counted as problems.
- **Math channels** combine virtual channels with a typed JSON expression (like queries, doc 06).
  Inputs sampled at different times are combined by sample and hold: a point whenever any input has
  a new value, none before every input has one. A cycle is refused when the channel is created.
- **Commands first (ADR 0001).** `analog.*` commands create, change, remove, list and read virtual
  channels; the mutating ones return their inverse (ADR 0008). MCP reaches them through
  `run_command` and the existing tools (`summarize`, `measure`, `search`, `render`, `save_capture`)
  include virtual channels; a tool of their own comes when agents need it often (doc 07).
- **Detection suggests, never creates.** Likely PWM levels, PDM streams, parallel buses and known
  ADCs are offered with evidence; a person or agent creates the channel.
- **Saved as a definition.** `.lacap` stores the definitions in the session state of the index;
  values are computed again on open. No new blob type and no format version change.

## Consequences
- A new source is a streaming decoder that emits points; scaling, storage, drawing, measurements,
  search, export and MCP work for it without change.
- Opening a capture with virtual channels computes them, which costs time on long PDM captures; a
  cached value blob can be added later as a minor `.lacap` version (unknown blobs are skipped).
- Values carry no more precision than their source: a PWM measured at 1 MHz from a 20 kHz signal
  has 50 steps, and the window shows the resolution so nobody trusts digits that are not there.
- Two-point calibration (gain and offset errors together) is not covered; it would be another
  scaling mode without changes to sources.
- The serial ADC needs the layer decoders of doc 06 ("Stacking"), which are built for it.
