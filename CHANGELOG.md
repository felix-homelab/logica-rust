# Changelog

All notable changes for people who use Logica, its MCP server or its command line are recorded in
this file. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html), and the rules
for entries are in [RELEASING.md](RELEASING.md#changelog).

Changes to the design documents are not recorded here: see the git history and the
[ADRs](docs/adr/).

## [Unreleased]

The first release: milestones M0 to M2 ([doc 11](docs/11-roadmap.md)). It works with recordings
and the virtual analyser; real hardware comes in 0.2.0. The installers are unsigned.

### Added

- Files: open FNIRSI `.demo`, sigrok `.sr` and VCD files, and save captures as `.lacap`, Logica's
  own single-file format. Saving is in place or as a new file, and a save that was interrupted is
  recovered. Captures longer than 2^32 samples work. (#2)
- Decoders: UART, SPI, I²C, I²S, CAN, JTAG, SWD and PWM, and the pulse-code protocols NEC
  infrared, DHT sensors, WS2812 LEDs and 1-Wire, each with automatic detection that says why it
  thinks a channel carries the protocol. Your own protocols can be described as JSON. (#2)
- Analysis: measurements, bus grouping and a byte analyser. (#2)
- MCP: agents such as Claude Code connect over MCP and work in the same session as the person:
  open, capture, decode, search, measure, render a range as a picture, and point things out with
  markers that appear in the window. `logica mcp` connects an agent over stdio and starts the app
  when it is not running; MCP over HTTP listens on 127.0.0.1 only, with a token. Every agent
  action shows in the activity feed, can be undone, and "Stop agent actions" pauses them. (#3, #4)
- CLI: the `logica` command, shipped with the app: `inspect` shows what a capture holds,
  `convert` turns any file Logica reads into `.lacap`, and `open` hands a file to the app. (#3,
  #4)
- App: the desktop window for Windows, Linux and macOS: tabs, channel list, waveform with cursors,
  selection and markers, analysis with evidence, decoders with settings, search, decoded table,
  measurements, device panel, command palette, light and dark themes, file associations and drag
  and drop. The app updates itself on Windows and Linux. (#4, #5)
- App: decoders group their channels in the protocol's order (SPI: CLK, CS, MOSI, MISO), and the
  decoded data shows as text under the frames, in each decoder's own format. (#7)
- App: closing asks about unsaved work; when the engine stops, it is restarted and a banner
  offers to open the files again; a log file records what happened. (#6)
- Devices: live capture with live decoding, from one device or several in one capture, lined up
  at their starts or by a shared signal. The virtual analyser (`logica-vla`) lets you try it
  without hardware. (#8)
- App: a redesigned window: colour means one thing, states are written as text, panels are flat
  with one scroll bar each, "Capture live" goes in steps that follow what is chosen, and nothing
  is cut off at the smallest window size. (#9, #10)
- Virtual analog channels: the duty, frequency, period or pulse width of a PWM signal as values
  in your own unit and range, computed from the capture, followed live while capturing and saved
  with the `.lacap`. A PWM that stops toggling reads 0 % or 100 % and says why. Agents create and
  read them through `run_command` (`analog.sources`, `analog.add`, `analog.values`). (#19)
- App: virtual analog channels in the window: a row of their own with a value axis, drawn as steps
  or a line and resized by dragging its bottom edge, and "Add virtual analog channel" in the
  channel list, with a dialog in steps for the source and its channel, its settings, the scaling,
  and the name and look. (#20)
- Devices: the virtual analyser's PWM can carry a level: with `dutyRange` its duty glides from one
  random value to the next, or with `sinePeriodMs` follows a sine. The built-in mix has one of
  each, CH11 (LEVEL) and CH12 (SINE), for trying virtual analog channels without hardware. (#21)
- Analysis: a PWM whose duty changes is recognised as a PWM carrying a level (it was taken for
  UART), with the range its duty moves in, and "Add PWM level" turns any PWM found into a virtual
  analog channel in one click. (#24)
- App: the Measurements panel reads virtual analog channels at the cursors: the value, the raw
  code, the difference from A to B and one step. (#25)
- Virtual analog channels from ADCs on SPI and I2C: a bit field of every transfer, unsigned or
  two's complement, with a filter for one input of a multiplexed ADC, and presets for the
  MCP3201, MCP3208 and ADS1115. (#26)
- App: the virtual analog dialog offers the ADC chips a source knows: choosing one fills the
  settings and the scaling, and for an ADC with several inputs it adds one channel per input. The
  virtual analyser sends an MCP3201 sine on CH13 to CH15 to try it. (#27)
- Virtual analog channels from a parallel bus of up to 16 bits, read on a clock or strobe edge
  (with an enable line), or without a clock once the word stays put, so skewed bits give no false
  values; with a preset for the ADC0804. (#28)
- Virtual analog channels from PDM or sigma-delta bitstreams (PDM microphones, isolated ADCs),
  filtered back into values by a sinc³ filter with a decimation ratio you choose. (#29)
- Measurements of virtual analog channels over the selection or the view: mean and RMS weighted
  by time, minimum, maximum and peak to peak, in the Measurements panel and for agents through
  the `measure` tool. (#31)
- Virtual analog values export as CSV: one table with time, sample and a column per channel (and
  its raw code), from the channel list or for agents through `analog.export`. (#32)
- Virtual analog channels can read from a decoder of the capture instead of its channels: an ADC
  on the SPI or I2C decoder, a PWM level on the PWM decoder. They take the decoder's channels and
  settings and follow it when it changes. (#33)
- App: decoders get a name of your own when added (the protocol's name otherwise) and can be
  renamed in their settings; undo takes a rename back. (#35)
- Devices: the virtual analyser also plays an MCP3208 (eight inputs, read in turn) and an ADS1115
  (I2C), next to the MCP3201, each sending sines in its preset's format (CH16 to CH21 of the
  built-in mix), and offers rates up to 1000 MS/s (1 GS/s). (#36)
- Search finds virtual analog values: where a channel crosses a level, rising or falling, or
  stretches where it stays above, below or between levels, optionally for longer than a time; in a
  query it also filters other hits by the value at their start. (#38)

### Changed

- **Breaking:** sample rates are written in samples per second (S/s, kS/s, MS/s, GS/s) in the window, MCP
  answers and the CLI, so they are not mistaken for frequencies, which stay in Hz. (#37)
- App: the mouse wheel scrolls through the channels; Ctrl+wheel (Cmd on macOS) zooms and
  Shift+wheel pans, so a capture with many channels can be scrolled without changing the zoom.
  (#37)

### Fixed

- App: a cursor set just before the window read the session again (after adding a decoder or a
  virtual channel, for example) is no longer taken back. (#30)

[Unreleased]: https://github.com/felix-homelab/logica-rust/commits/main
