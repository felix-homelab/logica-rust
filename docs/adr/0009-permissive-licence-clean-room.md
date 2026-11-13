# ADR 0009: Apache-2.0 licence; drivers and importers written clean-room

Status: accepted

## Context
The project is an open-source product on the owner's personal GitHub. The open DLA drivers
(libsigrok PR #311, megabyte1024/libsigrok) and sigrok's protocol decoders are GPL; porting them
would make the whole project GPL. The owner wants a permissive licence for adoption and
embeddability.

## Decision
- Licence: **Apache-2.0** (full text in `LICENSE`): permissive, with an explicit patent grant.
- No code is ported from sigrok or any other GPL project, or from vendor software. Drivers and
  third-party format importers are written clean-room under the "facts only" rule in
  [docs/clean-room.md](../clean-room.md), each with a `PROVENANCE.md`.
- Every dependency has a permissive licence compatible with Apache-2.0.
- `sigrok-cli` may be used later as a separate program (a bridge), which is not a derived work.
- The fx2lafw firmware binary ships next to the app as an aggregate, with its GPL-2.0-or-later
  notice and a source link.
- sigrok's Python decoders are not run.

## Consequences
- Driver work is slower: protocols come from written descriptions and our own USB captures.
- AI agents writing drivers follow the same rule (CLAUDE.md says so).
