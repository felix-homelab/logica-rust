# Provenance: virtual analyser

No third-party device or protocol. The wire protocol (`wire.rs`,
`tools/virtual-analyser/PROTOCOL.md`) is Logica's own design, made for its own test program
`logica-vla` (`tools/virtual-analyser`), which generates its signals with Logica's own generators
(`crates/protocols/src/synth.rs`). No sigrok, PulseView, DSView or vendor sources were used.

Written with AI assistance (Claude Code), November 2026, for review by the owner.
