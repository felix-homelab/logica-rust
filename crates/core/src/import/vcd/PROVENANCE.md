# Provenance: VCD importer

Clean-room rule: `docs/clean-room.md` (facts only).

## Sources used

- IEEE Std 1364 (Verilog), the four-state value change dump format: `$timescale`, `$scope` /
  `$upscope`, `$var`, `$enddefinitions`, `$dumpvars` and friends, `#time`, scalar changes
  `0 1 x z` followed by the identifier, vectors `b…` and reals `r…` followed by a space and the
  identifier, and the left-extension rule for short vector values.
- `docs/09-file-formats.md` for the choices: sample 0 is the first timestamp, one sample is the
  greatest common divisor of the time steps, x and z read as low (counted in a note), reals
  skipped, extended VCD not handled.
- Our own fixtures `fixtures/formats/vcd/` (one written by hand, one produced by `sigrok-cli`).

## Choices made here (also in doc 09)

- A vector becomes one channel per bit, named `name[k]` after its declared range, least
  significant position first.
- A name used in more than one scope gets its scope path (`top.sub.en`).
- Every declaration sharing an identifier gets its own channel with the same data.
- The last timestamp ends the capture; it is a sample of its own only if values change there.

## Sources not used

- No sigrok, PulseView, DSView or GTKWave source files.

Written with AI assistance (Claude Code), November 2026, for review by the owner.
