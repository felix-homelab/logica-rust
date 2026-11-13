//! Custom JSON protocols (Signal Detective's `tests/custom.test.ts`, without the Learn-tab
//! fingerprint checks).

mod common;

use std::sync::Arc;

use common::*;
use logica_protocols::custom::{self, CustomConfig, TEMPLATE_JSON};
use logica_protocols::protocol::{Origin, Protocol};
use logica_protocols::pulsecode::PulseCodeConfig;
use logica_protocols::synth::*;
use logica_protocols::types::*;
use serde_json::{Value, json};

fn built_in_ids() -> Vec<String> {
    registry().all().iter().map(|p| p.id().to_string()).collect()
}

fn validate(v: &Value) -> Result<CustomConfig, Vec<String>> {
    let ids = built_in_ids();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    custom::validate(v, &ids)
}

fn template() -> Value {
    serde_json::from_str(TEMPLATE_JSON).unwrap()
}

fn compile(v: &Value) -> Arc<dyn Protocol> {
    let cfg = custom::validate(v, &[]).unwrap();
    let lookup = |id: &str| registry().get(id).cloned();
    custom::compile(cfg, &lookup, Origin::Custom).unwrap()
}

#[test]
fn accepts_the_template_and_the_dmx512_preset() {
    assert!(validate(&template()).is_ok());
    let mut dmx: Value =
        serde_json::from_str(custom::BUILT_IN_JSON.iter().find(|(k, _)| *k == "dmx512").unwrap().1).unwrap();
    dmx["id"] = json!("my-dmx");
    assert!(validate(&dmx).is_ok());
}

#[test]
fn rejects_non_objects() {
    assert_eq!(validate(&json!("hello")).unwrap_err(), ["The config must be a JSON object."]);
    assert!(validate(&json!([1, 2])).is_err());
    assert!(custom::parse("{ not json", &[]).unwrap_err()[0].starts_with("The text is not valid JSON"));
}

#[test]
fn reports_every_problem_with_a_path() {
    let bad = json!({
        "family": "pulse-code", "id": "Bad ID", "name": "", "idle": "middle", "bitOrder": "msb",
        "frames": [{ "name": "x", "preamble": [{ "level": "low", "minUs": 10, "maxUs": 5 }],
                     "bit": { "first": "low", "encoding": "third", "zero": [1], "one": [1, 2], "other": [1, 2] } }],
        "info": { "what": "x", "confirm": "x", "decoderTips": "x" }
    });
    let errors = validate(&bad).unwrap_err();
    for e in [
        "config.name must be a non-empty text.",
        "config.id may only use lowercase letters, digits and \"-\".",
        "config.info.lookalikes must be a non-empty text.",
        "config.idle must be one of: \"high\", \"low\".",
        "config.frames[0].preamble[0]: minUs is larger than maxUs.",
        "config.frames[0].bit.encoding must be one of: \"first\", \"second\".",
        "config.frames[0].bit.zero must be [min, max] in microseconds, with min <= max.",
        "config.frames[0].bits must be { \"min\": n, \"max\": n } when bit is given.",
    ] {
        assert!(errors.iter().any(|x| x == e), "missing: {e}\nin {errors:#?}");
    }
}

#[test]
fn refuses_to_shadow_a_built_in_id() {
    let mut t = template();
    t["id"] = json!("uart");
    assert!(validate(&t).unwrap_err().contains(
        &"config.id \"uart\" is already used by a built-in protocol. Pick another id.".to_string()
    ));
}

#[test]
fn learn_tab_fields_are_accepted_and_ignored() {
    let mut t = template();
    t["info"]["fingerprint"] = json!({ "match": { "idle": ["high"] }, "hard": [] });
    t["info"]["sketch"] = json!({ "lanes": [] });
    assert!(validate(&t).is_ok());
}

#[test]
fn a_preset_needs_a_known_base() {
    let mut dmx: Value =
        serde_json::from_str(custom::BUILT_IN_JSON.iter().find(|(k, _)| *k == "dmx512").unwrap().1).unwrap();
    dmx["id"] = json!("x");
    dmx["base"] = json!("not-a-protocol");
    let cfg = custom::validate(&dmx, &[]).unwrap();
    let e = custom::compile(cfg, &|id| registry().get(id).cloned(), Origin::Custom).err().unwrap();
    assert_eq!(
        e,
        "The preset \"DMX512 lighting\" is based on \"not-a-protocol\", which is not a known protocol."
    );
}

fn sensor() -> PulseCodeConfig {
    match custom::validate(&template(), &[]).unwrap() {
        CustomConfig::PulseCode(c) => c,
        CustomConfig::Preset(_) => unreachable!(),
    }
}

fn sensor_capture(
    cfg: &PulseCodeConfig,
    bits: &[u8],
    twice: bool,
    index: u32,
) -> logica_protocols::synth::GenCapture {
    let mut l = Lane::high(1e6);
    pulse_code_send(&mut l, 0.002, cfg, 0, bits);
    if twice {
        pulse_code_send(&mut l, 0.01, cfg, 0, bits);
    }
    make_capture("t", 1e6, 0.02, vec![(index, "S", l)])
}

#[test]
fn a_custom_pulse_code_protocol_is_detected_and_decoded_once_registered() {
    let bits = [1, 0, 1, 0, 0, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0]; // 0xA5F0
    let g = sensor_capture(&sensor(), &bits, true, 2);
    let reg = logica_protocols::registry_with(vec![compile(&template())]);
    let store = g.store().unwrap();
    let a = logica_protocols::analyse::analyse_capture(&store, &g.meta(), &reg).unwrap();
    let d = a.buses[0].chosen[0].clone();
    assert_eq!(d.protocol_id, "my-sensor");
    for b in [
        logica_protocols::analyse::Batching::Whole,
        logica_protocols::analyse::Batching::Edges(1),
        logica_protocols::analyse::Batching::WithSnapshots(3),
    ] {
        let r = logica_protocols::analyse::decode_store(
            &reg,
            &store,
            &g.meta(),
            &d.protocol_id,
            &d.roles,
            &d.settings,
            b,
        )
        .unwrap();
        assert_eq!(
            r.packets.iter().map(|p| p.streams[0].bytes.clone()).collect::<Vec<_>>(),
            [vec![0xa5, 0xf0], vec![0xa5, 0xf0]]
        );
    }
    // not found without it
    let l = generated(&g);
    assert!(!chosen(&l).iter().any(|d| d.protocol_id == "my-sensor"));
}

#[test]
fn a_frame_cut_short_is_flagged() {
    let g = sensor_capture(&sensor(), &[1, 0, 1], false, 0);
    let reg = logica_protocols::registry_with(vec![compile(&template())]);
    let r = logica_protocols::analyse::decode_store(
        &reg,
        &g.store().unwrap(),
        &g.meta(),
        "my-sensor",
        &roles(&[("data", Some(0))]),
        &Settings::new(),
        logica_protocols::analyse::Batching::Whole,
    )
    .unwrap();
    assert_eq!(r.problem_counts.get(&ProblemCode::Incomplete), Some(&1));
}

#[test]
fn the_tolerance_setting_widens_the_timing_windows() {
    let mut stretched = sensor();
    let b = stretched.frames[0].bit.as_mut().unwrap();
    b.zero = [20.0 * 1.3, 40.0 * 1.3];
    b.one = [60.0 * 1.3, 80.0 * 1.3];
    b.other = [20.0 * 1.3, 80.0 * 1.3];
    let g = sensor_capture(&stretched, &[1; 16], false, 0);
    let reg = logica_protocols::registry_with(vec![compile(&template())]);
    let run = |tol: f64| {
        logica_protocols::analyse::decode_store(
            &reg,
            &g.store().unwrap(),
            &g.meta(),
            "my-sensor",
            &roles(&[("data", Some(0))]),
            &settings(&[("tolerance", tol.into())]),
            logica_protocols::analyse::Batching::Whole,
        )
        .unwrap()
    };
    assert_eq!(run(0.0).problem_counts.get(&ProblemCode::Incomplete), Some(&1));
    assert!(run(40.0).problem_counts.is_empty());
}

#[test]
fn dmx512_is_not_claimed_for_an_ordinary_9600_baud_line() {
    let mut l = Lane::high(1e6);
    uart_send(&mut l, 0.001, &text_bytes("hello there\r\n"), UartOptions::baud(9600.0));
    let c = generated(&make_capture("t", 1e6, 0.05, vec![(0, "TX", l)]));
    assert_eq!(
        c.analysis.buses[0].chosen.iter().map(|d| d.protocol_id.as_str()).collect::<Vec<_>>(),
        ["uart"]
    );
    let r = c.analysis.buses[0].rejections.iter().find(|r| r.protocol_id == "dmx512").unwrap();
    assert!(r.because.contains("baud 250000"), "{}", r.because);
}
