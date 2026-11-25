# Manual tests

Checks a person runs by hand because they need a real agent client, a display or hardware. Each
says what to do and what should happen; note the date, the versions and anything that differed in
the pull request that touched the area.

## Live capture from the virtual analyser (M2)

Setup: `cargo run -p logica-vla` in one terminal (it prints its signals and `Listening on
127.0.0.1:7171`), then `npm start`.

1. **Found.** The start screen lists "Virtual analyser: Bench" with "Capture live". (Started the
   other way round: open the device panel and press "Look for devices".)
2. **Live.** "Capture live" opens a tab "Live: Bench" with a "capturing" badge; the bar above the
   waveform counts the seconds; the waveform moves smoothly with the newest data on the right.
3. **Decode while it runs.** Add a UART decoder on CH0 (Decoders tab). Messages such as "status
   OK\r\n" appear under CH0 and in the decoded table, which keeps growing.
4. **Look back.** Drag the waveform to the right: the bar says you are looking back and the view
   stays put while data keeps arriving. "Follow" (or the End key) goes back to the newest data.
   Zooming with the wheel while following keeps following.
5. **Stop.** "Stop capture": the bar goes, the tab loses its badge, the capture keeps what arrived
   and says it was stopped (its notes), and it can be saved.
6. **Gone.** Stop `logica-vla` with Ctrl+C during a capture: the capture ends with a warning that
   the analyser closed the connection, keeping what arrived.

## Claude Code through `logica mcp` (M1)

Setup: build with `cargo build -p logica-cli`, then register the binary once:

```
claude mcp add logica -- /path/to/logica-rust/target/debug/logica mcp
```

Use a fresh data folder for a clean run: `LOGICA_DATA_DIR=/tmp/logica-manual claude`.

1. **Tools are offered.** `/mcp` lists `logica` as connected with 14 tools (session, list_devices,
   device, capture, open_file, save_capture, summarize, measure, decoder, get_decoded, search,
   render, show_user, run_command) and the prompts diagnose-bus and verify-expected.
2. **The exit scenario.** Ask: "Open fixtures/captures/mystery_C.demo with Logica and tell me what
   is on the wire." Claude opens the file and calls summarize; its answer names I2C on CH4 (SDA)
   and CH5 (SCL) at 400 kHz.
3. Ask: "Decode it and read what the EEPROM returned." Claude adds an I2C decoder and finds
   `CALIBRATED 2026` in the third transaction (about 660 ms).
4. Ask: "Mark that for me." Claude calls show_user; the answer gives the marker's time and sample.
5. Ask: "Rename the decoder to EEPROM, then undo that." The session afterwards shows the original
   name and the rename as undone.
6. **A picture.** Ask: "Show me the EEPROM read." Claude calls render and can describe the picture
   (rows from the top: time axis, the marker, CH4, CH5, the decoder).
7. **A device.** Ask: "Capture 3 million samples at 100 MHz from the synthetic mix device on
   channels 0, 1, 2 and 6 and summarize it." Claude connects, captures (progress may show) and
   finds UART, I2C, SPI and PWM.
8. **Errors read as instructions.** Ask Claude to open a file that does not exist; the tool error
   says the file could not be opened and why.
9. **Two agents, one engine.** With the first session still open, start a second Claude Code (or
   another MCP client) with the same `LOGICA_DATA_DIR`. Its session tool shows the first session's
   capture and marker, and its own actions appear as `agent:<its name>`. Closing the first session
   ends the engine; the second then gets "cannot be reached" (known limit until the app, M2).
10. **Nothing left behind.** After both sessions end, `logica.lock` is gone from the data folder.

## The desktop app (M2)

The end-to-end tests cover opening, analysing, decoding, an agent's marker appearing live, the
palette, undo and axe (`npm run test:e2e`). These need a person:

1. **Install and associations.** Install the build for your system from a release (or
   `npm run package`). Double-click a `.demo`, `.sr`, `.vcd` or `.lacap` file: it opens in a tab;
   with Logica already running, in the same window.
2. **Drag and drop.** Drag a capture file onto the window: it opens.
3. **Native dialogs and menus.** File > Open and File > Save as use the system's dialogs; every
   menu item shows its shortcut and the shortcut works.
4. **Look.** Light and dark follow the system (Settings > Theme overrides it); the waveform stays
   dark in both; focus is visible when moving with Tab; nothing moves when reduced motion is on.
5. **Waveform by hand.** Wheel and trackpad zoom and pan, pinch-zoom on a trackpad, click for
   cursor A, Alt+click for cursor B, Shift+drag to select; the measurements tab follows.
6. **Settings.** Switching MCP off removes `logica.lock`; on again writes it with a new port.
7. **An agent starts the app (S8, the owner's setup).** Quit Logica. On Windows, register
   `logica.exe mcp` (in the installation's `resources\bin`) with Claude Code in WSL
   (`claude mcp add logica -- /mnt/c/…/resources/bin/logica.exe mcp`). Ask Claude to open
   `mystery_C.demo` and mark the EEPROM read: the app starts minimised with "started by an
   agent", the marker appears live, and the activity feed lists Claude's actions with undo.
8. **Stop agent actions.** With an agent connected, switch "Stop agent actions" on and ask the
   agent to capture from the synthetic device: it is refused with the reason.
9. **Something goes wrong.** End the engine process with the task manager: the window says the
   engine was restarted and offers to open the files again; Help > Open the log folder shows
   `logica.log` with the reason. Close a tab or quit with unsaved changes: Logica asks first.
10. **Updates.** With a newer release published, the packaged app (Windows, Linux AppImage)
   downloads it and says it installs on quit; with Settings > Updates off it does not look.
