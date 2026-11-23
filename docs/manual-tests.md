# Manual tests

Checks a person runs by hand because they need a real agent client, a display or hardware. Each
says what to do and what should happen; note the date, the versions and anything that differed in
the pull request that touched the area.

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

## The window (spike S7, M1)

1. `npm run build` then `npm start -w apps/desktop` (needs a display). A window opens and reports
   the engine as connected with its command API version and the cold start time.
2. Close the window: the app quits and no `logica` engine process remains.
