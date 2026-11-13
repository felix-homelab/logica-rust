# Provenance: sigrok session (`.sr`) importer

Clean-room rule: `docs/clean-room.md` (facts only).

## Sources used

- The layout as summarised in `docs/09-file-formats.md` (ZIP with `version` = 2, an INI
  `metadata` with `samplerate` as text with a unit, `unitsize` 1 to 4, `probeN` = channel on bit
  N-1, and `logic-1-N` files of packed little-endian samples).
- Inspection of our own fixtures in `fixtures/formats/sr/` (made by the owner from known raw
  samples with `sigrok-cli` 0.7.2 run as a separate program; `fixtures/README.md`): entry names,
  the `[global]` and `[device 1]` sections, `capturefile`, `total probes`, `total analog`, and data
  bits at the original channel positions when only some channels were captured.

## Sources not used

- No libsigrok, sigrok-cli, PulseView or DSView source files. The sigrok wiki page was not
  consulted (owner's decision, November 2026); doc 09 and the fixtures were enough.

Written with AI assistance (Claude Code), November 2026, for review by the owner.
