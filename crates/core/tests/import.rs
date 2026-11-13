//! Importers against the fixtures (doc 10): every `.demo` sample as Signal Detective read it, the
//! `.sr` and VCD files of `fixtures/formats/`, and malformed and truncated files made here.

#![allow(clippy::cast_possible_truncation)]

use std::io::Write;
use std::path::PathBuf;

use logica_core::capture::NoteLevel;
use logica_core::import::{self, ImportError, Imported, StoreTarget};
use logica_core::io::MemFile;
use logica_core::{Level, Sample};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Prints past the test harness's output capture, so skips are visible in a plain `cargo test`.
fn say(msg: &str) {
    let _ = writeln!(std::io::stderr(), "{msg}");
}

type Importer = fn(
    &dyn logica_core::io::RandomAccess,
    &str,
    &StoreTarget,
    import::Progress<'_>,
) -> Result<Imported, ImportError>;

fn load_bytes(bytes: Vec<u8>, f: Importer) -> Result<Imported, ImportError> {
    let file = MemFile::from_bytes(bytes);
    f(&file, "t", &StoreTarget::memory(), &mut |_, _| {})
}

fn load(rel: &str, f: Importer) -> Result<Imported, ImportError> {
    load_bytes(std::fs::read(fixtures().join(rel)).unwrap(), f)
}

fn demo(name: &str) -> Option<Imported> {
    let local = ["I2C.demo", "SPI.demo", "UART.demo"].contains(&name);
    let path = if local { format!("captures/local/{name}") } else { format!("captures/{name}") };
    if !fixtures().join(&path).exists() {
        assert!(local, "{name} is missing from fixtures/captures");
        say(&format!("skipped: FNIRSI example files not present ({name})"));
        return None;
    }
    Some(load(&path, import::demo::import).unwrap())
}

fn edges(i: &Imported, slot: usize) -> Vec<Sample> {
    i.store.edges_in(slot, 0, i.store.sample_count()).unwrap()
}

/// Signal Detective's spike results: channel, name, initial level, edge count.
type DemoChannel = (u32, &'static str, u8, usize);
const DEMO: &[(&str, f64, &[DemoChannel])] = &[
    ("1_UART_easy.demo", 40e6, &[(0, "RX", 1, 288)]),
    ("2_UART_mystery_baud.demo", 40e6, &[(0, "RX", 1, 214)]),
    ("3_I2C_bus.demo", 10e6, &[(1, "SCL", 1, 642), (2, "SDA", 1, 162)]),
    (
        "4_SPI_wrong_mode.demo",
        50e6,
        &[(3, "CLK", 0, 432), (4, "MISO", 0, 82), (5, "CS#", 1, 8), (6, "MOSI", 0, 14)],
    ),
    ("I2C.demo", 10e6, &[(1, "SCL", 1, 2368), (2, "SDA", 1, 642)]),
    ("SPI.demo", 50e6, &[(3, "CLK", 0, 2560), (4, "MISO", 0, 648), (5, "CS#", 1, 80)]),
    ("UART.demo", 40e6, &[(0, "RX", 1, 288)]),
    ("mystery_A.demo", 40e6, &[(0, "channel 0", 1, 158), (1, "channel 1", 1, 240)]),
    ("mystery_B.demo", 40e6, &[(7, "channel 7", 1, 142)]),
    ("mystery_C.demo", 10e6, &[(4, "channel 4", 1, 150), (5, "channel 5", 1, 624)]),
    (
        "mystery_D.demo",
        50e6,
        &[
            (0, "channel 0", 1, 2000),
            (2, "channel 2", 1, 14),
            (3, "channel 3", 1, 8),
            (4, "channel 4", 1, 152),
            (5, "channel 5", 1, 208),
            (6, "channel 6", 1, 22),
        ],
    ),
];

#[test]
fn every_demo_sample_reads_as_signal_detective_read_it() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures().join("captures"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".demo"))
        .collect();
    on_disk.sort();
    for n in &on_disk {
        assert!(DEMO.iter().any(|(f, _, _)| f == n), "no expectation for {n}");
    }
    for (file, rate, chans) in DEMO {
        let Some(c) = demo(file) else { continue };
        assert_eq!(c.meta.sample_rate_hz, *rate, "{file}");
        assert_eq!(c.store.sample_count(), *rate as u64 + 8, "{file}: depth (1 s) + 8");
        assert_eq!(
            c.meta.channels.iter().map(|ch| ch.index).collect::<Vec<_>>(),
            chans.iter().map(|x| x.0).collect::<Vec<_>>()
        );
        for (slot, (_, name, init, count)) in chans.iter().enumerate() {
            assert_eq!(c.meta.channels[slot].name, *name, "{file}");
            assert_eq!(c.store.initial_level(slot).unwrap(), Level::from(*init == 1), "{file} {name}");
            let e = edges(&c, slot);
            assert_eq!(e.len(), *count, "{file} {name}");
            assert!(
                e.windows(2).all(|w| w[0] < w[1]) && e.last().is_none_or(|&l| l < c.store.sample_count())
            );
            let colour = c.meta.channels[slot].colour.as_deref().unwrap();
            assert!(colour.len() == 7 && colour.starts_with('#'));
        }
        let want_artefact = (*file == "I2C.demo").then_some(0..2049);
        assert_eq!(c.meta.artefact, want_artefact, "{file}");
    }
}

#[test]
fn saved_decoder_settings_are_read_but_kept_apart() {
    let opt = |c: &Imported, k: &str| {
        c.meta.saved_decoders[0].options.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
    };
    let ch = |c: &Imported, k: &str| {
        c.meta.saved_decoders[0].channels.iter().find(|(x, _)| x == k).map(|(_, v)| *v)
    };
    let m2 = demo("2_UART_mystery_baud.demo").unwrap();
    assert_eq!(opt(&m2, "baudrate").as_deref(), Some("115200"));
    let s4 = demo("4_SPI_wrong_mode.demo").unwrap();
    assert_eq!(opt(&s4, "cpha").as_deref(), Some("0"));
    assert!(demo("mystery_A.demo").unwrap().meta.saved_decoders.is_empty());
    if let Some(u) = demo("UART.demo") {
        assert_eq!(u.meta.saved_decoders[0].protocol, "uart");
        assert_eq!((ch(&u, "rx"), ch(&u, "tx")), (Some(Some(0)), Some(None)));
        assert_eq!(opt(&u, "parity").as_deref(), Some("none"));
    }
    if let Some(s) = demo("SPI.demo") {
        assert_eq!(s.meta.saved_decoders[0].protocol, "spi");
        assert_eq!(
            [ch(&s, "clk"), ch(&s, "miso"), ch(&s, "mosi"), ch(&s, "cs")],
            [Some(Some(3)), Some(Some(4)), Some(None), Some(Some(5))]
        );
    }
    if let Some(i) = demo("I2C.demo") {
        assert_eq!((ch(&i, "scl"), ch(&i, "sda")), (Some(Some(1)), Some(Some(2))));
        assert!(i.meta.notes.iter().any(|n| n.message.contains("first 2049 samples")));
    }
}

#[test]
fn progress_reaches_1() {
    let file = MemFile::from_bytes(std::fs::read(fixtures().join("captures/3_I2C_bus.demo")).unwrap());
    let mut seen = Vec::new();
    import::demo::import(&file, "t", &StoreTarget::memory(), &mut |f, _| seen.push(f)).unwrap();
    assert!(!seen.is_empty());
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(*seen.last().unwrap(), 1.0);
}

#[test]
fn truncated_demo_files_fail_clearly() {
    let bytes = std::fs::read(fixtures().join("captures/3_I2C_bus.demo")).unwrap();
    for cut in [10, bytes.len() / 3, bytes.len() / 2, bytes.len() - 10] {
        let e = load_bytes(bytes[..cut].to_vec(), import::demo::import).unwrap_err().to_string();
        assert!(e.contains("ZIP") || e.contains("cut short") || e.contains("damaged"), "cut {cut}: {e}");
    }
}

// ------------------------------------------------------------------ .sr

#[test]
fn sr_mixed_8_channels() {
    let c = load("formats/sr/mixed-8ch-1mhz.sr", import::sr::import).unwrap();
    assert_eq!(c.meta.sample_rate_hz, 1e6);
    assert_eq!(c.store.sample_count(), 30_000);
    let names: Vec<&str> = c.meta.channels.iter().map(|ch| ch.name.as_str()).collect();
    assert_eq!(names, ["TX", "PWM", "HIGH", "3", "4", "5", "6", "7"]);
    // TX idles high and starts its first byte at sample 1000
    assert_eq!(c.store.initial_level(0).unwrap(), Level::High);
    assert_eq!(edges(&c, 0)[0], 1000);
    // PWM: 1 kHz, 25 %, from sample 100
    let pwm: Vec<Sample> = (0..30).flat_map(|k| [100 + 1000 * k, 350 + 1000 * k]).collect();
    assert_eq!(edges(&c, 1), pwm);
    assert_eq!(c.store.initial_level(2).unwrap(), Level::High);
    for slot in 2..8 {
        assert!(edges(&c, slot).is_empty(), "slot {slot}");
    }
}

#[test]
fn sr_walking_ones_with_2_and_3_byte_samples() {
    for (file, n, count, rate) in [("walk-16ch.sr", 16u64, 64u64, 1e3), ("walk-24ch.sr", 24, 48, 2.5e6)] {
        let c = load(&format!("formats/sr/{file}"), import::sr::import).unwrap();
        assert_eq!(
            (c.store.slots() as u64, c.store.sample_count(), c.meta.sample_rate_hz),
            (n, count, rate),
            "{file}"
        );
        for b in 0..n {
            // high exactly at samples s with s % n == b
            let want: Vec<Sample> =
                (0..count).filter(|s| s % n == b || (s % n == (b + 1) % n && *s > 0)).collect();
            let want: Vec<Sample> = want.into_iter().filter(|&s| s > 0).collect();
            assert_eq!(edges(&c, b as usize), want, "{file} channel {b}");
            assert_eq!(c.store.initial_level(b as usize).unwrap(), Level::from(b == 0));
        }
    }
}

#[test]
fn sr_with_only_some_channels_keeps_their_numbers() {
    let c = load("formats/sr/selected-3of8.sr", import::sr::import).unwrap();
    let ch: Vec<(u32, &str)> = c.meta.channels.iter().map(|x| (x.index, x.name.as_str())).collect();
    assert_eq!(ch, [(0, "A"), (2, "B"), (5, "C")]);
    assert_eq!(c.store.sample_count(), 16);
    assert_eq!(edges(&c, 0), vec![1, 8, 9]);
    assert_eq!(edges(&c, 1), vec![2, 3, 10, 11]);
    assert_eq!(edges(&c, 2), vec![5, 6, 13, 14]);
}

#[test]
fn sr_split_over_three_data_files() {
    let c = load("formats/sr/counter-multifile.sr", import::sr::import).unwrap();
    assert_eq!((c.store.sample_count(), c.meta.sample_rate_hz), (9_000_000, 10e6));
    for b in 0..8u64 {
        let period = 4096u64 << b;
        let want: Vec<Sample> = (1..).map(|k| k * period).take_while(|&s| s < 9_000_000).collect();
        assert_eq!(edges(&c, b as usize), want, "bit {b}");
    }
}

fn make_sr(entries: &[(&str, &[u8])]) -> Vec<u8> {
    // A stored ZIP, written here so malformed variants are easy to make.
    let mut out = Vec::new();
    let mut dir = Vec::new();
    for (name, data) in entries {
        let crc = crc32(data);
        let at = out.len() as u32;
        let mut h = vec![0x50, 0x4b, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&[0, 0]);
        h.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        let mut c = vec![0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(name.len() as u16).to_le_bytes());
        c.extend_from_slice(&[0; 12]);
        c.extend_from_slice(&at.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        dir.extend_from_slice(&c);
    }
    let dir_at = out.len() as u32;
    out.extend_from_slice(&dir);
    out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(dir.len() as u32).to_le_bytes());
    out.extend_from_slice(&dir_at.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

#[test]
fn malformed_sr_files_say_what_is_wrong() {
    let meta = |extra: &str| {
        format!(
            "[global]\nsigrok version=0.5.2\n[device 1]\ncapturefile=logic-1\ntotal probes=2\nsamplerate=1 MHz\nprobe1=A\nprobe2=B\nunitsize=1\n{extra}"
        )
    };
    let err =
        |entries: &[(&str, &[u8])]| load_bytes(make_sr(entries), import::sr::import).unwrap_err().to_string();
    let m = meta("");
    assert!(err(&[("metadata", m.as_bytes())]).contains("no \"version\" entry"));
    assert!(err(&[("version", b"1"), ("metadata", m.as_bytes())]).contains("format version 1"));
    assert!(err(&[("version", b"2")]).contains("no \"metadata\""));
    assert!(err(&[("version", b"2"), ("metadata", m.as_bytes())]).contains("no sample data"));
    let bad_rate = m.replace("1 MHz", "fast");
    assert!(
        err(&[("version", b"2"), ("metadata", bad_rate.as_bytes()), ("logic-1-1", &[0])])
            .contains("\"fast\"")
    );
    let wide = m.replace("unitsize=1", "unitsize=5");
    assert!(
        err(&[("version", b"2"), ("metadata", wide.as_bytes()), ("logic-1-1", &[0])])
            .contains("5 bytes per sample")
    );
    let many = m.replace("total probes=2", "total probes=40");
    assert!(
        err(&[("version", b"2"), ("metadata", many.as_bytes()), ("logic-1-1", &[0])])
            .contains("at most 32 channels")
    );
    let high_bit = m.replace("probe2=B", "probe9=B");
    assert!(
        err(&[("version", b"2"), ("metadata", high_bit.as_bytes()), ("logic-1-1", &[0])]).contains("bit 8")
    );
    // a stray byte at the end of 2-byte samples, and analog channels: notes, not errors
    let two = m.replace("unitsize=1", "unitsize=2") + "total analog=2\n";
    let c = load_bytes(
        make_sr(&[("version", b"2"), ("metadata", two.as_bytes()), ("logic-1-1", &[1, 0, 3, 0, 9])]),
        import::sr::import,
    )
    .unwrap();
    assert_eq!(c.store.sample_count(), 2);
    assert!(c.meta.notes.iter().any(|n| n.message.contains("stray byte")));
    assert!(c.meta.notes.iter().any(|n| n.message.contains("analog")));
    // truncated fixture
    let bytes = std::fs::read(fixtures().join("formats/sr/mixed-8ch-1mhz.sr")).unwrap();
    for cut in [5, bytes.len() / 2, bytes.len() - 3] {
        let e = load_bytes(bytes[..cut].to_vec(), import::sr::import).unwrap_err().to_string();
        assert!(e.contains("ZIP") || e.contains("cut short") || e.contains("damaged"), "cut {cut}: {e}");
    }
}

// ------------------------------------------------------------------ VCD

#[test]
fn vcd_features() {
    let c = load("formats/vcd/features.vcd", import::vcd::import).unwrap();
    assert_eq!(c.meta.sample_rate_hz, 50e6, "10 ns timescale, steps of 2: 20 ns per sample");
    assert_eq!(c.store.sample_count(), 6);
    let names: Vec<&str> = c.meta.channels.iter().map(|ch| ch.name.as_str()).collect();
    assert_eq!(
        names,
        ["clk", "clk_copy", "top.en", "nibble[0]", "nibble[1]", "nibble[2]", "nibble[3]", "top.sub.en"]
    );
    let want: [(Level, &[Sample]); 8] = [
        (Level::Low, &[1, 2, 3, 4, 5]),
        (Level::Low, &[1, 2, 3, 4, 5]),
        (Level::Low, &[3, 4]),
        (Level::Low, &[2, 4]),
        (Level::Low, &[1, 2, 4]),
        (Level::Low, &[4]),
        (Level::Low, &[1, 2]),
        (Level::High, &[3, 4]),
    ];
    for (slot, (init, e)) in want.iter().enumerate() {
        assert_eq!(c.store.initial_level(slot).unwrap(), *init, "{}", names[slot]);
        assert_eq!(edges(&c, slot), e.to_vec(), "{}", names[slot]);
    }
    let notes: Vec<&str> = c.meta.notes.iter().map(|n| n.message.as_str()).collect();
    assert!(notes.iter().any(|n| n.starts_with("3 value(s) were x or z")), "{notes:?}");
    assert!(notes.iter().any(|n| n.starts_with("temp is a real-valued variable")), "{notes:?}");
}

#[test]
fn vcd_from_sigrok_matches_the_sr_it_was_made_from() {
    let v = load("formats/vcd/sigrok-mixed-8ch.vcd", import::vcd::import).unwrap();
    let s = load("formats/sr/mixed-8ch-1mhz.sr", import::sr::import).unwrap();
    assert_eq!(v.meta.sample_rate_hz, s.meta.sample_rate_hz);
    assert_eq!(v.store.sample_count(), s.store.sample_count());
    assert_eq!(
        v.meta.channels.iter().map(|c| &c.name).collect::<Vec<_>>(),
        s.meta.channels.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    for slot in 0..8 {
        assert_eq!(v.store.initial_level(slot).unwrap(), s.store.initial_level(slot).unwrap(), "slot {slot}");
        assert_eq!(edges(&v, slot), edges(&s, slot), "slot {slot}");
    }
}

#[test]
fn malformed_vcd_files_say_what_is_wrong() {
    let err = |text: &str| load_bytes(text.as_bytes().to_vec(), import::vcd::import).unwrap_err().to_string();
    let head =
        "$timescale 1 ns $end $scope module m $end $var wire 1 ! a $end $upscope $end $enddefinitions $end\n";
    assert_eq!(err("hello"), "This is not a VCD file: it does not start with a $ command.");
    assert!(err("$timescale 1 ns $end $var wire 1 ! a $end").contains("$enddefinitions is missing"));
    assert!(err("$timescale 1 ns $end $var wire 1 ! a").contains("no $end"));
    assert!(err("$timescale 7 ns $end $enddefinitions $end").contains("timescale \"7 ns\""));
    assert!(err(&format!("{head}#10 1! #5 0!")).contains("Time goes backwards"));
    assert!(err(&format!("{head}#0 pZ0 !")).contains("extended VCD"));
    assert!(err(&format!("{head}#0 q!")).contains("cannot read: \"q!\""));
    assert!(
        err("$timescale 1 ns $end $var real 64 r t $end $enddefinitions $end #0 r1.5 r")
            .contains("no signals")
    );
    assert!(err(&format!("{head}#0 b101")).contains("middle of a value change"));
    // undeclared identifiers are a note, not an error
    let c = load_bytes(format!("{head}#0 1! 1? #3 0!").into_bytes(), import::vcd::import).unwrap();
    assert!(
        c.meta.notes.iter().any(|n| n.level == NoteLevel::Warning && n.message.contains("never declared"))
    );
    assert_eq!(edges(&c, 0), vec![1]);
    assert_eq!(c.store.initial_level(0).unwrap(), Level::High);
    assert_eq!(c.store.sample_count(), 2, "steps of 3 ns: #3 is sample 1, and it has a change");
}
