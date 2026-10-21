//! Runs the MCP server inside the app per the settings, and the token and
//! status commands. The token lives in Credential Manager; the UI can copy
//! it to the clipboard but never shows it.

use rc_common::{Secret, credentials};
use rc_mcp::http::{HttpConfig, McpServer, serve};
use serde::Serialize;
use tauri::State;

use crate::AppState;
use crate::commands::{CmdResult, err};

#[derive(Default)]
pub struct McpRuntime {
    server: Option<McpServer>,
    error: Option<String>,
}

fn new_token() -> CmdResult<Secret> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(err)?;
    Ok(Secret::new(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

/// The saved token, creating one the first time it's needed.
fn token() -> CmdResult<Secret> {
    if let Some(t) = credentials::load_secret(credentials::MCP_TOKEN).map_err(err)? {
        return Ok(t);
    }
    let t = new_token()?;
    credentials::store_secret(credentials::MCP_TOKEN, &t).map_err(err)?;
    Ok(t)
}

/// Starts, restarts or stops the server to match the settings.
pub async fn apply(state: &AppState) {
    let s = match state.engine.db().call(|c| rc_store::settings::app(c)).await {
        Ok(s) => s.mcp,
        Err(e) => return tracing::warn!(error = %e, "MCP: couldn't read settings"),
    };
    let mut rt = state.mcp.lock().await;
    if let Some(old) = rt.server.take() {
        old.stop();
        // Let the old listener release the port.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    rt.error = None;
    if !s.enabled {
        return;
    }
    let token = if s.lan || s.require_token_local {
        match token() {
            Ok(t) => Some(t),
            Err(e) => {
                rt.error = Some(format!("Couldn't read the access token: {e}"));
                return;
            }
        }
    } else {
        None
    };
    match serve(state.engine.db().clone(), HttpConfig { port: s.port, lan: s.lan, token }).await {
        Ok(server) => rt.server = Some(server),
        Err(e) => {
            tracing::warn!(error = %e, "MCP server didn't start");
            rt.error = Some(e.to_string());
        }
    }
}

#[derive(Serialize)]
pub struct McpStatus {
    pub running: bool,
    pub error: Option<String>,
    /// For agents on this PC.
    pub local_url: Option<String>,
    /// For agents on your network (LAN mode).
    pub lan_url: Option<String>,
    pub token_required: bool,
    /// For Claude Desktop and other clients that launch a command.
    pub exe_path: String,
}

/// This PC's address on the local network (no packets are sent).
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.168.0.1:9").ok()?;
    s.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

#[tauri::command]
pub async fn mcp_status(state: State<'_, AppState>) -> CmdResult<McpStatus> {
    let s = state.engine.db().call(|c| rc_store::settings::app(c)).await.map_err(err)?.mcp;
    let rt = state.mcp.lock().await;
    let port = rt.server.as_ref().map(|x| x.addr.port());
    Ok(McpStatus {
        running: rt.server.is_some(),
        error: rt.error.clone(),
        local_url: port.map(|p| format!("http://127.0.0.1:{p}/mcp")),
        lan_url: port.filter(|_| s.lan).and_then(|p| lan_ip().map(|ip| format!("http://{ip}:{p}/mcp"))),
        token_required: s.lan || s.require_token_local,
        exe_path: std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
    })
}

/// The access token, for the UI to put on the clipboard.
#[tauri::command]
pub fn mcp_token() -> CmdResult<String> {
    Ok(token()?.expose().to_owned())
}

/// Replaces the token (agents using the old one stop working).
#[tauri::command]
pub async fn mcp_new_token(state: State<'_, AppState>) -> CmdResult<()> {
    credentials::store_secret(credentials::MCP_TOKEN, &new_token()?).map_err(err)?;
    tracing::info!("MCP access token replaced");
    apply(&state).await;
    Ok(())
}
