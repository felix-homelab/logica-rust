//! `.lacap` blobs for chunks and fine LODs (doc 09). The store writes these to its backing on
//! eviction, so a working capture is already a `.lacap` file in the making.
//!
//! Every blob: u32 type, u32 flags (0), u64 payload length, payload, u32 CRC-32 of the payload.

use super::chunk::Payload;
use super::lod::FineLod;
use super::{CHUNK_BITS_BYTES, CHUNK_SAMPLES, CHUNK_WORDS, FINE_BUCKETS, FINE_SHIFT, Level, MAX_OFFSETS};
use super::{bits::idx, chunk::ChunkKind, chunk_base};

pub const BLOB_CHUNK: u32 = 1;
pub const BLOB_FINE_LOD: u32 = 2;
#[expect(dead_code, reason = "annotation blobs are defined in M1 (doc 09)")]
pub const BLOB_ANNOTATIONS: u32 = 3;
pub const BLOB_CHANNEL_TABLE: u32 = 4;

/// Bytes before the payload.
pub const BLOB_HEADER: usize = 16;
/// Bytes after the payload (the CRC).
pub const BLOB_TRAILER: usize = 4;
const CHUNK_HEADER: usize = 24;
const FINE_HEADER: usize = 24;

/// Wraps `payload` as a blob of `kind`.
pub fn wrap(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BLOB_HEADER + payload.len() + BLOB_TRAILER);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
    out
}

/// Checks a whole blob (header, payload, CRC) of the expected kind and returns its payload.
pub fn unwrap(blob: &[u8], kind: u32) -> Result<&[u8], String> {
    if blob.len() < BLOB_HEADER + BLOB_TRAILER {
        return Err("the blob is cut short".into());
    }
    let got_kind = u32_at(blob, 0);
    if got_kind != kind {
        return Err(format!("expected a blob of type {kind}, found type {got_kind}"));
    }
    let len = u64_at(blob, 8);
    if len != (blob.len() - BLOB_HEADER - BLOB_TRAILER) as u64 {
        return Err("the blob length does not match the table".into());
    }
    let payload = &blob[BLOB_HEADER..blob.len() - BLOB_TRAILER];
    let crc = u32_at(blob, blob.len() - BLOB_TRAILER);
    if crc32fast::hash(payload) != crc {
        return Err("checksum mismatch".into());
    }
    Ok(payload)
}

pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// A chunk blob for channel `slot`, chunk `chunk`, with `transitions` from its summary.
pub fn encode_chunk(slot: u32, chunk: u64, entry: Level, transitions: u32, payload: &Payload) -> Vec<u8> {
    let mut p = Vec::with_capacity(CHUNK_HEADER + payload.heap_bytes());
    p.extend_from_slice(&slot.to_le_bytes());
    p.extend_from_slice(&(payload.kind() as u32).to_le_bytes());
    p.extend_from_slice(&chunk_base(chunk).to_le_bytes());
    p.extend_from_slice(&[entry as u8, 0, 0, 0]);
    p.extend_from_slice(&transitions.to_le_bytes());
    match payload {
        Payload::Offsets(o) => o.iter().for_each(|v| p.extend_from_slice(&v.to_le_bytes())),
        Payload::Bits(w) => w.iter().for_each(|v| p.extend_from_slice(&v.to_le_bytes())),
    }
    wrap(BLOB_CHUNK, &p)
}

/// Reads a chunk blob, checking that it belongs where the table says.
pub fn decode_chunk(blob: &[u8], slot: u32, chunk: u64) -> Result<(Payload, Level, u32), String> {
    let p = unwrap(blob, BLOB_CHUNK)?;
    if p.len() < CHUNK_HEADER {
        return Err("the chunk header is cut short".into());
    }
    if u32_at(p, 0) != slot || u64_at(p, 8) != chunk_base(chunk) {
        return Err("the blob belongs to another chunk".into());
    }
    let entry = match p[16] {
        0 => Level::Low,
        1 => Level::High,
        _ => return Err("the entry level is neither 0 nor 1".into()),
    };
    let transitions = u32_at(p, 20);
    let data = &p[CHUNK_HEADER..];
    let payload = match ChunkKind::from_byte(u8::try_from(u32_at(p, 4)).unwrap_or(u8::MAX)) {
        Some(ChunkKind::Offsets) => {
            if transitions > MAX_OFFSETS || data.len() != idx(u64::from(transitions)) * 4 {
                return Err("the offsets do not match their count".into());
            }
            let offs: Box<[u32]> = data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect();
            if offs.windows(2).any(|w| w[0] >= w[1])
                || offs.last().is_some_and(|&o| u64::from(o) >= CHUNK_SAMPLES)
            {
                return Err("the offsets are not ascending inside the chunk".into());
            }
            Payload::Offsets(offs)
        }
        Some(ChunkKind::Bits) => {
            if data.len() != CHUNK_BITS_BYTES {
                return Err("the raw bits have the wrong length".into());
            }
            let words: Box<[u64]> = data
                .as_chunks::<8>()
                .0
                .iter()
                .map(|c| u64::from_le_bytes(*c))
                .collect();
            debug_assert_eq!(words.len(), CHUNK_WORDS);
            Payload::Bits(words)
        }
        _ => return Err("unknown chunk kind".into()),
    };
    Ok((payload, entry, transitions))
}

/// A fine LOD blob.
pub fn encode_fine(slot: u32, chunk: u64, fine: &FineLod) -> Vec<u8> {
    let mut p = Vec::with_capacity(FINE_HEADER + fine.heap_bytes());
    p.extend_from_slice(&slot.to_le_bytes());
    p.extend_from_slice(&FINE_SHIFT.to_le_bytes());
    p.extend_from_slice(&chunk_base(chunk).to_le_bytes());
    p.extend_from_slice(&u32::try_from(fine.counts.len()).unwrap_or(0).to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    fine.counts
        .iter()
        .for_each(|c| p.extend_from_slice(&c.to_le_bytes()));
    p.extend_from_slice(&fine.start_levels);
    wrap(BLOB_FINE_LOD, &p)
}

pub fn decode_fine(blob: &[u8], slot: u32, chunk: u64) -> Result<FineLod, String> {
    let p = unwrap(blob, BLOB_FINE_LOD)?;
    if p.len() != FINE_HEADER + FINE_BUCKETS * 2 + FINE_BUCKETS / 8 {
        return Err("the fine LOD has the wrong length".into());
    }
    if u32_at(p, 0) != slot || u64_at(p, 8) != chunk_base(chunk) {
        return Err("the fine LOD belongs to another chunk".into());
    }
    if u32_at(p, 4) != FINE_SHIFT || idx(u64::from(u32_at(p, 16))) != FINE_BUCKETS {
        return Err("the fine LOD has an unknown bucket size".into());
    }
    let counts_end = FINE_HEADER + FINE_BUCKETS * 2;
    Ok(FineLod {
        counts: p[FINE_HEADER..counts_end]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect(),
        start_levels: p[counts_end..].into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_blobs_round_trip_and_catch_damage() {
        let p = Payload::Offsets(vec![0, 5, 1 << 23].into_boxed_slice());
        let blob = encode_chunk(3, 300, Level::High, 3, &p);
        assert_eq!(blob.len(), BLOB_HEADER + 24 + 12 + BLOB_TRAILER);
        assert_eq!(decode_chunk(&blob, 3, 300).unwrap(), (p, Level::High, 3));
        assert!(decode_chunk(&blob, 2, 300).unwrap_err().contains("another chunk"));
        let mut bad = blob.clone();
        bad[30] ^= 1;
        assert!(decode_chunk(&bad, 3, 300).unwrap_err().contains("checksum"));
        assert!(
            decode_chunk(&blob[..10], 3, 300)
                .unwrap_err()
                .contains("cut short")
        );
        assert!(
            decode_chunk(&blob[..blob.len() - 1], 3, 300)
                .unwrap_err()
                .contains("length")
        );
    }

    #[test]
    fn bits_and_fine_lods_round_trip() {
        let mut w = vec![0u64; CHUNK_WORDS];
        w[0] = 0b1010;
        let p = Payload::Bits(w.into_boxed_slice());
        let blob = encode_chunk(0, 1, Level::Low, 4, &p);
        let (back, entry, t) = decode_chunk(&blob, 0, 1).unwrap();
        assert_eq!((back, entry, t), (p, Level::Low, 4));

        let mut f = FineLod::new();
        f.counts[7] = 256;
        f.start_levels[1] = 3;
        let blob = encode_fine(1, 2, &f);
        assert_eq!(decode_fine(&blob, 1, 2).unwrap(), f);
        assert!(decode_fine(&blob, 1, 3).is_err());
        assert!(decode_chunk(&blob, 1, 2).unwrap_err().contains("type"));
    }

    #[test]
    fn offsets_that_are_not_ascending_are_damage() {
        let p = Payload::Offsets(vec![5, 5].into_boxed_slice());
        let blob = encode_chunk(0, 0, Level::Low, 2, &p);
        assert!(decode_chunk(&blob, 0, 0).unwrap_err().contains("ascending"));
    }
}
