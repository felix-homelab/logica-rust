//! Live decoding (doc 06): a decode that follows a growing capture gives exactly the decode of
//! the finished capture, however the data arrives.

use std::sync::Arc;

use logica_core::io::MemFile;
use logica_core::store::CaptureStore;
use logica_protocols::analyse::{Batching, LiveDecode, analyse_capture, decode_store};
use logica_protocols::examples::EXAMPLES;
use logica_protocols::types::{DecodeResult, roles};

/// Feeds a generated capture into a new store in `steps` pieces, advancing the live decode after
/// each, as the engine does while a device streams.
fn live(
    g: &logica_protocols::synth::GenCapture,
    protocol: &str,
    r: &logica_protocols::types::Roles,
    s: &logica_protocols::types::Settings,
    steps: u64,
) -> DecodeResult {
    let reg = logica_protocols::registry();
    let levels: Vec<_> = g.channels.iter().map(|c| c.2.initial).collect();
    let mut store = CaptureStore::new(&levels, Arc::new(MemFile::new()), 16, 64 << 20);
    let mut meta = g.meta();
    meta.state = logica_core::capture::CaptureState::Capturing;
    let mut dec = LiveDecode::new(&reg, &store, &meta, protocol, r, s).unwrap();
    let mut out = DecodeResult::default();
    let n = g.sample_count;
    let mut done = 0;
    for k in 1..=steps {
        // uneven steps, ending exactly at the capture's end
        let end = if k == steps { n } else { (n * k * k / (steps * steps)).max(done) };
        for (slot, (_, _, lane)) in g.channels.iter().enumerate() {
            let new: Vec<u64> = lane.edges.iter().copied().filter(|&e| e >= done && e < end).collect();
            store.push_edges(slot, &new).unwrap();
        }
        store.advance_to(end).unwrap();
        dec.advance(&store, &mut out).unwrap();
        done = end;
    }
    store.finish(n).unwrap();
    dec.finish(&store, n, &mut out).unwrap();
    out
}

#[test]
fn every_example_decodes_live_exactly_as_a_whole() {
    let reg = logica_protocols::registry();
    let mut checked = 0;
    for ex in &EXAMPLES {
        let g = (ex.build)();
        let store = g.store().unwrap();
        let meta = g.meta();
        let a = analyse_capture(&store, &meta, &reg).unwrap();
        for d in a.chosen() {
            let Ok(whole) =
                decode_store(&reg, &store, &meta, &d.protocol_id, &d.roles, &d.settings, Batching::Whole)
            else {
                continue;
            };
            for steps in [1, 7, 64] {
                let got = live(&g, &d.protocol_id, &d.roles, &d.settings, steps);
                assert_eq!(got, whole, "{} as {} in {steps} steps", ex.id, d.protocol_id);
            }
            checked += 1;
        }
    }
    assert!(checked >= 5, "only {checked} decodes checked");
}

#[test]
fn a_live_decode_explains_wrong_roles_and_waits_for_data() {
    let reg = logica_protocols::registry();
    let g = (EXAMPLES[0].build)();
    let levels: Vec<_> = g.channels.iter().map(|c| c.2.initial).collect();
    let store = CaptureStore::new(&levels, Arc::new(MemFile::new()), 16, 64 << 20);
    let meta = g.meta();
    let none = Default::default();
    let e = LiveDecode::new(&reg, &store, &meta, "uart", &roles(&[("data", Some(9))]), &none).err().unwrap();
    assert!(e.contains("Channel 9"), "{e}");
    let e = LiveDecode::new(&reg, &store, &meta, "nothing", &roles(&[]), &none).err().unwrap();
    assert!(e.contains("no decoder"), "{e}");
    // an empty store gives nothing yet, and says so
    let mut d = LiveDecode::new(&reg, &store, &meta, "uart", &roles(&[("data", Some(0))]), &none).unwrap();
    let mut out = DecodeResult::default();
    assert!(!d.advance(&store, &mut out).unwrap());
    assert!(out.frames.is_empty());
}
