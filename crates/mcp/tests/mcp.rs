//! The MCP server through a real MCP client in-process (doc 07 testing): tools, resources,
//! prompts, subscriptions and errors, under the legacy `initialize` handshake and the stateless
//! 2026-07-28 one.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, GetPromptRequestParams,
    Implementation, ProgressNotificationParam, ProtocolVersion, ReadResourceRequestParams, ResourceContents,
    ResourceUpdatedNotificationParam, SubscribeRequestParams, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient, ServiceExt};
use serde_json::{Map, Value, json};

use logica_engine::{Engine, EngineConfig};
use logica_mcp::{Logica, Shared};

/// A client that records the notifications it gets.
#[derive(Clone, Default)]
struct TestClient {
    /// The client name; "test-agent" when empty.
    name: &'static str,
    progress: Arc<Mutex<Vec<f64>>>,
    updated: Arc<Mutex<Vec<String>>>,
}

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientConfig {
        let name = if self.name.is_empty() { "test-agent" } else { self.name };
        ClientConfig::new(ClientCapabilities::default(), Implementation::new(name, "1.0"))
    }

    async fn on_progress(&self, p: ProgressNotificationParam, _: NotificationContext<RoleClient>) {
        self.progress.lock().unwrap().push(p.progress);
    }

    async fn on_resource_updated(
        &self,
        p: ResourceUpdatedNotificationParam,
        _: NotificationContext<RoleClient>,
    ) {
        self.updated.lock().unwrap().push(p.uri);
    }
}

type Client = RunningService<RoleClient, TestClient>;

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/captures").join(name).display().to_string()
}

fn legacy() -> ClientLifecycleMode {
    ClientLifecycleMode::Initialize
}

fn stateless() -> ClientLifecycleMode {
    ClientLifecycleMode::Discover { preferred_versions: vec![ProtocolVersion::V_2026_07_28] }
}

/// A server over its own engine and a client connected to it through an in-memory pipe.
async fn connect_to(shared: Arc<Shared>, mode: ClientLifecycleMode) -> (Client, TestClient) {
    connect_as("", shared, mode).await
}

async fn connect_as(
    name: &'static str,
    shared: Arc<Shared>,
    mode: ClientLifecycleMode,
) -> (Client, TestClient) {
    let (server_side, client_side) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(s) = Logica::new(shared).serve(server_side).await {
            let _ = s.waiting().await;
        }
    });
    let handler = TestClient { name, ..TestClient::default() };
    let client = handler.clone().serve_with_lifecycle(client_side, mode).await.expect("the client connects");
    (client, handler)
}

async fn connect(mode: ClientLifecycleMode) -> (Client, TestClient, Arc<Shared>) {
    // the synthetic device gives the session devices to summarise
    let shared = Shared::new(Engine::new(EngineConfig { synthetic: true, ..EngineConfig::default() }), false);
    let (c, h) = connect_to(shared.clone(), mode).await;
    (c, h, shared)
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

async fn call(client: &Client, tool: &str, args: Value) -> CallToolResult {
    client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(obj(args)))
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"))
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n")
}

/// A successful call: its text and structured content.
async fn ok(client: &Client, tool: &str, args: Value) -> (String, Value) {
    let r = call(client, tool, args).await;
    assert_ne!(r.is_error, Some(true), "{tool} failed: {}", text(&r));
    (text(&r), r.structured_content.clone().unwrap_or(Value::Null))
}

/// A failed call: its message.
async fn err(client: &Client, tool: &str, args: Value) -> String {
    let r = call(client, tool, args).await;
    assert_eq!(r.is_error, Some(true), "{tool} should fail: {}", text(&r));
    text(&r)
}

async fn read(client: &Client, uri: &str) -> Value {
    let r = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .unwrap_or_else(|e| panic!("{uri}: {e}"));
    match &r.contents[0] {
        ResourceContents::TextResourceContents { text, .. } => serde_json::from_str(text).unwrap(),
        other => panic!("{uri}: not text: {other:?}"),
    }
}

async fn the_agent_scenario(mode: ClientLifecycleMode) {
    let (client, _, _) = connect(mode).await;
    let (t, _) = ok(&client, "session", json!({})).await;
    assert!(t.starts_with("No captures are open."), "{t}");

    let (t, out) = ok(&client, "open_file", json!({ "path": fixture("mystery_C.demo") })).await;
    let c = out["capture"]["id"].as_str().unwrap().to_string();
    assert!(t.starts_with(&format!("Opened {c} \"mystery_C.demo\": 10,000,008 samples at 10 MS/s")), "{t}");

    // the summary leads with the answer
    let (t, out) = ok(&client, "summarize", json!({ "capture": c })).await;
    assert!(t.starts_with("I2C: scl on CH5, sda on CH4;"), "{t}");
    assert!(t.contains("400 kHz"), "{t}");
    assert!(out["buses"].is_array());

    let (t, out) = ok(
        &client,
        "decoder",
        json!({ "action": "add", "capture": c, "protocol": "i2c", "roles": { "scl": 5, "sda": 4 } }),
    )
    .await;
    let d = out["decoder"]["id"].as_str().unwrap().to_string();
    assert!(t.starts_with(&format!("Added decoder {d} I2C")), "{t}");

    let (t, out) = ok(
        &client,
        "get_decoded",
        json!({ "capture": c, "decoder": d, "packets": true, "text": "CALIBRATED" }),
    )
    .await;
    assert!(t.contains("CALIBRATED 2026"), "{t}");
    let at = out["packets"][0]["packet"]["start"].as_u64().unwrap();
    let (t, _) =
        ok(&client, "get_decoded", json!({ "capture": c, "decoder": d, "packets": true, "explain": true }))
            .await;
    assert!(t.starts_with("No problems in this decode; nothing to explain."), "{t}");
    assert!(!t.contains("Symptoms to compare with"), "{t}");

    let (t, _) = ok(
        &client,
        "show_user",
        json!({ "capture": c, "at": at, "label": "calibration date", "moveView": true }),
    )
    .await;
    assert!(t.starts_with("The person now sees marker m"), "{t}");

    assert!(t.contains(&format!("(sample {}", logica_protocols::format::int(at as f64))), "{t}");

    // a picture around the marker, with the rows named
    let rendered = call(&client, "render", json!({ "capture": c, "from": at, "to": at + 400 })).await;
    let rt = text(&rendered);
    assert!(
        rt.contains("Rows from the top: time axis")
            && rt.contains("channel CH4")
            && rt.contains("calibration date"),
        "{rt}"
    );
    let image = rendered.content.iter().find_map(|c| c.as_image()).expect("an image");
    assert_eq!(image.mime_type, "image/png");
    assert!(image.data.starts_with("iVBORw0KGgo"), "a base64 PNG");
    assert!(
        rendered.structured_content.as_ref().unwrap().get("pngBase64").is_none(),
        "the PNG is not sent twice"
    );

    // a decoder change, undone through run_command
    let (t, _) =
        ok(&client, "decoder", json!({ "action": "update", "capture": c, "decoder": d, "name": "EEPROM" }))
            .await;
    assert!(t.contains("EEPROM"), "{t}");
    let (t, _) = ok(&client, "run_command", json!({ "command": "history.undo", "input": {} })).await;
    assert!(t.starts_with("history.undo done."), "{t}");
    let s = read(&client, &format!("logica://captures/{c}/summary")).await;
    assert_ne!(s["decoders"][0]["config"]["name"], "EEPROM");

    // everything above is recorded as this agent's
    let (t, out) = ok(&client, "session", json!({})).await;
    let actors: Vec<&str> =
        out["recent"].as_array().unwrap().iter().map(|h| h["actor"].as_str().unwrap()).collect();
    assert!(!actors.is_empty() && actors.iter().all(|a| *a == "agent:test-agent"), "{actors:?}");
    assert!(t.contains("marker m") && t.contains("\"calibration date\""), "{t}");
    assert!(t.contains("(undone)"), "{t}");
    assert!(
        t.contains("other devices are available; see list_devices.") && !t.contains("synthetic/nec"),
        "{t}"
    );
    let recent: Vec<&str> =
        out["recent"].as_array().unwrap().iter().map(|h| h["summary"].as_str().unwrap()).collect();
    assert!(recent.iter().any(|r| r.starts_with("added an I2C decoder")), "{recent:?}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_agent_finds_reads_marks_and_undoes_with_the_legacy_handshake() {
    the_agent_scenario(legacy()).await;
}

#[tokio::test]
async fn an_agent_finds_reads_marks_and_undoes_statelessly() {
    the_agent_scenario(stateless()).await;
}

#[tokio::test]
async fn tools_are_few_with_plain_object_schemas_and_change_only_deliberately() {
    let (client, _, _) = connect(legacy()).await;
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let expected: Vec<&str> = logica_mcp::tools::TOOLS.iter().map(|t| t.name).collect();
    assert_eq!(names, expected);
    assert!(tools.len() <= 16, "every tool costs context in every session (ADR 0006)");
    for t in &tools {
        let s = &t.input_schema;
        assert_eq!(s.get("type"), Some(&json!("object")), "{}", t.name);
        for k in ["oneOf", "anyOf", "allOf"] {
            assert!(!s.contains_key(k), "{}: agent APIs refuse a top-level {k}", t.name);
        }
        // every reference resolves inside the tool's own schema
        let text = serde_json::to_string(s).unwrap();
        for r in text.split("\"$ref\":\"#/$defs/").skip(1) {
            let name = &r[..r.find('"').unwrap()];
            assert!(s["$defs"].get(name).is_some(), "{}: $defs/{name} is missing", t.name);
        }
    }
    // the snapshot: regenerate with LOGICA_UPDATE_SNAPSHOTS=1 and review the difference
    let snapshot: Value = tools
        .iter()
        .map(|t| (t.name.to_string(), json!({ "description": t.description, "inputSchema": *t.input_schema, "annotations": t.annotations })))
        .collect::<Map<_, _>>()
        .into();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/tools.json");
    let now = serde_json::to_string_pretty(&snapshot).unwrap() + "\n";
    if std::env::var_os("LOGICA_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &now).unwrap();
    }
    let stored = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        stored == now,
        "The MCP tools changed. If that is deliberate, run LOGICA_UPDATE_SNAPSHOTS=1 cargo test -p logica-mcp and review tests/snapshots/tools.json."
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_agent_adds_a_pwm_level_and_measures_it() {
    let (client, _, _) = connect(legacy()).await;
    let (_, out) = ok(&client, "open_file", json!({ "path": fixture("mystery_D.demo") })).await;
    let c = out["capture"]["id"].as_str().unwrap().to_string();
    let (_, out) = ok(
        &client,
        "run_command",
        json!({ "command": "analog.add", "input": { "capture": c, "source": "pwm", "roles": { "signal": 0 },
                "scaling": { "kind": "range", "low": 0, "high": 100 }, "name": "Fan" } }),
    )
    .await;
    let id = out["analog"]["id"].as_str().unwrap().to_string();
    let (t, _) = ok(&client, "measure", json!({ "action": "analog", "capture": c, "analog": id })).await;
    assert!(t.starts_with("Fan from "), "{t}");
    assert!(t.contains("mean 30") && t.contains("RMS 30"), "{t}");
    let (t, _) =
        ok(&client, "measure", json!({ "action": "analog", "capture": c, "analog": id, "from": 0, "to": 1 }))
            .await;
    assert!(t.ends_with("no value in this range."), "{t}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn errors_are_instructions_the_agent_can_read() {
    let (client, _, _) = connect(legacy()).await;
    let m = err(&client, "summarize", json!({ "capture": "c9" })).await;
    assert!(m.contains("c9"), "{m}");
    let m = err(&client, "decoder", json!({ "capture": "c1" })).await;
    assert_eq!(m, "decoder needs an action: add, update, remove.");
    let m = err(&client, "measure", json!({ "action": "median", "capture": "c1" })).await;
    assert!(m.starts_with("measure needs an action"), "{m}");
    let m = err(&client, "run_command", json!({})).await;
    assert!(m.contains("logica://commands"), "{m}");
    let m = err(&client, "run_command", json!({ "command": "decoder.add", "input": { "capture": 1 } })).await;
    assert!(m.contains("The input of decoder.add is not valid"), "{m}");
    let m = err(&client, "open_file", json!({ "path": "/nowhere/x.demo" })).await;
    assert!(m.contains("could not be opened"), "{m}");
    let m = err(&client, "capture", json!({ "device": "synthetic/mix" })).await;
    assert_eq!(m, "capture needs an action: start or stop.");
    // an unknown tool is a protocol error, not a result
    let e = client.call_tool(CallToolRequestParams::new("teleport")).await.unwrap_err();
    assert!(e.to_string().contains("There is no tool \"teleport\"."), "{e}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn resources_read_through_commands() {
    let (client, _, _) = connect(stateless()).await;
    let fixed: Vec<String> =
        client.list_all_resources().await.unwrap().iter().map(|r| r.uri.clone()).collect();
    assert_eq!(fixed, ["logica://session", "logica://commands", "logica://protocols"]);
    assert_eq!(client.list_all_resource_templates().await.unwrap().len(), 3);

    let commands = read(&client, "logica://commands").await;
    assert_eq!(commands["commands"]["history.undo"]["undo"]["kind"], "never");
    assert!(read(&client, "logica://protocols").await["protocols"].as_array().unwrap().len() > 10);
    assert_eq!(read(&client, "logica://protocols/i2c").await["protocol"]["id"], "i2c");

    let (_, out) = ok(&client, "open_file", json!({ "path": fixture("1_UART_easy.demo") })).await;
    let c = out["capture"]["id"].as_str().unwrap().to_string();
    let (_, out) = ok(&client, "summarize", json!({ "capture": c })).await;
    let det = out["buses"][0]["chosen"][0].clone();
    let (_, out) = ok(&client, "decoder", json!({ "action": "add", "capture": c, "protocol": det["protocolId"], "roles": det["roles"], "settings": det["settings"] })).await;
    let d = out["decoder"]["id"].as_str().unwrap().to_string();

    assert_eq!(read(&client, "logica://session").await["captures"][0]["id"], c.as_str());
    assert_eq!(
        read(&client, &format!("logica://captures/{c}/summary")).await["decoders"][0]["id"],
        d.as_str()
    );
    let page = read(&client, &format!("logica://captures/{c}/decoders/{d}/packets?from=0&cursor=0")).await;
    assert!(!page["packets"].as_array().unwrap().is_empty());

    for (uri, says) in [
        ("logica://nothing", "There is no resource logica://nothing."),
        ("logica://captures/c42/summary", "No capture with the id \"c42\" is open."),
        ("https://example.com/", "There is no resource"),
    ] {
        let e = client.read_resource(ReadResourceRequestParams::new(uri)).await.unwrap_err();
        assert!(e.to_string().contains(says), "{uri}: {e}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn prompts_describe_the_recipes() {
    let (client, _, _) = connect(legacy()).await;
    let prompts = client.list_all_prompts().await.unwrap();
    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["diagnose-bus", "verify-expected"]);
    let mut args = Map::new();
    args.insert("channels".into(), "CH4, CH5".into());
    let p =
        client.get_prompt(GetPromptRequestParams::new("diagnose-bus").with_arguments(args)).await.unwrap();
    let body = serde_json::to_string(&p.messages).unwrap();
    assert!(body.contains("Diagnose the bus on CH4, CH5") && body.contains("show_user"), "{body}");
    let e = client.get_prompt(GetPromptRequestParams::new("guess")).await.unwrap_err();
    assert!(e.to_string().contains("There is no prompt \"guess\"."), "{e}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_device_capture_waits_and_two_agents_meet_the_lease() {
    let shared = Shared::new(Engine::new(EngineConfig { synthetic: true, ..EngineConfig::default() }), false);
    let (a, _) = connect_to(shared.clone(), legacy()).await;
    let (b, _) = connect_as("other-agent", shared.clone(), stateless()).await;
    let (t, _) = ok(&a, "list_devices", json!({})).await;
    assert!(t.contains("synthetic/mix") && t.contains("not connected"), "{t}");
    ok(&a, "device", json!({ "action": "connect", "device": "synthetic/mix" })).await;
    let config =
        json!({ "channels": [0, 1, 2, 6], "sampleRateHz": 1e8, "samples": 3_000_000, "mode": "stream" });
    let (t, out) = ok(
        &a,
        "capture",
        json!({ "action": "start", "device": "synthetic/mix", "config": config, "title": "mix" }),
    )
    .await;
    assert!(t.starts_with("Captured c") && t.contains("3,000,000 samples at 100 MS/s"), "{t}");
    assert_eq!(out["capture"]["sampleCount"], 3_000_000);
    assert_eq!(out["cancelled"], false);

    // the device is the first agent's; the second gets told who holds it
    let m = err(&b, "capture", json!({ "action": "start", "device": "synthetic/mix" })).await;
    assert!(m.contains("in use by agent test-agent"), "{m}");
    // and the first one's actions are recorded under its own name
    let (_, s) = ok(&b, "session", json!({})).await;
    assert!(s["recent"].as_array().unwrap().iter().all(|h| h["actor"] == "agent:test-agent"));
    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
}

async fn wait_for(updated: &Arc<Mutex<Vec<String>>>, uri: &str) {
    for _ in 0..100 {
        if updated.lock().unwrap().iter().any(|u| u == uri) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no update for {uri}: {:?}", updated.lock().unwrap());
}

#[tokio::test]
#[allow(deprecated)]
async fn subscribers_hear_about_changes_in_both_protocol_styles() {
    let shared = Shared::new(Engine::new(EngineConfig::default()), false);
    // legacy: resources/subscribe on a session-style connection
    let (old, old_h) = connect_to(shared.clone(), legacy()).await;
    old.subscribe(SubscribeRequestParams::new("logica://session")).await.unwrap();
    let e = old.subscribe(SubscribeRequestParams::new("logica://elsewhere")).await.unwrap_err();
    assert!(e.to_string().contains("There is no resource"), "{e}");
    // 2026-07-28: subscriptions/listen
    let (new, _) = connect_to(shared.clone(), stateless()).await;
    let mut filter = SubscriptionFilter::default();
    filter.resource_subscriptions = Some(vec!["logica://session".into()]);
    let mut sub = new.listen(filter).await.unwrap();

    ok(&new, "open_file", json!({ "path": fixture("1_UART_easy.demo") })).await;
    wait_for(&old_h.updated, "logica://session").await;
    let n = tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("a notification arrives");
    assert!(format!("{n:?}").contains("logica://session"), "{n:?}");
    old.cancel().await.unwrap();
    new.cancel().await.unwrap();
}
