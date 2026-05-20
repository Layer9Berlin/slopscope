//! Minimal MCP (Model Context Protocol) server over stdio.
//!
//! MCP's stdio transport is JSON-RPC 2.0 with newline-delimited messages —
//! one JSON object per line, no embedded newlines. That's simple enough that
//! a hand-rolled loop beats pulling in an async runtime (`tokio` + `rmcp`)
//! for what is a synchronous request/response exchange: the only tool we
//! expose runs a blocking audit and returns.
//!
//! Protocol surface implemented:
//! - `initialize` / `notifications/initialized`
//! - `tools/list` / `tools/call`
//! - `ping`
//!
//! Everything else gets a JSON-RPC "method not found". Notifications (no
//! `id`) never get a response.
//!
//! stdout carries *only* protocol messages — the audit path never prints
//! there (git output is captured, tree-sitter is silent), so the channel
//! stays clean. Diagnostics go to stderr.

use crate::{audit, report};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// MCP protocol revision we implement against. Clients negotiate; we echo
/// the client's requested version back when it sends one (the protocol has
/// been stable enough across recent revisions that this maximises compat).
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

/// Run the server: read JSON-RPC from stdin, write responses to stdout,
/// until EOF (client disconnect).
pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    eprintln!("slopscope mcp server ready (stdio)");

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break; // EOF — client disconnected.
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // Malformed JSON — we have no id to attach an error to, so
                // log and move on rather than guess.
                eprintln!("mcp: skipping malformed message: {e}");
                continue;
            }
        };
        if let Some(response) = handle(&request) {
            // serde_json::to_string is single-line — satisfies the
            // "no embedded newlines" rule of the stdio transport.
            let s = serde_json::to_string(&response)?;
            writeln!(out, "{s}")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// Dispatch one JSON-RPC message. Returns `Some(response)` for requests,
/// `None` for notifications (a message with no `id`).
fn handle(req: &Value) -> Option<Value> {
    let method = req.get("method").and_then(Value::as_str)?;
    // A message with no `id` is a notification: never answer it. This
    // single rule handles `notifications/initialized` and friends.
    let id = req.get("id").cloned()?;

    Some(match method {
        "initialize" => ok(id, initialize_result(req)),
        "tools/list" => ok(id, tools_list_result()),
        "tools/call" => handle_tool_call(id, req),
        "ping" => ok(id, json!({})),
        other => error(id, -32601, &format!("method not found: {other}")),
    })
}

fn initialize_result(req: &Value) -> Value {
    // Echo the client's requested protocol version when present.
    let version = req
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "slopscope",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn tools_list_result() -> Value {
    json!({
        "tools": [ audit_tool_schema() ]
    })
}

fn audit_tool_schema() -> Value {
    json!({
        "name": "audit",
        "description": "Audit a git repository for 'slop' — the structural \
            signature of AI / vibe-coded codebases. Returns deterministic, \
            ground-truth findings (not opinions): committed secrets, \
            mega-commits, churn hotspots, god functions, swallowed errors, \
            suppressed type/lint checks, dead code gates, dependency \
            pile-ups, and more. Each finding carries a severity \
            (info / warn / critical), a stable check id, and concrete \
            evidence — file paths, commit hashes, counts. Use this before \
            trusting an unfamiliar repo, or to verify your own changes \
            didn't introduce slop.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the git repository to audit. \
                        Defaults to the current directory."
                },
                "format": {
                    "type": "string",
                    "enum": ["human", "json"],
                    "description": "Output format. 'human' (default) is a \
                        readable report; 'json' is the structured finding \
                        list for programmatic use."
                }
            }
        }
    })
}

/// Run the `audit` tool. Per MCP, a *tool-execution* failure (not a git
/// repo, unreadable path) is a normal result with `isError: true` — only
/// protocol-level problems (unknown tool, bad params) are JSON-RPC errors.
fn handle_tool_call(id: Value, req: &Value) -> Value {
    let params = req.get("params");
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str);
    if name != Some("audit") {
        return error(
            id,
            -32602,
            &format!("unknown tool: {}", name.unwrap_or("<missing>")),
        );
    }
    let args = params.and_then(|p| p.get("arguments"));
    let path = args
        .and_then(|a| a.get("path"))
        .and_then(Value::as_str)
        .unwrap_or(".");
    let format = args
        .and_then(|a| a.get("format"))
        .and_then(Value::as_str)
        .unwrap_or("human");

    match audit::run(&PathBuf::from(path)) {
        Ok(rep) => {
            let text = if format == "json" {
                report::json(&rep)
            } else {
                report::human(&rep)
            };
            ok(id, tool_text(&text, false))
        }
        Err(e) => ok(id, tool_text(&format!("audit failed: {e:#}"), true)),
    }
}

/// An MCP `tools/call` result: a single text content block.
fn tool_text(text: &str, is_error: bool) -> Value {
    json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": is_error,
    })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn req(method: &str, id: Value, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_returns_capabilities_and_server_info() {
        let r = handle(&req("initialize", json!(1), json!({}))).expect("response");
        assert_eq!(r["jsonrpc"], "2.0");
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["serverInfo"]["name"], "slopscope");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r["result"]["protocolVersion"], DEFAULT_PROTOCOL_VERSION);
    }

    #[test]
    fn initialize_echoes_client_protocol_version() {
        let r = handle(&req(
            "initialize",
            json!(1),
            json!({ "protocolVersion": "2024-11-05" }),
        ))
        .expect("response");
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn notifications_get_no_response() {
        // No `id` field → notification → None.
        let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle(&msg).is_none());
    }

    #[test]
    fn tools_list_advertises_audit() {
        let r = handle(&req("tools/list", json!(2), json!({}))).expect("response");
        let tools = r["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "audit");
        assert!(tools[0]["inputSchema"]["properties"]["path"].is_object());
    }

    #[test]
    fn ping_returns_empty_result() {
        let r = handle(&req("ping", json!(3), json!({}))).expect("response");
        assert!(r["result"].is_object());
        assert!(r.get("error").is_none());
    }

    #[test]
    fn unknown_method_is_a_jsonrpc_error() {
        let r = handle(&req("does/not/exist", json!(4), json!({}))).expect("response");
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn unknown_tool_is_a_jsonrpc_error() {
        let r = handle(&req(
            "tools/call",
            json!(5),
            json!({ "name": "frobnicate", "arguments": {} }),
        ))
        .expect("response");
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn audit_on_non_git_path_is_tool_error_not_protocol_error() {
        let dir = tempfile::tempdir().unwrap();
        let r = handle(&req(
            "tools/call",
            json!(6),
            json!({
                "name": "audit",
                "arguments": { "path": dir.path().to_str().unwrap() }
            }),
        ))
        .expect("response");
        // Not a git repo → tool result with isError, NOT a JSON-RPC error.
        assert!(r.get("error").is_none());
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("audit failed"));
    }

    #[test]
    fn audit_on_real_repo_returns_a_report() {
        // Build a tiny git repo so the audit has something to chew on.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(p)
                .args(args)
                .output()
                .expect("git");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(p.join("main.rs"), "fn main() {}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let r = handle(&req(
            "tools/call",
            json!(7),
            json!({
                "name": "audit",
                "arguments": { "path": p.to_str().unwrap(), "format": "json" }
            }),
        ))
        .expect("response");
        assert!(r.get("error").is_none());
        assert_eq!(r["result"]["isError"], false);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        // `format: json` → the text payload itself parses as the report.
        let parsed: Value = serde_json::from_str(text).expect("report is json");
        assert!(parsed["findings"].is_array());
    }
}
