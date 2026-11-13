//! The native `.lacap` container (doc 09, ADR 0005): header, blobs, a raw-deflate JSON index and a
//! footer. Random access by offset, no unpacking on open, files over 4 GB.
//!
//! A capture being recorded or imported lives in a working `.lacap` ([`new_capture`]): chunks
//! pushed out of the cache are appended as blobs, and [`save`] appends the remaining chunks, the
//! channel tables, the index and the footer. [`save_as`] writes a compact copy. [`open`] reads the
//! footer, the index and the tables; chunks are read when needed. If a save was interrupted, `open`
//! falls back to the last complete footer and says so in a note.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::capture::{CaptureMeta, Note, NoteLevel};
use crate::io::RandomAccess;
use crate::store::blob::{self, BLOB_CHANNEL_TABLE, BLOB_HEADER, BLOB_TRAILER, u32_at, u64_at};
use crate::store::chunk::{BlobLoc, ChunkKind, ChunkSummary};
use crate::store::lod::CoarseLod;
use crate::store::{CHUNK_SAMPLES, COARSE_BUCKETS, CaptureStore, Level, MAX_OFFSETS, Sample, StoreError};

pub const MAGIC: &[u8; 8] = b"LACAP\0\r\n";
pub const FOOTER_MAGIC: &[u8; 8] = b"LACAPEND";
pub const MAJOR: u16 = 1;
pub const MINOR: u16 = 0;
pub const HEADER_LEN: u64 = 16;
pub const FOOTER_LEN: u64 = 32;
const RECORD_SIZE: u32 = 40;
const TABLE_HEADER: usize = 16;
const COARSE_BYTES: usize = COARSE_BUCKETS * 2 + COARSE_BUCKETS / 8;
/// An index larger than this is refused (it would be a damaged or hostile file).
const MAX_INDEX: usize = 256 << 20;

#[derive(Debug)]
pub enum LacapError {
    NotLacap,
    Newer { major: u16, minor: u16 },
    NoCompleteSave,
    Damaged(String),
    Io(std::io::Error),
    Store(StoreError),
    StillCapturing,
}

impl fmt::Display for LacapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LacapError::NotLacap => f.write_str("This is not a .lacap file: it does not start with the LACAP header."),
            LacapError::Newer { major, minor } => write!(
                f,
                "This file was saved by a newer version of Logica (format {major}.{minor}); this version reads format {MAJOR}.x. Update Logica to open it."
            ),
            LacapError::NoCompleteSave => f.write_str(
                "This .lacap file has no complete save in it (it may have been cut off while it was first written), so it cannot be opened.",
            ),
            LacapError::Damaged(m) => write!(f, "This .lacap file is damaged: {m}."),
            LacapError::Io(e) => write!(f, "The .lacap file could not be read or written: {e}."),
            LacapError::Store(e) => e.fmt(f),
            LacapError::StillCapturing => {
                f.write_str("The capture is still running; it can be saved once it has ended.")
            }
        }
    }
}

impl std::error::Error for LacapError {}

impl From<std::io::Error> for LacapError {
    fn from(e: std::io::Error) -> Self {
        LacapError::Io(e)
    }
}

impl From<StoreError> for LacapError {
    fn from(e: StoreError) -> Self {
        LacapError::Store(e)
    }
}

pub type LacapResult<T> = Result<T, LacapError>;

/// The JSON index (doc 09).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    format: String,
    version: [u16; 2],
    chunk_size: u64,
    sample_count: Sample,
    initial_levels: Vec<u8>,
    capture: CaptureMeta,
    /// Per channel slot: offset and length of its channel table blob.
    tables: Vec<[u64; 2]>,
    /// Session state saved with the capture (decoder instances, markers, notes; defined in M1).
    #[serde(default)]
    session: serde_json::Value,
}

/// A capture read from a `.lacap` file.
#[derive(Debug)]
pub struct Opened {
    pub store: CaptureStore,
    pub meta: CaptureMeta,
    pub session: serde_json::Value,
    /// Set when an interrupted save was skipped; also added to `meta.notes`.
    pub recovered: bool,
}

fn header() -> [u8; 16] {
    let mut h = [0u8; 16];
    h[..8].copy_from_slice(MAGIC);
    h[8..10].copy_from_slice(&MAJOR.to_le_bytes());
    h[10..12].copy_from_slice(&MINOR.to_le_bytes());
    h
}

/// Starts a working `.lacap` in `file` (emptied first) and returns a store that records into it.
pub fn new_capture(
    file: Arc<dyn RandomAccess>,
    initial: &[Level],
    cache_budget: usize,
) -> LacapResult<CaptureStore> {
    file.set_len(0)?;
    file.write_all_at(0, &header())?;
    Ok(CaptureStore::new(initial, file, HEADER_LEN, cache_budget))
}

/// The bytes of one channel table blob, given where each chunk's blobs are.
fn encode_table(slot: u32, chunks: &[ChunkSummary], locs: &[(Option<BlobLoc>, Option<BlobLoc>)]) -> Vec<u8> {
    let non_quiet = chunks.iter().filter(|c| c.kind != ChunkKind::Quiet).count();
    let mut p = Vec::with_capacity(TABLE_HEADER + chunks.len() * 40 + non_quiet * COARSE_BYTES);
    p.extend_from_slice(&slot.to_le_bytes());
    p.extend_from_slice(&u32::try_from(chunks.len()).unwrap_or(u32::MAX).to_le_bytes());
    p.extend_from_slice(&RECORD_SIZE.to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    for (c, (payload, fine)) in chunks.iter().zip(locs) {
        p.extend_from_slice(&[c.kind as u8, c.entry as u8, c.exit as u8, u8::from(c.has_fine)]);
        p.extend_from_slice(&c.transitions.to_le_bytes());
        for loc in [payload, fine] {
            let l = loc.unwrap_or(BlobLoc { offset: 0, len: 0 });
            p.extend_from_slice(&l.offset.to_le_bytes());
            p.extend_from_slice(&l.len.to_le_bytes());
        }
    }
    for c in chunks {
        if let Some(lod) = &c.coarse {
            lod.counts
                .iter()
                .for_each(|v| p.extend_from_slice(&v.to_le_bytes()));
            p.extend_from_slice(&lod.start_levels);
        }
    }
    blob::wrap(BLOB_CHANNEL_TABLE, &p)
}

fn write_index_and_footer(
    file: &dyn RandomAccess,
    mut at: u64,
    store: &CaptureStore,
    meta: &CaptureMeta,
    session: &serde_json::Value,
    tables: Vec<[u64; 2]>,
) -> LacapResult<u64> {
    let initial_levels = (0..store.slots())
        .map(|s| store.initial_level(s).map(|l| l as u8))
        .collect::<Result<_, _>>()?;
    let index = Index {
        format: "lacap".into(),
        version: [MAJOR, MINOR],
        chunk_size: CHUNK_SAMPLES,
        sample_count: store.sample_count(),
        initial_levels,
        capture: meta.clone(),
        tables,
        session: session.clone(),
    };
    let json = serde_json::to_vec(&index)
        .map_err(|e| LacapError::Damaged(format!("the index could not be written ({e})")))?;
    let packed = miniz_oxide::deflate::compress_to_vec(&json, 6);
    file.write_all_at(at, &packed)?;
    let index_at = at;
    at += packed.len() as u64;
    let mut footer = Vec::with_capacity(32);
    footer.extend_from_slice(&index_at.to_le_bytes());
    footer.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    footer.extend_from_slice(&crc32fast::hash(&packed).to_le_bytes());
    footer.extend_from_slice(&0u32.to_le_bytes());
    footer.extend_from_slice(FOOTER_MAGIC);
    file.write_all_at(at, &footer)?;
    at += FOOTER_LEN;
    file.sync()?;
    Ok(at)
}

fn check_saveable(store: &CaptureStore, meta: &CaptureMeta) -> LacapResult<()> {
    if !store.is_finished() {
        return Err(LacapError::StillCapturing);
    }
    if meta.channels.len() != store.slots() {
        return Err(LacapError::Store(StoreError::Invalid(format!(
            "The capture describes {} channel(s) but holds data for {}.",
            meta.channels.len(),
            store.slots()
        ))));
    }
    Ok(())
}

/// Saves a finished capture into its own backing (a working `.lacap`, or an opened one): writes
/// the chunks still only in memory, then channel tables, index and footer. A partial append left
/// by an interrupted save is cut off first.
pub fn save(store: &CaptureStore, meta: &CaptureMeta, session: &serde_json::Value) -> LacapResult<()> {
    check_saveable(store, meta)?;
    let file = store.backing().clone();
    if file.size()? > store.append_position() {
        file.set_len(store.append_position())?;
    }
    store.flush()?;
    let mut tables = Vec::with_capacity(store.slots());
    for slot in 0..store.slots() {
        let chunks = store.chunks(slot)?;
        let locs: Vec<_> = chunks
            .iter()
            .map(|c| (c.payload_at.get().copied(), c.fine_at.get().copied()))
            .collect();
        let slot32 = u32::try_from(slot).unwrap_or(u32::MAX);
        let loc = store.append_raw(&encode_table(slot32, chunks, &locs))?;
        tables.push([loc.offset, loc.len]);
    }
    let end = write_index_and_footer(&*file, store.append_position(), store, meta, session, tables)?;
    store.set_append_position(end);
    Ok(())
}

/// Writes a compact copy of a finished capture to `dest` (emptied first): only live blobs.
pub fn save_as(
    store: &CaptureStore,
    meta: &CaptureMeta,
    session: &serde_json::Value,
    dest: &dyn RandomAccess,
) -> LacapResult<()> {
    check_saveable(store, meta)?;
    dest.set_len(0)?;
    dest.write_all_at(0, &header())?;
    let mut at = HEADER_LEN;
    let mut append = |bytes: &[u8]| -> LacapResult<BlobLoc> {
        dest.write_all_at(at, bytes)?;
        let loc = BlobLoc {
            offset: at,
            len: bytes.len() as u64,
        };
        at += loc.len;
        Ok(loc)
    };
    let mut tables = Vec::with_capacity(store.slots());
    for slot in 0..store.slots() {
        let chunks = store.chunks(slot)?;
        let slot32 = u32::try_from(slot).unwrap_or(u32::MAX);
        let mut locs = Vec::with_capacity(chunks.len());
        for (k, c) in (0u64..).zip(chunks) {
            if c.kind == ChunkKind::Quiet {
                locs.push((None, None));
                continue;
            }
            let p = store.payload(slot, k)?;
            let payload = append(&blob::encode_chunk(slot32, k, c.entry, c.transitions, &p))?;
            let fine = if c.has_fine {
                let f = store.fine_lod(slot, k)?;
                Some(append(&blob::encode_fine(slot32, k, &f))?)
            } else {
                None
            };
            locs.push((Some(payload), fine));
        }
        let loc = append(&encode_table(slot32, chunks, &locs))?;
        tables.push([loc.offset, loc.len]);
    }
    write_index_and_footer(dest, at, store, meta, session, tables)?;
    Ok(())
}

/// Checks a footer candidate ending at `end`; returns the index bytes if it is a complete save.
fn try_footer(file: &dyn RandomAccess, end: u64) -> LacapResult<Option<Vec<u8>>> {
    if end < HEADER_LEN + FOOTER_LEN {
        return Ok(None);
    }
    let start = end - FOOTER_LEN;
    let mut f = [0u8; 32];
    file.read_exact_at(start, &mut f)?;
    if &f[24..32] != FOOTER_MAGIC {
        return Ok(None);
    }
    let (index_at, index_len) = (u64_at(&f, 0), u64_at(&f, 8));
    if index_at < HEADER_LEN || index_at.checked_add(index_len) != Some(start) || index_len > MAX_INDEX as u64
    {
        return Ok(None);
    }
    let mut index = vec![0u8; usize::try_from(index_len).unwrap_or(0)];
    file.read_exact_at(index_at, &mut index)?;
    Ok((crc32fast::hash(&index) == u32_at(&f, 16)).then_some(index))
}

/// Finds the last complete save: the footer at the end, or else the last valid one before it.
/// Returns its index bytes and where the file's valid part ends.
fn find_last_save(file: &dyn RandomAccess, size: u64) -> LacapResult<(Vec<u8>, u64, bool)> {
    if let Some(index) = try_footer(file, size)? {
        return Ok((index, size, false));
    }
    // A footer magic right at the end that does not check out is damage, not an interrupted save
    // (the footer is written last).
    let mut tail = [0u8; 8];
    let damaged_end = size >= HEADER_LEN + FOOTER_LEN && {
        file.read_exact_at(size - 8, &mut tail)?;
        &tail == FOOTER_MAGIC
    };
    // Scan back for the footer magic, a block at a time (blocks overlap by 7 bytes).
    const BLOCK: u64 = 1 << 20;
    let mut hi = size;
    while hi > HEADER_LEN {
        let lo = hi.saturating_sub(BLOCK).max(HEADER_LEN);
        let mut buf = vec![0u8; usize::try_from(hi - lo).unwrap_or(0)];
        file.read_exact_at(lo, &mut buf)?;
        for i in (0..buf.len().saturating_sub(7)).rev() {
            if &buf[i..i + 8] == FOOTER_MAGIC {
                let end = lo + i as u64 + 8;
                if end < size
                    && let Some(index) = try_footer(file, end)?
                {
                    return Ok((index, end, true));
                }
            }
        }
        if lo == HEADER_LEN {
            break;
        }
        hi = lo + 7;
    }
    if damaged_end {
        return Err(LacapError::Damaged(
            "its index does not match its checksum".into(),
        ));
    }
    Err(LacapError::NoCompleteSave)
}

fn decode_table(
    bytes: &[u8],
    slot: usize,
    expect_chunks: u64,
    file_size: u64,
) -> Result<Vec<ChunkSummary>, String> {
    let p = blob::unwrap(bytes, BLOB_CHANNEL_TABLE)?;
    if p.len() < TABLE_HEADER {
        return Err("its header is cut short".into());
    }
    let count = u64::from(u32_at(p, 4));
    let rec = usize::try_from(u32_at(p, 8)).unwrap_or(0);
    if u32_at(p, 0) as usize != slot {
        return Err("it belongs to another channel".into());
    }
    if count != expect_chunks {
        return Err(format!(
            "it lists {count} chunks, but the capture needs {expect_chunks}"
        ));
    }
    if rec < RECORD_SIZE as usize {
        return Err("its records are too small".into());
    }
    let n = usize::try_from(count).map_err(|_| "it is too large".to_string())?;
    let records_end = TABLE_HEADER + n * rec;
    if p.len() < records_end {
        return Err("its records are cut short".into());
    }
    let level = |b: u8| match b {
        0 => Ok(Level::Low),
        1 => Ok(Level::High),
        _ => Err("a level is neither 0 nor 1".to_string()),
    };
    let loc_at = |r: &[u8], at: usize| -> Result<Option<BlobLoc>, String> {
        let l = BlobLoc {
            offset: u64_at(r, at),
            len: u64_at(r, at + 8),
        };
        if l.len == 0 {
            return Ok(None);
        }
        if l.offset < HEADER_LEN || l.offset.checked_add(l.len).is_none_or(|e| e > file_size) {
            return Err("a chunk lies outside the file".into());
        }
        Ok(Some(l))
    };
    let mut chunks = Vec::with_capacity(n);
    let mut coarse_at = records_end;
    for i in 0..n {
        let r = &p[TABLE_HEADER + i * rec..TABLE_HEADER + (i + 1) * rec];
        let kind = ChunkKind::from_byte(r[0]).ok_or_else(|| format!("chunk {i} has an unknown kind"))?;
        let mut c = ChunkSummary::quiet(level(r[1])?);
        c.kind = kind;
        c.exit = level(r[2])?;
        c.has_fine = r[3] & 1 == 1;
        c.transitions = u32_at(r, 4);
        let payload = loc_at(r, 8)?;
        let fine = loc_at(r, 24)?;
        // Every transition flips the level, so the exit level follows from the entry and the count.
        let consistent = match kind {
            ChunkKind::Quiet => c.transitions == 0 && payload.is_none() && !c.has_fine,
            ChunkKind::Offsets => (1..=MAX_OFFSETS).contains(&c.transitions) && payload.is_some(),
            ChunkKind::Bits => c.transitions > 0 && payload.is_some(),
        } && c.has_fine == fine.is_some()
            && (c.transitions % 2 == 1) == (c.entry != c.exit);
        if !consistent {
            return Err(format!("the record of chunk {i} contradicts itself"));
        }
        if let Some(prev) = chunks.last().map(|x: &ChunkSummary| x.exit)
            && prev != c.entry
        {
            return Err(format!("chunk {i} does not continue where chunk {} ends", i - 1));
        }
        if let Some(l) = payload {
            let _ = c.payload_at.set(l);
        }
        if let Some(l) = fine {
            let _ = c.fine_at.set(l);
        }
        if kind != ChunkKind::Quiet {
            if p.len() < coarse_at + COARSE_BYTES {
                return Err("the coarse level of detail is cut short".into());
            }
            let mut lod = CoarseLod {
                counts: [0; COARSE_BUCKETS],
                start_levels: [0; COARSE_BUCKETS / 8],
            };
            for (j, v) in lod.counts.iter_mut().enumerate() {
                *v = u16::from_le_bytes([p[coarse_at + 2 * j], p[coarse_at + 2 * j + 1]]);
            }
            lod.start_levels
                .copy_from_slice(&p[coarse_at + COARSE_BUCKETS * 2..coarse_at + COARSE_BYTES]);
            c.coarse = Some(Box::new(lod));
            coarse_at += COARSE_BYTES;
        }
        chunks.push(c);
    }
    Ok(chunks)
}

/// Opens a `.lacap` file. Chunks stay in the file and are read when needed.
pub fn open(file: Arc<dyn RandomAccess>, cache_budget: usize) -> LacapResult<Opened> {
    let size = file.size()?;
    if size < HEADER_LEN {
        return Err(LacapError::NotLacap);
    }
    let mut h = [0u8; 16];
    file.read_exact_at(0, &mut h)?;
    if &h[..8] != MAGIC {
        return Err(LacapError::NotLacap);
    }
    let (major, minor) = (
        u16::from_le_bytes([h[8], h[9]]),
        u16::from_le_bytes([h[10], h[11]]),
    );
    if major > MAJOR {
        return Err(LacapError::Newer { major, minor });
    }
    let (packed, valid_end, recovered) = find_last_save(&*file, size)?;
    let json = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, MAX_INDEX)
        .map_err(|_| LacapError::Damaged("the index cannot be unpacked".into()))?;
    let index: Index = serde_json::from_slice(&json)
        .map_err(|e| LacapError::Damaged(format!("the index cannot be read ({e})")))?;
    if index.format != "lacap" || index.chunk_size != CHUNK_SAMPLES {
        return Err(LacapError::Damaged(
            "the index describes another format or chunk size".into(),
        ));
    }
    let slots = index.capture.channels.len();
    if index.tables.len() != slots || index.initial_levels.len() != slots {
        return Err(LacapError::Damaged(
            "the index lists a different number of channels than it describes".into(),
        ));
    }
    let expect_chunks = index.sample_count.div_ceil(CHUNK_SAMPLES);
    let mut channels = Vec::with_capacity(slots);
    for slot in 0..slots {
        let [offset, len] = index.tables[slot];
        let damaged = |why: String| {
            LacapError::Damaged(format!("the channel table of channel {slot} is unusable: {why}"))
        };
        if len < (BLOB_HEADER + BLOB_TRAILER) as u64 || offset.checked_add(len).is_none_or(|e| e > valid_end)
        {
            return Err(damaged("it lies outside the file".into()));
        }
        let mut bytes = vec![0u8; usize::try_from(len).map_err(|_| damaged("it is too large".into()))?];
        file.read_exact_at(offset, &mut bytes)?;
        let chunks = decode_table(&bytes, slot, expect_chunks, valid_end).map_err(damaged)?;
        let initial = match (chunks.first(), index.initial_levels[slot]) {
            (Some(c), _) => c.entry,
            (None, 1) => Level::High,
            (None, _) => Level::Low,
        };
        channels.push((initial, chunks));
    }
    let mut meta = index.capture;
    if recovered {
        meta.notes.push(Note {
            level: NoteLevel::Warning,
            message: "The last save of this file was interrupted or is damaged; it was opened as it was at the save before. Saving again removes the unfinished part.".into(),
        });
    }
    let store = CaptureStore::from_parts(channels, index.sample_count, file, valid_end, cache_budget);
    Ok(Opened {
        store,
        meta,
        session: index.session,
        recovered,
    })
}
