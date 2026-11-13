//! The text half of every tool result (doc 07): the answer first, units spelled out, times in
//! seconds with their sample index for exact follow-up, bounded length. The structured half is
//! the command's output as it is.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

use logica_protocols::format;

/// Longest text a tool returns (about 2,000 tokens); the structured content and paging carry more.
pub const MAX_TEXT: usize = 8000;

/// What the texts need to know about each capture: rate and channel names.
#[derive(Default)]
pub struct Captures(BTreeMap<String, (f64, BTreeMap<u64, String>)>);

impl Captures {
    /// From the output of `session.get`.
    pub fn from_session(session: &Value) -> Self {
        let mut m = BTreeMap::new();
        for c in arr(&session["captures"]) {
            let names = arr(&c["channels"])
                .iter()
                .filter_map(|ch| Some((ch["index"].as_u64()?, s(&ch["name"]).to_string())))
                .collect();
            m.insert(s(&c["id"]).to_string(), (c["sampleRateHz"].as_f64().unwrap_or(0.0), names));
        }
        Captures(m)
    }

    fn rate(&self, capture: &str) -> f64 {
        self.0.get(capture).map_or(0.0, |c| c.0)
    }

    /// "1.25 ms (sample 12,500)"
    pub fn at(&self, capture: &str, sample: u64) -> String {
        let rate = self.rate(capture);
        if sample == 0 {
            "0 s (sample 0)".into()
        } else if rate > 0.0 {
            format!("{} (sample {})", format::time(sample as f64 / rate), num(sample))
        } else {
            format!("sample {}", num(sample))
        }
    }

    /// "CH4 (SDA)"; the name only when it says more than the number.
    pub fn channel(&self, capture: &str, ch: u64) -> String {
        let plain = format!("CH{ch}");
        match self.0.get(capture).and_then(|c| c.1.get(&ch)) {
            Some(n) if !n.is_empty() && *n != plain && *n != format!("channel {ch}") => {
                format!("{plain} ({n})")
            }
            _ => plain,
        }
    }
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn num(n: u64) -> String {
    format::int(n as f64)
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", num(n), if n == 1 { one } else { many })
}

/// Cuts a text at `MAX_TEXT`, at a line end, and says so.
pub fn bounded(mut t: String) -> String {
    if t.len() <= MAX_TEXT {
        return t;
    }
    let mut cut = MAX_TEXT;
    while !t.is_char_boundary(cut) {
        cut -= 1;
    }
    let cut = t[..cut].rfind('\n').unwrap_or(cut);
    t.truncate(cut);
    t.push_str("\n… (shortened; the structured content and the paging fields have the rest)");
    t
}

/// One line about a capture summary (as in `session.get` and `file.open`).
pub fn capture_line(c: &Value) -> String {
    let samples = c["sampleCount"].as_u64().unwrap_or(0);
    let rate = c["sampleRateHz"].as_f64().unwrap_or(0.0);
    let channels: Vec<String> = arr(&c["channels"])
        .iter()
        .map(|ch| {
            let i = ch["index"].as_u64().unwrap_or(0);
            let n = s(&ch["name"]);
            if n.is_empty() || n == format!("CH{i}") || n == format!("channel {i}") {
                format!("CH{i}")
            } else {
                format!("CH{i} ({n})")
            }
        })
        .collect();
    format!(
        "{} \"{}\": {} at {} ({}), channels {}",
        s(&c["id"]),
        s(&c["title"]),
        plural(samples, "sample", "samples"),
        format::hz(rate),
        format::time(c["seconds"].as_f64().unwrap_or(0.0)),
        channels.join(", ")
    )
}

fn problems_text(counts: &Value) -> String {
    let Some(m) = counts.as_object() else { return "no problems".into() };
    let total: u64 = m.values().filter_map(Value::as_u64).sum();
    if total == 0 {
        return "no problems".into();
    }
    let parts: Vec<String> =
        m.iter().filter_map(|(k, v)| Some(format!("{} {k}", num(v.as_u64()?)))).collect();
    format!("{} ({})", plural(total, "problem", "problems"), parts.join(", "))
}

fn roles_text(caps: &Captures, capture: &str, roles: &Value) -> String {
    let parts: Vec<String> = roles
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(role, ch)| Some(format!("{role} on {}", caps.channel(capture, ch.as_u64()?))))
                .collect()
        })
        .unwrap_or_default();
    parts.join(", ")
}

/// One line about a decoder summary.
pub fn decoder_line(caps: &Captures, capture: &str, d: &Value) -> String {
    format!(
        "{} {} ({}): {}, {}, {}",
        s(&d["id"]),
        s(&d["config"]["name"]),
        roles_text(caps, capture, &d["config"]["roles"]),
        plural(d["packets"].as_u64().unwrap_or(0), "packet", "packets"),
        plural(d["frames"].as_u64().unwrap_or(0), "frame", "frames"),
        problems_text(&d["problemCounts"])
    )
}

pub fn session(out: &Value) -> String {
    let caps = Captures::from_session(out);
    let mut t = String::new();
    let captures = arr(&out["captures"]);
    match captures.len() {
        0 => t.push_str("No captures are open. Open a file with open_file or capture from a device.\n"),
        n => {
            let front = out["active"].as_str().map(|a| format!(" (in front: {a})")).unwrap_or_default();
            let _ = writeln!(t, "{}{front}:", plural(n as u64, "capture is open", "captures are open"));
        }
    }
    for c in captures {
        let id = s(&c["id"]);
        let _ = writeln!(t, "- {}", capture_line(c));
        for d in arr(&c["decoders"]) {
            let _ = writeln!(t, "  decoder {}", decoder_line(&caps, id, d));
        }
        for m in arr(&c["markers"]) {
            let _ = writeln!(
                t,
                "  marker {} \"{}\" at {} by {}",
                s(&m["id"]),
                s(&m["label"]),
                caps.at(id, m["at"].as_u64().unwrap_or(0)),
                s(&m["by"])
            );
        }
        let v = &c["view"];
        if let (Some(from), Some(to)) = (v["from"].as_u64(), v["to"].as_u64())
            && to > from
        {
            let _ = write!(t, "  the person sees {} to {}", caps.at(id, from), caps.at(id, to));
            for (k, label) in [("cursorA", "cursor A"), ("cursorB", "cursor B")] {
                if let Some(x) = v[k].as_u64() {
                    let _ = write!(t, "; {label} at {}", caps.at(id, x));
                }
            }
            if let Some(sel) = v["selection"].as_array()
                && let (Some(a), Some(b)) =
                    (sel.first().and_then(Value::as_u64), sel.get(1).and_then(Value::as_u64))
            {
                let _ = write!(t, "; selected {} to {}", caps.at(id, a), caps.at(id, b));
            }
            t.push('\n');
        }
        for n in arr(&c["notes"]) {
            let _ = writeln!(t, "  {}: {}", s(&n["level"]), s(&n["message"]));
        }
    }
    // only the devices in use; list_devices has the rest
    let devices = arr(&out["devices"]);
    let in_use: Vec<&Value> =
        devices.iter().filter(|d| d["connected"].as_bool() == Some(true) || !d["holder"].is_null()).collect();
    if !in_use.is_empty() {
        t.push_str("Devices in use:\n");
        for d in &in_use {
            t.push_str(&device_line(d));
        }
    }
    let others = devices.len() - in_use.len();
    if others > 0 {
        let _ = writeln!(
            t,
            "{} available; see list_devices.",
            plural(others as u64, "other device is", "other devices are")
        );
    }
    for r in arr(&out["running"]) {
        let _ = writeln!(t, "Capturing: {} into {}", s(&r["device"]), s(&r["capture"]));
    }
    let recent = arr(&out["recent"]);
    if !recent.is_empty() {
        t.push_str("Recent activity (newest first):\n");
        for h in recent.iter().take(8) {
            let undone = if h["undone"].as_bool() == Some(true) { " (undone)" } else { "" };
            let _ = writeln!(t, "- #{} {} {}{undone}", h["id"], s(&h["actor"]), s(&h["summary"]));
        }
    }
    bounded(t)
}

fn device_line(d: &Value) -> String {
    let info = &d["info"];
    let state = match (d["capturing"].as_bool(), d["connected"].as_bool()) {
        (Some(true), _) => "capturing",
        (_, Some(true)) => "connected",
        _ => "not connected",
    };
    let holder = d["holder"].as_str().map(|h| format!(", lease held by {h}")).unwrap_or_default();
    let caps = &d["capabilities"];
    let rates: Vec<String> = arr(&caps["ratesHz"]).iter().filter_map(Value::as_f64).map(format::hz).collect();
    let rates = match rates.as_slice() {
        [] => String::new(),
        [one] => format!(", {one}"),
        [first, .., last] => format!(", {first} to {last}"),
    };
    format!(
        "- {} ({}, {}): {state}{holder}; {}{rates}\n",
        s(&info["id"]),
        s(&info["model"]),
        s(&info["driver"]),
        plural(info["channels"].as_u64().unwrap_or(0), "channel", "channels")
    )
}

pub fn devices(out: &Value) -> String {
    let list = arr(&out["devices"]);
    if list.is_empty() {
        return "No devices found.".into();
    }
    let mut t = format!("{}:\n", plural(list.len() as u64, "device", "devices"));
    for d in list {
        t.push_str(&device_line(d));
    }
    bounded(t)
}

/// `analysis.detect`, led by what was found.
pub fn summary(caps: &Captures, capture: &str, out: &Value) -> String {
    let mut t = String::new();
    let mut found = 0;
    for bus in arr(&out["buses"]) {
        for d in arr(&bus["chosen"]) {
            found += 1;
            let settings: Vec<String> = d["settings"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| format!("{k} {}", v.to_string().trim_matches('"'))).collect())
                .unwrap_or_default();
            let _ = writeln!(
                t,
                "{}: {}; {}; confidence {}.",
                s(&d["protocolId"]).to_uppercase(),
                roles_text(caps, capture, &d["roles"]),
                if settings.is_empty() { "no settings".into() } else { settings.join(", ") },
                format::pct(d["confidence"].as_f64().unwrap_or(0.0), 0)
            );
            for e in arr(&d["evidence"]).iter().take(4) {
                let sign = if e["supports"].as_bool() == Some(false) { "against" } else { "for" };
                let _ = writeln!(t, "  {sign}: {}", s(&e["text"]));
            }
            for w in arr(&d["warnings"]) {
                let _ = writeln!(t, "  warning: {}", s(w));
            }
        }
        let alts: Vec<String> = arr(&bus["alternatives"])
            .iter()
            .map(|a| {
                format!(
                    "{} ({})",
                    s(&a["protocolId"]),
                    format::pct(a["confidence"].as_f64().unwrap_or(0.0), 0)
                )
            })
            .collect();
        if !alts.is_empty() {
            let _ = writeln!(t, "  also possible: {}", alts.join(", "));
        }
        let unexplained: Vec<String> = arr(&bus["unexplained"])
            .iter()
            .filter_map(Value::as_u64)
            .map(|c| caps.channel(capture, c))
            .collect();
        if !unexplained.is_empty() {
            let _ = writeln!(t, "  not explained: {} ({})", unexplained.join(", "), s(&bus["bus"]["reason"]));
        }
    }
    if found == 0 {
        t.insert_str(0, "No protocol was recognised.\n");
    }
    let rate = caps.rate(capture);
    t.push_str("Channels:\n");
    for m in arr(&out["measurements"]) {
        let ch = m["channel"].as_u64().unwrap_or(0);
        let edges = m["edgeCount"].as_u64().unwrap_or(0);
        let _ = write!(
            t,
            "- {}: {}, rests {}",
            caps.channel(capture, ch),
            plural(edges, "edge", "edges"),
            s(&m["restLevel"])
        );
        if let Some(p) = m["periodic"]["periodSamples"].as_f64()
            && p > 0.0
            && rate > 0.0
        {
            let _ = write!(t, ", clock-like at {}", format::hz(rate / p));
        }
        if let Some(n) = m["narrowest"]["samples"].as_f64()
            && rate > 0.0
        {
            let _ = write!(t, ", narrowest pulse {}", format::time(n / rate));
        }
        let glitches = m["glitches"].as_u64().unwrap_or(0);
        if glitches > 0 {
            let _ = write!(t, ", {}", plural(glitches, "glitch", "glitches"));
        }
        t.push('\n');
    }
    for n in arr(&out["notes"]) {
        let _ = writeln!(t, "Note: {}", s(n));
    }
    if let Some(w) = out["window"].as_array()
        && let (Some(a), Some(b)) = (w.first().and_then(Value::as_u64), w.get(1).and_then(Value::as_u64))
    {
        let _ = writeln!(t, "Analysed {} to {}.", caps.at(capture, a), caps.at(capture, b));
    }
    bounded(t)
}

pub fn decoder(caps: &Captures, capture: &str, action: &str, out: &Value) -> String {
    match action {
        "remove" => "The decoder was removed (undo with run_command history.undo).".into(),
        _ => {
            let verb = if action == "add" { "Added" } else { "Updated" };
            let mut t = format!("{verb} decoder {}.\n", decoder_line(caps, capture, &out["decoder"]));
            for c in arr(&out["problemChanges"]) {
                let _ = writeln!(t, "{}: {} before, {} now", s(&c["code"]), c["before"], c["after"]);
            }
            for n in arr(&out["decoder"]["notes"]) {
                let _ = writeln!(t, "Note: {}", s(n));
            }
            bounded(t)
        }
    }
}

pub fn decoded(caps: &Captures, capture: &str, out: &Value) -> String {
    let mut t = String::new();
    let packets = arr(&out["packets"]);
    let frames = arr(&out["frames"]);
    if let Some(ex) = out["explanation"].as_object() {
        let groups = arr(&ex["problems"]);
        if groups.is_empty() {
            t.push_str("No problems in this decode; nothing to explain.\n");
        }
        for g in groups {
            let first = g["first"]
                .as_u64()
                .map(|f| format!(", first at {}", caps.at(capture, f)))
                .unwrap_or_default();
            let msgs: Vec<&str> = arr(&g["messages"]).iter().map(s).collect();
            let _ = writeln!(t, "{} × {}{first}: {}", g["count"], s(&g["code"]), msgs.join("; "));
        }
        if !groups.is_empty() {
            t.push_str("Symptoms to compare with (symptom → cause; what to do):\n");
            for c in arr(&ex["causes"]) {
                let _ = writeln!(t, "- {} → {}; {}", s(&c["symptom"]), s(&c["cause"]), s(&c["action"]));
            }
        }
    }
    for p in packets {
        let start = p["packet"]["start"].as_u64().unwrap_or(0);
        let _ = writeln!(t, "#{} at {}: {}", p["packet"]["index"], caps.at(capture, start), s(&p["text"]));
    }
    if packets.is_empty() {
        for f in frames {
            let problems: Vec<&str> = arr(&f["problems"]).iter().map(|p| s(&p["message"])).collect();
            let problems =
                if problems.is_empty() { String::new() } else { format!(" — {}", problems.join("; ")) };
            let _ = writeln!(
                t,
                "{} at {}: {} {}{problems}",
                s(&f["kind"]),
                caps.at(capture, f["start"].as_u64().unwrap_or(0)),
                s(&f["label"]),
                f["detail"].as_str().unwrap_or("")
            );
        }
    }
    let shown = if packets.is_empty() { frames.len() } else { packets.len() };
    let total = out["total"].as_u64().unwrap_or(0);
    let what = if packets.is_empty() { "frames" } else { "packets" };
    let more = match out["nextCursor"].as_u64() {
        Some(c) => format!("; more with cursor {c}"),
        None => String::new(),
    };
    let _ = writeln!(t, "Shown {shown} of {} {what}{more}.", num(total));
    bounded(t)
}

pub fn search(caps: &Captures, capture: &str, out: &Value) -> String {
    let hits = arr(&out["hits"]);
    let mut t = format!(
        "{}{}.\n",
        plural(hits.len() as u64, "hit", "hits"),
        if out["truncated"].as_bool() == Some(true) {
            " (more exist; raise limit or narrow the query)"
        } else {
            ""
        }
    );
    for h in hits.iter().take(60) {
        let _ = writeln!(t, "- {}: {}", caps.at(capture, h["start"].as_u64().unwrap_or(0)), s(&h["what"]));
    }
    bounded(t)
}

pub fn measure(caps: &Captures, capture: &str, action: &str, out: &Value) -> String {
    match action {
        "stats" => {
            let ch = caps.channel(capture, out["channel"].as_u64().unwrap_or(0));
            let mut parts = vec![
                plural(out["edges"].as_u64().unwrap_or(0), "edge", "edges"),
                format!("rests {}", s(&out["restLevel"])),
            ];
            if let Some(f) = out["frequencyHz"].as_f64() {
                parts.push(format!("frequency {}", format::hz(f)));
            }
            if let Some(p) = out["periodSeconds"].as_f64() {
                parts.push(format!("period {}", format::time(p)));
            }
            if let Some(d) = out["dutyCycle"].as_f64() {
                parts.push(format!("duty cycle {}", format::pct(d, 1)));
            }
            if let Some(n) = out["narrowestPulseSeconds"].as_f64() {
                parts.push(format!("narrowest pulse {}", format::time(n)));
            }
            let g = out["glitches"].as_u64().unwrap_or(0);
            if g > 0 {
                parts.push(plural(g, "glitch", "glitches"));
            }
            let mut t = format!(
                "{ch} from {} to {}: {}.",
                caps.at(capture, out["from"].as_u64().unwrap_or(0)),
                caps.at(capture, out["to"].as_u64().unwrap_or(0)),
                parts.join(", ")
            );
            if let Some(n) = out["note"].as_str() {
                let _ = write!(t, " {n}");
            }
            t
        }
        "edges" => {
            let edges = arr(&out["edges"]);
            let mut t = format!("{}:\n", plural(edges.len() as u64, "edge", "edges"));
            for e in edges.iter().take(100) {
                let _ = writeln!(
                    t,
                    "- {} {} to {}",
                    caps.channel(capture, e["channel"].as_u64().unwrap_or(0)),
                    caps.at(capture, e["sample"].as_u64().unwrap_or(0)),
                    s(&e["level"])
                );
            }
            if let Some(c) = out["nextCursor"].as_u64() {
                let _ = writeln!(t, "More with cursor {c}.");
            }
            bounded(t)
        }
        _ => {
            let f = out["frequencyHz"].as_f64().map(|f| format!(" ({})", format::hz(f))).unwrap_or_default();
            format!(
                "{} apart, {}{f}.",
                format::time(out["seconds"].as_f64().unwrap_or(0.0)),
                plural(out["samples"].as_u64().unwrap_or(0), "sample", "samples")
            )
        }
    }
}

pub fn marker(caps: &Captures, capture: &str, out: &Value) -> String {
    let m = &out["marker"];
    let at = caps.at(capture, m["at"].as_u64().unwrap_or(0));
    let range = m["end"].as_u64().map(|e| format!(" to {}", caps.at(capture, e))).unwrap_or_default();
    format!(
        "The person now sees marker {} \"{}\" at {at}{range}, with an agent badge.",
        s(&m["id"]),
        s(&m["label"])
    )
}

/// For results without a dedicated text: what ran and the output, compactly.
pub fn generic(command: &str, out: &Value) -> String {
    let body = out.to_string();
    let body = if body == "{}" || body == "null" { String::new() } else { format!(" {body}") };
    bounded(format!("{command} done.{body}"))
}
