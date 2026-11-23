//! `logica mcp` as a relay (doc 07, ADR 0006): MCP messages from stdin go to the engine's HTTP
//! endpoint, and what comes back (JSON or a server-sent event stream) goes to stdout, one message
//! per line. The relay keeps the session id and protocol version the server hands out, and opens
//! the stream for messages the server sends on its own.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const SESSION: &str = "mcp-session-id";
const VERSION: &str = "mcp-protocol-version";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";

/// The standard headers (SEP-2243) that MCP 2026-07-28 requires on every request after
/// `initialize`, mirroring the body so intermediaries can route without parsing it.
pub fn standard_headers(msg: &Value) -> Vec<(&'static str, String)> {
    let Some(method) = msg["method"].as_str() else { return Vec::new() };
    let mut out = vec![("mcp-method", header_value(method))];
    let key = match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => Some("uri"),
        "tasks/get" | "tasks/update" | "tasks/cancel" => Some("taskId"),
        _ => None,
    };
    if let Some(name) = key.and_then(|k| msg["params"][k].as_str()) {
        out.push(("mcp-name", header_value(name)));
    }
    out
}

/// A header value as SEP-2243 writes it: as is, or base64 between `=?base64?` and `?=` when it
/// is not plain visible ASCII.
fn header_value(v: &str) -> String {
    let edge = |b: Option<&u8>| matches!(b, Some(b' ' | b'\t'));
    let plain = v.is_empty()
        || (!edge(v.as_bytes().first())
            && !edge(v.as_bytes().last())
            && v.chars().all(|c| (' '..='~').contains(&c))
            && !(v.starts_with("=?base64?") && v.ends_with("?=")));
    if plain { v.to_string() } else { format!("=?base64?{}?=", base64(v.as_bytes())) }
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[derive(Default)]
struct State {
    session: Option<String>,
    version: Option<String>,
    listening: bool,
}

#[derive(Clone)]
struct Endpoint {
    port: u16,
    token: String,
    state: Arc<Mutex<State>>,
    out: mpsc::UnboundedSender<String>,
}

fn other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

impl Endpoint {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn request(&self, method: Method, body: Option<&Value>) -> Request<Full<Bytes>> {
        let mut b = Request::builder()
            .method(method)
            .uri(crate::http::PATH)
            .header(http::header::HOST, format!("127.0.0.1:{}", self.port))
            .header(http::header::AUTHORIZATION, format!("Bearer {}", self.token))
            .header(http::header::ACCEPT, "application/json, text/event-stream");
        let st = self.state();
        if let Some(s) = &st.session {
            b = b.header(SESSION, s);
        }
        // the message's own protocol version (stateless MCP), else the negotiated one
        let version = body
            .and_then(|m| m["params"]["_meta"][META_VERSION].as_str().map(String::from))
            .or_else(|| st.version.clone());
        if let Some(v) = version {
            b = b.header(VERSION, v);
        }
        if let Some(m) = body {
            for (k, v) in standard_headers(m) {
                b = b.header(k, v);
            }
        }
        drop(st);
        let bytes = match body {
            Some(m) => {
                b = b.header(http::header::CONTENT_TYPE, "application/json");
                Bytes::from(m.to_string())
            }
            None => Bytes::new(),
        };
        b.body(Full::new(bytes)).unwrap_or_else(|_| Request::new(Full::new(Bytes::new())))
    }

    async fn send(&self, req: Request<Full<Bytes>>) -> std::io::Result<http::Response<Incoming>> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)).await?;
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.map_err(other)?;
        tokio::spawn(conn);
        sender.send_request(req).await.map_err(other)
    }

    /// Writes one message to stdout, compact on one line.
    fn emit(&self, text: &str) {
        let line = match serde_json::from_str::<Value>(text) {
            Ok(v) => v.to_string(),
            Err(_) => return,
        };
        let _ = self.out.send(line);
    }

    /// Answers a request the engine could not answer, so the client does not wait for ever.
    fn fail(&self, msg: &Value, why: &str) {
        if let Some(id) = msg.get("id").filter(|_| msg.get("method").is_some()) {
            let e = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": why } });
            let _ = self.out.send(e.to_string());
        }
    }

    /// Forwards a response body: one JSON message, or every event of an event stream.
    async fn forward(&self, resp: http::Response<Incoming>) -> std::io::Result<()> {
        let sse = resp
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let mut body = resp.into_body();
        if !sse {
            let bytes = body.collect().await.map_err(other)?.to_bytes();
            if !bytes.is_empty() {
                self.emit(&String::from_utf8_lossy(&bytes));
            }
            return Ok(());
        }
        let mut buf = String::new();
        while let Some(frame) = body.frame().await {
            let Ok(data) = frame.map_err(other)?.into_data() else { continue };
            buf.push_str(&String::from_utf8_lossy(&data).replace("\r\n", "\n"));
            while let Some(end) = buf.find("\n\n") {
                let event: String = buf.drain(..end + 2).collect();
                let data: Vec<&str> = event
                    .lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .map(|d| d.strip_prefix(' ').unwrap_or(d))
                    .collect();
                // priming and keep-alive events carry no data
                if !data.is_empty() && !data.iter().all(|d| d.is_empty()) {
                    self.emit(&data.join("\n"));
                }
            }
        }
        Ok(())
    }

    async fn post(&self, msg: Value) {
        let initialize = msg["method"] == "initialize";
        let resp = match self.send(self.request(Method::POST, Some(&msg))).await {
            Ok(r) => r,
            Err(e) => {
                return self.fail(
                    &msg,
                    &format!("Logica's engine cannot be reached ({e}); is the app still running?"),
                );
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let body = resp.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
            // the server's own JSON-RPC error, if it sent one
            if serde_json::from_slice::<Value>(&body).is_ok_and(|v| v.get("jsonrpc").is_some()) {
                return self.emit(&String::from_utf8_lossy(&body));
            }
            let text = String::from_utf8_lossy(&body);
            return self
                .fail(&msg, &format!("Logica's engine answered HTTP {}: {}", status.as_u16(), text.trim()));
        }
        if initialize && let Some(s) = resp.headers().get(SESSION).and_then(|v| v.to_str().ok()) {
            self.state().session = Some(s.to_string());
        }
        if status == StatusCode::ACCEPTED || status == StatusCode::NO_CONTENT {
            return;
        }
        if initialize {
            // read the negotiated version before passing the answer on
            let (parts, body) = resp.into_parts();
            let bytes = body.collect().await.map(|b| b.to_bytes()).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).to_string();
            let messages: Vec<String> = if parts
                .headers
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/event-stream"))
            {
                text.lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .map(|d| d.trim().to_string())
                    .filter(|d| !d.is_empty())
                    .collect()
            } else {
                vec![text]
            };
            for m in messages {
                if let Some(v) = serde_json::from_str::<Value>(&m)
                    .ok()
                    .and_then(|v| v["result"]["protocolVersion"].as_str().map(String::from))
                {
                    self.state().version = Some(v);
                }
                self.emit(&m);
            }
            return;
        }
        if let Err(e) = self.forward(resp).await {
            self.fail(&msg, &format!("The engine's answer was cut off: {e}"));
        }
    }

    /// The stream of messages the server sends on its own (session-based MCP).
    async fn listen(&self) {
        {
            let mut st = self.state();
            if st.listening || st.session.is_none() {
                return;
            }
            st.listening = true;
        }
        if let Ok(resp) = self.send(self.request(Method::GET, None)).await
            && resp.status().is_success()
        {
            let _ = self.forward(resp).await;
        }
    }
}

/// Relays between `input`/`output` (stdin/stdout) and the engine on `port` until `input` ends.
pub async fn relay<R, W>(input: R, mut output: W, port: u16, token: &str) -> std::io::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if output.write_all(line.as_bytes()).await.is_err()
                || output.write_all(b"\n").await.is_err()
                || output.flush().await.is_err()
            {
                return;
            }
        }
    });
    let ep = Endpoint { port, token: token.to_string(), state: Arc::default(), out: tx };
    let mut tasks = JoinSet::new();
    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("Not JSON: {e}") } });
                let _ = ep.out.send(err.to_string());
                continue;
            }
        };
        if msg["method"] == "initialize" {
            // later messages need the session id this answer brings
            ep.post(msg).await;
            continue;
        }
        if msg["method"] == "notifications/initialized" {
            // the session is open once the server has this; then its own messages can flow
            ep.post(msg).await;
            let e2 = ep.clone();
            tasks.spawn(async move { e2.listen().await });
            continue;
        }
        let e2 = ep.clone();
        tasks.spawn(async move { e2.post(msg).await });
    }
    // the client closed its side: end the session and stop
    if ep.state().session.is_some() {
        let _ = ep.send(ep.request(Method::DELETE, None)).await;
    }
    tasks.abort_all();
    drop(ep);
    let _ = writer.await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_headers_mirror_the_message() {
        let h = |m: Value| standard_headers(&m);
        assert_eq!(
            h(json!({ "method": "tools/call", "params": { "name": "session" } })),
            [("mcp-method", "tools/call".into()), ("mcp-name", "session".into())]
        );
        assert_eq!(
            h(json!({ "method": "resources/read", "params": { "uri": "logica://session" } }))[1].1,
            "logica://session"
        );
        assert_eq!(
            h(json!({ "method": "prompts/get", "params": { "name": "diagnose-bus" } }))[1].1,
            "diagnose-bus"
        );
        assert_eq!(h(json!({ "method": "tools/list" })), [("mcp-method", "tools/list".into())]);
        assert!(h(json!({ "id": 1, "result": {} })).is_empty());
        // values that are not plain visible ASCII travel as base64
        assert_eq!(header_value("Grüße"), "=?base64?R3LDvMOfZQ==?=");
        assert_eq!(header_value(" x"), "=?base64?IHg=?=");
        assert_eq!(header_value("=?base64?abc?="), "=?base64?PT9iYXNlNjQ/YWJjPz0=?=");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b""), "");
    }
}
