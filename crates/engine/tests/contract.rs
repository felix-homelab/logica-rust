//! The contract is defined once in Rust and checked in as JSON Schema (doc 03).

use std::path::PathBuf;

use logica_engine::commands::COMMANDS;
use logica_engine::schema::{CONTRACT_PATH, contract, contract_text};

#[test]
fn every_command_has_a_handler_and_every_handler_a_spec() {
    let mut table = logica_engine::cmd::names();
    table.sort_unstable();
    let mut specs: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    specs.sort_unstable();
    assert_eq!(table, specs);
}

#[test]
fn the_checked_in_contract_is_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(CONTRACT_PATH);
    let now = contract_text();
    if std::env::var_os("LOGICA_UPDATE_CONTRACT").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &now).unwrap();
        return;
    }
    let saved = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        saved == now,
        "{CONTRACT_PATH} is out of date. Run `LOGICA_UPDATE_CONTRACT=1 cargo test -p logica-engine --test contract`, then `npm run generate -w packages/api`, and commit both."
    );
}

#[test]
fn the_contract_describes_inputs_outputs_and_events() {
    let c = contract();
    let open = &c["commands"]["file.open"];
    assert_eq!(open["safety"], "read");
    assert_eq!(open["undo"]["kind"], "inverse");
    assert!(open["input"]["$ref"].as_str().unwrap().ends_with("FileOpenIn"));
    assert!(c["$defs"]["FileOpenIn"]["properties"]["path"].is_object());
    assert_eq!(c["commands"]["capture.start"]["lease"], true);
    assert!(c["$defs"]["Event"].is_object());
}
