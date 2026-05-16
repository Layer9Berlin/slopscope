//! Bucket A — VCS hygiene. Files that should never have been committed.
//! Deterministic, ~100% precision: every finding is a fact about the
//! tracked-file set, not a judgement call.
//!
//! One file per signal. To add a signal: new file with `pub(crate) fn
//! check(ctx: &AuditContext) -> Vec<Finding>`, then one line in `check` below.

mod accidental_files;
mod committed_backups;
mod committed_build_output;
mod committed_large_files;
mod committed_secrets;
mod git_dir_outlier;
mod multiple_lockfiles;

use crate::audit::AuditContext;
use crate::finding::Finding;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(committed_secrets::check(ctx));
    findings.extend(committed_backups::check(ctx));
    findings.extend(committed_build_output::check(ctx));
    findings.extend(committed_large_files::check(ctx));
    findings.extend(accidental_files::check(ctx));
    findings.extend(multiple_lockfiles::check(ctx));
    findings.extend(git_dir_outlier::check(ctx));
    findings
}
