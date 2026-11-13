# 01 Vision and scope

## One sentence

A desktop logic analyser with a modern interface that talks to the analysers people actually own,
decodes and explains what it captures, and can be driven by an AI agent as easily as by a person.

## Why build it when PulseView, DSView and Saleae Logic 2 exist

Another general-purpose logic analyser only makes sense if it is clearly better at a few things the
others do not do:

1. **Agent-native.** The instrument has a typed command API, and the MCP server is a full client of
   it, not an afterthought. An agent can run "flash the firmware, capture the I2C bus, check that
   register 0x20 was written with 0x57" in one session while the person watches it happen live in
   the same view. No incumbent offers this (Saleae's automation API is close, but Saleae-only and
   without a shared live view).
2. **Explains, not only decodes.** Automatic bus grouping, protocol detection with plain-language
   evidence, "why this decode looks wrong" and register-level meaning, as in Signal Detective.
   Aimed at people learning, and useful to everyone at 2 a.m.
3. **Open tooling for underserved hardware, open to other tools' files.** First-class support for
   the FNIRSI DLA-32 Plus / DLA-16 Plus, which has no good open software, plus cheap FX2 clones.
   Files from Saleae Logic 2, sigrok/PulseView, simulators (VCD) and others open in the same
   interface.
4. **One install, a modern tool.** One installer for Windows, macOS and Linux gives the viewer,
   every driver and the MCP server; no browser, no separate server to start. It feels like a
   current desktop app (command palette, resizable panels, native dialogs, file associations).

If a feature does not serve one of these four, it waits.

## Who it is for

| User | Typical job |
|---|---|
| Learner (the owner) | "What is on these wires, which settings do I need, why does the decode look like garbage?" |
| Embedded developer | Capture a bus during a test, decode it, search for the failing transaction, share the capture |
| AI agent (Claude Code and others, via MCP) | Hardware-in-the-loop debugging: capture, decode, compare with expected bytes, report, point the person at the problem |

## In scope (version 1)

- Digital channels only; up to 32 channels per device.
- Capture from the FNIRSI DLA-32 Plus (tested by the owner) and DLA-16 Plus (same driver, untested
  until someone with the device reports back), and FX2-based analysers (original Saleae Logic and
  the many 8-channel 24 MHz clones).
- The capture modes a device supports: buffered (record to device memory, then download) and
  streaming; hardware triggers where the device has them, software and protocol triggers for
  streaming devices.
- Device outputs where they exist (the DLA's PWM generator), behind an explicit safety switch.
- Import FNIRSI `.demo`, sigrok `.sr`, VCD, Saleae binary export, CSV, raw binary; best-effort
  Saleae `.sal`. Export `.sr`, VCD, CSV, Saleae binary and the native `.lacap`, which keeps
  decoders, markers and notes.
- Detection and streaming decoding for at least everything Signal Detective covers (UART, I2C, SPI,
  PWM, 1-Wire, DHT, WS2812, NEC IR, CAN, I2S, SWD, JTAG, DMX512, custom pulse-code and preset
  protocols); layer decoders (Modbus RTU, SPI NOR flash, NMEA, AT commands), register maps for
  common chips, and capture comparison.
- MCP server (stdio relay and Streamable HTTP) with a curated set of tools, resources and prompts,
  sharing a live session with the app window.
- Unsigned installers for Windows (x64), macOS (arm64, x64; untested by the owner) and Linux (x64:
  AppImage and .deb), with auto-update on Windows and Linux.
- Explanations everywhere: evidence for every detection, "why" for every problem, protocol
  knowledge, byte analyser.

## Out of scope (version 1)

- Analog channels and oscilloscope functions.
- Pattern generation beyond outputs a device already has.
- Writing FNIRSI `.demo` or Saleae `.sal` files.
- Capture through Saleae's automation API, a sigrok bridge, and Pico-based analysers (after
  version 1; doc 05).
- Code plugins; version 1 has custom JSON protocols only (doc 06).
- The Learn tab (question-based identifier, calculators, learner guide); the explanations
  themselves are in version 1.
- Running sigrok's Python protocol decoders (not compatible with the licence, ADR 0009).
- A browser or web version, and anything on phones or tablets (ADR 0002).
- Multi-user collaboration over the network. One person and their agents on one machine.
- Replacing vendor firmware or FPGA bitstreams.

## Success measures

- An agent, given only the MCP server, can capture and correctly decode the practice signals of the
  known-signal rig (doc 10) and explain them, without help.
- The owner's DLA-32 Plus captures 32 channels in buffer mode and the decodes match the rig's known
  output.
- Opening a 1-billion-sample, 16-channel capture: first view in under 2 s once indexed, smooth pan
  and zoom (doc 08 budgets).
- Every sample capture of Signal Detective detects and decodes as its answer keys and tests say
  (the three FNIRSI example files are checked locally only, doc 10).
