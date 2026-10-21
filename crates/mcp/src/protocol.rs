//! MCP over JSON-RPC 2.0: `initialize`, `ping`, `tools/list`, `tools/call`
//! (plus empty resource and prompt lists for clients that ask). Shared by
//! the HTTP and stdio transports.

use rc_store::Db;
use serde_json::{Value, json};

use crate::tools::{self, ToolError};

/// Protocol versions this server speaks, newest first.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Rift Coach is a League of Legends coach running on the player's PC. These tools read \
the player's own stored games (Summoner's Rift metrics, patterns, AI reviews, goals, weekly summaries, \
higher-rank benchmarks). Everything is read-only. Percentiles are 0-100 where higher is better for the \
player. Data covers only tracked accounts; other players' identities are never included.";

pub const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

pub fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Handles one message. `None` for notifications and responses (nothing to send back).
pub async fn handle(db: &Db, msg: Value) -> Option<Value> {
    let Value::Object(m) = msg else {
        return Some(error(Value::Null, INVALID_REQUEST, "expected a single JSON-RPC message"));
    };
    let method = m.get("method").and_then(Value::as_str);
    let id = m.get("id").cloned();
    let (Some(method), Some(id)) = (method, id) else {
        // A notification (no id) or a response to us: nothing to answer.
        return None;
    };
    let params = m.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked.filter(|v| SUPPORTED_VERSIONS.contains(v)).unwrap_or(SUPPORTED_VERSIONS[0]);
            ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "rift-coach", "title": "Rift Coach", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({"tools": tools::definitions()})),
        "resources/list" => ok(id, json!({"resources": []})),
        "resources/templates/list" => ok(id, json!({"resourceTemplates": []})),
        "prompts/list" => ok(id, json!({"prompts": []})),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, INVALID_PARAMS, "tools/call needs a tool name"));
            };
            let args = params.get("arguments").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
            match tools::call(db, name, args).await {
                Ok(v) => ok(id, tool_result(&v, false)),
                Err(ToolError::Unknown(n)) => error(id, INVALID_PARAMS, format!("unknown tool: {n}")),
                Err(ToolError::Invalid(msg)) => ok(id, tool_result(&json!({"error": msg}), true)),
                Err(ToolError::Internal(msg)) => {
                    tracing::warn!(tool = name, error = %msg, "MCP tool failed");
                    error(id, INTERNAL_ERROR, msg)
                }
            }
        }
        _ => error(id, METHOD_NOT_FOUND, format!("method not found: {method}")),
    })
}

fn tool_result(v: &Value, is_error: bool) -> Value {
    // Compact: it all goes into the agent's context.
    let text = serde_json::to_string(v).unwrap_or_default();
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}
