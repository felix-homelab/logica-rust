//! `.lacap` (doc 09): round trips, saving in place, recovery from an interrupted save, damaged and
//! truncated files, and blob offsets above 4 GiB.

#![allow(clippy::cast_possible_truncation)]

mod common;

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use common::*;
use logica_core::capture::{CaptureMeta, CaptureState, ChannelInfo, Note, NoteLevel, Source, SourceKind};
use logica_core::io::{MemFile, RandomAccess};
use logica_core::lacap::{self, LacapError};
use logica_core::store::{CaptureStore, StoreError};
use logica_core::{Level, Sample};

fn meta(channels: usize) -> CaptureMeta {
    CaptureMeta {
        id: "c1".into(),
        title: "Test capture".into(),
        source: Source {
            kind: SourceKind::Synthetic,
            detail: "tests".into(),
        },
        sample_rate_hz: 100e6,
        trigger: Some(12),
        channels: (0..channels)
            .map(|i| ChannelInfo {
                index: i as u32 * 2,
                name: format!("CH{i}"),
                colour: Some("#e54242".into()),
                enabled: true,
                threshold_volts: None,
            })
            .collect(),
        segments: vec![0..1000, 2000..3000],
        notes: vec![Note {
            level: NoteLevel::Info,
            message: "made by a test".into(),
        }],
        state: CaptureState::Complete,
        artefact: None,
        saved_decoders: vec![],
    }
}

fn models(count: u64) -> Vec<Model> {
    vec![
        mixed_model(41, 0, count, Level::Low),
        Model {
            initial: Level::High,
            edges: vec![],
        },
        Model {
            initial: Level::Low,
            edges: vec![3, C, count - 1],
        },
    ]
}

/// A finished capture recorded into a working `.lacap` in `file`.
fn record(file: Arc<dyn RandomAccess>, models: &[Model], count: u64, budget: usize) -> CaptureStore {
    let levels: Vec<Level> = models.iter().map(|m| m.initial).collect();
    let mut s = lacap::new_capture(file, &levels, budget).unwrap();
    for (slot, m) in models.iter().enumerate() {
        s.push_edges(slot, &m.edges).unwrap();
    }
    s.finish(count).unwrap();
    s
}

fn reopen(bytes: Vec<u8>) -> Result<lacap::Opened, LacapError> {
    lacap::open(Arc::new(MemFile::from_bytes(bytes)), 1 << 30)
}

#[test]
fn save_in_place_and_save_as_round_trip() {
    let count = 2 * C + 4321;
    let ms = models(count);
    let file = Arc::new(MemFile::new());
    let s = record(file.clone(), &ms, count, 3 << 20); // small cache: some chunks evicted while recording
    let session = serde_json::json!({ "markers": [ { "at": 5 } ] });
    lacap::save(&s, &meta(3), &session).unwrap();
    let o = reopen(file.to_bytes()).unwrap();
    assert!(!o.recovered);
    assert_eq!(o.meta, meta(3));
    assert_eq!(o.session, session);
    assert_same(&s, &o.store);

    // a compact copy of the reopened capture, and of the original
    let copy = MemFile::new();
    lacap::save_as(&o.store, &o.meta, &o.session, &copy).unwrap();
    let o2 = reopen(copy.to_bytes()).unwrap();
    assert_same(&s, &o2.store);
    assert!(copy.size().unwrap() <= file.size().unwrap());

    // saving again appends; the newest save is what opens
    let mut m2 = meta(3);
    m2.title = "Renamed".into();
    lacap::save(&s, &m2, &session).unwrap();
    let o3 = reopen(file.to_bytes()).unwrap();
    assert_eq!(o3.meta.title, "Renamed");
    assert!(!o3.recovered);
    assert_same(&s, &o3.store);
}

#[test]
fn an_interrupted_save_opens_the_previous_state_and_the_next_save_cuts_it_off() {
    let count = C + 100;
    let ms = models(count);
    let file = Arc::new(MemFile::new());
    let s = record(file.clone(), &ms, count, 1 << 30);
    lacap::save(&s, &meta(3), &serde_json::Value::Null).unwrap();
    let good = file.to_bytes();
    // A second save that stopped half way: some table bytes, no footer.
    let mut m2 = meta(3);
    m2.title = "second".into();
    lacap::save(&s, &m2, &serde_json::Value::Null).unwrap();
    let full = file.to_bytes();
    for cut in [good.len() + 1, good.len() + 50, full.len() - 33, full.len() - 1] {
        let o = reopen(full[..cut].to_vec()).unwrap();
        assert!(o.recovered, "cut at {cut}");
        assert_eq!(o.meta.title, "Test capture");
        assert!(o.meta.notes.iter().any(|n| n.message.contains("interrupted")));
        assert_same(&s, &o.store);
    }
    // Saving the recovered capture cuts the partial append off and leaves a clean file.
    let partial = Arc::new(MemFile::from_bytes(full[..good.len() + 50].to_vec()));
    let o = lacap::open(partial.clone(), 1 << 30).unwrap();
    lacap::save(&o.store, &m2, &o.session).unwrap();
    let again = lacap::open(partial.clone(), 1 << 30).unwrap();
    assert!(!again.recovered);
    assert_eq!(again.meta.title, "second");
    assert_same(&s, &again.store);
}

#[test]
fn truncated_files_never_panic_and_say_why() {
    let count = C + 10;
    let file = Arc::new(MemFile::new());
    let s = record(file.clone(), &models(count), count, 1 << 30);
    lacap::save(&s, &meta(3), &serde_json::Value::Null).unwrap();
    let bytes = file.to_bytes();
    let mut cuts: Vec<usize> = (0..40).collect();
    cuts.extend((40..bytes.len()).step_by(bytes.len() / 97 + 1));
    cuts.extend(bytes.len() - 40..bytes.len());
    for cut in cuts {
        let err = reopen(bytes[..cut].to_vec()).unwrap_err();
        let msg = err.to_string();
        match err {
            LacapError::NotLacap => assert!(cut < 16, "cut {cut}: {msg}"),
            LacapError::NoCompleteSave => assert!(msg.contains("no complete save"), "{msg}"),
            other => panic!("cut {cut}: unexpected {other}"),
        }
    }
}

#[test]
fn damage_is_reported_by_place() {
    let count = 2 * C;
    let ms = vec![Model {
        initial: Level::Low,
        edges: vec![5, C + 5],
    }];
    let file = Arc::new(MemFile::new());
    let s = record(file.clone(), &ms, count, 1 << 30);
    let mut m = meta(1);
    m.channels.truncate(1);
    lacap::save(&s, &m, &serde_json::Value::Null).unwrap();
    let bytes = file.to_bytes();

    // a chunk blob
    let loc = *s.chunks(0).unwrap()[1].payload_at.get().unwrap();
    let mut b = bytes.clone();
    b[(loc.offset + 20) as usize] ^= 1;
    let o = reopen(b).unwrap();
    assert_eq!(
        o.store.level_at(0, 6).unwrap(),
        Level::High,
        "chunk 0 still reads"
    );
    let e = o.store.edges_in(0, 0, count).unwrap_err();
    assert!(
        matches!(
            e,
            StoreError::Damaged {
                slot: 0,
                chunk: 1,
                ..
            }
        ),
        "{e}"
    );

    // the index (just before the footer): no earlier save, so it is damage
    let mut b = bytes.clone();
    let n = b.len();
    b[n - 40] ^= 0xff;
    let e = reopen(b).unwrap_err();
    assert_eq!(
        e.to_string(),
        "This .lacap file is damaged: its index does not match its checksum."
    );

    // the channel table: the last blob before the index; flip a byte inside it
    let mut b = bytes.clone();
    let index_at = u64::from_le_bytes(b[b.len() - 32..b.len() - 24].try_into().unwrap()) as usize;
    b[index_at - 30] ^= 0xff;
    let e = reopen(b).unwrap_err();
    assert!(
        e.to_string()
            .starts_with("This .lacap file is damaged: the channel table of channel 0 is unusable"),
        "{e}"
    );
}

#[test]
fn foreign_and_newer_files_are_refused_clearly() {
    let e = reopen(b"PK\x03\x04 this is a zip".to_vec()).unwrap_err();
    assert_eq!(
        e.to_string(),
        "This is not a .lacap file: it does not start with the LACAP header."
    );
    let mut h = b"LACAP\0\r\n".to_vec();
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&3u16.to_le_bytes());
    h.extend_from_slice(&[0; 4]);
    h.extend_from_slice(&[0; 64]);
    let e = reopen(h).unwrap_err();
    assert!(
        e.to_string().contains("newer version of Logica (format 2.3)"),
        "{e}"
    );
}

#[test]
fn only_finished_captures_with_matching_channels_are_saved() {
    let file = Arc::new(MemFile::new());
    let mut s = lacap::new_capture(file, &[Level::Low], 1 << 20).unwrap();
    s.push_edges(0, &[4]).unwrap();
    let e = lacap::save(&s, &meta(1), &serde_json::Value::Null).unwrap_err();
    assert_eq!(
        e.to_string(),
        "The capture is still running; it can be saved once it has ended."
    );
    s.finish(10).unwrap();
    let e = lacap::save(&s, &meta(2), &serde_json::Value::Null).unwrap_err();
    assert!(
        e.to_string()
            .contains("describes 2 channel(s) but holds data for 1")
    );
}

#[test]
fn empty_captures_round_trip() {
    let file = Arc::new(MemFile::new());
    let mut s = lacap::new_capture(file.clone(), &[Level::High, Level::Low], 1 << 20).unwrap();
    s.finish(0).unwrap();
    lacap::save(&s, &meta(2), &serde_json::Value::Null).unwrap();
    let o = reopen(file.to_bytes()).unwrap();
    assert_eq!(o.store.sample_count(), 0);
    assert_eq!(o.store.initial_level(0).unwrap(), Level::High);
}

/// A sparse in-memory file, so blob offsets above 4 GiB can be tested without 4 GiB of memory.
#[derive(Default)]
struct SparseFile {
    pages: Mutex<BTreeMap<u64, Vec<u8>>>,
    size: Mutex<u64>,
}
const PAGE: u64 = 1 << 16;
impl RandomAccess for SparseFile {
    fn size(&self) -> io::Result<u64> {
        Ok(*self.size.lock().unwrap())
    }
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if offset + buf.len() as u64 > *self.size.lock().unwrap() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof"));
        }
        let pages = self.pages.lock().unwrap();
        for (i, b) in buf.iter_mut().enumerate() {
            let p = offset + i as u64;
            *b = pages.get(&(p / PAGE)).map_or(0, |v| v[(p % PAGE) as usize]);
        }
        Ok(())
    }
    fn write_all_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        let mut pages = self.pages.lock().unwrap();
        for (i, &b) in data.iter().enumerate() {
            let p = offset + i as u64;
            pages.entry(p / PAGE).or_insert_with(|| vec![0; PAGE as usize])[(p % PAGE) as usize] = b;
        }
        let mut size = self.size.lock().unwrap();
        *size = (*size).max(offset + data.len() as u64);
        Ok(())
    }
    fn set_len(&self, len: u64) -> io::Result<()> {
        *self.size.lock().unwrap() = len;
        self.pages.lock().unwrap().retain(|&p, _| p * PAGE < len);
        Ok(())
    }
    fn sync(&self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn blob_offsets_above_4_gib() {
    let file: Arc<SparseFile> = Arc::new(SparseFile::default());
    let dynfile: Arc<dyn RandomAccess> = file.clone();
    lacap::new_capture(dynfile.clone(), &[Level::Low], 1).unwrap();
    // Blobs start at 5 GiB: the gap is dead space, which readers never look at.
    let mut s = CaptureStore::new(&[Level::Low], dynfile.clone(), 5 << 30, 1);
    let edges: Vec<Sample> = (1..5000).map(|i| i * 3).collect();
    s.push_edges(0, &edges).unwrap();
    s.finish(C + 7).unwrap();
    let mut m = meta(1);
    m.channels.truncate(1);
    lacap::save(&s, &m, &serde_json::Value::Null).unwrap();
    assert!(s.chunks(0).unwrap()[0].payload_at.get().unwrap().offset > 1 << 32);
    let o = lacap::open(dynfile, 1).unwrap();
    assert_eq!(o.store.edges_in(0, 0, C).unwrap(), edges);
    assert_eq!(o.store.transitions_in(0, 0, 1 << 16).unwrap(), 4999);
}
