//! Finding and starting the desktop app (ADR 0007). The app records where it is installed in
//! `app.json` in the data folder each time it starts; `logica mcp` and `logica open` read it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::lock;

pub const FILE: &str = "app.json";

/// Passed to the app when an agent's `logica mcp` starts it: it opens minimised and says so.
pub const STARTED_BY_AGENT: &str = "--started-by-agent";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// The app's executable.
    pub executable: PathBuf,
    pub version: String,
}

/// The app recorded in `dir`, if its executable still exists.
pub fn installed(dir: &Path) -> Option<AppInfo> {
    let info: AppInfo = serde_json::from_slice(&std::fs::read(dir.join(FILE)).ok()?).ok()?;
    info.executable.is_file().then_some(info)
}

/// Whether a window can be shown: always on Windows and macOS; on Linux only with an X11 or
/// Wayland display (not in CI or over SSH).
pub fn display_available() -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        return true;
    }
    ["DISPLAY", "WAYLAND_DISPLAY"].iter().any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

/// Starts the app with `args`, detached from this process, so it runs on after it.
pub fn launch(app: &AppInfo, args: &[&std::ffi::OsStr]) -> std::io::Result<()> {
    std::process::Command::new(&app.executable)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
}

/// Waits until an engine announces itself in `dir`'s lock file.
pub fn wait_for_engine(dir: &Path, timeout: Duration) -> Option<lock::Lock> {
    let until = Instant::now() + timeout;
    loop {
        if let Some(l) = lock::running(dir) {
            return Some(l);
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("logica-app-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_recorded_app_counts_only_while_its_executable_exists() {
        let d = scratch("installed");
        assert_eq!(installed(&d), None);
        let exe = std::env::current_exe().unwrap();
        let info = AppInfo { executable: exe.clone(), version: "0.1.0".into() };
        std::fs::write(d.join(FILE), serde_json::to_vec(&info).unwrap()).unwrap();
        assert_eq!(installed(&d), Some(info));
        let gone = AppInfo { executable: d.join("uninstalled"), version: "0.1.0".into() };
        std::fs::write(d.join(FILE), serde_json::to_vec(&gone).unwrap()).unwrap();
        assert_eq!(installed(&d), None);
        std::fs::write(d.join(FILE), "not json").unwrap();
        assert_eq!(installed(&d), None);
        std::fs::remove_file(d.join(FILE)).unwrap();
        std::fs::remove_dir(&d).unwrap();
    }

    #[test]
    fn waiting_for_an_engine_gives_up() {
        let d = scratch("wait");
        let t = Instant::now();
        assert_eq!(wait_for_engine(&d, Duration::from_millis(250)), None);
        assert!(t.elapsed() >= Duration::from_millis(250));
        std::fs::remove_dir(&d).unwrap();
    }
}
