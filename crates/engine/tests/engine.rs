//! The engine through its command API, as every client uses it (doc 10, "Engine and API"):
//! commands in-process, undo and redo of every mutating command, leases, events.

// the base64 helper slices bits on purpose
#![allow(clippy::cast_possible_truncation)]

use std::path::PathBuf;

use serde_json::{Value, json};

use logica_engine::{Actor, Engine, EngineConfig, ErrorCode, Event, Invocation};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn agent() -> Actor {
    Actor::Agent("Claude Code".into())
}

fn run(e: &mut Engine, a: &Actor, command: &str, input: Value) -> Value {
    e.execute(a, &Invocation { command: command.into(), input })
        .unwrap_or_else(|x| panic!("{command}: {}", x.message))
}

fn fail(e: &mut Engine, a: &Actor, command: &str, input: Value) -> (ErrorCode, String) {
    let x = e.execute(a, &Invocation { command: command.into(), input }).expect_err(command);
    (x.code, x.message)
}

fn open(e: &mut Engine, file: &str) -> String {
    let path = fixtures().join("captures").join(file);
    let o = run(e, &Actor::User, "file.open", json!({ "path": path }));
    o["capture"]["id"].as_str().unwrap().to_string()
}

fn session(e: &mut Engine) -> Value {
    run(e, &Actor::User, "session.get", json!({}))
}

fn capture<'a>(s: &'a Value, id: &str) -> &'a Value {
    s["captures"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap()
}

#[test]
fn an_agent_finds_reads_and_marks_mystery_c() {
    // the M1 exit scenario, through the command API
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "mystery_C.demo");
    let a = run(&mut e, &agent(), "analysis.detect", json!({ "capture": c }));
    let d = a["buses"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|b| b["chosen"].as_array().unwrap())
        .find(|d| d["protocolId"] == "i2c")
        .unwrap();
    assert_eq!(d["roles"], json!({ "scl": 5, "sda": 4 }));
    let text: Vec<&str> =
        d["evidence"].as_array().unwrap().iter().map(|x| x["text"].as_str().unwrap()).collect();
    assert!(text.iter().any(|t| t.contains("400 kHz")));
    let dec = run(
        &mut e,
        &agent(),
        "decoder.add",
        json!({ "capture": c, "protocol": "i2c", "roles": d["roles"], "settings": d["settings"] }),
    );
    let id = dec["decoder"]["id"].as_str().unwrap().to_string();
    assert_eq!(dec["decoder"]["packets"], 3);
    let p = run(
        &mut e,
        &agent(),
        "decoder.frames",
        json!({ "capture": c, "decoder": id, "packets": true, "text": "CALIBRATED" }),
    );
    assert_eq!(p["total"], 1);
    assert!(p["packets"][0]["text"].as_str().unwrap().contains("CALIBRATED 2026"));
    let at = p["packets"][0]["packet"]["start"].as_u64().unwrap();
    let m = run(
        &mut e,
        &agent(),
        "marker.add",
        json!({ "capture": c, "at": at, "label": "EEPROM read", "moveView": true }),
    );
    assert_eq!(m["marker"]["by"], "agent:Claude Code");
    // the agent changes a setting, then undoes it
    let u = run(
        &mut e,
        &agent(),
        "decoder.update",
        json!({ "capture": c, "decoder": id, "settings": { "addressFormat": "8bit" } }),
    );
    assert_eq!(u["decoder"]["config"]["settings"]["addressFormat"], "8bit");
    run(&mut e, &agent(), "history.undo", json!({}));
    let s = session(&mut e);
    assert_eq!(capture(&s, &c)["decoders"][0]["config"]["settings"]["addressFormat"], "7bit");
    assert_eq!(
        capture(&s, &c)["markers"][0]["label"],
        "EEPROM read",
        "the marker stays: only the last action was undone"
    );
}

#[test]
fn every_mutating_command_is_undone_and_redone_exactly() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "3_I2C_bus.demo");
    let state = |e: &mut Engine| {
        let mut s = session(e);
        // revision and history move on; the capture itself must come back exactly
        capture(&s, &c)
            .clone()
            .as_object_mut()
            .map(std::mem::take)
            .map(Value::Object)
            .unwrap_or_else(|| s.take())
    };
    let steps: Vec<(&str, Value)> = vec![
        ("capture.rename", json!({ "capture": c, "title": "Bus test" })),
        ("channel.rename", json!({ "capture": c, "channel": 1, "name": "CLOCK" })),
        ("channel.colour", json!({ "capture": c, "channel": 2, "colour": "#123456" })),
        ("channel.group", json!({ "capture": c, "group": "I2C", "channels": [1, 2] })),
        ("channel.hide", json!({ "capture": c, "channel": 2, "hidden": true })),
        ("decoder.add", json!({ "capture": c, "protocol": "i2c", "roles": { "scl": 1, "sda": 2 } })),
        ("marker.add", json!({ "capture": c, "at": 1000, "label": "here" })),
        ("view.set", json!({ "capture": c, "from": 100, "to": 5000, "cursorA": 200 })),
    ];
    for (cmd, input) in steps {
        let before = state(&mut e);
        run(&mut e, &agent(), cmd, input.clone());
        let after = state(&mut e);
        assert_ne!(before, after, "{cmd} changed nothing");
        run(&mut e, &agent(), "history.undo", json!({}));
        assert_eq!(state(&mut e), before, "undo of {cmd}");
        run(&mut e, &agent(), "history.redo", json!({}));
        assert_eq!(state(&mut e), after, "redo of {cmd}");
    }
    // update and remove of the decoder and the marker, with the ids they had
    let s = session(&mut e);
    let dec = capture(&s, &c)["decoders"][0]["id"].as_str().unwrap().to_string();
    let mk = capture(&s, &c)["markers"][0]["id"].as_str().unwrap().to_string();
    for (cmd, input) in [
        ("decoder.update", json!({ "capture": c, "decoder": dec, "settings": { "addressFormat": "8bit" } })),
        ("marker.update", json!({ "capture": c, "marker": mk, "label": "there", "note": "a note" })),
        ("decoder.remove", json!({ "capture": c, "decoder": dec })),
        ("marker.remove", json!({ "capture": c, "marker": mk })),
    ] {
        let before = state(&mut e);
        run(&mut e, &agent(), cmd, input);
        let after = state(&mut e);
        run(&mut e, &agent(), "history.undo", json!({}));
        assert_eq!(state(&mut e), before, "undo of {cmd}");
        run(&mut e, &agent(), "history.redo", json!({}));
        assert_eq!(state(&mut e), after, "redo of {cmd}");
    }
}

#[test]
fn undo_works_per_actor_and_new_actions_discard_redo() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "1_UART_easy.demo");
    run(&mut e, &Actor::User, "channel.rename", json!({ "capture": c, "channel": 0, "name": "MINE" }));
    run(&mut e, &agent(), "channel.colour", json!({ "capture": c, "channel": 0, "colour": "#00ff00" }));
    // the person's undo takes their own last action, not the agent's
    run(&mut e, &Actor::User, "history.undo", json!({}));
    let s = session(&mut e);
    assert_eq!(capture(&s, &c)["channels"][0]["name"], "RX");
    assert_eq!(capture(&s, &c)["channels"][0]["colour"], "#00ff00");
    // the person can undo an agent's entry by id
    let entry =
        s["recent"].as_array().unwrap().iter().find(|h| h["actor"] == "agent:Claude Code").unwrap()["id"]
            .clone();
    run(&mut e, &Actor::User, "history.undo", json!({ "entry": entry }));
    assert_ne!(capture(&session(&mut e), &c)["channels"][0]["colour"], "#00ff00");
    // a new action by the person discards their redo
    run(&mut e, &Actor::User, "channel.hide", json!({ "capture": c, "channel": 0, "hidden": true }));
    let (code, msg) = fail(&mut e, &Actor::User, "history.redo", json!({}));
    assert_eq!(code, ErrorCode::NotFound, "{msg}");
    // the person's own view changes are not undo history
    run(&mut e, &Actor::User, "view.set", json!({ "capture": c, "from": 10, "to": 100 }));
    let last = session(&mut e)["recent"][0]["done"]["command"].clone();
    assert_eq!(last, "channel.hide");
    // saving is recorded but cannot be undone
    let dir = std::env::temp_dir().join(format!("logica-engine-undo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.lacap");
    run(&mut e, &Actor::User, "file.save", json!({ "capture": c, "path": path }));
    let entry = session(&mut e)["recent"][0]["id"].clone();
    let (code, msg) = fail(&mut e, &Actor::User, "history.undo", json!({ "entry": entry }));
    assert_eq!(code, ErrorCode::NotUndoable);
    assert!(msg.contains("Writing a file cannot be undone"), "{msg}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn opening_can_be_undone_and_redone_with_the_same_id() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "mystery_A.demo");
    run(&mut e, &Actor::User, "history.undo", json!({}));
    assert!(session(&mut e)["captures"].as_array().unwrap().is_empty());
    run(&mut e, &Actor::User, "history.redo", json!({}));
    assert_eq!(session(&mut e)["captures"][0]["id"], c);
}

#[test]
fn saving_and_reopening_keeps_decoders_markers_and_groups() {
    let dir = std::env::temp_dir().join(format!("logica-engine-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bus.lacap");
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "3_I2C_bus.demo");
    run(
        &mut e,
        &Actor::User,
        "decoder.add",
        json!({ "capture": c, "protocol": "i2c", "roles": { "scl": 1, "sda": 2 }, "id": "bus" }),
    );
    run(
        &mut e,
        &Actor::User,
        "marker.add",
        json!({ "capture": c, "at": 2_500_000, "label": "EEPROM write" }),
    );
    run(&mut e, &Actor::User, "channel.group", json!({ "capture": c, "group": "I2C", "channels": [1, 2] }));
    let saved = run(&mut e, &Actor::User, "file.save", json!({ "capture": c, "path": path }));
    assert!(saved["bytes"].as_u64().unwrap() > 0);
    let before = capture(&session(&mut e), &c).clone();

    let mut f = Engine::new(EngineConfig::default());
    let o = run(&mut f, &Actor::User, "file.open", json!({ "path": path }));
    let id = o["capture"]["id"].as_str().unwrap().to_string();
    let after = capture(&session(&mut f), &id).clone();
    assert_eq!(after["decoders"], before["decoders"]);
    assert_eq!(after["markers"], before["markers"]);
    assert_eq!(after["channels"], before["channels"]);
    assert_eq!(after["sampleCount"], before["sampleCount"]);
    // saving in place appends to the file it came from
    run(&mut f, &Actor::User, "marker.add", json!({ "capture": id, "at": 10, "label": "start" }));
    run(&mut f, &Actor::User, "file.save", json!({ "capture": id }));
    let mut g = Engine::new(EngineConfig::default());
    let o = run(&mut g, &Actor::User, "file.open", json!({ "path": path }));
    assert_eq!(o["capture"]["markers"].as_array().unwrap().len(), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_format_opens_and_unknown_ones_are_explained() {
    let mut e = Engine::new(EngineConfig::default());
    for f in ["formats/sr/mixed-8ch-1mhz.sr", "formats/vcd/features.vcd", "captures/mystery_D.demo"] {
        run(&mut e, &Actor::User, "file.open", json!({ "path": fixtures().join(f) }));
    }
    assert_eq!(session(&mut e)["captures"].as_array().unwrap().len(), 3);
    let (code, msg) =
        fail(&mut e, &Actor::User, "file.open", json!({ "path": fixtures().join("README.md") }));
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains("not a capture format"), "{msg}");
    let (code, _) = fail(&mut e, &Actor::User, "file.open", json!({ "path": "/no/such/file.demo" }));
    assert_eq!(code, ErrorCode::NotFound);
}

#[test]
fn inputs_are_checked_against_the_contract() {
    let mut e = Engine::new(EngineConfig::default());
    assert_eq!(session(&mut e)["apiVersion"], "1.0");
    let c = open(&mut e, "1_UART_easy.demo");
    let (code, msg) = fail(&mut e, &agent(), "decoder.add", json!({ "capture": c, "protocol": "uart" }));
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains("missing field `roles`"), "{msg}");
    let (_, msg) =
        fail(&mut e, &agent(), "marker.add", json!({ "capture": c, "at": 5, "label": "x", "colour": "red" }));
    assert!(msg.contains("unknown field `colour`"), "{msg}");
    let (code, msg) =
        fail(&mut e, &agent(), "decoder.add", json!({ "capture": c, "protocol": "uart", "roles": {} }));
    assert_eq!((code, msg.as_str()), (ErrorCode::InvalidInput, "Choose a channel for the data line."));
    let (code, msg) = fail(&mut e, &agent(), "nope.nope", json!({}));
    assert_eq!(code, ErrorCode::NotFound);
    assert!(msg.contains("logica://commands"));
    let (code, _) =
        fail(&mut e, &agent(), "marker.add", json!({ "capture": c, "at": 99_999_999_999u64, "label": "x" }));
    assert_eq!(code, ErrorCode::InvalidInput);
}

#[test]
fn leases_busy_errors_take_over_and_release() {
    let mut e = Engine::new(EngineConfig::default());
    let dev = json!({ "device": "synthetic/mix" });
    run(&mut e, &agent(), "device.connect", dev.clone());
    let (code, msg) = fail(
        &mut e,
        &Actor::User,
        "device.configure",
        json!({ "device": "synthetic/mix", "config": { "channels": [0], "sampleRateHz": 1e8, "samples": 1000, "mode": "stream" } }),
    );
    assert_eq!(code, ErrorCode::Busy);
    assert!(msg.contains("in use by agent Claude Code; take it over with lease.take"), "{msg}");
    let other = Actor::Agent("Other".into());
    let (code, msg) = fail(&mut e, &other, "lease.take", dev.clone());
    assert_eq!(code, ErrorCode::Busy);
    assert!(msg.contains("only the person can take it over"), "{msg}");
    // the person takes it over
    let rx = e.subscribe();
    run(&mut e, &Actor::User, "lease.take", dev.clone());
    assert!(rx.try_iter().any(|ev| matches!(ev, Event::LeaseChanged { holder: Some(Actor::User), .. })));
    let (code, msg) = fail(
        &mut e,
        &agent(),
        "device.configure",
        json!({ "device": "synthetic/mix", "config": { "channels": [0], "sampleRateHz": 1e8, "samples": 1000, "mode": "stream" } }),
    );
    assert_eq!(code, ErrorCode::Busy);
    assert!(msg.contains("in use by the person"), "{msg}");
    // leases end when the holding client goes away
    e.release_actor(&Actor::User);
    let s = session(&mut e);
    let d = s["devices"].as_array().unwrap().iter().find(|d| d["info"]["id"] == "synthetic/mix").unwrap();
    assert!(d["holder"].is_null());
}

#[test]
fn a_synthetic_capture_appears_and_decodes() {
    let mut e = Engine::new(EngineConfig::default());
    let rx = e.subscribe();
    let dev = "synthetic/mix";
    run(&mut e, &agent(), "device.connect", json!({ "device": dev }));
    let (code, msg) = fail(
        &mut e,
        &agent(),
        "device.configure",
        json!({ "device": dev, "config": { "channels": [0, 1, 2], "sampleRateHz": 1e6, "samples": 1000, "mode": "stream" } }),
    );
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains("runs at 100 MHz"), "{msg}");
    run(
        &mut e,
        &agent(),
        "device.configure",
        json!({ "device": dev, "config": { "channels": [0, 1, 2, 6], "sampleRateHz": 1e8, "samples": 3_000_000, "mode": "stream" } }),
    );
    let o = run(&mut e, &agent(), "capture.start", json!({ "device": dev, "title": "mix" }));
    let c = o["id"].as_str().unwrap().to_string();
    assert_eq!(o["capture"]["sampleCount"], 3_000_000);
    assert!(rx.try_iter().any(|ev| matches!(ev, Event::CaptureDone { samples: 3_000_000, .. })));
    let a = run(&mut e, &agent(), "analysis.detect", json!({ "capture": c }));
    let found: Vec<String> = a["buses"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|b| b["chosen"].as_array().unwrap())
        .map(|d| d["protocolId"].as_str().unwrap().to_string())
        .collect();
    assert!(
        found.contains(&"uart".to_string())
            && found.contains(&"i2c".to_string())
            && found.contains(&"pwm".to_string()),
        "{found:?}"
    );
    // a capture without waiting, then stopped
    run(
        &mut e,
        &agent(),
        "device.configure",
        json!({ "device": dev, "config": { "channels": [0], "sampleRateHz": 1e8, "samples": 50_000_000_000u64, "mode": "stream" } }),
    );
    let o = run(&mut e, &agent(), "capture.start", json!({ "device": dev, "wait": false }));
    assert_eq!(o["running"], true);
    let (code, _) = fail(
        &mut e,
        &agent(),
        "device.configure",
        json!({ "device": dev, "config": { "channels": [0], "sampleRateHz": 1e8, "samples": 10, "mode": "stream" } }),
    );
    assert_eq!(code, ErrorCode::Busy);
    run(&mut e, &agent(), "capture.stop", json!({ "device": dev }));
    let s = session(&mut e);
    let c2 = capture(&s, o["id"].as_str().unwrap());
    assert_eq!(c2["state"], "aborted");
}

/// Decodes the standard base64 of `render` (test helper).
fn unbase64(s: &str) -> Vec<u8> {
    const A: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    for q in s.as_bytes().chunks(4) {
        let v: Vec<u32> = q.iter().filter_map(|&c| A.find(c as char).map(|i| i as u32)).collect();
        let n = v.iter().enumerate().fold(0u32, |n, (i, x)| n | x << (18 - 6 * i));
        for i in 0..v.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

#[test]
fn render_draws_the_view_or_a_range_as_png() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "mystery_C.demo");
    run(
        &mut e,
        &agent(),
        "decoder.add",
        json!({ "capture": c, "protocol": "i2c", "roles": { "scl": 5, "sda": 4 } }),
    );
    run(&mut e, &agent(), "marker.add", json!({ "capture": c, "at": 6_600_000, "label": "EEPROM read" }));
    // without a range: the person's view (here the whole capture)
    let o = run(&mut e, &agent(), "render", json!({ "capture": c }));
    assert_eq!(
        (o["from"].as_u64(), o["to"].as_u64(), o["width"].as_u64()),
        (Some(0), Some(10_000_008), Some(1200))
    );
    let png = unbase64(o["pngBase64"].as_str().unwrap());
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        u32::from_be_bytes([png[20], png[21], png[22], png[23]]),
        o["height"].as_u64().unwrap() as u32
    );
    let rows: Vec<&str> = o["rows"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
    assert!(rows[0].starts_with("time axis, samples 0 to 10000008"), "{rows:?}");
    assert!(rows.contains(&"channel CH4") && rows.contains(&"channel CH5"), "{rows:?}");
    assert!(
        rows.iter().any(|r| r.starts_with("decoder i2c")) && rows.iter().any(|r| r.contains("EEPROM read")),
        "{rows:?}"
    );

    // a range, and widths kept within limits
    run(&mut e, &Actor::User, "view.set", json!({ "capture": c, "from": 6_600_000, "to": 6_600_400 }));
    let o = run(&mut e, &agent(), "render", json!({ "capture": c, "width": 10 }));
    assert_eq!(
        (o["from"].as_u64(), o["to"].as_u64(), o["width"].as_u64()),
        (Some(6_600_000), Some(6_600_400), Some(320))
    );
    assert!(o["rows"][0].as_str().unwrap().contains("660.00 ms to 660.04 ms"), "{}", o["rows"][0]);
    let o = run(
        &mut e,
        &agent(),
        "render",
        json!({ "capture": c, "from": 0, "to": 99_000_000, "width": 100_000 }),
    );
    assert_eq!((o["to"].as_u64(), o["width"].as_u64()), (Some(10_000_008), Some(4000)));

    // a render is a query: nothing in the history
    let before = run(&mut e, &agent(), "history.list", json!({}))["entries"].as_array().unwrap().len();
    run(&mut e, &agent(), "render", json!({ "capture": c, "from": 5, "to": 6 }));
    assert_eq!(run(&mut e, &agent(), "history.list", json!({}))["entries"].as_array().unwrap().len(), before);

    let (code, msg) = fail(&mut e, &agent(), "render", json!({ "capture": c, "from": 500, "to": 500 }));
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains("The range is empty"), "{msg}");
    let (code, _) = fail(&mut e, &agent(), "render", json!({ "capture": "c99" }));
    assert_eq!(code, ErrorCode::NotFound);
}

#[test]
fn measurements_edges_search_and_bytes() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "mystery_D.demo");
    let st = run(&mut e, &agent(), "measure.stats", json!({ "capture": c, "channel": 0 }));
    assert!((st["frequencyHz"].as_f64().unwrap() - 1000.0).abs() < 1.0, "{st}");
    assert!((st["dutyCycle"].as_f64().unwrap() - 0.3).abs() < 1e-6);
    // paged edges never repeat or skip one
    let mut all = Vec::new();
    let mut cursor: Option<u64> = None;
    loop {
        let p = run(
            &mut e,
            &agent(),
            "measure.edges",
            json!({ "capture": c, "channels": [0, 4], "limit": 64, "cursor": cursor }),
        );
        all.extend(
            p["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| (x["channel"].as_u64().unwrap(), x["sample"].as_u64().unwrap())),
        );
        match p["nextCursor"].as_u64() {
            Some(n) => cursor = Some(n),
            None => break,
        }
    }
    assert_eq!(all.len(), 2000 + 152);
    // search: rising edges of the PWM while the UART line is low
    let d = run(
        &mut e,
        &agent(),
        "decoder.add",
        json!({ "capture": c, "protocol": "uart", "roles": { "data": 4 }, "id": "log" }),
    );
    assert_eq!(d["decoder"]["packets"], 2);
    let h = run(
        &mut e,
        &agent(),
        "search.run",
        json!({ "capture": c, "query": { "decoded": { "decoder": "log", "label": "7A" } } }),
    );
    assert!(h["hits"].as_array().unwrap().is_empty(), "no 'z' in the log");
    let h = run(
        &mut e,
        &agent(),
        "search.run",
        json!({ "capture": c, "query": { "all": [ { "decoded": { "decoder": "log", "text": "'m'" } }, { "within": { "from": 0.5, "to": 1.0 } } ] } }),
    );
    assert_eq!(h["hits"].as_array().unwrap().len(), 1, "the m of Z=1000mg: {h}");
    let h = run(
        &mut e,
        &agent(),
        "search.run",
        json!({ "capture": c, "query": { "all": [ { "edge": { "channel": 0, "direction": "rising" } }, { "level": { "channel": 4, "level": "low" } } ] } }),
    );
    assert!(!h["hits"].as_array().unwrap().is_empty(), "the PWM rises while a UART bit is low");
    let h = run(
        &mut e,
        &agent(),
        "search.run",
        json!({ "capture": c, "query": { "pulse": { "channel": 0, "level": "high", "shorterThan": 0.0002 } } }),
    );
    assert!(h["hits"].as_array().unwrap().is_empty(), "every high pulse is 300 µs");
    let (code, _) = fail(
        &mut e,
        &agent(),
        "search.run",
        json!({ "capture": c, "query": { "within": { "from": 0, "to": 1 } } }),
    );
    assert_eq!(code, ErrorCode::InvalidInput);
    let b = run(&mut e, &agent(), "analysis.bytes", json!({ "hex": "01 03 00 00 00 02 C4 0B" }));
    assert!(b["findings"].as_array().unwrap().iter().any(|f| f["id"] == "modbus"));
    let between = run(&mut e, &agent(), "measure.between", json!({ "capture": c, "from": 0, "to": 50_000 }));
    assert_eq!(between["seconds"], 0.001);
}

#[test]
fn decoded_output_is_paged_and_explained() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open(&mut e, "2_UART_mystery_baud.demo");
    run(
        &mut e,
        &agent(),
        "decoder.add",
        json!({ "capture": c, "protocol": "uart", "roles": { "data": 0 }, "settings": { "baud": 115200 }, "id": "wrong" }),
    );
    let p = run(
        &mut e,
        &agent(),
        "decoder.frames",
        json!({ "capture": c, "decoder": "wrong", "problems": true, "limit": 5, "explain": true }),
    );
    assert!(p["total"].as_u64().unwrap() > 5);
    assert_eq!(p["frames"].as_array().unwrap().len(), 5);
    assert_eq!(p["nextCursor"], 5);
    let ex = &p["explanation"];
    assert_eq!(ex["problems"][0]["code"], "frame-error");
    assert!(ex["causes"].as_array().unwrap().iter().any(|s| s["cause"] == "Wrong baud rate"));
    // fixing the setting reports how the problems changed
    let u = run(
        &mut e,
        &agent(),
        "decoder.update",
        json!({ "capture": c, "decoder": "wrong", "settings": { "baud": 57600 } }),
    );
    assert_eq!(u["problemChanges"][0]["code"], "frame-error");
    assert_eq!(u["problemChanges"][0]["after"], 0);
}

#[test]
fn protocols_knowledge_and_custom_ones() {
    let mut e = Engine::new(EngineConfig::default());
    let l = run(&mut e, &agent(), "protocol.list", json!({}));
    assert!(l["protocols"].as_array().unwrap().len() >= 14);
    let i = run(&mut e, &agent(), "protocol.info", json!({ "protocol": "i2c" }));
    assert!(i["info"]["confirm"].as_str().unwrap().contains("start"));
    let template: Value = serde_json::from_str(logica_protocols::custom::TEMPLATE_JSON).unwrap();
    let p = run(&mut e, &agent(), "protocol.addCustom", json!({ "config": template }));
    assert_eq!(p["origin"], "custom");
    let (code, _) = fail(&mut e, &agent(), "protocol.addCustom", json!({ "config": template }));
    assert_eq!(code, ErrorCode::InvalidInput);
    let (_, msg) =
        fail(&mut e, &agent(), "protocol.addCustom", json!({ "config": { "family": "pulse-code" } }));
    assert!(msg.contains("config.id must be a non-empty text"), "{msg}");
    run(&mut e, &agent(), "protocol.removeCustom", json!({ "protocol": "my-sensor" }));
}

#[test]
fn events_reach_every_subscriber() {
    let mut e = Engine::new(EngineConfig::default());
    let a = e.subscribe();
    let b = e.subscribe();
    let c = open(&mut e, "1_UART_easy.demo");
    run(&mut e, &agent(), "decoder.add", json!({ "capture": c, "protocol": "uart", "roles": { "data": 0 } }));
    for rx in [a, b] {
        let evs: Vec<Event> = rx.try_iter().collect();
        assert!(evs.iter().any(|x| matches!(x, Event::DecoderUpdated { .. })));
        assert!(evs.iter().any(|x| matches!(x, Event::StateChanged { .. })));
        assert!(evs.iter().any(|x| matches!(x, Event::CommandExecuted { actor: Actor::Agent(_), summary, .. } if summary.contains("UART decoder on CH0"))));
    }
}
