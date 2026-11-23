//! Every command of the engine (doc 03): name, safety class, lease, long-running, and how it is
//! undone. Served as the resource `logica://commands`. Inputs and outputs are defined next to each
//! command's implementation (after review).

use crate::contract::{CommandSpec, Safety, Undo};

const fn query(name: &'static str, summary: &'static str) -> CommandSpec {
    CommandSpec { name, summary, safety: Safety::Read, lease: false, long_running: false, undo: Undo::Query }
}

const fn edit(name: &'static str, summary: &'static str) -> CommandSpec {
    CommandSpec {
        name,
        summary,
        safety: Safety::Read,
        lease: false,
        long_running: false,
        undo: Undo::Inverse,
    }
}

const fn never(name: &'static str, summary: &'static str, reason: &'static str) -> CommandSpec {
    CommandSpec {
        name,
        summary,
        safety: Safety::Read,
        lease: false,
        long_running: false,
        undo: Undo::Never { reason },
    }
}

const fn hardware(
    name: &'static str,
    summary: &'static str,
    long_running: bool,
    reason: &'static str,
) -> CommandSpec {
    CommandSpec {
        name,
        summary,
        safety: Safety::Capture,
        lease: true,
        long_running,
        undo: Undo::Never { reason },
    }
}

pub const COMMANDS: &[CommandSpec] = &[
    // session
    query(
        "session.get",
        "Open captures and their state, decoders with problem counts, markers, views, leases, recent activity.",
    ),
    never(
        "session.pauseAgents",
        "The person's switch to stop agents' capture and output commands, and to let them run again.",
        "A safety switch, not an edit; switch it back instead.",
    ),
    query(
        "waveform.get",
        "What to draw for a range: per channel the edges, or level-of-detail buckets when zoomed out, and decoder frames.",
    ),
    // files
    edit(
        "file.open",
        "Open a capture file (.lacap, .demo, .sr, .vcd) from a path on this machine; undo closes it.",
    ),
    never(
        "file.save",
        "Save a capture as .lacap, or export it as .sr or VCD.",
        "Writing a file cannot be undone.",
    ),
    never(
        "capture.close",
        "Close a capture and free its memory.",
        "Closing drops the capture; open it again from its file.",
    ),
    edit("capture.rename", "Rename a capture."),
    // devices and capturing
    query("device.list", "Connected and available devices with capabilities, lease holders and setup hints."),
    hardware(
        "device.connect",
        "Connect a device and take its lease; returns its capabilities.",
        false,
        "Hardware actions cannot be undone.",
    ),
    hardware(
        "device.disconnect",
        "Disconnect a device and release its lease.",
        false,
        "Hardware actions cannot be undone.",
    ),
    hardware(
        "device.configure",
        "Set channels, rate, length, mode and threshold; rejected with a reason when impossible.",
        false,
        "Hardware actions cannot be undone.",
    ),
    hardware(
        "capture.start",
        "Start a capture with the device's configuration; the capture appears while it streams in.",
        true,
        "Captured data cannot be undone; close the capture instead.",
    ),
    hardware("capture.stop", "Stop a running capture.", false, "Hardware actions cannot be undone."),
    edit("lease.take", "Take a device's lease (the person can always take it from an agent)."),
    edit("lease.release", "Release a device's lease."),
    // channels
    edit("channel.rename", "Rename a channel."),
    edit("channel.colour", "Change a channel's colour."),
    edit("channel.group", "Put channels into a named bus group, or ungroup them."),
    edit("channel.hide", "Hide or show a channel."),
    // views
    edit("view.set", "Move a tab's view: visible range, cursors A and B, selection."),
    // analysis and measurement
    query(
        "analysis.detect",
        "Measure channels, group them into buses and detect protocols with evidence, for the whole capture or a range.",
    ),
    query(
        "analysis.bytes",
        "What a run of bytes might mean: text, hidden text, Modbus, checksums, BCD, 16-bit pairs.",
    ),
    query("measure.stats", "Frequency, period, duty, pulse widths and edge count of a channel in a range."),
    query("measure.edges", "Edges of chosen channels in a range, paged."),
    query("measure.between", "Time between two edges."),
    // decoders
    edit(
        "decoder.add",
        "Add a decoder (protocol, roles, settings), or one from a detection; undo removes it.",
    ),
    edit("decoder.update", "Change a decoder's roles or settings; returns how its problem counts changed."),
    edit("decoder.remove", "Remove a decoder; undo restores it with the same id."),
    query(
        "decoder.frames",
        "Decoded packets or frames by time range, kind, problem and text, paged; optionally explained.",
    ),
    // search and comparison
    query("search.run", "Search edges and decoded content with a typed query; hits with times."),
    // markers and notes
    edit(
        "marker.add",
        "Add a marker or range note (with an agent badge when an agent adds it); optionally move the view there.",
    ),
    edit("marker.update", "Change a marker's label, note or place."),
    edit("marker.remove", "Remove a marker; undo restores it with the same id."),
    // knowledge
    query("protocol.list", "Every protocol the engine knows, with its roles and settings."),
    query("protocol.info", "What a protocol is, how to confirm it, decoder tips, lookalikes and symptoms."),
    never(
        "protocol.addCustom",
        "Add a custom JSON protocol (pulse-code or preset) for this session.",
        "Custom protocols are removed with protocol.removeCustom.",
    ),
    never("protocol.removeCustom", "Remove a custom protocol.", "Add it again with protocol.addCustom."),
    // rendering
    query(
        "render",
        "A PNG image of a time range with decoder rows and markers, for agents that can see images.",
    ),
    // history
    query("history.list", "Recent history entries with actors, newest first."),
    never(
        "history.undo",
        "Undo the actor's own last action, or any entry by id.",
        "Undo is undone with history.redo.",
    ),
    never(
        "history.redo",
        "Redo the actor's own last undone action, or any entry by id.",
        "Redo is undone with history.undo.",
    ),
];

/// The spec of a command, by name.
pub fn spec(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_grouped_and_described() {
        let mut names = std::collections::BTreeSet::new();
        for c in COMMANDS {
            assert!(names.insert(c.name), "duplicate {}", c.name);
            assert!(c.summary.ends_with('.') && c.summary.len() > 15, "{}", c.name);
            assert_eq!(c.lease, c.safety != Safety::Read, "{}: hardware commands need the lease", c.name);
        }
        assert!(spec("history.undo").is_some() && spec("nope").is_none());
    }
}
