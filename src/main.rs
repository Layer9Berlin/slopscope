use clap::{Parser, Subcommand};
use slopscope::{audit, finding::Severity, mcp, report};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "slopscope",
    version,
    about = "Detect slop in AI/vibe-coded codebases via process archaeology."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Full-repo audit across vcs-hygiene, steering-failure, inconsistency,
    /// verification, and complexity signals.
    Audit {
        /// Path to the repository (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit stable JSON instead of the human-readable report.
        #[arg(long)]
        json: bool,
    },
    /// Run as an MCP server over stdio — exposes the audit as a tool an
    /// agent can call. Speaks JSON-RPC 2.0 on stdin/stdout.
    Mcp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Audit { path, json } => run_audit(path, json),
        Command::Mcp => run_mcp(),
    }
}

/// Exit codes: 0 = clean shutdown, 3 = server error.
///
/// `mcp::serve` is async (the `rmcp` SDK is tokio-based), so we spin up a
/// runtime here rather than making all of `main` async — the `audit`
/// subcommand stays a plain synchronous path.
fn run_mcp() -> ExitCode {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("mcp error: could not start async runtime: {e}");
            return ExitCode::from(3);
        }
    };
    match runtime.block_on(mcp::serve()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mcp error: {e:#}");
            ExitCode::from(3)
        }
    }
}

/// Exit codes: 0 = clean / info-only, 1 = warnings, 2 = critical, 3 = error.
fn run_audit(path: PathBuf, json: bool) -> ExitCode {
    let report = match audit::run(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(3);
        }
    };

    if json {
        println!("{}", report::json(&report));
    } else {
        print!("{}", report::human(&report));
    }

    match report.worst() {
        Some(Severity::Critical) => ExitCode::from(2),
        Some(Severity::Warn) => ExitCode::from(1),
        _ => ExitCode::SUCCESS,
    }
}
