//! A read-only MCP (Model Context Protocol) server so Claude Desktop or
//! another AI agent can query the player's Rift Coach data.
//!
//! - [`http`]: Streamable HTTP on `127.0.0.1` by default; on the LAN only
//!   with a bearer token. Run inside the app.
//! - [`stdio`]: for clients that launch a command (Claude Desktop):
//!   `rift-coach.exe --mcp-stdio` opens the database read-only.

pub mod http;
pub mod protocol;
pub mod stdio;
pub mod tools;
