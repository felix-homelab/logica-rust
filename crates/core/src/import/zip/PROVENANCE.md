# Provenance: ZIP reader

## Sources used

- PKWARE's published `.ZIP File Format Specification` (APPNOTE.TXT): local file header, central
  directory header, end of central directory record, ZIP64 end of central directory record and
  locator, the ZIP64 extended information extra field (id 1), compression methods 0 (stored) and
  8 (deflate), CRC-32.
- Deflate itself is `miniz_oxide` (MIT OR Zlib OR Apache-2.0).

Signal Detective's `zip.ts` (the owner's work) was read for its error messages only.

Written with AI assistance (Claude Code), November 2026, for review by the owner.
