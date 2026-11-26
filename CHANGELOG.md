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

[Unreleased]: https://github.com/felix-homelab/logica-rust/commits/main
