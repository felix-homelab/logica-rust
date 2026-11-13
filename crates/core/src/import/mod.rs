//! Importers for other tools' capture files (doc 09). Each reads through [`RandomAccess`], streams
//! into a [`CaptureStore`], records surprises as capture notes, and fails with a message written
//! for the person. Importers for third-party formats are clean-room and have a `PROVENANCE.md`
//! next to their code (`docs/clean-room.md`).

pub mod demo;
pub mod sr;
pub mod vcd;
pub mod zip;

use std::fmt;
use std::sync::Arc;

use crate::capture::{CaptureMeta, Note, NoteLevel};
use crate::io::{MemFile, RandomAccess};
use crate::lacap::LacapError;
use crate::store::{CaptureStore, DEFAULT_CACHE_BUDGET, Level, StoreError};

/// Why a file could not be imported, worded for the person.
#[derive(Debug)]
pub struct ImportError {
    message: String,
}

impl ImportError {
    pub fn new(message: impl Into<String>) -> Self {
        ImportError { message: message.into() }
    }

    pub fn io(e: std::io::Error) -> Self {
        ImportError::new(format!("The file could not be read: {e}."))
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImportError {}

impl From<StoreError> for ImportError {
    fn from(e: StoreError) -> Self {
        ImportError::new(e.to_string())
    }
}

impl From<LacapError> for ImportError {
    fn from(e: LacapError) -> Self {
        ImportError::new(e.to_string())
    }
}

/// Where an imported capture is stored: a working `.lacap` (a file in the data folder) or memory.
#[derive(Clone)]
pub struct StoreTarget {
    pub backing: Arc<dyn RandomAccess>,
    pub cache_budget: usize,
}

impl StoreTarget {
    pub fn memory() -> Self {
        StoreTarget { backing: Arc::new(MemFile::new()), cache_budget: DEFAULT_CACHE_BUDGET }
    }

    pub(crate) fn create(&self, initial: &[Level]) -> Result<CaptureStore, ImportError> {
        Ok(crate::lacap::new_capture(self.backing.clone(), initial, self.cache_budget)?)
    }
}

/// An imported capture: its transitions and everything else.
#[derive(Debug)]
pub struct Imported {
    pub store: CaptureStore,
    pub meta: CaptureMeta,
}

/// Progress: a fraction from 0 to 1 and what is being done.
pub type Progress<'a> = &'a mut dyn FnMut(f64, &str);

pub(crate) fn warn(notes: &mut Vec<Note>, message: impl Into<String>) {
    notes.push(Note { level: NoteLevel::Warning, message: message.into() });
}

pub(crate) fn info(notes: &mut Vec<Note>, message: impl Into<String>) {
    notes.push(Note { level: NoteLevel::Info, message: message.into() });
}

/// Parses `key=value` lines; blank lines, `[sections]` and comments are skipped. Later keys win.
pub(crate) fn parse_ini(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('[') || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=').filter(|&i| i > 0) else {
            continue;
        };
        let (k, v) = (line[..eq].trim().to_string(), line[eq + 1..].trim().to_string());
        out.retain(|(x, _)| *x != k);
        out.push((k, v));
    }
    out
}

pub(crate) fn ini_get<'a>(ini: &'a [(String, String)], key: &str) -> Option<&'a str> {
    ini.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_handles_crlf_sections_and_odd_lines() {
        let m = parse_ini("\r\n[S]\r\na=1\r\nb = x=y\r\nnoequals\r\n=nokey\r\na=2\r\n");
        assert_eq!(ini_get(&m, "a"), Some("2"));
        assert_eq!(ini_get(&m, "b"), Some("x=y"));
        assert_eq!(m.len(), 2);
    }
}
