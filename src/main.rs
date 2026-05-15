use clap::{Parser, Subcommand};
use slopscope::{audit, finding::Severity, report};
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
    /// Full-repo audit: VCS hygiene (bucket A) + steering-failure signature (bucket B).
    Audit {
        /// Path to the repository (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit stable JSON instead of the human-readable report.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Audit { path, json } => run_audit(path, json),
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
