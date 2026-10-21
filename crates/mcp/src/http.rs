//! Streamable HTTP transport: `POST /mcp` with one JSON-RPC message,
//! answered with JSON (no server-initiated streams, so `GET` is 405).
//!
//! Bound to `127.0.0.1` unless LAN access is on, which requires a bearer
//! token. Requests from web pages (an `Origin` that isn't this PC) and, on
//! loopback, a `Host` that isn't loopback (DNS rebinding) are refused.

use std::net::{Ipv4Addr, SocketAddr};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use rc_common::Secret;
use rc_store::Db;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::protocol;

/// Default port (unassigned by IANA).
pub const DEFAULT_PORT: u16 = 7337;
const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub port: u16,
    /// Listen on all interfaces instead of loopback only.
    pub lan: bool,
    /// Required bearer token (always required with `lan`).
    pub token: Option<Secret>,
}

#[derive(Clone)]
struct AppCtx {
    db: Db,
    lan: bool,
    token: Option<Secret>,
}

/// A running server; dropping it (or [`McpServer::stop`]) shuts it down.
pub struct McpServer {
    pub addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl McpServer {
    pub fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("LAN access needs an access token")]
    TokenRequired,
    #[error("couldn't listen on port {0}: {1}")]
    Bind(u16, std::io::Error),
}

/// Starts the server on the current Tokio runtime.
pub async fn serve(db: Db, cfg: HttpConfig) -> Result<McpServer, ServeError> {
    if cfg.lan && cfg.token.as_ref().is_none_or(|t| t.expose().is_empty()) {
        return Err(ServeError::TokenRequired);
    }
    let ip = if cfg.lan { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST };
    let listener = tokio::net::TcpListener::bind((ip, cfg.port)).await.map_err(|e| ServeError::Bind(cfg.port, e))?;
    let addr = listener.local_addr().map_err(|e| ServeError::Bind(cfg.port, e))?;
    let ctx = AppCtx { db, lan: cfg.lan, token: cfg.token };
    let app = Router::new().route("/mcp", post(handle).get(not_allowed).delete(not_allowed)).with_state(ctx);
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = rx.await;
        });
        if let Err(e) = server.await {
            tracing::warn!(error = %e, "MCP server stopped with an error");
        }
    });
    tracing::info!(%addr, lan = cfg.lan, "MCP server listening");
    Ok(McpServer { addr, shutdown: Some(tx) })
}

async fn not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

/// Constant-time byte comparison (length leaks, which is fine for tokens).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn is_loopback_host(host: &str) -> bool {
    // Strip a port (and the brackets of an IPv6 literal).
    let h = match host.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    matches!(h.trim_start_matches('[').trim_end_matches(']'), "localhost" | "127.0.0.1" | "::1")
}

fn header_str(h: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

/// Refusal reason, if any.
fn check(ctx: &AppCtx, headers: &HeaderMap) -> Option<(StatusCode, &'static str)> {
    if let Some(origin) = header_str(headers, header::ORIGIN) {
        let host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
        if !is_loopback_host(host) {
            return Some((StatusCode::FORBIDDEN, "requests from web pages are not allowed"));
        }
    }
    if !ctx.lan && !header_str(headers, header::HOST).is_some_and(is_loopback_host) {
        return Some((StatusCode::FORBIDDEN, "unexpected Host header"));
    }
    if let Some(token) = &ctx.token {
        let given = header_str(headers, header::AUTHORIZATION).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
        if !same(given.trim().as_bytes(), token.expose().as_bytes()) {
            return Some((StatusCode::UNAUTHORIZED, "missing or wrong access token"));
        }
    }
    None
}

async fn handle(State(ctx): State<AppCtx>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some((status, why)) = check(&ctx, &headers) {
        return (status, why).into_response();
    }
    if body.len() > MAX_BODY {
        return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let e = protocol::error(Value::Null, protocol::PARSE_ERROR, "invalid JSON");
            return (StatusCode::BAD_REQUEST, json_body(&e)).into_response();
        }
    };
    match protocol::handle(&ctx.db, msg).await {
        Some(reply) => json_body(&reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn json_body(v: &Value) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], serde_json::to_vec(v).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert!(is_loopback_host("127.0.0.1:7337") && is_loopback_host("localhost") && is_loopback_host("[::1]:80"));
        assert!(!is_loopback_host("evil.example:7337") && !is_loopback_host("192.168.1.5:7337"));
        assert!(same(b"abc", b"abc") && !same(b"abc", b"abd") && !same(b"abc", b"ab"));
    }
}
