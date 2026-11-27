// Generated from schema/contract.json by scripts/generate.mjs. Do not edit by hand.
// Command API version 1.0.

export const API_VERSION = "1.0";

export interface CommandSpec {
  name: string;
  summary: string;
  safety: 'read' | 'capture' | 'output';
  lease: boolean;
  longRunning: boolean;
  undo: { kind: 'query' } | { kind: 'inverse' } | { kind: 'never'; reason: string };
}

export const COMMANDS: readonly CommandSpec[] = [
  {
    "name": "analog.add",
    "summary": "Add a virtual analog channel computed from digital channels (source, roles, settings, scaling); undo removes it.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "analog.at",
    "summary": "Every virtual analog channel's value, raw code and step at given samples, such as the cursors.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "analog.remove",
    "summary": "Remove a virtual analog channel; undo restores it with the same id.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "analog.sources",
    "summary": "The sources of virtual analog channels, with their channels and settings.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "analog.update",
    "summary": "Change a virtual analog channel's source, channels, settings, scaling, name, unit or style.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "analog.values",
    "summary": "Values of a virtual analog channel in a range, in its unit: points, or an envelope per bucket, with problems.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "analysis.bytes",
    "summary": "What a run of bytes might mean: text, hidden text, Modbus, checksums, BCD, 16-bit pairs.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "analysis.detect",
    "summary": "Measure channels, group them into buses and detect protocols with evidence, for the whole capture or a range.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "capture.close",
    "summary": "Close a capture and free its memory.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Closing drops the capture; open it again from its file."
    }
  },
  {
    "name": "capture.rename",
    "summary": "Rename a capture.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "capture.start",
    "summary": "Start a capture with the device's configuration; the capture appears while it streams in.",
    "safety": "capture",
    "lease": true,
    "longRunning": true,
    "undo": {
      "kind": "never",
      "reason": "Captured data cannot be undone; close the capture instead."
    }
  },
  {
    "name": "capture.stop",
    "summary": "Stop a running capture.",
    "safety": "capture",
    "lease": true,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Hardware actions cannot be undone."
    }
  },
  {
    "name": "channel.colour",
    "summary": "Change a channel's colour.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "channel.group",
    "summary": "Put channels into a named bus group, or ungroup them.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "channel.hide",
    "summary": "Hide or show a channel.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "channel.rename",
    "summary": "Rename a channel.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "decoder.add",
    "summary": "Add a decoder (protocol, roles, settings), or one from a detection; undo removes it.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "decoder.frames",
    "summary": "Decoded packets or frames by time range, kind, problem and text, paged; optionally explained.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "decoder.remove",
    "summary": "Remove a decoder; undo restores it with the same id.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "decoder.update",
    "summary": "Change a decoder's roles, settings, name or data format (how its data is written in the waveform); returns how its problem counts changed.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "device.configure",
    "summary": "Set channels, rate, length, mode and threshold; rejected with a reason when impossible.",
    "safety": "capture",
    "lease": true,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Hardware actions cannot be undone."
    }
  },
  {
    "name": "device.connect",
    "summary": "Connect a device and take its lease; returns its capabilities.",
    "safety": "capture",
    "lease": true,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Hardware actions cannot be undone."
    }
  },
  {
    "name": "device.disconnect",
    "summary": "Disconnect a device and release its lease.",
    "safety": "capture",
    "lease": true,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Hardware actions cannot be undone."
    }
  },
  {
    "name": "device.list",
    "summary": "Connected and available devices with capabilities, lease holders and setup hints.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "file.open",
    "summary": "Open a capture file (.lacap, .demo, .sr, .vcd) from a path on this machine; undo closes it.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "file.save",
    "summary": "Save a capture as .lacap, or export it as .sr or VCD.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Writing a file cannot be undone."
    }
  },
  {
    "name": "history.list",
    "summary": "Recent history entries with actors, newest first.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "history.redo",
    "summary": "Redo the actor's own last undone action, or any entry by id.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Redo is undone with history.undo."
    }
  },
  {
    "name": "history.undo",
    "summary": "Undo the actor's own last action, or any entry by id.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Undo is undone with history.redo."
    }
  },
  {
    "name": "lease.release",
    "summary": "Release a device's lease.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "lease.take",
    "summary": "Take a device's lease (the person can always take it from an agent).",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "marker.add",
    "summary": "Add a marker or range note (with an agent badge when an agent adds it); optionally move the view there.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "marker.remove",
    "summary": "Remove a marker; undo restores it with the same id.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "marker.update",
    "summary": "Change a marker's label, note or place.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "measure.between",
    "summary": "Time between two edges.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "measure.edges",
    "summary": "Edges of chosen channels in a range, paged.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "measure.stats",
    "summary": "Frequency, period, duty, pulse widths and edge count of a channel in a range.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "protocol.addCustom",
    "summary": "Add a custom JSON protocol (pulse-code or preset) for this session.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Custom protocols are removed with protocol.removeCustom."
    }
  },
  {
    "name": "protocol.info",
    "summary": "What a protocol is, how to confirm it, decoder tips, lookalikes and symptoms.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "protocol.list",
    "summary": "Every protocol the engine knows, with its roles and settings.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "protocol.removeCustom",
    "summary": "Remove a custom protocol.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "Add it again with protocol.addCustom."
    }
  },
  {
    "name": "render",
    "summary": "A PNG image of a time range with decoder rows and markers, for agents that can see images.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "search.run",
    "summary": "Search edges and decoded content with a typed query; hits with times.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "session.get",
    "summary": "Open captures and their state, decoders with problem counts, markers, views, leases, recent activity.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  },
  {
    "name": "session.pauseAgents",
    "summary": "The person's switch to stop agents' capture and output commands, and to let them run again.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "never",
      "reason": "A safety switch, not an edit; switch it back instead."
    }
  },
  {
    "name": "view.set",
    "summary": "Move a tab's view: visible range, cursors A and B, selection.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "inverse"
    }
  },
  {
    "name": "waveform.get",
    "summary": "What to draw for a range: per channel the edges, or level-of-detail buckets when zoomed out, and decoder frames.",
    "safety": "read",
    "lease": false,
    "longRunning": false,
    "undo": {
      "kind": "query"
    }
  }
];
