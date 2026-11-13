//! Every Signal Detective sample capture against its answer key and Signal Detective's test
//! expectations (`../signal-detective/tests/samples.test.ts`), through the streaming decoders in
//! every batching. The three FNIRSI files are checked when present (locally only, doc 10).

mod common;

use common::*;
use logica_protocols::types::*;

#[test]
fn uart_easy_115200_8n1_on_channel_0() {
    let l = sample("1_UART_easy.demo").unwrap();
    let d = only(&l, "uart", None);
    assert_eq!(d.roles, roles(&[("data", Some(0))]));
    assert_eq!(
        (
            n(&d, "baud"),
            t(&d, "dataBits"),
            t(&d, "parity"),
            t(&d, "stopBits"),
            b(&d, "invert"),
            t(&d, "bitOrder")
        ),
        (115_200.0, "8", "none", "1", false, "lsb")
    );
    assert_eq!(chosen(&l).len(), 1);
    let r = decode(&l, &d);
    no_problems(&r);
    assert_eq!(texts(&r), ["AT+LED=ON\\r\\n", "OK\\r\\n", "TEMP=23.5C\\r\\n", "KEY: SWORDFISH\\r\\n"]);
    assert_eq!(r.packets.iter().map(|p| at(&l, p, 3)).collect::<Vec<_>>(), [0.08, 0.1, 0.45, 0.8]);
}

#[test]
fn uart_mystery_baud_57600_despite_the_saved_115200() {
    let l = sample("2_UART_mystery_baud.demo").unwrap();
    let saved = &l.meta.saved_decoders[0];
    assert_eq!(saved.options.iter().find(|(k, _)| k == "baudrate").unwrap().1, "115200");
    let d = only(&l, "uart", None);
    assert_eq!(
        (n(&d, "baud"), t(&d, "dataBits"), t(&d, "parity"), t(&d, "stopBits")),
        (57_600.0, "8", "none", "1")
    );
    assert!(d.notes["baud"].reason.contains("17.36 µs"), "{}", d.notes["baud"].reason);
    let r = decode(&l, &d);
    no_problems(&r);
    assert_eq!(texts(&r), ["BAUD RATE DETECTIVE\\r\\n", "CODE=7391\\r\\n"]);
    assert_eq!(r.packets.iter().map(|p| at(&l, p, 3)).collect::<Vec<_>>(), [0.2, 0.6]);
    // garbage and frame errors with the saved 115200
    let r = decode_as(&l, "uart", &d.roles, &with(&d, "baud", 115_200.0.into()));
    assert!(count(&r, ProblemCode::FrameError) > 5);
    assert!(!texts(&r).concat().contains("DETECTIVE"));
}

#[test]
fn mystery_a_two_uart_lines_at_38400() {
    let l = sample("mystery_A.demo").unwrap();
    for ch in [0, 1] {
        let d = only(&l, "uart", Some(ch));
        assert_eq!(
            (n(&d, "baud"), t(&d, "dataBits"), t(&d, "parity"), t(&d, "stopBits")),
            (38_400.0, "8", "none", "1")
        );
    }
    let notes = l.analysis.notes.join(" ");
    assert!(notes.contains("Channels 0 and 1 ") && notes.contains("(TX and RX)"), "{notes}");
    let reason = &only(&l, "uart", Some(0)).notes["baud"].reason;
    assert!(reason.contains("26.0") && reason.contains(" µs"), "{reason}");
    let host = decode(&l, &only(&l, "uart", Some(0)));
    let module = decode(&l, &only(&l, "uart", Some(1)));
    no_problems(&host);
    no_problems(&module);
    assert_eq!(texts(&host), ["AT\\r\\n", "AT+NAME?\\r\\n", "AT+PSWD?\\r\\n"]);
    assert_eq!(texts(&module), ["OK\\r\\n", "+NAME:HC-05\\r\\nOK\\r\\n", "+PSWD:4821\\r\\nOK\\r\\n"]);
    assert_eq!(host.packets.iter().map(|p| at(&l, p, 2)).collect::<Vec<_>>(), [0.1, 0.3, 0.6]);
    // the module answers about 6 ms later
    for (h, m) in host.packets.iter().zip(&module.packets) {
        let dt = (m.start as f64 - h.start as f64) / l.meta.sample_rate_hz;
        assert!((dt - 0.006).abs() < 0.0005, "{dt}");
    }
}

#[test]
fn mystery_b_modbus_rtu_19200_8e1_on_channel_7() {
    let l = sample("mystery_B.demo").unwrap();
    let d = only(&l, "uart", Some(7));
    assert_eq!(
        (n(&d, "baud"), t(&d, "dataBits"), t(&d, "parity"), t(&d, "stopBits")),
        (19_200.0, "8", "even", "1")
    );
    let ev = evidence_text(&d);
    assert!(ev.contains("11 bit times apart"), "{ev}");
    assert!(ev.contains("4 of 4 packets end with a valid Modbus CRC-16"), "{ev}");
    let r = decode(&l, &d);
    no_problems(&r);
    assert_eq!(
        r.packets.iter().map(|p| hexes(&p.streams[0].bytes)).collect::<Vec<_>>(),
        [
            "01 03 00 00 00 02 C4 0B",
            "01 03 04 00 EB 01 F6 0B D1",
            "01 03 00 00 00 02 C4 0B",
            "01 03 04 00 EE 01 EF DA 1A"
        ]
    );
    assert_eq!(r.packets.iter().map(|p| at(&l, p, 3)).collect::<Vec<_>>(), [0.2, 0.212, 0.7, 0.712]);
    let r = decode_as(&l, "uart", &d.roles, &with(&d, "parity", "none".into()));
    assert!(count(&r, ProblemCode::FrameError) > 0);
    let r = decode_as(&l, "uart", &d.roles, &with(&d, "parity", "odd".into()));
    assert_eq!(count(&r, ProblemCode::ParityError), 34);
}

#[test]
fn real_uart_demo_says_hello() {
    let Some(l) = sample("UART.demo") else { return };
    let saved = &l.meta.saved_decoders[0];
    let opt = |k: &str| saved.options.iter().find(|(x, _)| x == k).unwrap().1.clone();
    let d = only(&l, "uart", None);
    assert_eq!(role(&d, "data"), saved.channels.iter().find(|(k, _)| k == "rx").unwrap().1);
    assert_eq!(n(&d, "baud"), opt("baudrate").parse::<f64>().unwrap());
    assert_eq!(
        (t(&d, "dataBits"), t(&d, "parity"), b(&d, "invert")),
        (opt("data_bits").as_str(), opt("parity").as_str(), false)
    );
    let r = decode(&l, &d);
    no_problems(&r);
    assert_eq!(texts(&r), vec!["hello"; 9]);
}
