//! Verification — does the repo have the scaffolding that makes it
//! verifiable at all? Tests, a CI gate, a typecheck step. None of these are
//! about *quality of code* — that's other categories' job. They diagnose
//! whether anyone is checking.
//!
//! Cheap to build: filename and basic content checks, no git history needed.

mod ci_presence;
mod test_source_ratio;

use crate::audit::AuditContext;
use crate::finding::Finding;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(ci_presence::check(ctx));
    findings.extend(test_source_ratio::check(ctx));
    findings
}
