//! The random-access file interface (doc 03, "Services the engine gets from outside").
//!
//! Synchronous positional reads and writes, so level cursors and decoders stay plain sequential
//! code. Real files are implemented in the `logica-fileio` crate; [`MemFile`] serves tests and small
//! captures.

use std::io;
use std::sync::RwLock;

/// A file read and written by position. All methods take `&self`, so several readers (decoders,
/// the waveform) can share one file while the store appends to it.
pub trait RandomAccess: Send + Sync {
    /// Current size in bytes.
    fn size(&self) -> io::Result<u64>;
    /// Fills `buf` from `offset`; an error if the file ends first.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    /// Writes all of `data` at `offset`, growing the file if needed.
    fn write_all_at(&self, offset: u64, data: &[u8]) -> io::Result<()>;
    /// Cuts or extends the file to `len` bytes (used to drop a partial append, doc 09).
    fn set_len(&self, len: u64) -> io::Result<()>;
    /// Flushes written data to stable storage where that means something.
    fn sync(&self) -> io::Result<()>;
}

/// A file held in memory.
#[derive(Debug, Default)]
pub struct MemFile {
    data: RwLock<Vec<u8>>,
}

impl MemFile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { data: RwLock::new(bytes) }
    }

    /// A copy of the whole content.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.data.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn to_usize(v: u64) -> io::Result<usize> {
    usize::try_from(v).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "offset too large for memory"))
}

impl RandomAccess for MemFile {
    fn size(&self) -> io::Result<u64> {
        Ok(self.data.read().unwrap_or_else(|e| e.into_inner()).len() as u64)
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let data = self.data.read().unwrap_or_else(|e| e.into_inner());
        let start = to_usize(offset)?;
        let end = start
            .checked_add(buf.len())
            .filter(|&e| e <= data.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "read past the end of the file"))?;
        buf.copy_from_slice(&data[start..end]);
        Ok(())
    }

    fn write_all_at(&self, offset: u64, src: &[u8]) -> io::Result<()> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        let start = to_usize(offset)?;
        let end = start
            .checked_add(src.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "write too large"))?;
        if data.len() < end {
            data.resize(end, 0);
        }
        data[start..end].copy_from_slice(src);
        Ok(())
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        let len = to_usize(len)?;
        self.data.write().unwrap_or_else(|e| e.into_inner()).resize(len, 0);
        Ok(())
    }

    fn sync(&self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_grow_and_reads_check_bounds() {
        let f = MemFile::new();
        f.write_all_at(4, b"abc").unwrap();
        assert_eq!(f.size().unwrap(), 7);
        let mut buf = [0u8; 3];
        f.read_exact_at(4, &mut buf).unwrap();
        assert_eq!(&buf, b"abc");
        let err = f.read_exact_at(5, &mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        f.set_len(5).unwrap();
        assert_eq!(f.to_bytes(), b"\0\0\0\0a");
    }

    #[test]
    fn huge_offsets_are_errors_not_panics() {
        let f = MemFile::new();
        let mut buf = [0u8; 1];
        assert!(f.read_exact_at(u64::MAX, &mut buf).is_err());
        assert!(f.write_all_at(u64::MAX, b"x").is_err());
    }
}
