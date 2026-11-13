# Provenance: FNIRSI DLA_Logic `.demo` importer

Clean-room rule: `docs/clean-room.md` (facts only).

## Sources used

- The format description in `../signal-detective/README.md` ("The .demo format"), the owner's own
  reverse engineering of the sample files (Signal Detective is the owner's work, ISC licence,
  decision D12). Its `src/core/capture/demo.ts` and `artifact.ts` were read for behaviour (which
  notes to give, the start-up artefact rule); this code is newly written in Rust, not ported.
- The sample captures in `fixtures/captures/` (made by Signal Detective's generator) and, locally
  only, FNIRSI's example files `I2C.demo`, `SPI.demo`, `UART.demo` (opened as data; never
  committed, doc 10).

## Sources not used

- No DLA_Logic code, binaries or decompiled output; no sigrok, PulseView or DSView sources.

## Unverified

- A set mask bit inside the sample range (read as constant from the `value` bit, with a warning),
  the `0-` file prefix, `vernier.ini` and `TriggerSamplingDepth`: no sample exercises them.

Written with AI assistance (Claude Code), November 2026, for review by the owner.
