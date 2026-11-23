//! MCP over Streamable HTTP on 127.0.0.1 (doc 07): the bearer token is checked here, before the
//! SDK sees a request; the SDK checks `Host` (loopback only) and `Origin` (any browser origin is
//! refused, against DNS rebinding).

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{Logica, Shared};

pub const PATH: &str = "/mcp";

/// A running HTTP endpoint.
pub struct HttpServer {
    pub addr: SocketAddr,
    pub token: String,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl HttpServer {
    pub async fn stop(self) {
        self.stop.cancel();
        let _ = self.task.await;
    }
}

fn plain(status: StatusCode, text: &str) -> Response<BoxBody<Bytes, Infallible>> {
    let mut r = Response::new(Full::new(Bytes::from(text.to_string())).boxed());
    *r.status_mut() = status;
    r.headers_mut()
        .insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("text/plain; charset=utf-8"));
    if status == StatusCode::UNAUTHORIZED {
        r.headers_mut().insert(http::header::WWW_AUTHENTICATE, http::HeaderValue::from_static("Bearer"));
    }
    r
}

/// Compares in time independent of where the first difference is.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorized(req: &Request<Incoming>, token: &str) -> bool {
    req.headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| same(t.trim().as_bytes(), token.as_bytes()))
}

/// Serves MCP on 127.0.0.1 on a free port.
pub async fn serve(shared: Arc<Shared>, token: String) -> std::io::Result<HttpServer> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let stop = CancellationToken::new();
    let config = StreamableHttpServerConfig::default()
        .enforce_origin_validation()
        .with_sse_keep_alive(Some(Duration::from_secs(15)))
        .with_cancellation_token(stop.child_token());
    let mcp = StreamableHttpService::new(
        move || Ok(Logica::new(shared.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let (stop2, token2) = (stop.clone(), token.clone());
    let task = tokio::spawn(async move {
        loop {
            let stream = tokio::select! {
                () = stop2.cancelled() => return,
                r = listener.accept() => match r {
                    Ok((s, _)) => s,
                    Err(_) => continue,
                },
            };
            let (mcp, token, stop) = (mcp.clone(), token2.clone(), stop2.clone());
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let (mcp, token) = (mcp.clone(), token.clone());
                    async move {
                        Ok::<_, Infallible>(if req.uri().path() != PATH {
                            plain(StatusCode::NOT_FOUND, "Logica serves MCP at /mcp only.")
                        } else if !authorized(&req, &token) {
                            plain(
                                StatusCode::UNAUTHORIZED,
                                "A bearer token is needed; logica mcp reads it from the lock file.",
                            )
                        } else {
                            mcp.handle(req).await
                        })
                    }
                });
                let conn = http1::Builder::new().serve_connection(TokioIo::new(stream), svc);
                tokio::pin!(conn);
                tokio::select! {
                    _ = conn.as_mut() => {}
                    () = stop.cancelled() => conn.as_mut().graceful_shutdown(),
                }
            });
        }
    });
    Ok(HttpServer { addr, token, stop, task })
}
