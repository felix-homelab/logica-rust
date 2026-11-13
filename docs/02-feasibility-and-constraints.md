# 02 Feasibility and constraints

What is known (with sources) and what must still be proven in a spike. Researched November 2026.

## Verdicts

| Feature | Verdict | Why |
|---|---|---|
| Rust engine inside an Electron app | Works | Electron's `utilityProcess` runs Node off the main thread; the engine is loaded there as a Node-API native module, which is ABI-stable across Node and Electron versions. The window is React. Spike S7 proves it on Windows and Linux (ADR 0002). |
| Talk to USB analysers | Works with limits | A Rust USB library in the engine (chosen in spike S3). Needs WinUSB on Windows and a udev rule on Linux (below). Serial devices come after version 1. |
| MCP server built in | Works | The engine serves MCP over Streamable HTTP on 127.0.0.1. Claude Code reaches it over stdio through `logica mcp`, which relays to the running app and starts it if needed (ADR 0007). |
| FNIRSI DLA-16 / DLA-32 Plus | Feasible, experimental | The protocol is described in public open-source work (below), not by FNIRSI. Buffer mode is the first target. The driver is clean-room (ADR 0009). Spike S3, right after M2, proves the basics. |
| Current Saleae (Logic 8, Pro 8, Pro 16, MSO) | Feasible, after version 1 | No open USB protocol. Saleae's gRPC Automation API (Logic 2 or the headless server) is the supported route. In version 1, Saleae users export binary from Logic 2 and open it here. |
| Original Saleae Logic and FX2 clones | Feasible | Cypress FX2 based; sigrok's open fx2lafw firmware is uploaded over USB, then samples stream over bulk transfers. |
| Sustained high-rate streaming | To be measured | Spike S2. Native code removes garbage-collection pauses and allows SIMD unpacking; the limit is expected to be USB and the device, not the engine. DLA stream mode needs up to 200 MB/s over USB 3. |
| Signed installers for Windows and macOS | Costs money | Decided: unsigned for all three systems (D8). Windows shows SmartScreen warnings. macOS arm64 needs at least an ad-hoc signature to run at all; users must remove the quarantine flag or allow the app in System Settings, and auto-update does not work on macOS without signing. |

## Operating-system constraints

- **Windows:** libusb-based access needs the device bound to the WinUSB driver. Devices installed
  with a vendor driver (for example the WCH driver used by FNIRSI's DLA_Logic) are invisible until
  rebound with Zadig, and rebinding breaks the vendor software until it is undone. This is the most
  annoying user-facing constraint; the app must explain it step by step.
- **Linux:** needs a udev rule granting the user access to the device. Ship the rules file.
- **macOS:** generally works without drivers.
- **WSL2 (the owner's setup):** run the desktop app on Windows, not inside WSL. Then USB works
  natively and no `usbipd` forwarding is needed. Claude Code inside WSL reaches the Windows app's
  MCP server by running the Windows `logica.exe mcp` through WSL interop (stdio works across it; it
  starts the app if needed) or over HTTP with WSL's mirrored networking, where `127.0.0.1` reaches
  Windows. Spike S8 verifies both. Running the app inside WSL (WSLg) works for files but needs
  `usbipd` for hardware.

## Hardware facts

### FNIRSI DLA-16 Plus / DLA-32 Plus

- USB 3.0, USB ID `1a86:5537`. 0x1a86 is WCH (QinHeng); the ID is shared with other WCH-based
  devices, so open the device and check its responses rather than trusting VID:PID. The USB
  controller is thought to be a WCH CH569 with an FPGA behind it (forum speculation).
- Buffer mode limits reported by the sigrok driver author: up to 1 GS/s with 1–8 channels,
  500 MS/s with 9–16, 250 MS/s with 17–32. Download over USB 2 is slower but works.
- Stream mode: 32 channels at 50 MS/s is 200 MB/s and needs USB 3. When the reader falls behind,
  the device ring buffer is overwritten and old data is replayed (observed by testers; about
  128 MiB replay distance). The stream format rounds channel counts up to 16 or 32 wire lanes.
- Features: single-stage triggers, adjustable threshold (−5 V to +5 V in 0.1 V steps per FNIRSI's
  spec), four PWM outputs (1 Hz to 20 MHz, 3.3 V).
- Open work: libsigrok PR #311 ("fnirsi-dla32", experimental, with `contrib/fnirsi-dla32.md`
  describing transport setup and limitations) and a DLA-16 Plus driver at
  `github.com/megabyte1024/libsigrok`. Both are GPL; only their written descriptions may be used
  (clean-room.md).
- FNIRSI states DLA_Logic is based on sigrok; its source is not published. Nothing from DLA_Logic
  may be copied.

### Saleae

- Automation API: gRPC over HTTP/2, default port 10430, enabled in Logic 2 settings or with
  `--automation`. The `.proto` file is public in `saleae/logic2-automation`. Supported devices:
  Logic 8, Logic Pro 8, Logic Pro 16 (not the original Logic, Logic 4 or Logic 16). A headless
  automation server exists for those devices and Logic MSO.
- Logic 2 offers simulation devices through the API, so it can be tested without Saleae hardware.
- `.sal` files are ZIP archives (`meta.json`, `digital-N.bin`, `analog-N.bin`); Saleae says the
  internal format is undocumented and will change. The binary *export* format (versions 0 and 1)
  is documented.
- Prior art: the Rust crate `saleae-logic2-automation-mcp` wraps the automation API as an MCP
  server, which shows people want agent control of analysers.

### FX2-based analysers

- fx2lafw is open firmware (GPL-2.0-or-later) for Cypress FX2 and clones such as the CBM9002A,
  used by the original Saleae Logic, USBee AX clones and many cheap 8-channel 24 MHz analysers.
  sigrok publishes pre-built binaries.

## Licensing constraints

- Porting sigrok code (drivers or protocol decoders) would make the project a derivative work under
  the GPL. Facts about a USB protocol are not copyrightable; translating driver code is. So drivers
  are re-implemented from protocol descriptions and our own USB captures under the "facts only"
  rule ([clean-room.md](clean-room.md)).
- Shipping the fx2lafw firmware binary next to the app is aggregation, which is fine with its GPL
  notice and a source link.
- FNIRSI's example `.demo` files that ship with DLA_Logic are FNIRSI's; they are not redistributed
  (doc 10).

## Sources

- [Electron: utilityProcess](https://www.electronjs.org/docs/latest/api/utility-process), [security checklist](https://www.electronjs.org/docs/latest/tutorial/security), [native modules](https://www.electronjs.org/docs/latest/tutorial/using-native-node-modules), [code signing](https://www.electronjs.org/docs/latest/tutorial/code-signing)
- [Node.js: Node-API](https://nodejs.org/api/n-api.html)
- [Microsoft: WSL mirrored networking mode](https://learn.microsoft.com/windows/wsl/networking#mirrored-mode-networking)
- [libsigrok PR #311: fnirsi-dla32](https://github.com/sigrokproject/libsigrok/pull/311)
- [EEVblog: FNIRSI DLA-16 Plus / DLA-32 Plus thread](https://www.eevblog.com/forum/testgear/fnirsi-dla-16-plus-dla-32-plus-logic-analyzer/)
- [megabyte1024/libsigrok (DLA-16 Plus driver)](https://github.com/megabyte1024/libsigrok)
- [Saleae: launching Logic 2 and the automation socket](https://docs.saleae.com/automation/guides/launching-logic2)
- [Saleae automation API docs](https://saleae.github.io/logic2-automation/automation.html)
- [saleae.proto](https://github.com/saleae/logic2-automation/blob/develop/proto/saleae/grpc/saleae.proto)
- [Saleae forum: headless automation](https://discuss.saleae.com/t/headless-logic2-automation-support/3793)
- [Saleae forum: .sal capture format](https://discuss.saleae.com/t/logic-2-capture-format-sal/1858)
- [Saleae binary export format v0](https://www.saleae.com/support/logic-software/saving-and-exporting-data/binary-export-format-logic-2), [2025 update (v1)](https://www.saleae.com/support/logic-software/saving-and-exporting-data/binary-and-csv-export-formats-2025-update)
- [sigrok: fx2lafw](https://sigrok.org/wiki/Fx2lafw)
- [crates.io: saleae-logic2-automation-mcp](https://crates.io/crates/saleae-logic2-automation-mcp)
