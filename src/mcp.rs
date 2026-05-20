//! MCP server — exposes the slopscope audit as a tool over the Model
//! Context Protocol.
//!
//! Built on the official `rmcp` SDK rather than a hand-rolled JSON-RPC
//! loop: MCP is the spec's first-class product surface, so capability
//! negotiation, protocol-revision tracking, and alternative transports
//! should come from the SDK, not our maintenance budget.
//!
//! One tool, `audit`. The audit is synchronous and blocking (git
//! subprocesses, tree-sitter parsing, file IO), so it runs inside
//! `spawn_blocking` to keep the async executor free.

use crate::{audit, report};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::transport::stdio;
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use std::path::PathBuf;

/// Arguments for the `audit` tool. Field doc comments become the JSON-schema
/// descriptions the client sees.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AuditParams {
    /// Path to the git repository to audit. Defaults to the current directory.
    #[serde(default)]
    path: Option<String>,
    /// Output format: "human" (default, a readable report) or "json" (the
    /// structured finding list for programmatic use).
    #[serde(default)]
    format: Option<String>,
}

/// The MCP server. Holds the generated tool router; otherwise stateless.
#[derive(Clone)]
pub struct SlopscopeServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SlopscopeServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Audit a git repository for 'slop' — the structural \
            signature of AI / vibe-coded codebases. Returns deterministic, \
            ground-truth findings (not opinions): committed secrets, \
            mega-commits, churn hotspots, god functions, swallowed errors, \
            suppressed type/lint checks, dead code gates, dependency \
            pile-ups, and more. Each finding carries a severity \
            (info / warn / critical), a stable check id, and concrete \
            evidence — file paths, commit hashes, counts. Use this before \
            trusting an unfamiliar repo, or to verify your own changes \
            didn't introduce slop."
    )]
    async fn audit(
        &self,
        Parameters(params): Parameters<AuditParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let path = params.path.unwrap_or_else(|| ".".to_string());
        let want_json = params.format.as_deref() == Some("json");

        // `audit::run` is blocking — git subprocess, tree-sitter, file IO.
        // Run it off the async executor so the runtime stays responsive.
        let outcome = tokio::task::spawn_blocking(move || {
            audit::run(&PathBuf::from(&path)).map(|rep| {
                if want_json {
                    report::json(&rep)
                } else {
                    report::human(&rep)
                }
            })
        })
        .await;

        // A tool-execution failure (not a git repo, unreadable path) is a
        // normal result with is_error set — per MCP, only protocol-level
        // problems are JSON-RPC errors, which the SDK handles for us.
        Ok(match outcome {
            Ok(Ok(text)) => CallToolResult::success(vec![Content::text(text)]),
            Ok(Err(e)) => {
                CallToolResult::error(vec![Content::text(format!("audit failed: {e:#}"))])
            }
            Err(join_err) => CallToolResult::error(vec![Content::text(format!(
                "audit task panicked: {join_err}"
            ))]),
        })
    }
}

// `router = self.tool_router` makes the handler dispatch through the
// stored router field. Without it the macro defaults to calling
// `Self::tool_router()` afresh on every request — rebuilding the router
// each time and leaving our field unused.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for SlopscopeServer {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` (alias of `InitializeResult`) is `#[non_exhaustive]`,
        // so we start from `Default` and set the fields we care about
        // rather than using a struct literal.
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        // `Implementation::from_build_env()` resolves the *rmcp* crate's
        // package name, not ours — so set name/version explicitly from
        // slopscope's own build env.
        info.server_info =
            Implementation::new("slopscope", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "slopscope detects 'slop' in AI/vibe-coded codebases. Call the \
             `audit` tool with a repository path to get deterministic \
             findings an agent can act on."
                .to_string(),
        );
        info
    }
}

/// Run the MCP server over stdio until the client disconnects.
pub async fn serve() -> anyhow::Result<()> {
    eprintln!("slopscope mcp server ready (stdio)");
    let service = SlopscopeServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Build a minimal git repo so the audit has something to chew on.
    fn temp_git_repo() -> tempfile::TempDir {
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
        dir
    }

    #[tokio::test]
    async fn audit_on_non_git_path_is_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        let server = SlopscopeServer::new();
        let result = server
            .audit(Parameters(AuditParams {
                path: Some(dir.path().to_str().unwrap().to_string()),
                format: None,
            }))
            .await
            .expect("tool call should not be a protocol error");
        // Not a git repo → tool result flagged as an error.
        assert_eq!(result.is_error, Some(true));
        let text = result.content[0]
            .as_text()
            .unwrap()
            .text
            .clone();
        assert!(text.contains("audit failed"), "got: {text}");
    }

    #[tokio::test]
    async fn audit_on_real_repo_returns_json_report() {
        let dir = temp_git_repo();
        let server = SlopscopeServer::new();
        let result = server
            .audit(Parameters(AuditParams {
                path: Some(dir.path().to_str().unwrap().to_string()),
                format: Some("json".to_string()),
            }))
            .await
            .expect("tool call");
        assert_ne!(result.is_error, Some(true));
        let text = result.content[0]
            .as_text()
            .unwrap()
            .text
            .clone();
        // format=json → the payload itself parses as the report.
        let parsed: serde_json::Value =
            serde_json::from_str(&text).expect("report is json");
        assert!(parsed["findings"].is_array());
    }

    #[tokio::test]
    async fn audit_defaults_to_human_format() {
        let dir = temp_git_repo();
        let server = SlopscopeServer::new();
        let result = server
            .audit(Parameters(AuditParams {
                path: Some(dir.path().to_str().unwrap().to_string()),
                format: None,
            }))
            .await
            .expect("tool call");
        let text = result.content[0]
            .as_text()
            .unwrap()
            .text
            .clone();
        // Human report starts with the "slopscope audit —" banner.
        assert!(text.starts_with("slopscope audit"), "got: {text}");
    }

    #[test]
    fn server_info_advertises_tools_and_name() {
        let server = SlopscopeServer::new();
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
        assert_eq!(info.server_info.name, "slopscope");
    }
}
