//! Logica's `vla` driver against the real analyser on a loopback port: listing, capturing,
//! decoding what arrives, stopping, and the refusals a person can run into.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use logica_core::capture::SourceKind;
use logica_core::device::{CaptureConfig, CaptureMode, StopSignal, StoreSink, VirtualDriver, device_source};
use logica_core::import::StoreTarget;
use logica_drivers::vla::VlaDriver;
use logica_protocols::analyse::{Batching, decode_store};
use logica_protocols::types::{FrameKind, roles};
use logica_vla::Config;

/// Starts an analyser on a free port; returns its address.
fn start(mut config: Config) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    config.listen.clone_from(&at);
    std::thread::spawn(move || logica_vla::server::serve(listener, config, Arc::new(|_: &str| {})));
    at
}

fn fast(seed: u64) -> Config {
    let mut c = Config::example();
    c.speed = 0.0;
    c.seed = Some(seed);
    c
}

fn cfg(channels: Vec<u32>, rate: f64, samples: u64) -> CaptureConfig {
    CaptureConfig {
        channels,
        sample_rate_hz: rate,
        samples,
        mode: CaptureMode::Stream,
        threshold_volts: None,
    }
}

#[test]
fn a_capture_streams_in_and_decodes() {
    let at = start(fast(3));
    let driver = VlaDriver::new(vec![at.clone()]);
    let list = driver.enumerate();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, format!("vla/{at}"));
    assert_eq!(list[0].model, "Virtual analyser: Bench");
    assert_eq!(list[0].channels, 11);

    let mut dev = driver.open(&list[0].id).unwrap();
    assert_eq!(dev.capabilities().rates_hz, vec![4e6, 10e6, 25e6, 100e6]);
    assert_eq!(dev.capabilities().modes, vec![CaptureMode::Stream]);
    // 1 MS/s is too slow for the example's SPI, so the analyser does not offer it
    assert!(dev.configure(&cfg(vec![0], 1e6, 100)).unwrap_err().0.contains("not a rate this device offers"));
    dev.configure(&cfg(vec![0, 3, 4, 5, 6], 4e6, 2_000_000)).unwrap();
    let mut sink = StoreSink::new(StoreTarget::memory());
    let outcome = dev.run(&mut sink, &StopSignal::new()).unwrap();
    assert_eq!(outcome.samples, 2_000_000);
    assert!(!outcome.stopped);
    let (store, meta) = sink.finish(&outcome, device_source(dev.info()), "t").unwrap();
    assert_eq!(meta.source.kind, SourceKind::Device);
    assert_eq!(
        meta.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["TX", "CLK", "CS", "MOSI", "MISO"]
    );
    assert!(meta.notes.iter().any(|n| n.message.contains("seed 3")), "{:?}", meta.notes);

    let reg = logica_protocols::registry();
    let none = Default::default();
    let uart =
        decode_store(&reg, &store, &meta, "uart", &roles(&[("data", Some(0))]), &none, Batching::Whole)
            .unwrap();
    let text: String = uart
        .frames
        .iter()
        .filter(|f| f.kind == FrameKind::Data)
        .filter_map(|f| f.value)
        .map(|v| char::from(u8::try_from(logica_protocols::num::round(v)).unwrap_or(b'?')))
        .collect();
    assert!(text.contains("\r\n"), "{text:?}");
    let spi = decode_store(
        &reg,
        &store,
        &meta,
        "spi",
        &roles(&[("clk", Some(3)), ("cs", Some(4)), ("mosi", Some(5)), ("miso", Some(6))]),
        &none,
        Batching::Whole,
    )
    .unwrap();
    assert!(spi.packets.len() > 3, "{} SPI packets", spi.packets.len());

    // the same seed gives the same capture again
    let mut again = StoreSink::new(StoreTarget::memory());
    dev.run(&mut again, &StopSignal::new()).unwrap();
    let s2 = again.store().unwrap();
    assert_eq!(s2.edges_in(1, 0, 2_000_000).unwrap(), store.edges_in(1, 0, 2_000_000).unwrap());
}

#[test]
fn stopping_ends_a_real_time_capture_early() {
    let mut c = fast(1);
    c.speed = 1.0;
    let at = start(c);
    let driver = VlaDriver::new(vec![at.clone()]);
    let mut dev = driver.open(&format!("vla/{at}")).unwrap();
    // a minute at real time, stopped after a quarter of a second
    dev.configure(&cfg(vec![0, 1, 2], 4e6, 240_000_000)).unwrap();
    let stop = StopSignal::new();
    let s2 = stop.clone();
    let t = Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        s2.stop();
    });
    let mut sink = StoreSink::new(StoreTarget::memory());
    let outcome = dev.run(&mut sink, &stop).unwrap();
    assert!(outcome.stopped);
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    // real time: about a quarter second of signal arrived, not the whole minute
    assert!((400_000..=2_000_000).contains(&outcome.samples), "{} samples", outcome.samples);
    let (store, meta) = sink.finish(&outcome, device_source(dev.info()), "t").unwrap();
    assert_eq!(store.sample_count(), outcome.samples);
    assert!(meta.notes.iter().any(|n| n.message.contains("stopped after")));
}

#[test]
fn the_analyser_explains_refusals_and_is_listed_while_busy() {
    let mut c = fast(2);
    c.speed = 1.0;
    let at = start(c);
    let driver = VlaDriver::new(vec![at.clone()]);
    let id = format!("vla/{at}");
    let mut first = driver.open(&id).unwrap();
    first.configure(&cfg(vec![0], 4e6, 400_000_000)).unwrap();
    let stop = StopSignal::new();
    let s2 = stop.clone();
    let running = std::thread::spawn(move || {
        let mut sink = StoreSink::new(StoreTarget::memory());
        first.run(&mut sink, &s2)
    });
    std::thread::sleep(Duration::from_millis(150));
    // still listed while it captures, but a second capture is refused with the reason
    assert_eq!(driver.enumerate().len(), 1);
    let mut second = driver.open(&id).unwrap();
    second.configure(&cfg(vec![1], 4e6, 1000)).unwrap();
    let mut sink = StoreSink::new(StoreTarget::memory());
    let e = second.run(&mut sink, &StopSignal::new()).unwrap_err();
    assert!(e.0.contains("busy with another capture"), "{e}");
    stop.stop();
    assert!(running.join().unwrap().unwrap().stopped);
}

#[test]
fn nothing_is_listed_where_nothing_answers() {
    // a port that was free a moment ago
    let at = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
    let driver = VlaDriver::new(vec![at.clone(), "not an address".into()]);
    assert!(driver.enumerate().is_empty());
    let e = driver.open(&format!("vla/{at}")).err().unwrap();
    assert!(e.0.contains("No virtual analyser answers"), "{e}");
    let e = driver.open("vla/not an address").err().unwrap();
    assert!(e.0.contains("is not an address"), "{e}");
}

#[test]
fn several_analysers_are_listed_in_order_and_slow_addresses_are_asked_at_once() {
    let mut second = fast(4);
    second.name = "Second".into();
    let (a, b) = (start(fast(3)), start(second));
    // two programs that accept but never greet: each costs the greeting timeout (2 s)
    let silent: Vec<(TcpListener, String)> = (0..2)
        .map(|_| {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let at = l.local_addr().unwrap().to_string();
            (l, at)
        })
        .collect();
    let driver = VlaDriver::new(vec![a.clone(), silent[0].1.clone(), b.clone(), silent[1].1.clone()]);
    let t = Instant::now();
    let list = driver.enumerate();
    let took = t.elapsed();
    let models: Vec<&str> = list.iter().map(|d| d.model.as_str()).collect();
    assert_eq!(models, ["Virtual analyser: Bench", "Virtual analyser: Second"]);
    assert_eq!(list[1].id, format!("vla/{b}"));
    assert!(took < Duration::from_millis(3500), "asked one after the other: {took:?}");
    // the names of the channels come with the capabilities
    let dev = driver.open(&list[0].id).unwrap();
    assert_eq!(dev.capabilities().channel_names[0], "TX");
    assert_eq!(dev.capabilities().channel_names.len(), dev.capabilities().channels.len());
    drop(silent);
}

#[test]
fn a_program_that_is_not_an_analyser_is_named_as_such() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for s in listener.incoming() {
            use std::io::Write;
            let _ = s.unwrap().write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
        }
    });
    let e = VlaDriver::new(vec![at.clone()]).open(&format!("vla/{at}")).err().unwrap();
    assert!(e.0.contains("is not a virtual analyser"), "{e}");
}
