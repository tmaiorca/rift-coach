//! The MCP server over HTTP: handshake, tool listing and calls, and the
//! access checks (token, web-page origins, DNS rebinding).

use rc_common::Secret;
use rc_mcp::http::{HttpConfig, serve};
use rc_store::Db;
use serde_json::{Value, json};

async fn post(url: &str, body: Value, token: Option<&str>, origin: Option<&str>) -> (u16, Option<Value>) {
    let mut req = reqwest::Client::new().post(url).json(&body);
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    if let Some(o) = origin {
        req = req.header("Origin", o);
    }
    let res = req.send().await.expect("send");
    let status = res.status().as_u16();
    let text = res.text().await.expect("body");
    (status, serde_json::from_str(&text).ok())
}

fn call(id: i64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

fn tool_json(reply: &Value) -> Value {
    let text = reply["result"]["content"][0]["text"].as_str().expect("text");
    serde_json::from_str(text).expect("json text")
}

#[tokio::test]
async fn serves_read_only_tools_with_access_checks() {
    let db = Db::open_in_memory().expect("db");
    db.with(|c| rc_store::accounts::upsert_account(c, "Me", "NA1", "na1", "americas", true)).expect("acct");

    let server =
        serve(db.clone(), HttpConfig { port: 0, lan: false, token: Some(Secret::new("s3cret")) }).await.expect("serve");
    let url = format!("http://127.0.0.1:{}/mcp", server.addr.port());
    let tok = Some("s3cret");

    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}});
    let (s, r) = post(&url, init.clone(), tok, None).await;
    assert_eq!(s, 200);
    let r = r.expect("reply");
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["serverInfo"]["name"], "rift-coach");

    // Notifications get 202 and no body.
    let (s, _) = post(&url, json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), tok, None).await;
    assert_eq!(s, 202);

    let (_, r) = post(&url, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), tok, None).await;
    let tools = r.expect("reply")["result"]["tools"].as_array().cloned().expect("tools");
    assert!(tools.iter().any(|t| t["name"] == "get_match"));
    assert!(tools.iter().all(|t| t["annotations"]["readOnlyHint"] == true));

    let (_, r) = post(&url, call(3, "list_accounts", json!({})), tok, None).await;
    let accounts = tool_json(&r.expect("reply"));
    assert_eq!(accounts["accounts"][0]["riot_id"], "Me#NA1");

    let (_, r) = post(&url, call(4, "list_matches", json!({"riot_id": "me#na1"})), tok, None).await;
    assert_eq!(tool_json(&r.expect("reply"))["matches"], json!([]));

    // Bad arguments are tool errors the agent can read; unknown tools are protocol errors.
    let (_, r) = post(&url, call(5, "list_matches", json!({"riot_id": "Someone#Else"})), tok, None).await;
    let r = r.expect("reply");
    assert_eq!(r["result"]["isError"], true);
    let (_, r) = post(&url, call(6, "drop_tables", json!({})), tok, None).await;
    assert_eq!(r.expect("reply")["error"]["code"], -32602);

    // Access checks.
    assert_eq!(post(&url, init.clone(), None, None).await.0, 401);
    assert_eq!(post(&url, init.clone(), Some("wrong"), None).await.0, 401);
    assert_eq!(post(&url, init.clone(), tok, Some("https://evil.example")).await.0, 403);
    let rebinding = reqwest::Client::new()
        .post(&url)
        .header("Host", "evil.example")
        .header("Authorization", "Bearer s3cret")
        .json(&init)
        .send()
        .await
        .expect("send");
    assert_eq!(rebinding.status().as_u16(), 403);
    let get = reqwest::Client::new().get(&url).send().await.expect("send");
    assert_eq!(get.status().as_u16(), 405);

    // LAN access without a token is refused outright.
    assert!(serve(db, HttpConfig { port: 0, lan: true, token: None }).await.is_err());
    server.stop();
}
