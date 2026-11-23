//! MCP over HTTP (doc 07 testing): token, Origin and Host checks, loopback only; the relay with
//! real MCP clients under both protocol styles; two agents on one engine; the lock file.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
    ResourceUpdatedNotificationParam, SubscribeRequestParams,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient};
use serde_json::{Map, Value, json};

use logica_engine::{Engine, EngineConfig};
use logica_mcp::http::{self, HttpServer};
use logica_mcp::lock::{self, Lock};
use logica_mcp::{Shared, relay};

#[derive(Clone)]
struct Agent {
    name: &'static str,
    updated: Arc<Mutex<Vec<String>>>,
}

impl ClientHandler for Agent {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(ClientCapabilities::default(), Implementation::new(self.name, "1.0"))
    }

    async fn on_resource_updated(
        &self,
        p: ResourceUpdatedNotificationParam,
        _: NotificationContext<RoleClient>,
    ) {
        self.updated.lock().unwrap().push(p.uri);
    }
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/captures").join(name).display().to_string()
}

async fn engine() -> (Arc<Shared>, HttpServer) {
    let shared = Shared::new(Engine::new(EngineConfig::default()), false);
    let server = http::serve(shared.clone(), lock::new_token().unwrap()).await.unwrap();
    (shared, server)
}

/// An MCP client that reaches the server through the relay, as `logica mcp` does.
async fn through_relay(
    server: &HttpServer,
    name: &'static str,
    mode: ClientLifecycleMode,
) -> (RunningService<RoleClient, Agent>, Agent) {
    let (to_relay, relay_in) = tokio::io::duplex(1 << 20);
    let (relay_out, from_relay) = tokio::io::duplex(1 << 20);
    let (port, token) = (server.addr.port(), server.token.clone());
    tokio::spawn(async move { relay::relay(relay_in, relay_out, port, &token).await });
    let agent = Agent { name, updated: Arc::default() };
    let client = agent
        .clone()
        .serve_with_lifecycle((from_relay, to_relay), mode)
        .await
        .expect("connects through the relay");
    (client, agent)
}

async fn tool(c: &RunningService<RoleClient, Agent>, name: &str, args: Value) -> (String, Value) {
    let args: Map<String, Value> = args.as_object().cloned().unwrap();
    let r = c.call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(args)).await.unwrap();
    let text =
        r.content.iter().filter_map(|x| x.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n");
    assert_ne!(r.is_error, Some(true), "{name}: {text}");
    (text, r.structured_content.unwrap_or(Value::Null))
}

/// A raw POST of `initialize` with the given headers: (status, body).
async fn raw(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "raw", "version": "1" } } });
    let mut b = ::http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if !headers.iter().any(|(k, _)| *k == "host") {
        b = b.header("host", format!("127.0.0.1:{port}"));
    }
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let resp = sender.send_request(b.body(Full::new(Bytes::from(init.to_string()))).unwrap()).await.unwrap();
    let status = resp.status().as_u16();
    // an event stream stays open: read until the answer (after the priming event) or a pause
    let mut body = resp.into_body();
    let mut text = String::new();
    while let Ok(Some(Ok(f))) = tokio::time::timeout(Duration::from_millis(500), body.frame()).await {
        if let Ok(d) = f.into_data() {
            text.push_str(&String::from_utf8_lossy(&d));
        }
        if text.contains("\"jsonrpc\"") {
            break;
        }
    }
    (status, text)
}

#[tokio::test]
async fn http_needs_the_token_refuses_browsers_and_listens_on_loopback_only() {
    let (_, server) = engine().await;
    assert!(server.addr.ip().is_loopback());
    let port = server.addr.port();
    let bearer = format!("Bearer {}", server.token);

    let (s, body) = raw(port, "/mcp", &[]).await;
    assert_eq!(s, 401);
    assert!(body.contains("bearer token"), "{body}");
    assert_eq!(raw(port, "/mcp", &[("authorization", "Bearer not-the-token")]).await.0, 401);
    assert_eq!(raw(port, "/mcp", &[("authorization", &format!("Basic {}", server.token))]).await.0, 401);
    assert_eq!(raw(port, "/other", &[("authorization", &bearer)]).await.0, 404);
    // a web page cannot use it (DNS rebinding): any Origin, and foreign Host names, are refused
    assert_eq!(
        raw(port, "/mcp", &[("authorization", &bearer), ("origin", "http://evil.example")]).await.0,
        403
    );
    assert_eq!(
        raw(port, "/mcp", &[("authorization", &bearer), ("origin", &format!("http://127.0.0.1:{port}"))])
            .await
            .0,
        403
    );
    let (s, _) = raw(port, "/mcp", &[("authorization", &bearer), ("host", "evil.example")]).await;
    assert!((400..500).contains(&s), "foreign Host answered {s}");

    let (s, body) = raw(port, "/mcp", &[("authorization", &bearer)]).await;
    assert_eq!(s, 200);
    assert!(body.contains("\"serverInfo\"") && body.contains("logica"), "{body}");
    server.stop().await;
}

async fn wait_for(updated: &Arc<Mutex<Vec<String>>>, uri: &str) {
    for _ in 0..150 {
        if updated.lock().unwrap().iter().any(|u| u == uri) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no update for {uri}: {:?}", updated.lock().unwrap());
}

#[tokio::test]
#[allow(deprecated)]
async fn the_relay_carries_sessions_and_the_servers_own_messages() {
    let (_, server) = engine().await;
    let (a, a_agent) = through_relay(&server, "claude", ClientLifecycleMode::Initialize).await;
    let (t, out) = tool(&a, "open_file", json!({ "path": fixture("mystery_C.demo") })).await;
    assert!(t.starts_with("Opened c1"), "{t}");
    let c = out["capture"]["id"].as_str().unwrap().to_string();
    let (t, _) = tool(&a, "summarize", json!({ "capture": c })).await;
    assert!(t.starts_with("I2C: scl on CH5, sda on CH4;"), "{t}");

    // a notification the server sends on its own arrives through the relay's GET stream
    a.subscribe(SubscribeRequestParams::new("logica://session")).await.unwrap();
    tool(&a, "show_user", json!({ "capture": c, "at": 1000, "label": "here" })).await;
    wait_for(&a_agent.updated, "logica://session").await;

    // a stateless (2026-07-28) client through another relay shares the engine
    let mode = ClientLifecycleMode::Discover { preferred_versions: vec![ProtocolVersion::V_2026_07_28] };
    let (b, _) = through_relay(&server, "codex", mode).await;
    let (t, out) = tool(&b, "session", json!({})).await;
    assert!(t.contains("mystery_C.demo") && t.contains("\"here\""), "{t}");
    let actors: Vec<&str> =
        out["recent"].as_array().unwrap().iter().map(|h| h["actor"].as_str().unwrap()).collect();
    assert!(actors.contains(&"agent:claude"), "{actors:?}");
    tool(&b, "show_user", json!({ "capture": c, "at": 2000, "label": "mine" })).await;
    let (_, out) = tool(&a, "session", json!({})).await;
    let by: Vec<&str> =
        out["captures"][0]["markers"].as_array().unwrap().iter().map(|m| m["by"].as_str().unwrap()).collect();
    assert_eq!(by, ["agent:claude", "agent:codex"]);

    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
    server.stop().await;
}

#[tokio::test]
async fn the_relay_answers_for_an_engine_that_is_gone() {
    let (_, server) = engine().await;
    let (a, _) = through_relay(&server, "claude", ClientLifecycleMode::Initialize).await;
    server.stop().await;
    let r = a.call_tool(CallToolRequestParams::new("session")).await;
    let e = r.expect_err("the engine is gone");
    assert!(e.to_string().contains("cannot be reached"), "{e}");
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("logica-lock-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(lock::path(&d));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn the_lock_file_finds_running_engines_and_ignores_stale_ones() {
    let dir = scratch("rules");
    assert_eq!(lock::read(&dir), None);
    let (_, server) = engine().await;
    let me = Lock {
        pid: std::process::id(),
        port: server.addr.port(),
        token: server.token.clone(),
        api_version: "1.0".into(),
        headless: true,
    };
    lock::write(&dir, &me).unwrap();
    assert_eq!(lock::running(&dir), Some(me.clone()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(lock::path(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "only this user may read the token");
    }
    // a second engine finds it taken
    let other = Lock { port: 1, token: "x".into(), ..me.clone() };
    assert_eq!(lock::write(&dir, &other).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    // someone else's lock is not removed
    lock::remove_if_ours(&dir, &other);
    assert!(lock::path(&dir).exists());

    // once the engine stops, its lock is stale: ignored and replaced
    server.stop().await;
    assert_eq!(lock::running(&dir), None);
    lock::write(&dir, &other).unwrap();
    assert_eq!(lock::read(&dir), Some(other.clone()));
    lock::remove_if_ours(&dir, &other);
    assert!(!lock::path(&dir).exists());

    // a damaged lock is no lock
    std::fs::write(lock::path(&dir), "not json").unwrap();
    assert_eq!(lock::read(&dir), None);
    std::fs::remove_file(lock::path(&dir)).unwrap();
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(lock::new_token().unwrap().len(), 64);
    assert_ne!(lock::new_token().unwrap(), lock::new_token().unwrap());
}
