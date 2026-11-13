# ADR 0003: Drivers are written against a Transport interface

Status: accepted

## Context
Drivers run in the engine, both in the app and in the headless CLI. They must also run in CI
without hardware, and a Windows-only way of talking to the DLA (FFI to WCH's driver library,
decision D6) may be needed later.

## Decision
Define a `Transport` trait for USB: control transfers, and bulk in and out with several outstanding
requests. Implementations: the real USB library (chosen in spike S3), a **record** transport that
wraps a real one and writes a transcript, and a **replay** transport for tests. Drivers never use a
USB library directly.

## Consequences
- One driver per device, testable without hardware through replay transcripts (doc 10).
- A Windows-only FFI transport can be added without touching drivers.
- Serial ports come back as a second transport kind with serial devices after version 1.
