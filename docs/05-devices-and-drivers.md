# 05 Devices and drivers

## Driver interface

The traits live in `crates/core`, so drivers depend on `core` alone.

- A **USB driver** has an id (`fnirsi-dla`, `fx2lafw`, …), a name and USB filters (vendor ID,
  optional product ID). `probe` opens a transport and identifies the device (or says it is not
  ours); `open` returns a device.
- A **virtual driver** has no transport (the virtual analyser; the synthetic device in tests): it
  enumerates its devices and opens them by id. A driver whose devices come and go (a separate
  program that may or may not be running) says so (`hotplug`), and `device.list` asks it again
  each time; devices that are open stay listed.
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
| Synthetic | `synthetic` | M0, tests only | Not offered in the app since live capture (the virtual analyser below replaces it there); the engine registers it only when `EngineConfig::synthetic` is set, which tests, the scale test and the benchmarks do. Plays a program, repeated to the requested length: every generated protocol example, or `mix` (UART, I2C, SPI and PWM on 8 channels at 100 MS/s, with an optional dense 25 MHz stretch). Streams one tile at a time, so 5e9 samples run in bounded memory. Like real hardware, a capture is exactly the requested length; a frame cut off at the end is reported by its decoder as incomplete |
| Virtual analyser | `vla` | M2, for live capture | `logica-vla` (`tools/virtual-analyser`), a separate program that streams configurable signals (UART, I2C, SPI, PWM, clocks, noise; random payloads and pauses; a seed) in real time over TCP, in a protocol of Logica's own (`PROTOCOL.md` there). The engine connects out to it as a client, so ADR 0006 (ways into the engine) is unaffected. Where to look: `LOGICA_VLA` (comma-separated `host:port`), else `127.0.0.1:7171` to `7174` (so a few can run side by side; nobody listening is answered at once on loopback); tests pass their own addresses and no others. Stream mode only; one capture at a time. A `shared` signal follows the computer's clock, so analysers on one computer see the same edges, and `clockPpm` sets how far an analyser's sample clock is off: together they let several analysers be lined up in one capture |
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

## Several devices in one capture

`capture.start` with `with` captures several devices at the same time into one capture
(`crates/engine/src/group.rs`). Each device is configured first, all at the same rate (a capture
has one rate). Each runs on its own thread; a merger lines their data up and hands one stream to
the capture, so the store, live decoding and the window see one device. The capture's channels
are the first device's, then the next one's, numbered on (the second device's CH0 follows the
first's highest channel) and named by device ("Bench: TX", "Second: TX"; two alike are numbered).
The capture moves on only as far as every device has delivered, and ends where the first of them
ended; one device stopping or failing stops the others, keeping what all delivered.

Devices do not share a clock. Without help they are lined up at their starts: each device's
sample 0 at the capture's sample 0, which is close (they start a moment apart) but not exact, and
they drift apart by their clocks' difference (tens of ppm for crystals). With `align` (the channel
on each device that sees one shared signal: a common clock, or any line wired to all of them) the
merger lines them up by that signal's rising edges:

- It waits for 0.2 s of signal on every device, then finds the shift that puts most of a device's
  first rising edges within a twentieth of the usual spacing of the first device's. Between
  equally good shifts, as a steady clock gives one per period, it takes the smallest, so with a
  steady clock the devices must start within half its period of each other; a slow clock or an
  irregular signal has no such limit.
- While the capture runs, every further rising edge is matched, and each device's samples are
  mapped onto the first device's by a straight line through all matched pairs (offset and drift).
  Edges never move before what the capture already reached.
- A note says where each device's sample 0 fell, and at the end how far its clock ran off
  (ppm). If no shift matches within the first second, the device is lined up at its start, with a
  warning.

Tested with scripted devices (a known shift and 300 ppm: every edge of the shared clock within two
samples, the drift found to within 2 ppm) and with two virtual analysers, one 250 ppm off. Not yet
tried with real hardware; a device that delivers packed or bit blocks is converted to level changes
for the merger, which is not measured at high rates.

## Permissions and setup help

The device panel shows, per operating system, what to do when a device is not found:

- Windows: WinUSB / Zadig steps with screenshots, and how to undo them.
- Linux: the udev rules file (installed by the .deb, which returns before M3; for the AppImage, a button that shows the
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
