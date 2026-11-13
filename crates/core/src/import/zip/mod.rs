//! A small ZIP reader over [`RandomAccess`], for `.demo` and `.sr` (doc 09). It reads the central
//! directory once and inflates one entry at a time, streaming, so a capture is never unpacked in
//! memory as a whole. Stored and deflate entries; ZIP64; the CRC-32 of every entry is checked.
//! Written from PKWARE's published APPNOTE (see PROVENANCE.md).

use std::collections::HashMap;

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use crate::io::RandomAccess;

use super::ImportError;

const EOCD: u32 = 0x0605_4b50;
const EOCD64_LOCATOR: u32 = 0x0706_4b50;
const EOCD64: u32 = 0x0606_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    method: u16,
    compressed: u64,
    /// Uncompressed size.
    pub size: u64,
    crc: u32,
    local_header: u64,
}

/// An opened archive: its entries by name.
pub struct ZipArchive<'a> {
    file: &'a dyn RandomAccess,
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

fn err(m: impl Into<String>) -> ImportError {
    ImportError::new(m)
}

fn read(file: &dyn RandomAccess, at: u64, len: usize) -> Result<Vec<u8>, ImportError> {
    let mut b = vec![0u8; len];
    file.read_exact_at(at, &mut b).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            err("The ZIP archive is cut short. Was the file fully copied?")
        } else {
            ImportError::io(e)
        }
    })?;
    Ok(b)
}

impl<'a> ZipArchive<'a> {
    /// Reads the central directory. `what` names the expected format in messages, for example
    /// "a DLA_Logic .demo file".
    pub fn open(file: &'a dyn RandomAccess, what: &str) -> Result<Self, ImportError> {
        let size = file.size().map_err(ImportError::io)?;
        if size < 22 {
            return Err(err(format!("The file is too small to be a ZIP archive, so it cannot be {what}.")));
        }
        // The end-of-central-directory record is within the last 22 + 65,535 bytes.
        let tail_len = size.min(22 + 0xffff);
        let tail = read(file, size - tail_len, usize::try_from(tail_len).unwrap_or(0))?;
        let eocd = (0..=tail.len() - 22)
            .rev()
            .find(|&i| u32_at(&tail, i) == EOCD)
            .ok_or_else(|| err(format!("This is not a ZIP archive, so it cannot be {what}.")))?;
        let eocd_at = size - tail_len + eocd as u64;
        let mut count = u64::from(u16_at(&tail, eocd + 10));
        let mut dir_len = u64::from(u32_at(&tail, eocd + 12));
        let mut dir_at = u64::from(u32_at(&tail, eocd + 16));
        if count == 0xffff || dir_at == 0xffff_ffff || dir_len == 0xffff_ffff {
            // ZIP64: the locator sits right before the classic record.
            if eocd_at < 20 {
                return Err(err("The ZIP64 directory of this archive is missing."));
            }
            let loc = read(file, eocd_at - 20, 20)?;
            if u32_at(&loc, 0) != EOCD64_LOCATOR {
                return Err(err("The ZIP64 directory of this archive is missing."));
            }
            let rec = read(file, u64_at(&loc, 8), 56)?;
            if u32_at(&rec, 0) != EOCD64 {
                return Err(err("The ZIP64 directory of this archive is damaged."));
            }
            count = u64_at(&rec, 32);
            dir_len = u64_at(&rec, 40);
            dir_at = u64_at(&rec, 48);
        }
        if dir_at.checked_add(dir_len).is_none_or(|e| e > size) || dir_len > 1 << 30 {
            return Err(err(
                "The ZIP directory points outside the file; the archive is damaged or cut short.",
            ));
        }
        let dir = read(file, dir_at, usize::try_from(dir_len).unwrap_or(0))?;
        let mut entries = Vec::new();
        let mut by_name = HashMap::new();
        let mut p = 0usize;
        for _ in 0..count {
            if p + 46 > dir.len() || u32_at(&dir, p) != CENTRAL {
                return Err(err("The ZIP directory is damaged."));
            }
            let name_len = usize::from(u16_at(&dir, p + 28));
            let extra_len = usize::from(u16_at(&dir, p + 30));
            let comment_len = usize::from(u16_at(&dir, p + 32));
            if p + 46 + name_len + extra_len > dir.len() {
                return Err(err("The ZIP directory is damaged."));
            }
            let name = String::from_utf8_lossy(&dir[p + 46..p + 46 + name_len]).replace('\\', "/");
            let mut e = Entry {
                name,
                method: u16_at(&dir, p + 10),
                crc: u32_at(&dir, p + 16),
                compressed: u64::from(u32_at(&dir, p + 20)),
                size: u64::from(u32_at(&dir, p + 24)),
                local_header: u64::from(u32_at(&dir, p + 42)),
            };
            // ZIP64 extra field (id 1): the values that are 0xffffffff in the record, in order.
            let extra = &dir[p + 46 + name_len..p + 46 + name_len + extra_len];
            let mut x = 0usize;
            while x + 4 <= extra.len() {
                let (id, len) = (u16_at(extra, x), usize::from(u16_at(extra, x + 2)));
                if id == 1 {
                    let mut q = x + 4;
                    for field in [&mut e.size, &mut e.compressed, &mut e.local_header] {
                        if *field == 0xffff_ffff && q + 8 <= x + 4 + len.min(extra.len() - x - 4) {
                            *field = u64_at(extra, q);
                            q += 8;
                        }
                    }
                }
                x += 4 + len;
            }
            by_name.insert(e.name.clone(), entries.len());
            entries.push(e);
            p += 46 + name_len + extra_len + comment_len;
        }
        Ok(ZipArchive { file, entries, by_name })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.by_name.get(name).map(|&i| &self.entries[i])
    }

    /// Streams the uncompressed bytes of `name` to `sink` in pieces; returns the total length.
    /// The CRC and the size are checked at the end.
    pub fn stream(
        &self,
        name: &str,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), ImportError>,
    ) -> Result<u64, ImportError> {
        let e = self.entry(name).ok_or_else(|| err(format!("The archive has no entry \"{name}\".")))?;
        let damaged = || err(format!("The entry \"{name}\" in the archive is damaged."));
        let h = read(self.file, e.local_header, 30)?;
        if u32_at(&h, 0) != LOCAL {
            return Err(damaged());
        }
        let start = e.local_header + 30 + u64::from(u16_at(&h, 26)) + u64::from(u16_at(&h, 28));
        let size = self.file.size().map_err(ImportError::io)?;
        if start.checked_add(e.compressed).is_none_or(|end| end > size) {
            return Err(err(format!("The entry \"{name}\" is cut short. Was the file fully copied?")));
        }
        let mut crc = crc32fast::Hasher::new();
        let mut total = 0u64;
        const PIECE: u64 = 1 << 18;
        match e.method {
            0 => {
                let mut at = 0u64;
                while at < e.compressed {
                    let n = PIECE.min(e.compressed - at);
                    let b = read(self.file, start + at, usize::try_from(n).unwrap_or(0))?;
                    crc.update(&b);
                    sink(&b)?;
                    at += n;
                }
                total = e.compressed;
            }
            8 => {
                let mut state = InflateState::new_boxed(DataFormat::Raw);
                let mut out = vec![0u8; 1 << 20];
                let mut at = 0u64;
                let mut input: Vec<u8> = Vec::new();
                let mut used = 0usize;
                let mut stalls = 0u32;
                loop {
                    if used == input.len() && at < e.compressed {
                        let n = PIECE.min(e.compressed - at);
                        input = read(self.file, start + at, usize::try_from(n).unwrap_or(0))?;
                        used = 0;
                        at += n;
                    }
                    // Never `Finish`: that mode expects the rest of the output to fit in `out`.
                    let r = inflate(&mut state, &input[used..], &mut out, MZFlush::None);
                    used += r.bytes_consumed;
                    if r.bytes_written > 0 {
                        crc.update(&out[..r.bytes_written]);
                        total += r.bytes_written as u64;
                        if total > e.size {
                            return Err(damaged());
                        }
                        sink(&out[..r.bytes_written])?;
                    }
                    match r.status {
                        Ok(MZStatus::StreamEnd) => break,
                        Ok(_) | Err(MZError::Buf) => {
                            // No progress with all input given: the stream is cut short.
                            let stalled = r.bytes_consumed == 0 && r.bytes_written == 0;
                            stalls = if stalled { stalls + 1 } else { 0 };
                            if stalled && (used == input.len() && at == e.compressed || stalls > 4) {
                                return Err(damaged());
                            }
                        }
                        Err(_) => return Err(damaged()),
                    }
                }
            }
            m => {
                return Err(err(format!(
                    "The entry \"{name}\" uses ZIP compression method {m}, which this tool does not read."
                )));
            }
        }
        if total != e.size || crc.finalize() != e.crc {
            return Err(damaged());
        }
        Ok(total)
    }

    /// The whole entry, if it exists; refused above `limit` bytes.
    pub fn read(&self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, ImportError> {
        let Some(e) = self.entry(name) else {
            return Ok(None);
        };
        if e.size > limit {
            return Err(err(format!("The entry \"{name}\" is larger than expected ({} bytes).", e.size)));
        }
        let mut out = Vec::with_capacity(usize::try_from(e.size).unwrap_or(0));
        self.stream(name, &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })?;
        Ok(Some(out))
    }

    /// The entry as text (UTF-8, invalid bytes replaced), if it exists.
    pub fn read_text(&self, name: &str) -> Result<Option<String>, ImportError> {
        Ok(self.read(name, 16 << 20)?.map(|b| String::from_utf8_lossy(&b).into_owned()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::io::MemFile;

    /// Builds a ZIP archive (stored or deflated entries) for tests.
    pub fn make_zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut dir = Vec::new();
        for (name, data) in entries {
            let (method, body) = if deflate {
                (8u16, miniz_oxide::deflate::compress_to_vec(data, 6))
            } else {
                (0u16, data.to_vec())
            };
            let crc = crc32fast::hash(data);
            let at = out.len() as u32;
            let mut h = Vec::new();
            h.extend_from_slice(&LOCAL.to_le_bytes());
            h.extend_from_slice(&[20, 0, 0, 0]);
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&[0; 4]);
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(body.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&h);
            out.extend_from_slice(&body);
            let mut c = Vec::new();
            c.extend_from_slice(&CENTRAL.to_le_bytes());
            c.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            c.extend_from_slice(&method.to_le_bytes());
            c.extend_from_slice(&[0; 4]);
            c.extend_from_slice(&crc.to_le_bytes());
            c.extend_from_slice(&(body.len() as u32).to_le_bytes());
            c.extend_from_slice(&(data.len() as u32).to_le_bytes());
            c.extend_from_slice(&(name.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0; 12]);
            c.extend_from_slice(&at.to_le_bytes());
            c.extend_from_slice(name.as_bytes());
            dir.extend_from_slice(&c);
        }
        let dir_at = out.len() as u32;
        out.extend_from_slice(&dir);
        out.extend_from_slice(&EOCD.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(dir.len() as u32).to_le_bytes());
        out.extend_from_slice(&dir_at.to_le_bytes());
        out.extend_from_slice(&[0; 2]);
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries_in_pieces() {
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i / 7 % 251) as u8).collect();
        for deflate in [false, true] {
            let z = MemFile::from_bytes(make_zip(&[("a.txt", b"hello"), ("dir\\big.bin", &big)], deflate));
            let a = ZipArchive::open(&z, "a test").unwrap();
            assert_eq!(a.names().collect::<Vec<_>>(), vec!["a.txt", "dir/big.bin"]);
            assert_eq!(a.read_text("a.txt").unwrap().unwrap(), "hello");
            let mut pieces = 0;
            let mut got = Vec::new();
            a.stream("dir/big.bin", &mut |b| {
                pieces += 1;
                got.extend_from_slice(b);
                Ok(())
            })
            .unwrap();
            assert_eq!(got, big);
            assert!(pieces > 1);
            assert!(a.read("missing", 10).unwrap().is_none());
            assert!(a.read("dir/big.bin", 10).is_err());
        }
    }

    #[test]
    fn damaged_and_foreign_archives_get_clear_messages() {
        let open = |b: Vec<u8>| {
            ZipArchive::open(&MemFile::from_bytes(b), "a DLA_Logic .demo file").err().map(|e| e.to_string())
        };
        assert_eq!(
            open(vec![1, 2, 3]).unwrap(),
            "The file is too small to be a ZIP archive, so it cannot be a DLA_Logic .demo file."
        );
        assert_eq!(
            open(b"hello, not a zip at all, just some text".to_vec()).unwrap(),
            "This is not a ZIP archive, so it cannot be a DLA_Logic .demo file."
        );
        let good = make_zip(&[("x", &[7u8; 5000])], true);
        // flip a byte in the compressed data: the CRC or the inflater catches it
        let mut bad = good.clone();
        bad[40] ^= 0x55;
        let f = MemFile::from_bytes(bad);
        let a = ZipArchive::open(&f, "t").unwrap();
        assert_eq!(
            a.read("x", 1 << 20).unwrap_err().to_string(),
            "The entry \"x\" in the archive is damaged."
        );
        // a directory that points past the end
        let mut cut = good.clone();
        let n = cut.len();
        cut[n - 6..n - 2].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(open(cut).unwrap().contains("ZIP64"));
    }
}
