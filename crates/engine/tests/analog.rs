//! Virtual analog channels through the command API (ADR 0010): add, update, remove with undo and
//! redo, values, errors that say why, saving and reopening, and following a live capture.

// samples per millisecond from a sample rate are whole and small
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use logica_engine::{Actor, Engine, EngineConfig, ErrorCode, Event, Invocation};

fn agent() -> Actor {
    Actor::Agent("Claude Code".into())
}

fn run(e: &mut Engine, command: &str, input: Value) -> Value {
    e.execute(&agent(), &Invocation { command: command.into(), input })
        .unwrap_or_else(|x| panic!("{command}: {}", x.message))
}

fn fail(e: &mut Engine, command: &str, input: Value) -> (ErrorCode, String) {
    let x = e.execute(&agent(), &Invocation { command: command.into(), input }).expect_err(command);
    (x.code, x.message)
}

/// `mystery_D.demo` has a 1 kHz PWM at 30 % duty on channel 0.
fn open_mystery_d(e: &mut Engine) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/captures/mystery_D.demo");
    run(e, "file.open", json!({ "path": path }))["capture"]["id"].as_str().unwrap().to_string()
}

fn capture(e: &mut Engine, id: &str) -> Value {
    let s = run(e, "session.get", json!({}));
    s["captures"].as_array().unwrap().iter().find(|c| c["id"] == id).cloned().unwrap_or(Value::Null)
}

/// Adds a channel and returns its id.
fn add(e: &mut Engine, input: Value) -> String {
    run(e, "analog.add", input)["analog"]["id"].as_str().unwrap().to_string()
}

fn duty(c: &str) -> Value {
    json!({ "capture": c, "source": "pwm", "roles": { "signal": 0 },
            "scaling": { "kind": "range", "low": 0, "high": 100 } })
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn sources_are_listed_with_their_channels_and_settings() {
    let mut e = Engine::new(EngineConfig::default());
    let s = run(&mut e, "analog.sources", json!({}));
    let pwm = s["sources"].as_array().unwrap().iter().find(|s| s["id"] == "pwm").unwrap();
    assert_eq!(pwm["roles"][0]["key"], "signal");
    let keys: Vec<_> =
        pwm["settings"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["measure", "activeLevel", "expectedPeriod"]);
}

#[test]
fn a_pwm_duty_channel_reads_the_duty_in_per_cent() {
    let mut e = Engine::new(EngineConfig::default());
    let rx = e.subscribe();
    let c = open_mystery_d(&mut e);
    let o = run(&mut e, "analog.add", duty(&c));
    let a = &o["analog"];
    let id = a["id"].as_str().unwrap();
    assert!(id.starts_with("analog-"), "{id}");
    assert_eq!(a["config"]["name"], "PWM level");
    assert_eq!(a["config"]["unit"], "%");
    assert_eq!(a["config"]["style"], "step");
    assert_eq!(a["config"]["height"], 160);
    assert_eq!(a["output"]["kind"], "fraction");
    assert!(a["points"].as_u64().unwrap() > 100, "{a}");
    assert!(
        close(a["min"].as_f64().unwrap(), 30.0, 1.0) && close(a["max"].as_f64().unwrap(), 30.0, 1.0),
        "{a}"
    );
    assert!(rx.try_iter().any(|ev| matches!(ev, Event::AnalogUpdated { .. })));
    assert_eq!(capture(&mut e, &c)["analog"][0]["id"], id);

    // A few points over a short range, an envelope per bucket over the whole capture.
    let rate = capture(&mut e, &c)["sampleRateHz"].as_f64().unwrap();
    let ms = (rate / 1000.0) as u64;
    let first = run(
        &mut e,
        "analog.values",
        json!({ "capture": c, "analog": id, "from": 0, "to": 100 * ms, "buckets": 100 }),
    );
    let pts = first["points"].as_array().unwrap();
    assert!(pts.len() >= 90, "{}", pts.len());
    let p = pts.iter().find(|p| p["value"].is_number()).unwrap();
    assert!(close(p["value"].as_f64().unwrap(), 30.0, 1.0));
    assert!(p["code"].is_null(), "a duty is not a code");
    assert_eq!(first["unit"], "%");
    let n = capture(&mut e, &c)["sampleCount"].as_u64().unwrap();
    let all = run(
        &mut e,
        "analog.values",
        json!({ "capture": c, "analog": id, "from": 0, "to": n, "buckets": 10 }),
    );
    assert!(all["points"].is_null());
    let b = all["buckets"].as_array().unwrap();
    assert_eq!(b.len(), 10);
    assert_eq!(all["bucket"].as_u64().unwrap(), n.div_ceil(10));

    // The same signal's frequency, in Hz with a gain of 1.
    let f = run(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 0 },
        "settings": { "measure": "frequency" }, "scaling": { "kind": "linear", "gain": 1 } }),
    );
    assert_eq!(f["analog"]["config"]["unit"], "Hz");
    assert!(close(f["analog"]["min"].as_f64().unwrap(), 1000.0, 20.0), "{}", f["analog"]);
}

#[test]
fn add_update_and_remove_are_undone_and_redone_exactly() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    let state = |e: &mut Engine| capture(e, &c)["analog"].clone();
    let mut input = duty(&c);
    input["id"] = json!("fan");
    let steps = [
        ("analog.add", input),
        // Scaling only: nothing is computed again.
        (
            "analog.update",
            json!({ "capture": c, "analog": "fan", "scaling": { "kind": "range", "low": 0, "high": 5, "offset": 0.1 }, "unit": "V", "style": "line", "name": "Fan" }),
        ),
        // A different measurement: computed again, and the scaling must fit.
        (
            "analog.update",
            json!({ "capture": c, "analog": "fan", "settings": { "measure": "period" }, "scaling": { "kind": "linear", "gain": 1000 }, "unit": "ms" }),
        ),
        // A taller row: only the window draws it differently.
        ("analog.update", json!({ "capture": c, "analog": "fan", "height": 320 })),
        ("analog.remove", json!({ "capture": c, "analog": "fan" })),
    ];
    for (cmd, input) in steps {
        let before = state(&mut e);
        run(&mut e, cmd, input);
        let after = state(&mut e);
        assert_ne!(before, after, "{cmd} changed nothing");
        run(&mut e, "history.undo", json!({}));
        assert_eq!(state(&mut e), before, "undo of {cmd}");
        run(&mut e, "history.redo", json!({}));
        assert_eq!(state(&mut e), after, "redo of {cmd}");
    }
    // Back past remove, height and period: the scaling-only change rescaled the values, 30 % of
    // 5 V plus 0.1 V.
    for _ in 0..3 {
        run(&mut e, "history.undo", json!({}));
    }
    let a = state(&mut e)[0].clone();
    assert!(close(a["min"].as_f64().unwrap(), 1.6, 0.05), "{a}");
    assert_eq!(a["config"]["style"], "line");
}

#[test]
fn mistakes_are_refused_with_a_reason() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    let mut bad = duty(&c);
    bad["source"] = json!("theremin");
    let (code, msg) = fail(&mut e, "analog.add", bad);
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains("analog.sources"), "{msg}");

    let (_, msg) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 0 },
        "settings": { "measure": "frequency" }, "scaling": { "kind": "range", "low": 0, "high": 1 } }),
    );
    assert!(msg.contains("gain and an offset"), "{msg}");
    let (_, msg) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 0 },
        "scaling": { "kind": "range", "low": 2, "high": 2 } }),
    );
    assert!(msg.contains("lowest and highest value are the same"), "{msg}");
    let (_, msg) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": {},
        "scaling": { "kind": "range", "low": 0, "high": 1 } }),
    );
    assert_eq!(msg, "Choose a channel for the signal.");
    let (_, msg) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 40 },
        "scaling": { "kind": "range", "low": 0, "high": 1 } }),
    );
    assert!(msg.contains("Channel 40"), "{msg}");
    let mut low = duty(&c);
    low["height"] = json!(10);
    let (_, msg) = fail(&mut e, "analog.add", low);
    assert_eq!(msg, "A row is 40 to 800 pixels high; 10 is not possible.");
    let (code, _) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 0 },
        "scaling": { "kind": "range", "low": 0, "high": 1 }, "colour": "red" }),
    );
    assert_eq!(code, ErrorCode::InvalidInput, "unknown fields are refused");

    let id = add(&mut e, duty(&c));
    // An update that fails leaves the channel as it was.
    let before = capture(&mut e, &c)["analog"].clone();
    let (_, msg) = fail(
        &mut e,
        "analog.update",
        json!({ "capture": c, "analog": id, "settings": { "measure": "frequency" } }),
    );
    assert!(msg.contains("gain and an offset"), "{msg}");
    assert_eq!(capture(&mut e, &c)["analog"], before);
    let (code, _) = fail(&mut e, "analog.remove", json!({ "capture": c, "analog": "analog-9" }));
    assert_eq!(code, ErrorCode::NotFound);
    let v = |from: u64, to: u64, buckets: u32| json!({ "capture": c, "analog": id, "from": from, "to": to, "buckets": buckets });
    let (_, msg) = fail(&mut e, "analog.values", v(10, 10, 5));
    assert!(msg.contains("empty"), "{msg}");
    let (_, msg) = fail(&mut e, "analog.values", v(0, 10, 0));
    assert!(msg.contains("1 to 8192"), "{msg}");
    let (_, msg) = fail(&mut e, "analog.values", v(0, 10, 8193));
    assert!(msg.contains("1 to 8192"), "{msg}");
    // A range past the end of the capture has no values but is no error.
    let far = run(&mut e, "analog.values", v(1 << 40, (1 << 40) + 10, 5));
    assert_eq!(far["points"], json!([]));
}

#[test]
fn saving_and_reopening_keeps_virtual_channels() {
    let dir = std::env::temp_dir().join(format!("logica-engine-analog-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pwm.lacap");
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    let mut input = duty(&c);
    input["id"] = json!("fan");
    input["name"] = json!("Fan speed");
    run(&mut e, "analog.add", input);
    run(&mut e, "file.save", json!({ "capture": c, "path": path }));
    let before = capture(&mut e, &c)["analog"].clone();

    let mut f = Engine::new(EngineConfig::default());
    let id = run(&mut f, "file.open", json!({ "path": path }))["capture"]["id"].as_str().unwrap().to_string();
    assert_eq!(capture(&mut f, &id)["analog"], before);
    // The reserved id is not handed out again.
    let next = run(&mut f, "analog.add", duty(&id));
    assert_ne!(next["analog"]["id"], "fan");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Polls as the app's pump does, until `done` or a few seconds have passed.
fn poll_until(e: &mut Engine, mut done: impl FnMut(&mut Engine) -> bool) {
    let t = Instant::now();
    while !done(e) {
        assert!(t.elapsed() < Duration::from_secs(5), "waited too long");
        std::thread::sleep(Duration::from_millis(20));
        e.poll();
    }
}

#[test]
fn a_channel_follows_a_live_capture() {
    // The virtual analyser's example has a 1 kHz PWM at 25 % on channel 7.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    let mut cfg = logica_vla::config::Config::example();
    cfg.seed = Some(3);
    cfg.listen.clone_from(&at);
    std::thread::spawn(move || logica_vla::server::serve(listener, cfg, Arc::new(|_: &str| {})));
    let mut e = Engine::new(EngineConfig { vla_addresses: vec![at.clone()], ..EngineConfig::default() });
    let rx = e.subscribe();
    let dev = format!("vla/{at}");
    run(&mut e, "device.connect", json!({ "device": dev }));
    run(
        &mut e,
        "device.configure",
        json!({ "device": dev, "config": { "channels": [7], "sampleRateHz": 4e6, "samples": 240_000_000u64, "mode": "stream" } }),
    );
    let c = run(&mut e, "capture.start", json!({ "device": dev, "wait": false }))["id"]
        .as_str()
        .unwrap()
        .to_string();
    poll_until(&mut e, |e| capture(e, &c)["sampleCount"].as_u64().unwrap_or(0) > 40_000);
    let a = run(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 7 },
        "scaling": { "kind": "range", "low": 0, "high": 100 } }),
    );
    assert_eq!(a["analog"]["live"], true);
    let points = |e: &mut Engine| capture(e, &c)["analog"][0]["points"].as_u64().unwrap();
    let first = points(&mut e);
    poll_until(&mut e, |e| points(e) > first + 20);
    assert!(rx.try_iter().filter(|ev| matches!(ev, Event::AnalogUpdated { .. })).count() >= 2);
    run(&mut e, "capture.stop", json!({ "device": dev }));
    let s = capture(&mut e, &c)["analog"][0].clone();
    assert_eq!(s["live"], false);
    assert!(
        close(s["min"].as_f64().unwrap(), 25.0, 0.5) && close(s["max"].as_f64().unwrap(), 25.0, 0.5),
        "{s}"
    );
}

#[test]
fn sessions_saved_before_virtual_channels_existed_still_open() {
    let old = json!({ "decoders": {}, "markers": [], "groups": {}, "hidden": [] });
    let s: logica_engine::state::SavedSession = serde_json::from_value(old).unwrap();
    assert!(s.analog.is_empty());
}
