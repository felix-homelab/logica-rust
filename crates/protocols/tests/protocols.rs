//! Every protocol's generated example is detected and decoded (Signal Detective's
//! `tests/protocols.test.ts`), wrong settings show recognisable symptoms, and quiet or noisy lines
//! are not confidently called anything. Every decode runs in every batching (doc 06).

#![allow(clippy::cast_possible_truncation)]

mod common;

use common::*;
use logica_core::Level;
use logica_protocols::examples::{EXAMPLES, example};
use logica_protocols::synth::*;
use logica_protocols::types::*;

fn detect_and_decode(id: &str) -> (Loaded, Detection, DecodeResult) {
    let ex = example(id).unwrap();
    let l = generated(&(ex.build)());
    let d = chosen(&l);
    assert_eq!(d.iter().map(|x| x.protocol_id.as_str()).collect::<Vec<_>>(), [ex.protocol_id], "{id}");
    assert!(d[0].confidence >= 0.6, "{id}: confidence {}", d[0].confidence);
    let r = decode(&l, &d[0]);
    (l, d[0].clone(), r)
}

#[test]
fn every_example_has_a_test_here() {
    let mut ids: Vec<&str> = EXAMPLES.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["can", "dht22", "dmx512", "i2s", "jtag", "nec", "onewire", "servo", "swd", "ws2812"]);
}

#[test]
fn nec() {
    let (_, _, r) = detect_and_decode("nec");
    assert_eq!(
        labels(&r, 0),
        [
            "leader",
            "space",
            "end",
            "address 00",
            "address inverted FF",
            "command 45",
            "command inverted BA",
            "leader",
            "repeat",
            "end"
        ]
    );
    assert_eq!(r.packets.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["Command", "Repeat"]);
}

#[test]
fn dht22() {
    let (_, _, r) = detect_and_decode("dht22");
    let f: Vec<&str> = r
        .frames
        .iter()
        .filter(|x| x.kind == FrameKind::Field)
        .map(|x| x.detail.as_deref().unwrap())
        .collect();
    assert_eq!(
        f,
        [
            "humidity = 65.2 %RH (DHT22) (raw 652)",
            "temperature = 23.1 °C (DHT22) (raw 231)",
            "checksum = 117 (raw 117)"
        ]
    );
}

#[test]
fn ws2812() {
    let (_, _, r) = detect_and_decode("ws2812");
    assert_eq!(labels(&r, 0)[..9], ["G 00", "R FF", "B 00", "G FF", "R 00", "B 00", "G 00", "R 00", "B FF"]);
    assert_eq!(r.packets.len(), 2);
}

#[test]
fn onewire() {
    let (_, _, r) = detect_and_decode("onewire");
    let cmds: Vec<&str> =
        r.frames.iter().filter(|f| f.kind == FrameKind::Command).map(|f| f.label.as_str()).collect();
    assert_eq!(cmds, ["CC Skip ROM", "44 Convert T", "CC Skip ROM", "BE Read scratchpad"]);
    assert_eq!(r.packets[1].streams[0].bytes[2..4], [0x91, 0x01]);
    assert_eq!(r.frames.iter().filter(|f| f.kind == FrameKind::Presence).count(), 2);
}

#[test]
fn can() {
    let (_, _, r) = detect_and_decode("can");
    assert_eq!(
        r.packets.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(),
        ["ID 0x123", "ID 0x7DF", "ID 0x18DAF110"]
    );
    assert_eq!(
        r.packets.iter().map(|p| p.streams[0].bytes.clone()).collect::<Vec<_>>(),
        [vec![0x11, 0x22, 0x33], vec![0x02, 0x01, 0x0c, 0, 0, 0, 0, 0], vec![0xde, 0xad]]
    );
    assert_eq!(
        labels(&r, 0).into_iter().filter(|l| l.starts_with("CRC")).collect::<Vec<_>>(),
        ["CRC ok", "CRC ok", "CRC ok"]
    );
}

#[test]
fn i2s() {
    let (_, d, r) = detect_and_decode("i2s");
    assert_eq!((n(&d, "wordBits"), t(&d, "format")), (16.0, "i2s"));
    let left: Vec<f64> =
        r.frames.iter().filter(|f| f.label.starts_with('L')).map(|f| f.value.unwrap()).collect();
    let right: Vec<f64> =
        r.frames.iter().filter(|f| f.label.starts_with('R')).map(|f| f.value.unwrap()).collect();
    assert_eq!(left[..4], [1000.0, 2000.0, 3000.0, 4000.0]);
    assert_eq!(right[1..4], [-1000.0, -2000.0, -3000.0]);
}

#[test]
fn swd() {
    let (_, _, r) = detect_and_decode("swd");
    assert_eq!(
        labels(&r, 0),
        [
            "line reset",
            "DP R IDCODE",
            "OK",
            "2BA01477",
            "DP W ABORT",
            "OK",
            "0000001E",
            "DP W SELECT",
            "OK",
            "00000000",
            "AP R DRW",
            "WAIT"
        ]
    );
}

#[test]
fn jtag() {
    let (_, d, r) = detect_and_decode("jtag");
    assert_eq!(d.roles, roles(&[("tck", Some(0)), ("tms", Some(1)), ("tdi", Some(2)), ("tdo", Some(3))]));
    assert_eq!(labels(&r, 0), ["IR in E out 1", "DR in 00000000 out 4BA00477"]);
    assert!(labels(&r, 1).contains(&"Shift-DR".to_string()));
}

#[test]
fn dmx512() {
    let (_, d, r) = detect_and_decode("dmx512");
    assert_eq!((n(&d, "baud"), t(&d, "stopBits")), (250_000.0, "2"));
    assert_eq!(labels(&r, 0), ["break", "00", "FF", "80", "40", "00", "break", "00", "FF", "80", "40", "00"]);
}

#[test]
fn servo() {
    let (_, d, r) = detect_and_decode("servo");
    assert!(evidence_text(&d).contains("hobby servo"));
    assert!(labels(&r, 0).iter().all(|l| l == "7.5 %"));
}

#[test]
fn every_built_in_protocol_is_complete() {
    let mut ids = std::collections::BTreeSet::new();
    for p in registry().all() {
        assert!(ids.insert(p.id().to_string()), "duplicate id {}", p.id());
        let i = p.info();
        for (k, v) in [
            ("what", &i.what),
            ("confirm", &i.confirm),
            ("decoderTips", &i.decoder_tips),
            ("lookalikes", &i.lookalikes),
        ] {
            assert!(v.len() > 10, "{}: {k} is missing", p.id());
        }
        assert!(p.has_decoder() || p.alias_of().is_some(), "{} has no decoder", p.id());
        if p.has_decoder() {
            assert!(!p.roles().is_empty(), "{}", p.id());
            for r in p.roles() {
                assert!(!r.noun.is_empty() && !r.label.is_empty(), "{}: role {}", p.id(), r.key);
            }
        }
        let keys: std::collections::BTreeSet<&str> = p.settings().iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys.len(), p.settings().len(), "{}: duplicate setting", p.id());
    }
}

#[test]
fn every_decoder_can_be_snapshotted_before_any_data() {
    use logica_protocols::stream::{BoundChannel, DecoderSetup};
    for p in registry().all().iter().filter(|p| p.has_decoder()) {
        let channels: Vec<BoundChannel> = p
            .roles()
            .iter()
            .enumerate()
            .map(|(i, r)| BoundChannel { role: r.key.clone(), channel: i as u32, name: String::new() })
            .collect();
        let setup = DecoderSetup {
            sample_rate_hz: 1e6,
            start: 0,
            start_levels: vec![Level::High; channels.len()],
            settings: SettingSpec::with_defaults(p.settings(), &Settings::new()),
            channels,
        };
        let d = p.decoder(&setup, None).unwrap_or_else(|e| panic!("{}: {e}", p.id()));
        let snap = d.snapshot();
        assert!(!snap.is_null(), "{}", p.id());
        let mut back = p.decoder(&setup, Some(snap)).unwrap_or_else(|e| panic!("{}: {e}", p.id()));
        back.finish(10);
        back.drain();
    }
}

// ------------------------------------------------------------------ wrong settings

fn uart_capture(o: UartOptions, text: &str) -> Loaded {
    let rate = 4e6;
    let mut l = Lane::new(rate, if o.invert { Level::Low } else { Level::High });
    uart_send(&mut l, 0.001, &text_bytes(text), o);
    generated(&make_capture("t", rate, 0.01, vec![(0, "TX", l)]))
}

fn decode_uart(l: &Loaded, s: &[(&str, SettingValue)]) -> DecodeResult {
    decode_as(l, "uart", &roles(&[("data", Some(0))]), &settings(s))
}

#[test]
fn uart_inverted_line_is_detected_and_decoded_with_invert_on() {
    let l = uart_capture(UartOptions { invert: true, ..UartOptions::baud(9600.0) }, "Hello, world!\r\n");
    let d = only(&l, "uart", None);
    assert_eq!((n(&d, "baud"), b(&d, "invert")), (9600.0, true));
    assert_eq!(registry().get("uart").unwrap().title(&d.settings), "UART, inverted");
    assert_eq!(decode(&l, &d).packets[0].streams[0].bytes, text_bytes("Hello, world!\r\n"));
    // decoded without invert: frame errors
    let r = decode_uart(&l, &[("baud", 9600.0.into())]);
    assert!(count(&r, ProblemCode::FrameError) > 0);
    // the alias decodes as its target
    let r = decode_as(
        &l,
        "uart-inverted",
        &roles(&[("data", Some(0))]),
        &settings(&[("baud", 9600.0.into()), ("invert", true.into())]),
    );
    assert!(r.frames.len() > 5);
}

#[test]
fn uart_7e1_is_found() {
    let l = uart_capture(
        UartOptions { data_bits: 7, parity: "even", ..UartOptions::baud(4800.0) },
        "Hello, world!\r\n",
    );
    let d = only(&l, "uart", None);
    assert_eq!((n(&d, "baud"), t(&d, "dataBits"), t(&d, "parity")), (4800.0, "7", "even"));
}

#[test]
fn uart_baud_3_percent_off_decodes_10_percent_does_not() {
    let l = uart_capture(UartOptions::baud(9600.0), "Hello, world!\r\n");
    assert!(decode_uart(&l, &[("baud", (9600.0 * 1.03).into())]).problem_counts.is_empty());
    assert!(!decode_uart(&l, &[("baud", (9600.0 * 1.1).into())]).problem_counts.is_empty());
}

#[test]
fn uart_msb_first_reverses_the_bits() {
    let l = uart_capture(UartOptions::baud(9600.0), "A");
    let r = decode_uart(&l, &[("baud", 9600.0.into()), ("bitOrder", "msb".into())]);
    assert_eq!(r.frames[0].value, Some(f64::from(0x82))); // 0x41 reversed
}

#[test]
fn uart_baud_beyond_the_sample_rate_is_reported_not_crashed_on() {
    let l = uart_capture(UartOptions::baud(9600.0), "Hello, world!\r\n");
    let r = decode_uart(&l, &[("baud", 10e6.into())]);
    assert!(r.frames.is_empty());
    assert!(r.notes.join(" ").contains("higher than the sample rate"));
}

fn spi_capture(mode: u8, bytes: &[u8], msb: bool) -> Loaded {
    let rate = 10e6;
    let (mut clk, mut cs, mut mosi) = (Lane::low(rate), Lane::high(rate), Lane::low(rate));
    spi_frame(&mut clk, Some(&mut cs), Some(&mut mosi), None, 0.0001, 500e3, mode, bytes, &[], msb);
    generated(&make_capture("t", rate, 0.001, vec![(0, "CLK", clk), (1, "CS", cs), (2, "MOSI", mosi)]))
}

#[test]
fn spi_16_bit_words_lsb_first() {
    let l = spi_capture(0, &[0x12, 0x34], false);
    let r = decode_as(
        &l,
        "spi",
        &roles(&[("clk", Some(0)), ("cs", Some(1)), ("mosi", Some(2)), ("miso", None)]),
        &settings(&[("mode", "0".into()), ("wordSize", 16.0.into()), ("bitOrder", "lsb".into())]),
    );
    assert_eq!(r.frames[0].value, Some(f64::from(0x3412)));
}

#[test]
fn spi_without_a_data_line_is_a_setup_error() {
    let l = generated(&(example("i2s").unwrap().build)());
    let e = logica_protocols::analyse::decode_store(
        registry(),
        &l.store,
        &l.meta,
        "spi",
        &roles(&[("clk", Some(0)), ("cs", None), ("mosi", None), ("miso", None)]),
        &Settings::new(),
        logica_protocols::analyse::Batching::Whole,
    )
    .unwrap_err();
    assert_eq!(e, "Choose a channel for MOSI or MISO (or both).");
}

#[test]
fn spi_wrong_sampling_edge_shifts_the_bits_and_a_wrong_cpol_is_pointed_out() {
    // Mode 1 data changes just after the rising edge. Reading on the rising edge (mode 0) catches
    // the previous bit, so 9F turns into 4F, as in the answer key for 4_SPI_wrong_mode.
    let l = spi_capture(1, &[0x9f], true);
    let label = |mode: &str| {
        decode_as(
            &l,
            "spi",
            &roles(&[("clk", Some(0)), ("cs", Some(1)), ("mosi", Some(2))]),
            &settings(&[("mode", mode.into())]),
        )
    };
    assert_eq!(label("1").frames[0].label, "9F");
    assert_eq!(label("0").frames[0].label, "4F");
    // mode 2 reads on the same (falling) edge as mode 1, but expects the clock to rest high
    assert_eq!(label("2").frames[0].label, "9F");
    assert!(label("2").notes.join(" ").contains("clock rests low, but mode 2 expects it to rest high"));
}

#[test]
fn spi_without_chip_select_splits_at_clock_pauses() {
    let rate = 10e6;
    let (mut clk, mut mosi) = (Lane::low(rate), Lane::low(rate));
    let t = spi_frame(&mut clk, None, Some(&mut mosi), None, 0.0001, 500e3, 0, &[0xa5, 0x5a], &[], true);
    spi_frame(&mut clk, None, Some(&mut mosi), None, t + 0.0002, 500e3, 0, &[0x3c], &[], true);
    let l = generated(&make_capture("t", rate, 0.001, vec![(0, "CLK", clk), (2, "MOSI", mosi)]));
    let r = decode_as(
        &l,
        "spi",
        &roles(&[("clk", Some(0)), ("mosi", Some(2))]),
        &settings(&[("mode", "0".into())]),
    );
    assert_eq!(r.packets.iter().map(|p| stream(p, "MOSI")).collect::<Vec<_>>(), ["A5 5A", "3C"]);
    assert_eq!(r.packets[0].title, "Clock burst");
    assert!(r.notes.join(" ").contains("No chip select"));
}

#[test]
fn i2c_a_byte_cut_short_is_reported() {
    let rate = 4e6;
    let (mut scl, mut sda) = (Lane::high(rate), Lane::high(rate));
    i2c_transaction(
        &mut scl,
        &mut sda,
        0.0001,
        100e3,
        &[I2cTransfer { address: 0x3c, read: false, data: vec![0x00, 0xaf], nack: false }],
    );
    // a glitchy stop in the middle of the next transfer
    let t = 0.001;
    sda.set(t, 0u8);
    scl.set(t + 5e-6, 0u8);
    sda.set(t + 6e-6, 1u8);
    scl.set(t + 10e-6, 1u8);
    scl.set(t + 15e-6, 0u8);
    scl.set(t + 20e-6, 1u8);
    sda.set(t + 22e-6, 0u8);
    sda.set(t + 25e-6, 1u8);
    let l = generated(&make_capture("t", rate, 0.002, vec![(0, "SCL", scl), (1, "SDA", sda)]));
    let r = decode_as(&l, "i2c", &roles(&[("scl", Some(0)), ("sda", Some(1))]), &Settings::new());
    assert_eq!(r.packets[0].title, "Write to 0x3C");
    assert_eq!(count(&r, ProblemCode::Incomplete), 1);
}

#[test]
fn can_a_corrupted_bit_gives_a_crc_or_stuffing_error() {
    let rate = 8e6;
    let mut l = Lane::high(rate);
    can_frame(&mut l, 0.0001, 500e3, 0x100, &[0x55], false);
    // move one edge in the data field by a whole bit: it lands on the next edge, and two changes
    // on one sample cancel out (Signal Detective kept both; the store cannot hold that)
    assert_eq!(l.edges[20] + 16, l.edges[21]);
    l.edges.drain(20..22);
    let c = generated(&make_capture("t", rate, 0.001, vec![(0, "RX", l)]));
    let r = decode_as(&c, "can", &roles(&[("rx", Some(0))]), &settings(&[("bitrate", 500_000.0.into())]));
    assert!(count(&r, ProblemCode::Checksum) + count(&r, ProblemCode::Stuffing) > 0);
}

#[test]
fn unknown_protocols_and_missing_channels_give_messages() {
    let l = generated(&(example("nec").unwrap().build)());
    let d = |p: &str, r: &[(&str, Option<u32>)]| {
        logica_protocols::analyse::decode_store(
            registry(),
            &l.store,
            &l.meta,
            p,
            &roles(r),
            &Settings::new(),
            logica_protocols::analyse::Batching::Whole,
        )
        .unwrap_err()
    };
    assert_eq!(d("nope", &[]), "There is no decoder for nope.");
    assert_eq!(d("uart", &[("data", Some(9))]), "Channel 9 (the data line) is not in this capture.");
    assert_eq!(d("uart", &[]), "Choose a channel for the data line.");
}

// ------------------------------------------------------------------ quiet and noisy

#[test]
fn a_channel_that_never_changes_is_listed_explained_by_nothing() {
    let l = generated(&make_capture("t", 1e6, 0.01, vec![(3, "X", Lane::high(1e6))]));
    assert_eq!(l.analysis.buses.len(), 1);
    assert!(l.analysis.buses[0].chosen.is_empty());
    assert!(l.analysis.buses[0].bus.reason.contains("never changes"));
}

#[test]
fn random_noise_is_not_confidently_called_anything() {
    let mut l = Lane::high(1e6);
    let mut seed: u64 = 7;
    let mut rnd = || {
        seed = (seed.wrapping_mul(1_103_515_245).wrapping_add(12_345)) & 0x7fff_ffff;
        seed as f64 / f64::from(0x7fff_ffff)
    };
    let mut t = 0.0001;
    for i in 0..400 {
        t += 1e-6 * (1.0 + (rnd() * 200.0).floor());
        l.set(t, u8::from(i % 2 == 1));
    }
    let c = generated(&make_capture("t", 1e6, 0.2, vec![(0, "N", l)]));
    let best = c.analysis.buses[0].chosen.first();
    assert!(best.is_none_or(|d| d.confidence < 0.75), "{best:?}");
}
