# ADR 0008: Engine-wide undo history and a device lease

Status: accepted

## Context
A person and one or more agents act on the same session. People expect Ctrl+Z for their own actions,
and the activity feed should let them undo what an agent did. Undo is painful to retrofit. A device
is one physical resource: an agent reconfiguring it while the person sets up a trigger produces
confusing results.

## Decision
- **Undo.** Every mutating command that changes session state (decoders, markers and notes, channel
  names, colours and groups, capture metadata) returns its inverse command. The engine keeps one
  history with the actor of each entry. `history.undo` and `history.redo` work on it: by default on
  the actor's own last action, or on any entry by id. A new action by an actor discards that actor's
  redo entries. Captured data, hardware actions and file writes are not undoable and say why.
- **Redo is exact.** A command that creates something (a decoder, a marker) records the id it
  created, so redo recreates the same id and later history entries still refer to it.
- **Lease.** Hardware commands (`device.configure`, `capture.start`, `output.*`) need the device's
  lease. The first actor to use a device holds it; others get a clear error ("The DLA-32 Plus is in
  use by agent Claude Code; ask the person to take it over"). The person can always take the lease
  from an agent. Leases end on disconnect, on release, or when the holding client goes away.
- Analysis state (decoders, markers, view) has no locks: last writer wins, visible in the activity
  feed and undoable.

## Consequences
- Each mutating command needs an inverse; this is part of its definition and is tested (undo
  restores the state, redo reapplies it).
- Agents get predictable errors instead of fighting the person over the device.
- `session` reports who holds which lease, so agents can explain why they wait.
