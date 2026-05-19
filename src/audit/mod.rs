pub mod code_smell;
pub mod complexity;
pub mod inconsistency;
pub mod source;
pub mod steering_failure;
pub mod vcs_hygiene;
pub mod verification;
pub(crate) mod util;

use crate::finding::{Finding, Severity};
use crate::git::{self, Commit};
use anyhow::{bail, Result};
use serde::Serialize;
use source::SourceFile;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Everything the signals need, gathered once up front. Every signal is a pure
/// function of this context — `fn check(ctx: &AuditContext) -> Vec<Finding>` —
/// so adding a signal is: new file, one line in the bucket's `check`.
pub struct AuditContext {
    pub root: PathBuf,
    /// Files tracked by git — the deterministic "what was committed" set.
    pub tracked: Vec<String>,
    /// Full commit log with per-file churn, newest first.
    pub commits: Vec<Commit>,
    /// Per-path sequence of content blob hashes, oldest first — a recurring
    /// hash means the file returned to an exact prior content state.
    pub blob_history: HashMap<String, Vec<String>>,
    /// Recognized-source tracked files, loaded into memory. The substrate
    /// every content-scanning signal (suppressed checks, stub returns,
    /// swallowed errors, narrator comments, …) walks. Filtered to exclude
    /// generated/vendored/oversize/binary; see [`source::load_all`].
    pub source_files: Vec<SourceFile>,
}

#[derive(Debug, Serialize)]
pub struct AuditReport {
    pub root: String,
    pub commits_analyzed: usize,
    pub tracked_files: usize,
    pub findings: Vec<Finding>,
}

impl AuditReport {
    /// Worst severity across all findings, if any.
    pub fn worst(&self) -> Option<Severity> {
        self.findings.iter().map(|f| f.severity).max()
    }
}

pub fn run(root: &Path) -> Result<AuditReport> {
    if !git::is_git_repo(root) {
        bail!(
            "{} is not a git repository — slopscope's signal is git history",
            root.display()
        );
    }

    let tracked = git::tracked_files(root)?;
    let source_files = source::load_all(root, &tracked);
    let ctx = AuditContext {
        root: root.to_path_buf(),
        tracked,
        commits: git::commit_log(root)?,
        blob_history: git::blob_history(root)?,
        source_files,
    };

    let mut findings = Vec::new();
    findings.extend(vcs_hygiene::check(&ctx));
    findings.extend(steering_failure::check(&ctx));
    findings.extend(inconsistency::check(&ctx));
    findings.extend(verification::check(&ctx));
    findings.extend(complexity::check(&ctx));
    findings.extend(code_smell::check(&ctx));

    // Stable ordering: severity desc, then category, then check id.
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| format!("{:?}", a.category).cmp(&format!("{:?}", b.category)))
            .then_with(|| a.check.cmp(&b.check))
    });

    Ok(AuditReport {
        root: ctx.root.display().to_string(),
        commits_analyzed: ctx.commits.len(),
        tracked_files: ctx.tracked.len(),
        findings,
    })
}
