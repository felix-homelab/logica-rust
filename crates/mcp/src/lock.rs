//! The lock file (doc 07, ADR 0007): how `logica mcp` finds the running engine. Whoever serves MCP
//! over HTTP (the app, or a windowless engine) writes it with the port and a random token, readable
//! by this user only, and removes it when it stops.

use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "logica.lock";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub api_version: String,
    /// Written by an engine without a window rather than by the app.
    pub headless: bool,
}

/// The data folder: `LOGICA_DATA_DIR`, or where Electron puts the app's user data, so the app and
/// the CLI agree (doc 03).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LOGICA_DATA_DIR") {
        return PathBuf::from(d);
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home)
    } else if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
    };
    base.join("Logica")
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// A new random token: 32 bytes as hex.
pub fn new_token() -> std::io::Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b)
        .map_err(|e| std::io::Error::other(format!("No random numbers for the token: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// The lock in `dir`, if there is one and it can be read.
pub fn read(dir: &Path) -> Option<Lock> {
    serde_json::from_slice(&std::fs::read(path(dir)).ok()?).ok()
}

/// Whether something answers on the lock's port.
pub fn alive(lock: &Lock) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, lock.port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// The lock of a running engine, if any; a lock nobody answers for is stale and ignored.
pub fn running(dir: &Path) -> Option<Lock> {
    read(dir).filter(alive)
}

/// Writes the lock, readable and writable by this user only. Fails with `AlreadyExists` when a
/// lock of a running engine is there (another one won the race); a stale lock is replaced.
pub fn write(dir: &Path, lock: &Lock) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let p = path(dir);
    if running(dir).is_some() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "Another engine is running."));
    }
    let _ = std::fs::remove_file(&p);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&p)?;
    f.write_all(&serde_json::to_vec_pretty(lock).map_err(std::io::Error::other)?)?;
    f.sync_all()
}

/// Removes the lock if it is still this one (another engine may have replaced a stale copy).
pub fn remove_if_ours(dir: &Path, lock: &Lock) {
    if read(dir).as_ref() == Some(lock) {
        let _ = std::fs::remove_file(path(dir));
    }
}
