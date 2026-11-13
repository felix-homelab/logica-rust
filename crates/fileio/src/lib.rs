//! [`RandomAccess`] on real files, for the engine, the CLI, tests and benchmarks. `logica-core`
//! does no I/O of its own (doc 03); this crate is the one place it touches `std::fs`.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use logica_core::io::RandomAccess;

/// A file on disk, read and written by position.
#[derive(Debug)]
pub struct StdFile {
    file: File,
    writable: bool,
}

impl StdFile {
    /// Opens an existing file for reading only; writes fail with `PermissionDenied`.
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(StdFile {
            file: File::open(path)?,
            writable: false,
        })
    }

    /// Opens an existing file for reading and writing.
    pub fn open_rw(path: &Path) -> io::Result<Self> {
        Ok(StdFile {
            file: OpenOptions::new().read(true).write(true).open(path)?,
            writable: true,
        })
    }

    /// Creates a file, or empties an existing one, for reading and writing.
    pub fn create(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(StdFile { file, writable: true })
    }

    fn check_writable(&self) -> io::Result<()> {
        if self.writable {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the file is open for reading only",
            ))
        }
    }
}

impl RandomAccess for StdFile {
    fn size(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        imp::read_exact_at(&self.file, offset, buf)
    }

    fn write_all_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.check_writable()?;
        imp::write_all_at(&self.file, offset, data)
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        self.check_writable()?;
        self.file.set_len(len)
    }

    fn sync(&self) -> io::Result<()> {
        if self.writable {
            self.file.sync_data()
        } else {
            Ok(())
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::unix::fs::FileExt;

    pub fn read_exact_at(f: &File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        f.read_exact_at(buf, offset)
    }

    pub fn write_all_at(f: &File, offset: u64, data: &[u8]) -> io::Result<()> {
        f.write_all_at(data, offset)
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::windows::fs::FileExt;

    pub fn read_exact_at(f: &File, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        while !buf.is_empty() {
            match f.seek_read(buf, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "read past the end of the file",
                    ));
                }
                Ok(n) => {
                    buf = &mut buf[n..];
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn write_all_at(f: &File, mut offset: u64, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            match f.seek_write(data, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "could not write to the file",
                    ));
                }
                Ok(n) => {
                    data = &data[n..];
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("logica-fileio-{}-{name}", std::process::id()))
    }

    #[test]
    fn positional_reads_writes_and_truncation() {
        let p = temp_path("rw");
        let f = StdFile::create(&p).unwrap();
        f.write_all_at(3, b"xyz").unwrap();
        let mut buf = [0u8; 3];
        f.read_exact_at(3, &mut buf).unwrap();
        assert_eq!(&buf, b"xyz");
        assert_eq!(
            f.read_exact_at(4, &mut buf).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        f.set_len(4).unwrap();
        assert_eq!(f.size().unwrap(), 4);
        drop(f);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn read_only_files_refuse_writes() {
        let p = temp_path("ro");
        std::fs::write(&p, b"abc").unwrap();
        let f = StdFile::open(&p).unwrap();
        assert_eq!(
            f.write_all_at(0, b"x").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(f.set_len(0).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        drop(f);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn missing_files_are_errors() {
        assert_eq!(
            StdFile::open(&temp_path("missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
