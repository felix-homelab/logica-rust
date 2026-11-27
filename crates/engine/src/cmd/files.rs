//! Opening, saving, closing and renaming captures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::import::{self, StoreTarget};
use logica_core::io::RandomAccess;
use logica_core::lacap;
use logica_fileio::StdFile;

use crate::contract::{CommandError, ErrorCode};
use crate::engine::{Ctx, Done, Engine, err};
use crate::state::{CaptureSummary, SavedSession};

use super::{Nothing, inv, out};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileOpenIn {
    /// A path on the machine running the engine.
    pub path: String,
    /// The capture id to use (redo uses the id it had); a new one when absent.
    pub id: Option<String>,
    /// The tab title; the file name when absent.
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CaptureOut {
    pub capture: CaptureSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRef {
    pub capture: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Lacap,
    Demo,
    Sr,
    Vcd,
}

fn io_err(path: &Path, e: &std::io::Error) -> CommandError {
    let code = if e.kind() == std::io::ErrorKind::NotFound { ErrorCode::NotFound } else { ErrorCode::Failed };
    err(code, format!("{} could not be opened: {e}.", path.display()))
}

/// The format from the extension, or else from the first bytes.
fn format_of(path: &Path, file: &dyn RandomAccess) -> Result<Format, CommandError> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("lacap") => return Ok(Format::Lacap),
        Some("demo") => return Ok(Format::Demo),
        Some("sr") => return Ok(Format::Sr),
        Some("vcd") => return Ok(Format::Vcd),
        _ => {}
    }
    let mut head = [0u8; 8];
    let n = usize::try_from(file.size().unwrap_or(0).min(8)).unwrap_or(0);
    file.read_exact_at(0, &mut head[..n]).map_err(|e| io_err(path, &e))?;
    if head.starts_with(lacap::MAGIC) {
        Ok(Format::Lacap)
    } else if head.starts_with(b"PK") {
        // a ZIP: a .demo has channel.ini, an .sr has version
        let zip = import::zip::ZipArchive::open(file, "a capture")
            .map_err(|e| err(ErrorCode::InvalidInput, e.to_string()))?;
        if zip.entry("channel.ini").is_some() { Ok(Format::Demo) } else { Ok(Format::Sr) }
    } else if head.iter().take(n).find(|b| !b.is_ascii_whitespace()) == Some(&b'$') {
        Ok(Format::Vcd)
    } else {
        Err(err(
            ErrorCode::InvalidInput,
            format!(
                "{} is not a capture format this version reads (.lacap, FNIRSI .demo, sigrok .sr, VCD). Saleae, CSV and raw binary follow in milestone M5.",
                path.display()
            ),
        ))
    }
}

pub(crate) fn open(e: &mut Engine, _: &Ctx, i: FileOpenIn) -> Result<Done, CommandError> {
    let path = PathBuf::from(&i.path);
    let id = match &i.id {
        Some(id) if e.captures.contains_key(id) => {
            return Err(err(
                ErrorCode::InvalidInput,
                format!("A capture with the id \"{id}\" is already open."),
            ));
        }
        Some(id) => {
            e.reserve_id(id);
            id.clone()
        }
        None => e.new_id("c"),
    };
    let name = path.file_name().map_or_else(|| i.path.clone(), |n| n.to_string_lossy().into_owned());
    let title = i.title.clone().unwrap_or_else(|| name.clone());
    let source = StdFile::open(&path).map_err(|x| io_err(&path, &x))?;
    let format = format_of(&path, &source)?;
    let budget = e.cfg.cache_budget;
    let (store, mut meta, session, backed) = if format == Format::Lacap {
        drop(source);
        // read-write, so saving appends in place (doc 09); read-only if that is not allowed
        let file: Arc<dyn RandomAccess> = match StdFile::open_rw(&path) {
            Ok(f) => Arc::new(f),
            Err(_) => Arc::new(StdFile::open(&path).map_err(|x| io_err(&path, &x))?),
        };
        let o = lacap::open(file, budget).map_err(|x| err(ErrorCode::InvalidInput, x.to_string()))?;
        let session: SavedSession = serde_json::from_value(o.session).unwrap_or_default();
        (o.store, o.meta, session, true)
    } else {
        let target = match &e.cfg.data_dir {
            Some(dir) => {
                let work = dir.join(format!("{id}.lacap"));
                // the working folder may not exist yet (a first start)
                std::fs::create_dir_all(dir).map_err(|x| io_err(dir, &x))?;
                let f = StdFile::create(&work).map_err(|x| io_err(&work, &x))?;
                StoreTarget { backing: Arc::new(f), cache_budget: budget }
            }
            None => StoreTarget { backing: Arc::new(logica_core::io::MemFile::new()), cache_budget: budget },
        };
        let importer = match format {
            Format::Demo => import::demo::import,
            Format::Sr => import::sr::import,
            _ => import::vcd::import,
        };
        let imp = importer(&source, &name, &target, &mut |_, _| {})
            .map_err(|x| err(ErrorCode::InvalidInput, x.to_string()))?;
        (imp.store, imp.meta, SavedSession::default(), false)
    };
    meta.id = id.clone();
    meta.title = title.clone();
    e.add_capture(id.clone(), meta, store, Some(path.clone()), backed);
    // restore what was saved with the capture
    for (did, cfg) in session.decoders {
        e.reserve_id(&did);
        let result = match super::decoders::run_decode(e, &id, &cfg) {
            Ok(r) => r,
            Err(x) => {
                let c = e.capture_mut(&id)?;
                c.meta.notes.push(logica_core::capture::Note {
                    level: logica_core::capture::NoteLevel::Warning,
                    message: format!("The saved decoder {did} could not be restored: {}", x.message),
                });
                continue;
            }
        };
        let d = super::decoders::decoder(e, cfg, result);
        e.capture_mut(&id)?.decoders.insert(did, d);
    }
    for (aid, cfg) in session.analog {
        e.reserve_id(&aid);
        let c = e.capture_mut(&id)?;
        // math channels are computed below, once the channels they read are there
        let made = if cfg.source == crate::math::MATH {
            crate::analog::Analog::empty(cfg)
        } else {
            crate::analog::Analog::compute(cfg, &c.store, &c.meta)
        };
        match made {
            Ok(a) => {
                c.analog.insert(aid, a);
            }
            Err(x) => c.meta.notes.push(logica_core::capture::Note {
                level: logica_core::capture::NoteLevel::Warning,
                message: format!("The saved virtual analog channel {aid} could not be restored: {x}"),
            }),
        }
    }
    crate::math::refresh(e.capture_mut(&id)?);
    let c = e.capture_mut(&id)?;
    for m in session.markers {
        c.markers.insert(m.id.clone(), m);
    }
    c.groups = session.groups;
    c.hidden = session.hidden.into_iter().collect();
    for m in c.markers.keys().cloned().collect::<Vec<_>>() {
        e.reserve_id(&m);
    }
    let summary = CaptureSummary::of(&id, e.capture(&id)?);
    let canonical = FileOpenIn { path: i.path.clone(), id: Some(id.clone()), title: Some(title) };
    Ok(Done::edit(
        out(&CaptureOut { capture: summary }),
        inv("capture.close", &CaptureRef { capture: id }),
        format!("opened {name}"),
    )
    .canonical(out(&canonical)))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileSaveIn {
    pub capture: String,
    /// Where to save; a path ending in .lacap. Without it, the .lacap the capture came from.
    pub path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FileSaveOut {
    pub path: String,
    pub bytes: u64,
}

pub(crate) fn save(e: &mut Engine, _: &Ctx, i: FileSaveIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    if c.meta.state == logica_core::capture::CaptureState::Capturing {
        return Err(err(
            ErrorCode::Busy,
            "This capture is still running; stop it with capture.stop, then save it.",
        ));
    }
    let target = match (&i.path, c.backed_by_path, &c.path) {
        (Some(p), _, _) => PathBuf::from(p),
        (None, true, Some(p)) => p.clone(),
        // saved before with "save as": save to that file again
        (None, false, Some(p))
            if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("lacap")) =>
        {
            p.clone()
        }
        _ => return Err(err(ErrorCode::InvalidInput, "Choose where to save: a path ending in .lacap.")),
    };
    if target.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).as_deref() != Some("lacap") {
        return Err(err(
            ErrorCode::InvalidInput,
            "Captures are saved as .lacap; exporting to .sr, VCD or CSV comes in milestone M5.",
        ));
    }
    let session = SavedSession {
        decoders: c.decoders.iter().map(|(id, d)| (id.clone(), d.config.clone())).collect(),
        analog: c.analog.iter().map(|(id, a)| (id.clone(), a.config.clone())).collect(),
        markers: c.markers.values().cloned().collect(),
        groups: c.groups.clone(),
        hidden: c.hidden.iter().copied().collect(),
    };
    let session = out(&session);
    let fail = |x: lacap::LacapError| {
        err(ErrorCode::Failed, format!("{} could not be saved: {x}", target.display()))
    };
    let same = c.backed_by_path && c.path.as_ref().is_some_and(|p| same_file(p, &target));
    if same {
        lacap::save(&c.store, &c.meta, &session).map_err(fail)?;
    } else {
        let f = StdFile::create(&target).map_err(|x| io_err(&target, &x))?;
        lacap::save_as(&c.store, &c.meta, &session, &f).map_err(fail)?;
    }
    let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    let title = c.meta.title.clone();
    let c = e.capture_mut(&i.capture)?;
    c.unsaved = false;
    if !same {
        c.path = Some(target.clone());
        c.backed_by_path = false;
    }
    let shown = target.display().to_string();
    Ok(Done::never(out(&FileSaveOut { path: shown.clone(), bytes }), format!("saved {title} to {shown}")))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

pub(crate) fn close(e: &mut Engine, ctx: &Ctx, i: CaptureRef) -> Result<Done, CommandError> {
    let title = e.capture(&i.capture)?.meta.title.clone();
    // closing a running capture stops it, which needs the device's lease
    if let Some(r) = e.running.iter().find(|r| r.capture == i.capture) {
        let held = r.devices.iter().find(|d| e.leases.get(*d).is_some_and(|h| *h != ctx.actor));
        let device = held.cloned().unwrap_or_default();
        match held.and(e.leases.get(&device)) {
            Some(h) if *h != ctx.actor && ctx.actor != crate::contract::Actor::User => {
                return Err(err(
                    ErrorCode::Busy,
                    format!(
                        "{title} is still being captured from {device}, which someone else holds; ask the person to stop it."
                    ),
                ));
            }
            _ => e.stop_capture_of(&i.capture),
        }
    }
    e.captures.remove(&i.capture);
    if e.active.as_deref() == Some(&i.capture) {
        e.active = e.captures.keys().next().cloned();
    }
    Ok(Done::never(out(&Nothing {}), format!("closed {title}")))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRenameIn {
    pub capture: String,
    pub title: String,
}

pub(crate) fn rename(e: &mut Engine, _: &Ctx, i: CaptureRenameIn) -> Result<Done, CommandError> {
    if i.title.trim().is_empty() {
        return Err(err(ErrorCode::InvalidInput, "A capture needs a title."));
    }
    let c = e.capture_mut(&i.capture)?;
    let old = std::mem::replace(&mut c.meta.title, i.title.clone());
    let back = CaptureRenameIn { capture: i.capture, title: old.clone() };
    Ok(Done::edit(out(&Nothing {}), inv("capture.rename", &back), format!("renamed {old} to {}", i.title)))
}
