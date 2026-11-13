//! The `logica` command: inspect and convert on the fixtures, refusals with their reasons, exit
//! codes, and the working folder cleaned up.

use std::path::{Path, PathBuf};
use std::process::Command;

use logica_cli::{EXIT_FAILED, EXIT_OK, EXIT_USAGE, run};
use serde_json::Value;

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(rel).display().to_string()
}

/// (exit code, stdout, stderr)
fn logica(args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&args, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

/// A folder for this test's outputs, removed file by file at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("logica-cli-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).display().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for e in std::fs::read_dir(&self.0).unwrap().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
fn help_version_and_wrong_command_lines() {
    for args in [&[][..], &["help"], &["--help"], &["-h"]] {
        let (code, out, _) = logica(args);
        assert_eq!(code, EXIT_OK);
        assert!(out.contains("logica inspect <file>") && out.contains("logica convert <in> <out>"));
    }
    let (code, out, _) = logica(&["version"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.starts_with("logica ") && out.contains("command API 1.0"), "{out}");

    let demo = fixture("captures/mystery_C.demo");
    for (args, says) in [
        (vec!["frobnicate"], "There is no command \"frobnicate\"."),
        (vec!["inspect"], "inspect needs a file"),
        (vec!["inspect", &demo, &demo], "one file at a time"),
        (vec!["inspect", &demo, "--verbose"], "inspect does not know the option --verbose."),
        (vec!["inspect", &demo, "--json", "--index"], "Choose --json or --index, not both."),
        (vec!["convert", &demo], "convert needs two files"),
    ] {
        let (code, out, err) = logica(&args);
        assert_eq!(code, EXIT_USAGE, "{args:?}");
        assert!(out.is_empty());
        assert!(err.contains(says) && err.contains("logica help"), "{args:?}: {err}");
    }
    let (code, _, err) = logica(&["mcp"]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("not built yet"), "{err}");
}

#[test]
fn inspect_reads_every_format() {
    let (code, out, err) = logica(&["inspect", &fixture("captures/mystery_C.demo")]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.starts_with("mystery_C.demo\n"));
    assert!(out.contains("10,000,008 samples at 10 MHz"), "{out}");
    assert!(out.contains("Channels   2: CH4 (channel 4), CH5 (channel 5)"), "{out}");

    let (code, out, _) = logica(&["inspect", &fixture("formats/sr/mixed-8ch-1mhz.sr")]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("sigrok session file") && out.contains("CH0 (TX)"), "{out}");

    // notes from the importer are shown, warnings as warnings
    let (code, out, _) = logica(&["inspect", &fixture("formats/vcd/features.vcd")]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("Note       temp is a real-valued variable"), "{out}");
    assert!(out.contains("Warning    3 value(s) were x or z"), "{out}");

    let (code, out, _) = logica(&["inspect", "--json", &fixture("captures/mystery_C.demo")]);
    assert_eq!(code, EXIT_OK);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["sampleCount"], 10_000_008);
    assert_eq!(v["sampleRateHz"], 10e6);
    assert_eq!(v["channels"].as_array().unwrap().len(), 2);
}

#[test]
fn inspect_explains_files_it_cannot_read() {
    let s = Scratch::new("unreadable");
    let (code, _, err) = logica(&["inspect", &s.file("missing.demo")]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("could not be opened"), "{err}");

    let text = s.file("notes.txt");
    std::fs::write(&text, "just some notes\n").unwrap();
    let (code, _, err) = logica(&["inspect", &text]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("is not a capture format this version reads"), "{err}");

    let (code, _, err) = logica(&["inspect", "--index", &fixture("captures/mystery_C.demo")]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("is not a .lacap file, so it has no index"), "{err}");
}

#[test]
fn convert_writes_a_lacap_that_holds_the_same_capture() {
    let s = Scratch::new("convert");
    for (name, input) in [
        ("demo", fixture("captures/3_I2C_bus.demo")),
        ("sr", fixture("formats/sr/walk-24ch.sr")),
        ("vcd", fixture("formats/vcd/sigrok-mixed-8ch.vcd")),
    ] {
        let out_path = s.file(&format!("{name}.lacap"));
        let (code, out, err) = logica(&["convert", &input, &out_path]);
        assert_eq!(code, EXIT_OK, "{name}: {err}");
        assert!(out.starts_with("Converted ") && out.contains(".lacap"), "{out}");

        let (_, before, _) = logica(&["inspect", "--json", &input]);
        let (_, after, _) = logica(&["inspect", "--json", &out_path]);
        let (before, after): (Value, Value) =
            (serde_json::from_str(&before).unwrap(), serde_json::from_str(&after).unwrap());
        for key in ["sampleCount", "sampleRateHz", "channels", "notes", "source"] {
            assert_eq!(before[key], after[key], "{name}: {key}");
        }
        let (code, out, _) = logica(&["inspect", &out_path]);
        assert_eq!(code, EXIT_OK);
        assert!(out.contains("File       .lacap format 1.0"), "{out}");

        let (code, index, _) = logica(&["inspect", "--index", &out_path]);
        assert_eq!(code, EXIT_OK);
        let index: Value = serde_json::from_str(&index).unwrap();
        assert_eq!(index["format"], "lacap");
        assert_eq!(index["sampleCount"], before["sampleCount"]);
    }
}

#[test]
fn convert_refuses_to_overwrite_to_export_or_to_write_onto_its_input() {
    let s = Scratch::new("refusals");
    let input = fixture("captures/1_UART_easy.demo");
    let out_path = s.file("uart.lacap");
    assert_eq!(logica(&["convert", &input, &out_path]).0, EXIT_OK);

    let (code, _, err) = logica(&["convert", &input, &out_path]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("already exists; add --force to replace it"), "{err}");
    let (code, _, err) = logica(&["convert", "--force", &input, &out_path]);
    assert_eq!(code, EXIT_OK, "{err}");

    let (code, _, err) = logica(&["convert", &input, &s.file("uart.vcd")]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("converts to .lacap only") && err.contains("M5"), "{err}");
    assert!(!Path::new(&s.file("uart.vcd")).exists());

    let (code, _, err) = logica(&["convert", "--force", &out_path, &out_path]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("the same file"), "{err}");

    let (code, _, err) = logica(&["convert", &s.file("missing.sr"), &s.file("x.lacap")]);
    assert_eq!(code, EXIT_FAILED);
    assert!(err.contains("could not be opened"), "{err}");
    assert!(!Path::new(&s.file("x.lacap")).exists());
}

#[test]
fn the_index_of_an_interrupted_save_is_the_one_before_and_says_so() {
    let s = Scratch::new("interrupted");
    let lacap = s.file("c.lacap");
    assert_eq!(logica(&["convert", &fixture("captures/mystery_A.demo"), &lacap]).0, EXIT_OK);
    // a save that stopped before its footer leaves bytes after the last complete one
    let mut bytes = std::fs::read(&lacap).unwrap();
    bytes.extend_from_slice(&[0u8; 77]);
    std::fs::write(&lacap, bytes).unwrap();

    let (code, out, err) = logica(&["inspect", "--index", &lacap]);
    assert_eq!(code, EXIT_OK);
    assert!(err.contains("was interrupted (77 bytes at the end are unfinished)"), "{err}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["format"], "lacap");

    let (code, out, _) = logica(&["inspect", &lacap]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("Warning    The last save of this file was interrupted"), "{out}");
}

#[test]
fn the_binary_exits_with_the_codes_and_leaves_no_working_files() {
    let bin = env!("CARGO_BIN_EXE_logica");
    let run_bin = |args: &[&str]| {
        let child = Command::new(bin).args(args).spawn_and_wait();
        let leftovers = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("logica-cli-{}-", child.0)))
            .count();
        (child.1, leftovers)
    };
    assert_eq!(run_bin(&["inspect", &fixture("captures/mystery_B.demo")]), (EXIT_OK, 0));
    assert_eq!(run_bin(&["inspect", "nothing-here.demo"]), (EXIT_FAILED, 0));
    assert_eq!(run_bin(&["no-such-command"]), (EXIT_USAGE, 0));
}

trait SpawnAndWait {
    /// (process id, exit code), with the output discarded.
    fn spawn_and_wait(&mut self) -> (u32, i32);
}

impl SpawnAndWait for Command {
    fn spawn_and_wait(&mut self) -> (u32, i32) {
        let mut child = self
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the logica binary starts");
        let id = child.id();
        (id, child.wait().unwrap().code().unwrap())
    }
}
