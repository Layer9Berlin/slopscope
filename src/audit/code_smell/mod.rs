//! Code smell — *places the agent gave up or hand-waved*. Suppressed
//! type/lint checks, `NotImplemented` stubs, swallowed errors, dead
//! `if (false)` gates, AI-narrator comments. Different from bucket B
//! (steering-failure): B reads git history, this reads file contents.
//!
//! Every signal here walks [`AuditContext::source_files`] — files loaded
//! once into memory by [`super::source::load_all`] — and pattern-matches per
//! language. Patterns aren't AST-precise; the rule is "false positives must
//! be cheap to ignore, false negatives are the real cost".

mod dead_gates;
mod narrator_comments;
mod stub_returns;
mod suppressed_checks;
mod swallowed_errors;

use crate::audit::AuditContext;
use crate::finding::Finding;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(suppressed_checks::check(ctx));
    findings.extend(stub_returns::check(ctx));
    findings.extend(swallowed_errors::check(ctx));
    findings.extend(dead_gates::check(ctx));
    findings.extend(narrator_comments::check(ctx));
    findings
}
