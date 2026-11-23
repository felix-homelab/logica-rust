//! Golden summaries (doc 07 testing): what `summarize` tells an agent about every committed
//! fixture, kept as reviewed snapshots so a change in wording or detection is a visible diff.
//! Regenerate with LOGICA_UPDATE_SNAPSHOTS=1 and review the difference.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use logica_engine::{Actor, Engine, EngineConfig, Invocation};
use logica_mcp::text::{self, Captures};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const FIXTURES: &[&str] = &[
    "fixtures/captures/1_UART_easy.demo",
    "fixtures/captures/2_UART_mystery_baud.demo",
    "fixtures/captures/3_I2C_bus.demo",
    "fixtures/captures/4_SPI_wrong_mode.demo",
    "fixtures/captures/mystery_A.demo",
    "fixtures/captures/mystery_B.demo",
    "fixtures/captures/mystery_C.demo",
    "fixtures/captures/mystery_D.demo",
    "fixtures/formats/sr/counter-multifile.sr",
    "fixtures/formats/sr/mixed-8ch-1mhz.sr",
    "fixtures/formats/sr/selected-3of8.sr",
    "fixtures/formats/sr/walk-16ch.sr",
    "fixtures/formats/sr/walk-24ch.sr",
    "fixtures/formats/vcd/features.vcd",
    "fixtures/formats/vcd/sigrok-mixed-8ch.vcd",
];

/// What an agent reads after open_file and summarize.
fn summary_of(rel: &str) -> String {
    let mut e = Engine::new(EngineConfig::default());
    let a = Actor::Agent("golden".into());
    let mut run = |c: &str, i: Value| {
        e.execute(&a, &Invocation { command: c.into(), input: i })
            .unwrap_or_else(|x| panic!("{rel}: {c}: {}", x.message))
    };
    let path = root().join(rel);
    let opened = run("file.open", json!({ "path": path }));
    let id = opened["capture"]["id"].as_str().unwrap().to_string();
    let caps = Captures::from_session(&run("session.get", json!({})));
    let analysis = run("analysis.detect", json!({ "capture": id }));
    // the path differs per machine; the rest of the line does not
    let line = text::capture_line(&opened["capture"]);
    format!("open_file: Opened {line}.\n\nsummarize:\n{}", text::summary(&caps, &id, &analysis))
}

#[test]
fn summaries_of_every_fixture_match_their_reviewed_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/summaries");
    let update = std::env::var_os("LOGICA_UPDATE_SNAPSHOTS").is_some();
    let mut changed = Vec::new();
    for rel in FIXTURES {
        let name = format!("{}.txt", Path::new(rel).file_name().unwrap().to_string_lossy());
        let now = summary_of(rel);
        assert!(now.len() < text::MAX_TEXT + 600, "{rel}: the summary is not bounded");
        let path = dir.join(&name);
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &now).unwrap();
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(now.as_str()) {
            changed.push(name);
        }
    }
    assert!(
        changed.is_empty(),
        "These summaries changed: {changed:?}. If that is deliberate, run LOGICA_UPDATE_SNAPSHOTS=1 cargo test -p logica-mcp --test golden and review tests/snapshots/summaries."
    );
}
