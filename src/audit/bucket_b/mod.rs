//! Bucket B — steering-failure signature. The novel differentiator: reading
//! git history + filenames to diagnose *how* a repo got into its state.
//!
//! One file per signal. The commit-history signals are **content-based** —
//! they read diffs (per-file churn, blob-hash recurrence), not commit-message
//! text, because message text is unreliable: agents narrate inconsistently and
//! humans write terse "fix" subjects for legitimate work.

mod author_concentration;
mod churn_hotspots;
mod duplicate_siblings;
mod history_compression;
mod mega_commits;
mod reverted_content;
mod rollback_artifacts;
mod root_sprawl;
mod status_docs;

use crate::audit::AuditContext;
use crate::finding::Finding;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(churn_hotspots::check(ctx));
    findings.extend(reverted_content::check(ctx));
    findings.extend(duplicate_siblings::check(ctx));
    findings.extend(mega_commits::check(ctx));
    findings.extend(history_compression::check(ctx));
    findings.extend(status_docs::check(ctx));
    findings.extend(rollback_artifacts::check(ctx));
    findings.extend(root_sprawl::check(ctx));
    findings.extend(author_concentration::check(ctx));
    findings
}
