//! The `logica` command (doc 03). The file tools run the engine in-process as the `cli` actor and
//! go through the command API like every other client (ADR 0001).
//!
//! Exit codes: 0 done, 1 the command failed (the message says why), 2 the command line is wrong.

use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use logica_core::lacap::{self, LacapError};
use logica_engine::state::CaptureSummary;
use logica_engine::{Actor, Engine, EngineConfig, Invocation};
use logica_fileio::StdFile;
use logica_protocols::format;

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

const HELP: &str = "\
logica: file tools for Logica captures.

Usage:
  logica inspect <file>              What a capture holds: source, length, sample rate, channels,
                                     saved decoders and markers, notes.
  logica inspect <file> --json       The same as JSON (the capture summary of the command API).
  logica inspect <file> --index      The index of a .lacap file as stored, without opening it.
  logica convert <in> <out>          Convert a capture (.lacap, FNIRSI .demo, sigrok .sr, VCD) to
                                     .lacap. Add --force to replace an existing <out>.
  logica mcp [--allow-outputs]       Speak MCP on stdin and stdout for an agent (for Claude Code:
                                     claude mcp add logica -- logica mcp). Relays to the running
                                     engine; without one (no app yet), becomes that engine without a
                                     window, which other agents then share. --allow-outputs permits
                                     commands that drive device outputs.
  logica help                        This text.
  logica version                     The version of logica and of its command API.

Exporting to .sr, VCD and CSV comes in milestone M5.
";

enum Fail {
    /// The command line is wrong.
    Usage(String),
    /// The command ran and failed.
    Failed(String),
}

type Result<T> = std::result::Result<T, Fail>;

/// Runs `logica` with `args` (without the program name) and returns the exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let result = match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => write_out(out, HELP),
        Some("version" | "--version" | "-V") => write_out(
            out,
            &format!(
                "logica {} (command API {})\n",
                env!("CARGO_PKG_VERSION"),
                logica_engine::contract::api_version()
            ),
        ),
        Some("inspect") => inspect(&args[1..], out, err),
        Some("convert") => convert(&args[1..], out),
        Some("mcp") => mcp(&args[1..]),
        Some(other) => Err(Fail::Usage(format!("There is no command \"{other}\"."))),
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(Fail::Usage(m)) => {
            let _ = writeln!(err, "logica: {m}\nRun `logica help` to see the commands.");
            EXIT_USAGE
        }
        Err(Fail::Failed(m)) => {
            let _ = writeln!(err, "logica: {m}");
            EXIT_FAILED
        }
    }
}

fn write_out(out: &mut dyn Write, text: &str) -> Result<()> {
    out.write_all(text.as_bytes()).map_err(|e| Fail::Failed(format!("The output could not be written: {e}.")))
}

/// Positional arguments and flags; `--` ends the flags.
struct Args {
    positional: Vec<String>,
    flags: Vec<String>,
}

fn parse(command: &str, args: &[String], known: &[&str]) -> Result<Args> {
    let mut a = Args { positional: Vec::new(), flags: Vec::new() };
    let mut rest = args.iter();
    while let Some(x) = rest.next() {
        if x == "--" {
            a.positional.extend(rest.by_ref().cloned());
        } else if x.starts_with('-') && x.len() > 1 {
            if !known.contains(&x.as_str()) {
                return Err(Fail::Usage(format!("{command} does not know the option {x}.")));
            }
            if !a.flags.contains(x) {
                a.flags.push(x.clone());
            }
        } else {
            a.positional.push(x.clone());
        }
    }
    Ok(a)
}

impl Args {
    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

/// A folder for the engine's working files, removed again when done.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!("logica-cli-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| {
            Fail::Failed(format!("The working folder {} could not be made: {e}.", dir.display()))
        })?;
        Ok(WorkDir(dir))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        // only the engine's working files are in here
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// The engine, in-process. Field order matters: the engine (and its open files) goes first.
struct Session {
    engine: Engine,
    _work: WorkDir,
}

impl Session {
    fn new() -> Result<Self> {
        let work = WorkDir::new()?;
        let engine = Engine::new(EngineConfig { data_dir: Some(work.0.clone()), ..EngineConfig::default() });
        Ok(Session { engine, _work: work })
    }

    fn call(&mut self, command: &str, input: Value) -> Result<Value> {
        self.engine
            .execute(&Actor::Cli, &Invocation { command: command.into(), input })
            .map_err(|e| Fail::Failed(e.message))
    }

    fn open(&mut self, path: &Path) -> Result<(String, CaptureSummary)> {
        let o = self.call("file.open", json!({ "path": path }))?;
        let c: CaptureSummary = serde_json::from_value(o["capture"].clone())
            .map_err(|e| Fail::Failed(format!("The engine's answer could not be read: {e}.")))?;
        Ok((c.id.clone(), c))
    }
}

fn one_file(command: &str, a: &Args) -> Result<PathBuf> {
    match a.positional.as_slice() {
        [f] => Ok(PathBuf::from(f)),
        [] => Err(Fail::Usage(format!("{command} needs a file: logica {command} <file>."))),
        _ => Err(Fail::Usage(format!("{command} takes one file at a time."))),
    }
}

fn is_lacap(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("lacap"))
}

fn inspect(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    let a = parse("inspect", args, &["--json", "--index"])?;
    let path = one_file("inspect", &a)?;
    if a.has("--json") && a.has("--index") {
        return Err(Fail::Usage("Choose --json or --index, not both.".into()));
    }
    if a.has("--index") {
        let f = StdFile::open(&path)
            .map_err(|e| Fail::Failed(format!("{} could not be opened: {e}.", path.display())))?;
        let i = lacap::read_index(&f).map_err(|e| match e {
            LacapError::NotLacap => Fail::Failed(format!(
                "{} is not a .lacap file, so it has no index. Leave out --index to see what it holds.",
                path.display()
            )),
            other => Fail::Failed(format!("{}: {other}", path.display())),
        })?;
        if i.recovered {
            let _ = writeln!(
                err,
                "logica: the last save of {} was interrupted ({} bytes at the end are unfinished); this is the index of the save before.",
                path.display(),
                i.unfinished_bytes
            );
        }
        return write_out(out, &format!("{:#}\n", i.index));
    }
    let mut s = Session::new()?;
    let (_, c) = s.open(&path)?;
    if a.has("--json") {
        let v = serde_json::to_value(&c).unwrap_or(Value::Null);
        return write_out(out, &format!("{v:#}\n"));
    }
    let container = if is_lacap(&path) {
        StdFile::open(&path).ok().and_then(|f| lacap::read_index(&f).ok()).map(|i| (i.version, i.size))
    } else {
        None
    };
    write_out(out, &summary(&path, &c, container))
}

/// "12,000,000 samples"
fn samples(n: u64) -> String {
    format!("{} sample{}", num(n), if n == 1 { "" } else { "s" })
}

/// "1,000"
fn num(n: u64) -> String {
    format::int(n as f64)
}

fn sample_at(n: u64) -> String {
    format!("sample {}", num(n))
}

/// "CH4 (SDA)": the channel number, and its name when it is not just the number.
fn channel(c: &CaptureSummary, index: u32) -> String {
    let plain = format!("CH{index}");
    match c.channels.iter().find(|ch| ch.index == index) {
        Some(ch) if ch.name != plain && !ch.name.is_empty() => format!("{plain} ({})", ch.name),
        _ => plain,
    }
}

fn bytes(n: u64) -> String {
    let x = n as f64;
    match n {
        0..1_000 => format!("{n} bytes"),
        1_000..1_000_000 => format!("{} kB", format::sig(x / 1e3, 3)),
        1_000_000..1_000_000_000 => format!("{} MB", format::sig(x / 1e6, 3)),
        _ => format!("{} GB", format::sig(x / 1e9, 3)),
    }
}

/// The readable form of `inspect`.
fn summary(path: &Path, c: &CaptureSummary, container: Option<((u16, u16), u64)>) -> String {
    let mut t = String::new();
    let name =
        path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let _ = writeln!(t, "{name}");
    let mut line = |label: &str, text: String| {
        let _ = writeln!(t, "  {label:<10} {text}");
    };
    line("Source", c.source.detail.clone());
    if let Some(((major, minor), size)) = container {
        line("File", format!(".lacap format {major}.{minor}, {}", bytes(size)));
    }
    line(
        "Length",
        format!(
            "{} at {}: {}",
            samples(c.sample_count),
            format::hz(c.sample_rate_hz),
            format::time(c.seconds)
        ),
    );
    if let Some(at) = c.trigger {
        line("Trigger", format!("at {}", sample_at(at)));
    }
    if let Some((from, to)) = c.artefact {
        line("Left out", format!("{} to {}: a start-up artefact, not analysed", sample_at(from), num(to)));
    }
    let names: Vec<String> = c.channels.iter().map(|ch| channel(c, ch.index)).collect();
    line("Channels", format!("{}: {}", names.len(), names.join(", ")));
    for d in &c.decoders {
        let roles: Vec<String> = d
            .config
            .roles
            .iter()
            .filter_map(|(role, ch)| ch.map(|ch| format!("{role} on {}", channel(c, ch))))
            .collect();
        let problems: u64 = d.problem_counts.values().sum();
        let problems = match problems {
            0 => String::new(),
            1 => ", 1 problem".into(),
            n => format!(", {n} problems"),
        };
        line("Decoder", format!("{} ({}): {} frames{problems}", d.config.name, roles.join(", "), d.frames));
    }
    for m in &c.markers {
        let at = match m.end {
            Some(end) => format!("{} to {}", sample_at(m.at), num(end)),
            None => sample_at(m.at),
        };
        let label = if m.label.is_empty() { String::new() } else { format!("\"{}\" ", m.label) };
        line("Marker", format!("{label}at {at}"));
    }
    for d in &c.saved_decoders {
        let roles: Vec<String> = d
            .channels
            .iter()
            .filter_map(|(role, ch)| ch.map(|ch| format!("{role} on {}", channel(c, ch))))
            .collect();
        line(
            "In file",
            format!("a {} decoder ({}), set up in the program that saved it", d.protocol, roles.join(", ")),
        );
    }
    for n in &c.notes {
        let level = match n.level {
            logica_core::capture::NoteLevel::Info => "Note",
            logica_core::capture::NoteLevel::Warning => "Warning",
        };
        line(level, n.message.clone());
    }
    t
}

fn convert(args: &[String], out: &mut dyn Write) -> Result<()> {
    let a = parse("convert", args, &["--force"])?;
    let (from, to) = match a.positional.as_slice() {
        [f, t] => (PathBuf::from(f), PathBuf::from(t)),
        _ => return Err(Fail::Usage("convert needs two files: logica convert <in> <out>.".into())),
    };
    if !is_lacap(&to) {
        return Err(Fail::Failed(format!(
            "{} does not end in .lacap. This version converts to .lacap only; exporting to .sr, VCD and CSV comes in milestone M5.",
            to.display()
        )));
    }
    let same = match (from.canonicalize(), to.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => from == to,
    };
    if same {
        return Err(Fail::Failed(
            "The input and the output are the same file; choose another output.".into(),
        ));
    }
    if to.exists() && !a.has("--force") {
        return Err(Fail::Failed(format!("{} already exists; add --force to replace it.", to.display())));
    }
    let mut s = Session::new()?;
    let (id, c) = s.open(&from)?;
    let saved = s.call("file.save", json!({ "capture": id, "path": to }))?;
    let size = saved["bytes"].as_u64().unwrap_or(0);
    write_out(
        out,
        &format!(
            "Converted {} to {}: {} channels, {} at {}, {}.\n",
            from.display(),
            to.display(),
            c.channels.len(),
            samples(c.sample_count),
            format::hz(c.sample_rate_hz),
            bytes(size)
        ),
    )
}

/// `logica mcp`: an engine without a window, serving one agent over stdin and stdout. Nothing
/// else may be written to stdout; messages go to stderr.
fn mcp(args: &[String]) -> Result<()> {
    let a = parse("mcp", args, &["--headless", "--allow-outputs"])?;
    if !a.positional.is_empty() {
        return Err(Fail::Usage("mcp takes no files; agents open them with the open_file tool.".into()));
    }
    let work = WorkDir::new()?;
    let options = logica_mcp::Options {
        data_dir: logica_mcp::lock::data_dir(),
        work_dir: work.0.clone(),
        allow_outputs: a.has("--allow-outputs"),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Fail::Failed(format!("The MCP server could not start: {e}.")))?;
    let result = runtime.block_on(logica_mcp::stdio_entry(options));
    // the engine and its working files are gone before the folder is removed
    drop(runtime);
    drop(work);
    result.map_err(|e| Fail::Failed(e.to_string()))
}
