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
    assert_eq!(pwm["presets"], json!([]));
    // ADCs on SPI and I2C, with the chips they know
    let spi = s["sources"].as_array().unwrap().iter().find(|s| s["id"] == "spi-adc").unwrap();
    let ids: Vec<_> = spi["presets"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["mcp3201", "mcp3208"]);
    assert_eq!(spi["roles"].as_array().unwrap().len(), 4);
    let i2c = s["sources"].as_array().unwrap().iter().find(|s| s["id"] == "i2c-adc").unwrap();
    assert_eq!(i2c["presets"][0]["settings"]["address"], 72.0);
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

#[test]
fn values_at_the_cursors() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    let none = run(&mut e, "analog.at", json!({ "capture": c, "at": [0] }));
    assert_eq!(none["channels"], json!([]), "no channels, nothing to read");
    let id = add(&mut e, duty(&c));
    let rate = capture(&mut e, &c)["sampleRateHz"].as_f64().unwrap();
    let mid = (rate * 0.5) as u64;
    let o = run(&mut e, "analog.at", json!({ "capture": c, "at": [mid, 0] }));
    let ch = &o["channels"][0];
    assert_eq!(
        (ch["analog"].as_str().unwrap(), ch["name"].as_str().unwrap(), ch["unit"].as_str().unwrap()),
        (id.as_str(), "PWM level", "%")
    );
    assert!(ch["step"].is_null(), "a duty has no code steps");
    // in the order asked: half a second in it reads 30 %; before the first period, nothing
    assert_eq!(ch["values"][0]["sample"], mid);
    assert!(close(ch["values"][0]["value"].as_f64().unwrap(), 30.0, 1.0), "{ch}");
    assert!(ch["values"][0]["code"].is_null());
    assert!(ch["values"][1]["value"].is_null(), "{ch}");
    let (code, msg) = fail(&mut e, "analog.at", json!({ "capture": c, "at": [] }));
    assert_eq!(code, ErrorCode::InvalidInput);
    assert_eq!(msg, "Give 1 to 16 samples to read the values at.");
    let (_, msg) = fail(&mut e, "analog.at", json!({ "capture": c, "at": vec![1u64; 17] }));
    assert!(msg.contains("1 to 16"), "{msg}");
}

#[test]
fn numbers_over_a_range_in_the_channels_unit() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    let mut input = duty(&c);
    // 0 to 5 V with a 0.1 V trim: 30 % reads 1.6 V
    input["scaling"] = json!({ "kind": "range", "low": 0, "high": 5, "offset": 0.1 });
    input["unit"] = json!("V");
    let id = add(&mut e, input);
    let st = run(&mut e, "analog.stats", json!({ "capture": c, "analog": id }));
    assert_eq!(st["unit"], "V");
    for k in ["min", "max", "mean", "rms"] {
        assert!(close(st[k].as_f64().unwrap(), 1.6, 0.02), "{k}: {st}");
    }
    assert!(st["peakToPeak"].as_f64().unwrap() < 0.02, "{st}");
    assert!(st["count"].as_u64().unwrap() > 100);
    // the time before the first period has no value
    let covered = st["covered"].as_f64().unwrap();
    assert!(covered > 0.9 && covered < 1.0, "{covered}");
    // a range before the first value has no numbers, and that is no error
    let early = run(&mut e, "analog.stats", json!({ "capture": c, "analog": id, "from": 0, "to": 1 }));
    assert!(early["mean"].is_null() && early["min"].is_null(), "{early}");
    assert_eq!(early["covered"], 0.0);
    let (code, msg) =
        fail(&mut e, "analog.stats", json!({ "capture": c, "analog": id, "from": 10, "to": 10 }));
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains("empty"), "{msg}");
    let (code, _) = fail(&mut e, "analog.stats", json!({ "capture": c, "analog": "nope" }));
    assert_eq!(code, ErrorCode::NotFound);
}

#[test]
fn values_are_exported_as_csv() {
    let dir = std::env::temp_dir().join(format!("logica-engine-csv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    let mut duty_in = duty(&c);
    duty_in["name"] = json!("Fan, duty");
    let a = add(&mut e, duty_in);
    let f = add(
        &mut e,
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 0 },
        "settings": { "measure": "frequency" }, "scaling": { "kind": "linear", "gain": 1 }, "name": "Fan speed" }),
    );
    let path = dir.join("fan.csv");
    let o = run(&mut e, "analog.export", json!({ "capture": c, "path": path, "analogs": [a, f] }));
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // a quoted name with a comma; no code columns, since a duty and a frequency are no codes
    assert_eq!(lines[0], "time_s,sample,\"Fan, duty (%)\",Fan speed (Hz)");
    assert_eq!(o["rows"].as_u64().unwrap() as usize, lines.len() - 1);
    assert_eq!(o["bytes"].as_u64().unwrap(), text.len() as u64);
    // both channels have a value from the same periods, so every row holds both
    let row: Vec<&str> = lines[1].split(',').collect();
    assert_eq!(row.len(), 4);
    assert!(
        close(row[2].parse::<f64>().unwrap(), 30.0, 1.0)
            && close(row[3].parse::<f64>().unwrap(), 1000.0, 20.0),
        "{}",
        lines[1]
    );
    let rate = capture(&mut e, &c)["sampleRateHz"].as_f64().unwrap();
    let sample: f64 = row[1].parse().unwrap();
    assert!(close(row[0].parse::<f64>().unwrap(), sample / rate, 1e-9));
    // a range from the middle starts with the values held there
    let mid = (rate * 0.5) as u64;
    let o = run(&mut e, "analog.export", json!({ "capture": c, "path": path, "from": mid, "to": mid + 10 }));
    let text = std::fs::read_to_string(&path).unwrap();
    let first = text.lines().nth(1).unwrap();
    assert!(first.starts_with(&format!("{},{mid},", mid as f64 / rate)), "{first}");
    assert!(o["rows"].as_u64().unwrap() >= 1);
    // writing a file is recorded but not undone
    let entry = run(&mut e, "session.get", json!({}))["recent"][0].clone();
    assert_eq!(entry["done"]["command"], "analog.export");
    let (code, msg) = fail(&mut e, "history.undo", json!({ "entry": entry["id"] }));
    assert_eq!(code, ErrorCode::NotUndoable, "{msg}");
    // refusals that say what to do
    let (code, msg) =
        fail(&mut e, "analog.export", json!({ "capture": c, "path": dir.join("no/such/dir/x.csv") }));
    assert_eq!(code, ErrorCode::Failed);
    assert!(msg.contains("could not be written"), "{msg}");
    let (code, _) = fail(&mut e, "analog.export", json!({ "capture": c, "path": path, "analogs": ["nope"] }));
    assert_eq!(code, ErrorCode::NotFound);
    let c2 = open_mystery_d(&mut e);
    let (_, msg) = fail(&mut e, "analog.export", json!({ "capture": c2, "path": path }));
    assert!(msg.contains("no virtual analog channel to export"), "{msg}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_channel_reads_from_a_decoder_and_follows_it() {
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    let d = run(&mut e, "decoder.add", json!({ "capture": c, "protocol": "pwm", "roles": { "signal": 0 } }))
        ["decoder"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // no channels given: they come from the decoder
    let o = run(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": {}, "decoder": d,
        "scaling": { "kind": "range", "low": 0, "high": 100 } }),
    );
    let id = o["analog"]["id"].as_str().unwrap().to_string();
    assert_eq!(o["analog"]["config"]["decoder"], d.as_str());
    assert_eq!(o["analog"]["config"]["roles"]["signal"], 0);
    let mean = |e: &mut Engine| {
        run(e, "analog.stats", json!({ "capture": c, "analog": id }))["mean"].as_f64().unwrap()
    };
    assert!(close(mean(&mut e), 30.0, 1.0));
    // the decoder counts the low time now: the channel follows, and an undo takes it back
    run(
        &mut e,
        "decoder.update",
        json!({ "capture": c, "decoder": d, "settings": { "activeLevel": "low" } }),
    );
    assert!(close(mean(&mut e), 70.0, 1.0), "{}", mean(&mut e));
    run(&mut e, "history.undo", json!({}));
    assert!(close(mean(&mut e), 30.0, 1.0));
    // the decoder goes: the channel keeps working and says why; back with undo, the note goes
    run(&mut e, "decoder.remove", json!({ "capture": c, "decoder": d }));
    assert!(close(mean(&mut e), 30.0, 1.0));
    let notes = capture(&mut e, &c)["analog"][0]["notes"].to_string();
    assert!(notes.contains("was removed"), "{notes}");
    run(&mut e, "history.undo", json!({}));
    let notes = capture(&mut e, &c)["analog"][0]["notes"].to_string();
    assert!(!notes.contains("was removed"), "{notes}");
    // a decoder the source cannot read, and one that is not there
    let spi = run(
        &mut e,
        "decoder.add",
        json!({ "capture": c, "protocol": "spi",
        "roles": { "clk": 5, "cs": 3, "mosi": 6, "miso": 2 } }),
    )["decoder"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (code, msg) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": {}, "decoder": spi,
        "scaling": { "kind": "range", "low": 0, "high": 1 } }),
    );
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains("is read by the source spi-adc"), "{msg}");
    let (code, _) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "pwm", "roles": {}, "decoder": "nope",
        "scaling": { "kind": "range", "low": 0, "high": 1 } }),
    );
    assert_eq!(code, ErrorCode::NotFound);
    // an ADC on the SPI decoder takes its channels and mode
    let o = run(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "spi-adc", "roles": {}, "decoder": spi,
        "settings": { "bits": 8 }, "scaling": { "kind": "range", "low": 0, "high": 3.3 } }),
    );
    assert_eq!(o["analog"]["config"]["roles"]["clk"], 5);
    assert_eq!(o["analog"]["config"]["settings"]["bits"], 8.0);
    // changing such a channel keeps the decoder's bus settings
    let adc = o["analog"]["id"].as_str().unwrap().to_string();
    let u = run(
        &mut e,
        "analog.update",
        json!({ "capture": c, "analog": adc, "settings": { "bits": 10 }, "replaceSettings": true }),
    );
    assert_eq!(u["analog"]["config"]["settings"]["mode"], "0");
    assert_eq!(u["analog"]["config"]["settings"]["bits"], 10.0);
    // the sources say which decoders they read and which settings then come from them
    let s = run(&mut e, "analog.sources", json!({}));
    let spi = s["sources"].as_array().unwrap().iter().find(|s| s["id"] == "spi-adc").unwrap().clone();
    assert_eq!(spi["decoders"], json!(["spi"]));
    assert_eq!(spi["decoderSettings"], json!(["mode", "bitOrder", "wordSize", "csPolarity"]));
    let pwm = s["sources"].as_array().unwrap().iter().find(|s| s["id"] == "pwm").unwrap().clone();
    assert_eq!(pwm["decoderSettings"], json!(["activeLevel"]));
}

#[test]
fn a_live_capture_at_one_gigasample_a_second() {
    // the virtual analyser sends transitions, so a fast rate costs little: 0.2 s at 1 GS/s
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    let mut cfg = logica_vla::config::Config::example();
    cfg.seed = Some(9);
    cfg.listen.clone_from(&at);
    std::thread::spawn(move || logica_vla::server::serve(listener, cfg, Arc::new(|_: &str| {})));
    let mut e = Engine::new(EngineConfig { vla_addresses: vec![at.clone()], ..EngineConfig::default() });
    let dev = format!("vla/{at}");
    run(&mut e, "device.connect", json!({ "device": dev }));
    run(
        &mut e,
        "device.configure",
        json!({ "device": dev, "config": { "channels": [12], "sampleRateHz": 1e9, "samples": 200_000_000u64, "mode": "stream" } }),
    );
    let c = run(&mut e, "capture.start", json!({ "device": dev, "wait": false }))["id"]
        .as_str()
        .unwrap()
        .to_string();
    poll_until(&mut e, |e| capture(e, &c)["state"] == "complete");
    assert_eq!(capture(&mut e, &c)["sampleCount"], 200_000_000u64);
    // the SINE level (10 % to 90 % once a second): a fifth of a second of it rises from the middle
    let id = add(
        &mut e,
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 12 },
        "scaling": { "kind": "range", "low": 0, "high": 100 } }),
    );
    let st = run(&mut e, "analog.stats", json!({ "capture": c, "analog": id }));
    assert!(st["count"].as_u64().unwrap() >= 195, "{st}");
    assert!(
        close(st["min"].as_f64().unwrap(), 50.0, 1.0) && close(st["max"].as_f64().unwrap(), 88.0, 2.0),
        "{st}"
    );
}

/// One second of the virtual analyser's SINE level (CH12: 10 % to 90 %, once a second) at
/// 4 MS/s, with a PWM level on it in per cent.
fn sine_capture() -> (Engine, String, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    let mut cfg = logica_vla::config::Config::example();
    cfg.seed = Some(4);
    cfg.listen.clone_from(&at);
    cfg.speed = 0.0;
    std::thread::spawn(move || logica_vla::server::serve(listener, cfg, Arc::new(|_: &str| {})));
    let mut e = Engine::new(EngineConfig { vla_addresses: vec![at.clone()], ..EngineConfig::default() });
    let dev = format!("vla/{at}");
    run(&mut e, "device.connect", json!({ "device": dev }));
    run(
        &mut e,
        "device.configure",
        json!({ "device": dev, "config": { "channels": [12], "sampleRateHz": 4e6, "samples": 4_000_000u64, "mode": "stream" } }),
    );
    let c = run(&mut e, "capture.start", json!({ "device": dev }))["id"].as_str().unwrap().to_string();
    let id = add(
        &mut e,
        json!({ "capture": c, "source": "pwm", "roles": { "signal": 12 },
        "scaling": { "kind": "range", "low": 0, "high": 100 }, "unit": "%", "name": "Sine" }),
    );
    (e, c, id)
}

#[test]
fn search_finds_crossings_and_stretches_of_values() {
    let (mut e, c, id) = sine_capture();
    let search = |e: &mut Engine, query: Value| run(e, "search.run", json!({ "capture": c, "query": query }));
    let secs = |hits: &Value| {
        hits["hits"].as_array().unwrap().iter().map(|h| h["seconds"].as_f64().unwrap()).collect::<Vec<_>>()
    };
    // 0.5 + 0.4 sin(2πt) reaches 70 % at 1/12 s rising and 5/12 s falling
    let up = search(&mut e, json!({ "analog": { "analog": id, "crosses": 70, "direction": "rising" } }));
    assert_eq!(secs(&up).len(), 1, "{up}");
    assert!(close(secs(&up)[0], 1.0 / 12.0, 0.002), "{up}");
    assert_eq!(up["hits"][0]["what"], "Sine rises through 70 %");
    let any = search(&mut e, json!({ "analog": { "analog": id, "crosses": 70 } }));
    assert_eq!(secs(&any).len(), 2);
    assert!(close(secs(&any)[1], 5.0 / 12.0, 0.002), "{any}");
    // stretches: above 70 % for a third of a second, below 30 % from 7/12 to 11/12 s
    let above = search(&mut e, json!({ "analog": { "analog": id, "above": 70 } }));
    let h = &above["hits"][0];
    assert_eq!(above["hits"].as_array().unwrap().len(), 1);
    let len = (h["end"].as_u64().unwrap() - h["start"].as_u64().unwrap()) as f64 / 4e6;
    assert!(close(len, 1.0 / 3.0, 0.003), "{above}");
    assert!(h["what"].as_str().unwrap().starts_with("Sine above 70 % for "), "{above}");
    let below = search(&mut e, json!({ "analog": { "analog": id, "below": 30, "longerThan": 0.3 } }));
    assert!(close(secs(&below)[0], 7.0 / 12.0, 0.002), "{below}");
    let none = search(&mut e, json!({ "analog": { "analog": id, "below": 30, "shorterThan": 0.2 } }));
    assert_eq!(none["hits"], json!([]));
    let band = search(&mut e, json!({ "analog": { "analog": id, "above": 40, "below": 60 } }));
    assert!(band["hits"].as_array().unwrap().len() >= 2, "{band}");
    // as a filter: rising edges of CH12 while the level is above 70 % (about a third of 1000)
    let edges = run(
        &mut e,
        "search.run",
        json!({ "capture": c, "limit": 1000, "query": { "all": [ { "edge": { "channel": 12, "direction": "rising" } },
        { "analog": { "analog": id, "above": 70 } } ] } }),
    );
    let n = secs(&edges);
    assert!((320..=345).contains(&n.len()), "{} edges", n.len());
    assert!(n.iter().all(|&t| t > 1.0 / 12.0 - 0.002 && t < 5.0 / 12.0 + 0.002));
    // mistakes say what to do
    let (code, msg) =
        fail(&mut e, "search.run", json!({ "capture": c, "query": { "analog": { "analog": id } } }));
    assert_eq!(code, ErrorCode::InvalidInput);
    assert_eq!(msg, "An analog search needs a level: crosses, above or below.");
    let (_, msg) = fail(
        &mut e,
        "search.run",
        json!({ "capture": c, "query": { "analog": { "analog": id, "crosses": 1, "above": 2 } } }),
    );
    assert!(msg.contains("not both"), "{msg}");
    let (_, msg) = fail(
        &mut e,
        "search.run",
        json!({ "capture": c, "query": { "analog": { "analog": id, "above": 60, "below": 40 } } }),
    );
    assert!(msg.contains("above must be the lower one"), "{msg}");
    let (code, _) = fail(
        &mut e,
        "search.run",
        json!({ "capture": c, "query": { "analog": { "analog": "nope", "above": 1 } } }),
    );
    assert_eq!(code, ErrorCode::NotFound);
}

#[test]
fn math_channels_combine_others_and_follow_them() {
    let dir = std::env::temp_dir().join(format!("logica-engine-math-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::new(EngineConfig::default());
    let c = open_mystery_d(&mut e);
    // two inputs sampled at different moments: periods from rising edges (30 %) and from falling
    // edges (the low time, 70 %)
    let mut hi = duty(&c);
    hi["name"] = json!("High");
    let hi = add(&mut e, hi);
    let mut lo = duty(&c);
    lo["name"] = json!("Low");
    lo["settings"] = json!({ "activeLevel": "low" });
    let lo = add(&mut e, lo);
    let math = |expr: Value| {
        json!({ "capture": c, "source": "math", "roles": {}, "expression": expr,
        "scaling": { "kind": "linear", "gain": 1 }, "unit": "%" })
    };
    let ch = |id: &str| json!({ "channel": id });
    let sum = run(&mut e, "analog.add", math(json!({ "add": [ch(&hi), ch(&lo)] })));
    let sum_id = sum["analog"]["id"].as_str().unwrap().to_string();
    assert_eq!(sum["analog"]["config"]["name"], "Math");
    // without a unit of its own, the first channel's
    let plain = run(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "math", "roles": {},
        "expression": { "add": [ch(&hi), ch(&lo)] }, "scaling": { "kind": "linear", "gain": 1 } }),
    );
    assert_eq!(plain["analog"]["config"]["unit"], "%");
    run(&mut e, "history.undo", json!({}));
    let st = |e: &mut Engine, id: &str| run(e, "analog.stats", json!({ "capture": c, "analog": id }));
    let s = st(&mut e, &sum_id);
    assert!(
        close(s["min"].as_f64().unwrap(), 100.0, 1.0) && close(s["max"].as_f64().unwrap(), 100.0, 1.0),
        "{s}"
    );
    // a value wherever either input has a new one: about twice as many as each
    let n_hi = st(&mut e, &hi)["count"].as_u64().unwrap();
    let n_sum = s["count"].as_u64().unwrap();
    assert!(n_sum + 2 >= 2 * n_hi && n_sum <= 2 * n_hi + 2, "{n_sum} values from {n_hi} and its partner");

    // an input changes: the math channel follows
    run(
        &mut e,
        "analog.update",
        json!({ "capture": c, "analog": hi, "scaling": { "kind": "range", "low": 0, "high": 200 } }),
    );
    assert!(close(st(&mut e, &sum_id)["mean"].as_f64().unwrap(), 130.0, 1.5));
    run(&mut e, "history.undo", json!({}));
    assert!(close(st(&mut e, &sum_id)["mean"].as_f64().unwrap(), 100.0, 1.0));

    // an input goes: no values and a note; back with undo
    run(&mut e, "analog.remove", json!({ "capture": c, "analog": lo }));
    let gone = capture(&mut e, &c)["analog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == sum_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(gone["points"], 0);
    assert!(gone["notes"].to_string().contains("does not have"), "{gone}");
    run(&mut e, "history.undo", json!({}));
    assert!(close(st(&mut e, &sum_id)["mean"].as_f64().unwrap(), 100.0, 1.0));

    // division by zero leaves gaps with a problem that says why
    let div = add(&mut e, math(json!({ "div": [ch(&hi), { "sub": [ch(&hi), ch(&hi)] }] })));
    let d = capture(&mut e, &c)["analog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == div.as_str())
        .unwrap()
        .clone();
    assert!(d["problemCounts"]["protocol"].as_u64().unwrap() > 0, "{d}");
    let v = run(
        &mut e,
        "analog.values",
        json!({ "capture": c, "analog": div, "from": 0, "to": 1000000, "buckets": 10 }),
    );
    assert!(v["problems"][0]["message"].as_str().unwrap().contains("Division by zero"), "{v}");

    // a formula that would read itself is refused, naming the chain; so are unknown channels
    let twice = add(&mut e, math(json!({ "mul": [ch(&sum_id), { "constant": 2 }] })));
    let (code, msg) = fail(
        &mut e,
        "analog.update",
        json!({ "capture": c, "analog": sum_id,
        "expression": { "add": [ch(&hi), ch(&twice)] } }),
    );
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(msg.contains(&format!("would read {sum_id} itself, through {twice}")), "{msg}");
    let (code, _) = fail(&mut e, "analog.add", math(json!({ "add": [ch(&hi), ch("analog-999")] })));
    assert_eq!(code, ErrorCode::NotFound);
    let (_, msg) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "math", "roles": {},
        "scaling": { "kind": "linear", "gain": 1 } }),
    );
    assert!(msg.contains("needs a formula"), "{msg}");
    let (_, msg) = fail(&mut e, "analog.add", math(json!({ "sub": [ch(&hi)] })));
    assert!(msg.contains("exactly two"), "{msg}");
    let mut bad = duty(&c);
    bad["expression"] = json!({ "add": [ch(&hi), ch(&lo)] });
    let (_, msg) = fail(&mut e, "analog.add", bad);
    assert!(msg.contains("Only a math channel has a formula"), "{msg}");
    let (_, msg) = fail(
        &mut e,
        "analog.add",
        json!({ "capture": c, "source": "math", "roles": {},
        "expression": { "add": [ch(&hi), ch(&lo)] }, "scaling": { "kind": "range", "low": 0, "high": 1 } }),
    );
    assert!(msg.contains("gain and an offset"), "{msg}");

    // the sources list it; saved and opened again, math channels come back after their inputs
    let sources = run(&mut e, "analog.sources", json!({}));
    assert!(sources["sources"].as_array().unwrap().iter().any(|s| s["id"] == "math"));
    let path = dir.join("math.lacap");
    run(&mut e, "file.save", json!({ "capture": c, "path": path }));
    let mut f = Engine::new(EngineConfig::default());
    let id = run(&mut f, "file.open", json!({ "path": path }))["capture"]["id"].as_str().unwrap().to_string();
    let reopened = run(&mut f, "analog.stats", json!({ "capture": id, "analog": twice }));
    assert!(close(reopened["mean"].as_f64().unwrap(), 200.0, 2.0), "{reopened}");
    std::fs::remove_dir_all(&dir).unwrap();
}
