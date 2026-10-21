//! stdio transport: newline-delimited JSON-RPC on stdin/stdout, for MCP
//! clients that start the server as a command. Nothing else may be
//! written to stdout.

use rc_store::Db;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol;

/// Serves until stdin closes.
pub async fn run(db: Db) -> std::io::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => protocol::handle(&db, msg).await,
            Err(_) => Some(protocol::error(Value::Null, protocol::PARSE_ERROR, "invalid JSON")),
        };
        if let Some(r) = reply {
            let mut bytes = serde_json::to_vec(&r).unwrap_or_default();
            bytes.push(b'\n');
            out.write_all(&bytes).await?;
            out.flush().await?;
        }
    }
    Ok(())
}
