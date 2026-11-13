# 05 Devices and drivers

## Driver interface

The traits live in `crates/core`, so drivers depend on `core` alone.

- A **USB driver** has an id (`fnirsi-dla`, `fx2lafw`, …), a name and USB filters (vendor ID,
  optional product ID). `probe` opens a transport and identifies the device (or says it is not
  ours); `open` returns a device.
- A **virtual driver** has no transport (the synthetic device): it enumerates its devices and opens
  them by id.
- A **device** has its info (id, model, serial, firmware, channel count) and **capabilities** (rates
  per channel count, depth, modes, triggers, thresholds, outputs, options). It can be configured
  (rejected with an explanation when impossible), started with a block sink and a stop signal
  (returning the sample count, whether it was stopped, and notes), and closed.
- Output control (PWM) is added with the first device that has outputs (M3).

Capabilities are data, so the UI and MCP can show valid choices and reject impossible ones ("250 MS/s
is the maximum with 17 or more channels") before touching hardware.

A **Transport** wraps USB: control transfers, and bulk in and out with several outstanding
requests (ADR 0003). Implementations: the real USB library (chosen in spike S3), **replay** (tests)
and **record** (wraps a real transport and writes a transcript for later replay). All run in the
engine; the window never touches USB.

Every driver is written clean-room ([clean-room.md](clean-room.md)) and has a `PROVENANCE.md`.

## Device plan

| Device | Driver | Milestone | Notes |
|---|---|---|---|
| Synthetic | `synthetic` | M0 | Plays a program, repeated to the requested length: every generated protocol example, or `mix` (UART, I2C, SPI and PWM on 8 channels at 100 MHz, with an optional dense 25 MHz stretch). Streams one tile at a time, so 5e9 samples run in bounded memory. Like real hardware, a capture is exactly the requested length; a frame cut off at the end is reported by its decoder as incomplete |
| FNIRSI DLA-32 Plus / DLA-16 Plus | `fnirsi-dla` | spike S3 after M2; driver M3 | Buffer mode first, then threshold, trigger, PWM outputs, then stream mode. DLA-16 Plus untested (the owner has no device) |
| FX2 analysers (original Saleae Logic, 8 ch / 24 MHz clones, USBee AX clones) | `fx2lafw` | M4 | Upload fx2lafw firmware, wait for re-enumeration, stream bulk data. No hardware trigger: software and protocol triggers in the engine |

After version 1: Saleae Logic 8 / Pro 8 / Pro 16 / MSO through Saleae's automation API
(`saleae-automation`; until then, export binary from Logic 2 and open it here), any sigrok device
through `sigrok-cli` run as a separate program (`sigrok-bridge`), and Raspberry Pi Pico analysers
over serial (`pico-la`).

### FNIRSI DLA-32 Plus / DLA-16 Plus

What is known is in doc 02. The driver is built under the "facts only" rule from:

1. the written protocol description in libsigrok PR #311 (`contrib/fnirsi-dla32.md`) and the PR
   discussion (not the C code);
2. our own USB captures of DLA_Logic (Wireshark with USBPcap on Windows, usbmon on Linux), stored as
   transcripts in `fixtures/usb/fnirsi-dla/`.

Spike S3 answers whether this works at all before any driver is built: open the device on Windows with
WinUSB, run the identification exchange, and record one buffer capture as a transcript.

Steps (M3):

1. Identification: open `1a86:5537`, run the identification exchange, reject other WCH devices.
2. Buffer capture: configure channels and rate, arm, wait, download. Sparse channel packing (only
   enabled channels are transferred) must keep physical channel numbers.
3. Threshold and trigger (single stage: rising, falling, high, low, any edge).
4. PWM outputs, behind the output safety switch.
5. Stream mode: detect when the reader falls behind and report data loss instead of showing
   replayed data. Never promise more than the measured sustained rate.

Windows (D6): DLA_Logic installs a WCH vendor driver; libusb-based access needs WinUSB instead.
Version 1 ships a step-by-step Zadig guide and a "switch back to DLA_Logic" guide, and the device
panel detects and explains the situation. A Windows-only transport that calls WCH's driver library
through FFI, so both programs could coexist, is evaluated after spike S3. It could only call a
library that DLA_Logic already installed; we do not redistribute WCH's DLL.

### fx2lafw

- Ship the pre-built fx2lafw firmware with its GPL-2.0-or-later notice and a source link
  (aggregation).
- Sequence: find an unconfigured FX2 (known VID:PIDs), upload firmware to RAM with vendor control
  requests, wait for re-enumeration under the fx2lafw VID:PID (hot-plug event), re-open, configure
  and stream. The device panel shows "loading firmware…" so the brief disconnect is no surprise.
- Streaming needs several outstanding bulk-in transfers; spike S2 measures how many and the
  sustained rate. Expected ceiling: the FX2's USB 2 limit (24 MS/s at 8 channels).
- The protocol comes from fx2lafw's documented behaviour and our own USB captures, not from
  libsigrok's driver code.
- Test devices: an FX2 clone, and the owner's Saleae Logic if it is the original FX2-based model.

## Permissions and setup help

The device panel shows, per operating system, what to do when a device is not found:

- Windows: WinUSB / Zadig steps with screenshots, and how to undo them.
- Linux: the udev rules file (installed by the .deb; for the AppImage, a button that shows the
  install command) and the command to reload rules.
- macOS: usually nothing; untested by the owner.
- WSL: recommend the Windows build; otherwise `usbipd list`, `usbipd bind`, `usbipd attach --wsl`.

## Safety and sharing

- Outputs (PWM, any pin driving) are off by default. The person switches them on in the app for the
  session. Agents can request it; the app shows a dialog (and an OS notification when the window is
  in the background). Confirmation never goes through MCP elicitation, because the agent's client
  could answer it without a person (doc 07).
- Hardware commands need the device lease (ADR 0008). The person can take it from an agent at any
  time.
- Threshold changes are allowed for agents and logged in the activity feed.
