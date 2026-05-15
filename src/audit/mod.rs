pub mod bucket_a;
pub mod bucket_b;
pub(crate) mod util;

use crate::finding::{Finding, Severity};
use crate::git::{self, Commit};
use anyhow::{bail, Result};
use serde::Serialize;
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

    let ctx = AuditContext {
        root: root.to_path_buf(),
        tracked: git::tracked_files(root)?,
        commits: git::commit_log(root)?,
        blob_history: git::blob_history(root)?,
    };

    let mut findings = Vec::new();
    findings.extend(bucket_a::check(&ctx));
    findings.extend(bucket_b::check(&ctx));

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
