//! Complexity — structural tells of code an agent kept piling onto without
//! refactoring. The shape of this bucket is deliberately narrow: we don't
//! pretend to measure "maintainability" the way SonarQube et al. do.
//! Instead, each signal cross-references a cheap structural metric (file
//! size, directory count) against recent git activity, so only *agent-
//! amplified* complexity is flagged — a 2000-line file that hasn't been
//! touched in a year is not slop.

mod oversized_growing_files;

use crate::audit::AuditContext;
use crate::finding::Finding;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(oversized_growing_files::check(ctx));
    findings
}
