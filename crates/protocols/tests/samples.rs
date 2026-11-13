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

#[test]
fn i2c_bus_100khz_scl_1_sda_2() {
    let l = sample("3_I2C_bus.demo").unwrap();
    let d = only(&l, "i2c", None);
    assert_eq!(d.roles, roles(&[("scl", Some(1)), ("sda", Some(2))]));
    assert!(evidence_text(&d).contains("100 kHz: I2C standard mode"), "{}", evidence_text(&d));
    assert!(d.warnings.join(" ").contains("0x27 got no acknowledge"), "{:?}", d.warnings);
    let r = decode(&l, &d);
    no_problems(&r);
    let p = &r.packets;
    assert_eq!(p.iter().map(|x| at(&l, x, 2)).collect::<Vec<_>>(), [0.1, 0.25, 0.5, 0.75]);
    assert_eq!(p[0].title, "Write to 0x27 (no answer)");
    assert!(p[0].streams.is_empty());
    assert_eq!(stream(&p[1], "Write to 0x50"), "00 10 4F 50 45 4E 20 53 45 53 41 4D 45");
    assert_eq!(p[2].title, "Write to 0x50, Read from 0x50");
    assert_eq!(stream(&p[2], "Write to 0x50"), "00 10");
    assert_eq!(
        logica_protocols::format::as_text(&p[2].stream("Read from 0x50").unwrap().bytes),
        "OPEN SESAME"
    );
    assert_eq!(stream(&p[3], "Write to 0x48"), "00");
    assert_eq!(stream(&p[3], "Read from 0x48"), "19 80");
    // the master NACKs the last byte of each read, and that is explained as normal
    let nacks: Vec<&Frame> = r.frames.iter().filter(|f| f.kind == FrameKind::Nack).collect();
    assert_eq!(nacks.len(), 3);
    assert!(nacks[0].detail.as_deref().unwrap().contains("no device answered"));
    assert!(nacks[1].detail.as_deref().unwrap().contains("This is normal"));
    assert_eq!(r.frames.iter().filter(|f| f.kind == FrameKind::RepeatedStart).count(), 2);
    // nothing useful with SCL and SDA swapped
    let r = decode_as(&l, "i2c", &roles(&[("scl", Some(2)), ("sda", Some(1))]), &Settings::new());
    assert!(r.frames.iter().filter(|f| f.kind == FrameKind::Data).count() < 5);
}

#[test]
fn spi_wrong_mode_finds_mode_1_despite_the_saved_mode_0() {
    let l = sample("4_SPI_wrong_mode.demo").unwrap();
    assert_eq!(l.meta.saved_decoders[0].options.iter().find(|(k, _)| k == "cpha").unwrap().1, "0");
    let d = only(&l, "spi", None);
    assert_eq!(d.roles, roles(&[("clk", Some(3)), ("cs", Some(5)), ("mosi", Some(6)), ("miso", Some(4))]));
    assert_eq!(
        (t(&d, "mode"), t(&d, "bitOrder"), n(&d, "wordSize"), t(&d, "csPolarity")),
        ("1", "msb", 8.0, "low")
    );
    let reason = &d.notes["mode"].reason;
    assert!(
        reason.contains("changes on the rising edge") && reason.contains("read on the falling edge"),
        "{reason}"
    );
    let r = decode(&l, &d);
    no_problems(&r);
    let p = &r.packets;
    assert_eq!(p.iter().map(|x| at(&l, x, 4)).collect::<Vec<_>>(), [0.15, 0.4, 0.4002, 0.7]);
    assert_eq!(
        p.iter().map(|x| stream(x, "MOSI")).collect::<Vec<_>>(),
        ["9F 00 00 00", "06", "05 00", &format!("03 00 10 00{}", " 00".repeat(16))]
    );
    assert_eq!(p[..3].iter().map(|x| stream(x, "MISO")).collect::<Vec<_>>(), ["00 EF 40 18", "00", "00 02"]);
    assert_eq!(
        logica_protocols::format::as_text(&p[3].stream("MISO").unwrap().bytes[4..]),
        "MODE 1 UNLOCKED!"
    );
    // the garbage from the answer key with the saved mode 0
    let r = decode_as(&l, "spi", &d.roles, &with(&d, "mode", "0".into()));
    assert_eq!(stream(&r.packets[0], "MOSI"), "4F 80 00 00");
    assert_eq!(stream(&r.packets[0], "MISO"), "00 77 A0 0C");
}

#[test]
fn mystery_c_i2c_at_400khz_sda_4_scl_5() {
    let l = sample("mystery_C.demo").unwrap();
    let d = only(&l, "i2c", None);
    assert_eq!(d.roles, roles(&[("scl", Some(5)), ("sda", Some(4))]));
    assert!(evidence_text(&d).contains("400 kHz: I2C fast mode"), "{}", evidence_text(&d));
    let r = decode(&l, &d);
    no_problems(&r);
    let p = &r.packets;
    assert_eq!(p.iter().map(|x| at(&l, x, 2)).collect::<Vec<_>>(), [0.12, 0.4, 0.66]);
    assert_eq!(stream(&p[0], "Write to 0x68"), "00");
    assert_eq!(stream(&p[0], "Read from 0x68"), "56 34 12 04 12 11 26");
    assert_eq!(stream(&p[1], "Write to 0x68"), "11");
    assert_eq!(stream(&p[1], "Read from 0x68"), "18 40");
    assert_eq!(stream(&p[2], "Write to 0x57"), "00 00");
    assert_eq!(
        logica_protocols::format::as_text(&p[2].stream("Read from 0x57").unwrap().bytes),
        "CALIBRATED 2026"
    );
}

#[test]
fn mystery_d_pwm_spi_mode_3_and_a_uart_log() {
    let l = sample("mystery_D.demo").unwrap();
    let buses: Vec<Vec<u32>> = l.analysis.buses.iter().map(|b| b.bus.channels.clone()).collect();
    assert_eq!(buses, vec![vec![0], vec![2, 3, 5, 6], vec![4]]);
    // channel 0: 1 kHz PWM at 30 % duty
    let d = only(&l, "pwm", Some(0));
    let ev0 = &d.evidence[0].text;
    assert!(ev0.contains("1 ms (1 kHz)") && ev0.contains("300 µs") && ev0.contains("30.0 % duty"), "{ev0}");
    let r = decode(&l, &d);
    assert!(r.frames.len() > 900);
    assert!(r.frames.iter().all(|f| (f.value.unwrap() - 0.3).abs() < 1e-6));
    // channels 2, 3, 5, 6: SPI mode 3
    let d = only(&l, "spi", None);
    assert_eq!(d.roles, roles(&[("clk", Some(5)), ("cs", Some(3)), ("mosi", Some(6)), ("miso", Some(2))]));
    assert_eq!((t(&d, "mode"), t(&d, "bitOrder"), n(&d, "wordSize")), ("3", "msb", 8.0));
    assert!(evidence_text(&d).contains("1 MHz"));
    let r = decode(&l, &d);
    no_problems(&r);
    assert_eq!(r.packets.iter().map(|p| at(&l, p, 3)).collect::<Vec<_>>(), [0.1, 0.15, 0.151, 0.5]);
    assert_eq!(
        r.packets.iter().map(|p| stream(p, "MOSI")).collect::<Vec<_>>(),
        ["8F FF", "20 57", "23 08", "E8 FF FF FF FF FF FF"]
    );
    let miso = ["FF 33", "FF FF", "FF FF", "FF 00 01 00 FE 80 3E"];
    assert_eq!(r.packets.iter().map(|p| stream(p, "MISO")).collect::<Vec<_>>(), miso);
    // mode 0 gives the same bytes, as the answer key says
    let r = decode_as(&l, "spi", &d.roles, &with(&d, "mode", "0".into()));
    assert_eq!(r.packets.iter().map(|p| stream(p, "MISO")).collect::<Vec<_>>(), miso);
    // channel 4: the UART debug log
    let d = only(&l, "uart", Some(4));
    assert_eq!((n(&d, "baud"), t(&d, "dataBits"), t(&d, "parity")), (115_200.0, "8", "none"));
    let r = decode(&l, &d);
    no_problems(&r);
    assert_eq!(texts(&r), ["accel init ok\\r\\n", "Z=1000mg\\r\\n"]);
    assert_eq!(r.packets.iter().map(|p| at(&l, p, 2)).collect::<Vec<_>>(), [0.16, 0.51]);
}

#[test]
fn real_i2c_demo_despite_the_start_up_artefact() {
    let Some(l) = sample("I2C.demo") else { return };
    let saved = &l.meta.saved_decoders[0];
    let ch = |k: &str| saved.channels.iter().find(|(x, _)| x == k).unwrap().1;
    assert!(l.meta.artefact.is_some());
    let d = only(&l, "i2c", None);
    assert_eq!(d.roles, roles(&[("scl", ch("scl")), ("sda", ch("sda"))]));
    let r = decode(&l, &d);
    no_problems(&r);
    assert_eq!(r.packets.len(), 40);
    // each transaction writes 01 n to address 0x01, with n counting up
    let second: Vec<u8> = r
        .packets
        .iter()
        .map(|p| {
            assert_eq!(p.title, "Write to 0x01");
            let b = &p.streams[0].bytes;
            assert_eq!(b[0], 0x01);
            b[1]
        })
        .collect();
    for w in second.windows(2) {
        assert_eq!(w[1], w[0].wrapping_add(1));
    }
}

#[test]
fn real_spi_demo_mode_0_and_the_sample_rate_warning() {
    let Some(l) = sample("SPI.demo") else { return };
    let saved = &l.meta.saved_decoders[0];
    let ch = |k: &str| saved.channels.iter().find(|(x, _)| x == k).unwrap().1;
    let d = only(&l, "spi", None);
    assert_eq!(role(&d, "clk"), ch("clk"));
    assert_eq!(role(&d, "cs"), ch("cs"));
    // one data line: it cannot be told whether it is MOSI or MISO, so accept either
    assert_eq!(
        [role(&d, "mosi"), role(&d, "miso")].into_iter().flatten().collect::<Vec<_>>(),
        [ch("miso").unwrap()]
    );
    assert!(d.notes["role:mosi"].confidence < 0.5);
    assert_eq!(t(&d, "mode"), "0");
    assert!(d.warnings.join(" ").contains("2.5 samples per period"), "{:?}", d.warnings);
    let r = decode(&l, &d);
    assert_eq!(r.packets.len(), 40);
    // Four copies of one byte per frame, counting up. At 2.5 samples per clock, one bit in the
    // last byte of frames 12 to 14 lands on the clock edge itself and reads wrongly. The decoder
    // flags exactly those words as timing problems, and no others.
    let words: Vec<&Vec<u8>> = r.packets.iter().map(|p| &p.streams[0].bytes).collect();
    for w in words.windows(2) {
        assert_eq!(w[1][0], w[0][0].wrapping_add(1));
    }
    let odd: Vec<(usize, String)> = words
        .iter()
        .enumerate()
        .filter(|(_, b)| b.iter().any(|&x| x != b[0]))
        .map(|(i, b)| (i, hexes(b)))
        .collect();
    assert_eq!(
        odd,
        [(11, "C9 C9 C9 D9".to_string()), (12, "CA CA CA CE".into()), (13, "CB CB CB CF".into())]
    );
    assert_eq!(
        r.problem_counts.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
        [(ProblemCode::Timing, 3)]
    );
    let flagged: Vec<(u64, String)> = r
        .frames
        .iter()
        .filter(|f| f.problems.iter().any(|p| p.code == ProblemCode::Timing))
        .map(|f| (f.packet, f.label.clone()))
        .collect();
    assert_eq!(flagged, [(11, "D9".to_string()), (12, "CE".into()), (13, "CF".into())]);
    let msg = &r.frames.iter().find(|f| !f.problems.is_empty()).unwrap().problems[0].message;
    assert!(msg.contains("same sample as the clock edge"), "{msg}");
}

#[test]
fn never_reads_the_saved_decoder_settings() {
    for file in ["2_UART_mystery_baud.demo", "4_SPI_wrong_mode.demo"] {
        let l = sample(file).unwrap();
        let mut meta = l.meta.clone();
        meta.saved_decoders.clear();
        let a = logica_protocols::analyse::analyse_capture(&l.store, &meta, registry()).unwrap();
        let without: Vec<Settings> = a.chosen().map(|d| d.settings.clone()).collect();
        let with: Vec<Settings> = chosen(&l).into_iter().map(|d| d.settings).collect();
        assert_eq!(without, with, "{file}");
    }
}
