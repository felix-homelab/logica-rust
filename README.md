# Logica

Repository: `logica-rust`.

A desktop logic analyser that a person and an AI agent can operate together. It captures from real
hardware (FNIRSI DLA-32 Plus / DLA-16 Plus and FX2-based analysers in version 1), opens capture
files from other tools (Saleae, sigrok, VCD and more), finds and decodes protocols automatically,
and explains what it sees. A built-in MCP server lets agents such as Claude Code drive the whole
instrument: connect, capture, decode, search, measure and point things out to the person.

The engine is written in Rust; the window is an Electron app with a React interface (ADR 0002).
Open source under the Apache License 2.0 (`LICENSE`, ADR 0009). The command-line tool is
`logica`.

**Status: milestone M0 (foundations) built: the engine's capture store, file formats, protocol
engine and synthetic device, with no window yet.** It started with [docs/KICKOFF_PROMPT.md](docs/KICKOFF_PROMPT.md);
progress is tracked in [docs/progress.md](docs/progress.md).

## Documents

| # | Document | What it settles |
|---|---|---|
| 01 | [Vision and scope](docs/01-vision-and-scope.md) | Who it is for, what it does, what it deliberately does not do |
| 02 | [Feasibility and constraints](docs/02-feasibility-and-constraints.md) | What USB, operating systems, vendors and licences allow; verdict per feature, with sources |
| 03 | [Architecture](docs/03-architecture.md) | One engine and its clients, processes, repository layout, the command API |
| 04 | [Data model and storage](docs/04-data-model-and-storage.md) | Chunked transition storage, level of detail, very large captures, ingest |
| 05 | [Devices and drivers](docs/05-devices-and-drivers.md) | Driver and transport interfaces, plan per device, setup help, safety |
| 06 | [Protocol engine](docs/06-protocol-engine.md) | Streaming decoders, detection, stacking, register maps, queries, custom protocols |
| 07 | [MCP server](docs/07-mcp-server.md) | Transports, security, tools, resources, prompts |
| 08 | [User interface](docs/08-ui.md) | Desktop look, layout, panels, performance budgets, accessibility |
| 09 | [File formats](docs/09-file-formats.md) | Import and export, and the `.lacap` specification |
| 10 | [Testing and quality](docs/10-testing-and-quality.md) | Definition of done, test layers, fixtures, replay testing, CI |
| 11 | [Roadmap](docs/11-roadmap.md) | Spikes and milestones with exit criteria |
| 12 | [Decisions and risks](docs/12-decisions-and-risks.md) | What the owner decided, and what could go wrong |
| — | [Clean-room rule](docs/clean-room.md) | Which sources drivers and importers may be built from |
| — | [Benchmarks](docs/benchmarks.md) | Measured numbers and the baseline to beat |
| ADR | [Architecture decision records](docs/adr/) | The few decisions that are expensive to reverse |

## Where it comes from

- `../signal-detective`, the owner's learning tool (TypeScript, ISC licence), is the reference for
  the protocol knowledge: detectors, evidence texts, decoders, sample captures and answer keys.
  Its sample captures are in `fixtures/captures/`; its behaviour is the acceptance test; its code
  is not carried over (decision D12).
- A TypeScript prototype of this project (November 2026) tried the storage and decoding design
  and a first desktop window. Its measured numbers are the baseline in
  [docs/benchmarks.md](docs/benchmarks.md); none of its code is carried over.
