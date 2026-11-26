//! Live capture (doc 08): a capture from a streaming device is in the session while it runs,
//! grows as the engine polls, can be decoded while it grows, and ends complete or stopped.
//! Captures from a real virtual analyser (`logica-vla`) on a loopback port.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use logica_engine::{Actor, Engine, EngineConfig, ErrorCode, Event, Invocation};
use logica_vla::Config;

fn agent() -> Actor {
    Actor::Agent("claude".into())
}

fn run(e: &mut Engine, a: &Actor, command: &str, input: Value) -> Value {
    e.execute(a, &Invocation { command: command.into(), input })
        .unwrap_or_else(|x| panic!("{command} failed: {}", x.message))
}

fn fail(e: &mut Engine, a: &Actor, command: &str, input: Value) -> (ErrorCode, String) {
    let x = e.execute(a, &Invocation { command: command.into(), input }).unwrap_err();
    (x.code, x.message)
}

fn summary(e: &mut Engine, id: &str) -> Value {
    let s = run(e, &Actor::User, "session.get", json!({}));
    s["captures"].as_array().unwrap().iter().find(|c| c["id"] == id).cloned().unwrap_or(Value::Null)
}

/// An analyser in real time on a free port, and an engine that looks there.
fn setup(seed: u64) -> (Engine, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    let mut c = Config::example();
    c.seed = Some(seed);
    c.listen.clone_from(&at);
    std::thread::spawn(move || logica_vla::server::serve(listener, c, Arc::new(|_: &str| {})));
    let e = Engine::new(EngineConfig { vla_addresses: vec![at.clone()], ..EngineConfig::default() });
    (e, format!("vla/{at}"))
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

fn configure(e: &mut Engine, a: &Actor, dev: &str, channels: &[u32], samples: u64) {
    run(e, a, "device.connect", json!({ "device": dev }));
    run(
        e,
        a,
        "device.configure",
        json!({ "device": dev, "config": { "channels": channels, "sampleRateHz": 4e6, "samples": samples, "mode": "stream" } }),
    );
}

#[test]
fn a_live_capture_grows_decodes_while_running_and_stops() {
    let (mut e, dev) = setup(11);
    let rx = e.subscribe();
    let list = run(&mut e, &agent(), "device.list", json!({}));
    let d =
        list["devices"].as_array().unwrap().iter().find(|d| d["info"]["id"] == dev.as_str()).unwrap().clone();
    assert_eq!(d["info"]["model"], "Virtual analyser: Bench");
    assert_eq!(d["capabilities"]["modes"], json!(["stream"]));

    // a minute of real time on UART and SPI
    configure(&mut e, &agent(), &dev, &[0, 3, 4, 5, 6], 240_000_000);
    let o = run(&mut e, &agent(), "capture.start", json!({ "device": dev, "title": "Live", "wait": false }));
    let id = o["id"].as_str().unwrap().to_string();
    assert_eq!(o["running"], true);

    // it joins the session with its first data, in the state "capturing", as the active tab
    poll_until(&mut e, |e| summary(e, &id)["sampleCount"].as_u64().unwrap_or(0) > 400_000);
    let s = summary(&mut e, &id);
    assert_eq!(s["state"], "capturing");
    assert_eq!(s["title"], "Live");
    assert_eq!(
        s["channels"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(),
        ["TX", "CLK", "CS", "MOSI", "MISO"]
    );
    let session = run(&mut e, &Actor::User, "session.get", json!({}));
    assert_eq!(session["active"], id.as_str());

    // a decoder added while capturing follows the new data
    let dec = run(
        &mut e,
        &agent(),
        "decoder.add",
        json!({ "capture": id, "protocol": "spi", "roles": { "clk": 3, "cs": 4, "mosi": 5, "miso": 6 } }),
    );
    let did = dec["decoder"]["id"].as_str().unwrap().to_string();
    let frames = |e: &mut Engine| {
        run(e, &agent(), "decoder.frames", json!({ "capture": id, "decoder": did, "limit": 1 }))["total"]
            .as_u64()
            .unwrap()
    };
    let first = frames(&mut e);
    poll_until(&mut e, |e| frames(e) > first + 10);
    let w = run(
        &mut e,
        &Actor::User,
        "waveform.get",
        json!({ "capture": id, "from": 0, "to": 400_000, "buckets": 400 }),
    );
    assert!(!w["decoders"].as_array().unwrap().is_empty());

    // saving waits for the end; progress events came along
    let (code, msg) =
        fail(&mut e, &Actor::User, "file.save", json!({ "capture": id, "path": "/tmp/x.lacap" }));
    assert_eq!(code, ErrorCode::Busy);
    assert!(msg.contains("still running"), "{msg}");
    let events: Vec<Event> = rx.try_iter().collect();
    assert!(events.iter().any(|ev| matches!(ev, Event::CaptureProgress { capture, fraction: Some(f), .. } if *capture == id && *f > 0.0 && *f < 0.1)));
    assert!(events.iter().any(|ev| matches!(ev, Event::DecoderUpdated { capture, .. } if *capture == id)));

    run(&mut e, &agent(), "capture.stop", json!({ "device": dev }));
    let s = summary(&mut e, &id);
    assert_eq!(s["state"], "aborted");
    let end = s["sampleCount"].as_u64().unwrap();
    assert!(end > 400_000 && end < 240_000_000, "{end}");
    assert!(
        s["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["message"].as_str().unwrap().contains("stopped after"))
    );
    assert!(
        rx.try_iter()
            .any(|ev| matches!(ev, Event::CaptureDone { capture, stopped: true, .. } if capture == id))
    );

    // what was decoded live is what a decode of the finished capture gives
    let live = frames(&mut e);
    let again = run(
        &mut e,
        &agent(),
        "decoder.add",
        json!({ "capture": id, "protocol": "spi", "roles": { "clk": 3, "cs": 4, "mosi": 5, "miso": 6 } }),
    );
    let whole = run(
        &mut e,
        &agent(),
        "decoder.frames",
        json!({ "capture": id, "decoder": again["decoder"]["id"], "limit": 1 }),
    )["total"]
        .as_u64()
        .unwrap();
    assert_eq!(live, whole);
    // a finished capture saves
    let path = std::env::temp_dir().join(format!("logica-live-{}.lacap", std::process::id()));
    run(&mut e, &Actor::User, "file.save", json!({ "capture": id, "path": path.display().to_string() }));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_short_capture_completes_and_closing_a_running_one_stops_it() {
    let (mut e, dev) = setup(12);
    configure(&mut e, &Actor::User, &dev, &[0], 200_000);
    let o = run(&mut e, &Actor::User, "capture.start", json!({ "device": dev, "wait": false }));
    let id = o["id"].as_str().unwrap().to_string();
    poll_until(&mut e, |e| summary(e, &id)["state"] == "complete");
    assert_eq!(summary(&mut e, &id)["sampleCount"], 200_000);

    // closing a capture while it runs stops the device first; the device is free again
    configure(&mut e, &Actor::User, &dev, &[0], 240_000_000);
    let o = run(&mut e, &Actor::User, "capture.start", json!({ "device": dev, "wait": false }));
    let id = o["id"].as_str().unwrap().to_string();
    poll_until(&mut e, |e| !summary(e, &id).is_null());
    // an agent cannot stop the person's capture by closing it
    let (code, msg) = fail(&mut e, &agent(), "capture.close", json!({ "capture": id }));
    assert_eq!(code, ErrorCode::Busy);
    assert!(msg.contains("ask the person to stop it"), "{msg}");
    run(&mut e, &Actor::User, "capture.close", json!({ "capture": id }));
    assert!(summary(&mut e, &id).is_null());
    let list = run(&mut e, &Actor::User, "device.list", json!({}));
    let d =
        list["devices"].as_array().unwrap().iter().find(|d| d["info"]["id"] == dev.as_str()).unwrap().clone();
    assert_eq!(d["capturing"], false);
    configure(&mut e, &Actor::User, &dev, &[0], 1000);
}

#[test]
fn a_refused_capture_explains_and_never_joins_the_session() {
    let (mut e, dev) = setup(13);
    configure(&mut e, &Actor::User, &dev, &[0], 240_000_000);
    let o = run(&mut e, &Actor::User, "capture.start", json!({ "device": dev, "wait": false }));
    let first = o["id"].as_str().unwrap().to_string();
    poll_until(&mut e, |e| !summary(e, &first).is_null());
    // a second engine on the same analyser: the analyser is busy and says so
    let mut other = Engine::new(EngineConfig {
        vla_addresses: vec![dev.trim_start_matches("vla/").into()],
        ..EngineConfig::default()
    });
    let rx = other.subscribe();
    configure(&mut other, &Actor::User, &dev, &[0], 1000);
    let o = run(&mut other, &Actor::User, "capture.start", json!({ "device": dev, "wait": true }));
    assert!(o["capture"].is_null());
    let why: Vec<String> = rx
        .try_iter()
        .filter_map(|ev| match ev {
            Event::CommandExecuted { summary, .. } => Some(summary),
            _ => None,
        })
        .collect();
    assert!(why.iter().any(|w| w.contains("busy with another capture")), "{why:?}");
    let s = run(&mut other, &Actor::User, "session.get", json!({}));
    assert!(s["captures"].as_array().unwrap().is_empty());
    run(&mut e, &Actor::User, "capture.stop", json!({ "device": dev }));
}

#[test]
fn the_newest_data_is_drawn_while_it_arrives() {
    let (mut e, dev) = setup(14);
    // PWM at 1 kHz: two edges a millisecond, in every part of the capture
    configure(&mut e, &Actor::User, &dev, &[7], 240_000_000);
    let o = run(&mut e, &Actor::User, "capture.start", json!({ "device": dev, "wait": false }));
    let id = o["id"].as_str().unwrap().to_string();
    for _ in 0..8 {
        poll_until(&mut e, |e| summary(e, &id)["sampleCount"].as_u64().unwrap_or(0) > 200_000);
        std::thread::sleep(Duration::from_millis(37));
        e.poll();
        let count = summary(&mut e, &id)["sampleCount"].as_u64().unwrap();
        // the newest 20 ms, mostly in the chunk still being written
        let w = run(
            &mut e,
            &Actor::User,
            "waveform.get",
            json!({ "capture": id, "from": count - 80_000, "to": count, "buckets": 800 }),
        );
        let t = &w["traces"][0];
        let edges = t["edges"].as_array().map_or(0, Vec::len);
        let counted: u64 = t["counts"].as_array().map_or(0, |c| c.iter().map(|x| x.as_u64().unwrap()).sum());
        assert!(
            (38..=42).contains(&(edges as u64 + counted)),
            "{count}: {edges} edges, {counted} counted: {t}"
        );
    }
    run(&mut e, &Actor::User, "capture.stop", json!({ "device": dev }));
}

/// An analyser on a free port, as fast as the engine takes the data; returns its device id.
fn analyser(name: &str, seed: u64, ppm: f64) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    let mut c = Config::example();
    c.name = name.into();
    c.seed = Some(seed);
    c.speed = 0.0;
    c.clock_ppm = ppm;
    c.listen.clone_from(&at);
    std::thread::spawn(move || logica_vla::server::serve(listener, c, Arc::new(|_: &str| {})));
    format!("vla/{at}")
}

fn two(ppm: f64) -> (Engine, String, String) {
    let (a, b) = (analyser("Bench", 21, 0.0), analyser("Second", 22, ppm));
    let addresses = [&a, &b].iter().map(|d| d.trim_start_matches("vla/").to_string()).collect();
    (Engine::new(EngineConfig { vla_addresses: addresses, ..EngineConfig::default() }), a, b)
}

fn notes(c: &Value) -> Vec<String> {
    c["notes"].as_array().unwrap().iter().map(|n| n["message"].as_str().unwrap().to_string()).collect()
}

#[test]
fn two_analysers_capture_into_one_capture_lined_up_by_a_shared_clock() {
    let (mut e, a, b) = two(250.0);
    run(&mut e, &Actor::User, "device.list", json!({}));
    let all: Vec<u32> = (0..12).collect();
    configure(&mut e, &Actor::User, &a, &all, 4_000_000);
    configure(&mut e, &Actor::User, &b, &all, 4_000_000);
    let o = run(
        &mut e,
        &Actor::User,
        "capture.start",
        json!({ "device": a, "with": [b], "align": { a.clone(): 10, b.clone(): 10 }, "title": "Both" }),
    );
    let id = o["id"].as_str().unwrap().to_string();
    let c = summary(&mut e, &id);
    assert_eq!(c["state"], "complete", "{c}");
    // as far as both have data: Second's fast clock covers 250 ppm (1000 samples) less time, and
    // it may start a few ms before Bench on a busy machine (4000 samples a ms)
    let n = c["sampleCount"].as_u64().unwrap();
    assert!((3_950_000..=4_000_000).contains(&n), "{n}");
    let channels = c["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 24);
    assert_eq!((channels[0]["index"].clone(), channels[0]["name"].clone()), (json!(0), json!("Bench: TX")));
    assert_eq!(
        (channels[22]["index"].clone(), channels[22]["name"].clone()),
        (json!(22), json!("Second: SYNC"))
    );
    let n = notes(&c);
    assert!(
        n.iter().any(|m| m.starts_with("Second is lined up with Bench CH10 SYNC by its CH10 SYNC")),
        "{n:?}"
    );
    let drift = n.iter().find_map(|m| m.strip_prefix("Second's clock ran ")).expect("a drift note");
    let ppm: f64 = drift.split(' ').next().unwrap().parse().unwrap();
    assert!((ppm - 250.0).abs() < 5.0, "{drift}");
    assert_eq!(c["source"]["kind"], "device");
    assert!(c["source"]["detail"].as_str().unwrap().contains("lined up by a shared signal"));

    // the second analyser's UART decodes on its channel in the shared capture
    let d = run(
        &mut e,
        &Actor::User,
        "decoder.add",
        json!({ "capture": id, "protocol": "uart", "roles": { "data": 11 } }),
    );
    let did = d["decoder"]["id"].as_str().unwrap().to_string();
    let f = run(&mut e, &Actor::User, "decoder.frames", json!({ "capture": id, "decoder": did, "limit": 1 }));
    assert!(f["total"].as_u64().unwrap() > 0, "{f}");

    // both devices are free again
    let list = run(&mut e, &Actor::User, "device.list", json!({}));
    assert!(list["devices"].as_array().unwrap().iter().all(|d| d["capturing"] == false));
}

#[test]
fn capturing_together_is_refused_with_reasons() {
    let (mut e, a, b) = two(0.0);
    run(&mut e, &Actor::User, "device.list", json!({}));
    configure(&mut e, &Actor::User, &a, &[0, 10], 400_000);
    run(&mut e, &Actor::User, "device.connect", json!({ "device": b }));
    let start = |w: Value| {
        let mut i = json!({ "device": a, "with": [b] });
        if let Value::Object(m) = w {
            i.as_object_mut().unwrap().extend(m);
        }
        i
    };
    let (_, m) = fail(&mut e, &Actor::User, "capture.start", start(json!({})));
    assert!(m.contains(&format!("Configure {b} first")), "{m}");
    run(
        &mut e,
        &Actor::User,
        "device.configure",
        json!({ "device": b, "config": { "channels": [0], "sampleRateHz": 10e6, "samples": 400_000, "mode": "stream" } }),
    );
    let (code, m) = fail(&mut e, &Actor::User, "capture.start", start(json!({})));
    assert_eq!(code, ErrorCode::InvalidInput);
    assert!(m.contains("one capture has one rate"), "{m}");
    configure(&mut e, &Actor::User, &b, &[0], 400_000);
    let (_, m) = fail(&mut e, &Actor::User, "capture.start", start(json!({ "with": [b, b] })));
    assert!(m.contains("is listed twice"), "{m}");
    let (_, m) = fail(&mut e, &Actor::User, "capture.start", start(json!({ "align": { a.clone(): 10 } })));
    assert!(m.contains(&format!("align has no channel for {b}")), "{m}");
    let (_, m) = fail(
        &mut e,
        &Actor::User,
        "capture.start",
        start(json!({ "align": { a.clone(): 10, b.clone(): 10 } })),
    );
    assert!(m.contains(&format!("{b} does not capture channel 10")), "{m}");
    let (_, m) =
        fail(&mut e, &Actor::User, "capture.start", json!({ "device": a, "align": { a.clone(): 10 } }));
    assert!(m.contains("name the others in with"), "{m}");
    // nothing was taken: each device still captures on its own
    let o = run(&mut e, &Actor::User, "capture.start", json!({ "device": b }));
    assert_eq!(summary(&mut e, o["id"].as_str().unwrap())["state"], "complete");
}

#[test]
fn stopping_one_device_of_a_shared_capture_stops_it_all() {
    let (mut e, a, b) = two(0.0);
    run(&mut e, &Actor::User, "device.list", json!({}));
    // far longer than the test waits
    configure(&mut e, &Actor::User, &a, &[0, 10], 4_000_000_000);
    configure(&mut e, &Actor::User, &b, &[0, 10], 4_000_000_000);
    let o = run(&mut e, &Actor::User, "capture.start", json!({ "device": a, "with": [b], "wait": false }));
    let id = o["id"].as_str().unwrap().to_string();
    poll_until(&mut e, |e| summary(e, &id)["sampleCount"].as_u64().unwrap_or(0) > 1_000_000);
    let s = run(&mut e, &Actor::User, "session.get", json!({}));
    let running: Vec<&str> =
        s["running"].as_array().unwrap().iter().map(|r| r["device"].as_str().unwrap()).collect();
    assert_eq!(running, [a.as_str(), b.as_str()], "one entry per device, same capture");
    run(&mut e, &Actor::User, "capture.stop", json!({ "device": b }));
    let c = summary(&mut e, &id);
    assert_eq!(c["state"], "aborted");
    assert!(run(&mut e, &Actor::User, "session.get", json!({}))["running"].as_array().unwrap().is_empty());
}
