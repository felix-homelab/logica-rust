# Clean-room rule for drivers and file formats

The project has a permissive licence (ADR 0009). Code from GPL projects (libsigrok and its device
drivers, sigrok decoders, PulseView, DSView) and from vendor software (DLA_Logic, Logic 2) must not
end up in it, not even translated into Rust.

## The rule: facts only

| Allowed sources | Forbidden sources |
|---|---|
| Written protocol descriptions, for example `contrib/fnirsi-dla32.md` in libsigrok PR #311 and the PR discussion | Any source file of libsigrok, sigrok decoders, PulseView, DSView (`.c`, `.h`, `.cpp`, `.py`) |
| Forum posts and articles that describe behaviour | Code snippets quoted in forums or issues |
| Our own USB captures (Wireshark with USBPcap, usbmon) of any software talking to the device | Decompiled or disassembled vendor software (DLA_Logic, Logic 2) |
| Published format specifications (Saleae binary export, VCD, the sigrok `.sr` layout as documented on the sigrok wiki) | |
| Datasheets | |

This applies to people **and** to AI agents. Agents working in this repository must not open, fetch
or search sigrok source files, and must not write driver code "from memory" of such code; they work
from the allowed sources and from transcripts in `fixtures/usb/`.

## Provenance note

Every driver and every importer for a third-party format has a `PROVENANCE.md` next to its code:
which allowed sources were used (with links or transcript names), and who wrote it. A review checks
that the note exists and lists only allowed sources.

## If someone has read forbidden code

They do not write or review the affected driver. Say so in the pull request; another contributor
takes it.
