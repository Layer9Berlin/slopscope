//! Inconsistency — pile-up of tools doing the same job. The fingerprint of
//! an agent picking a different answer each time: `eslint` + `biome`, two
//! HTTP clients, two state libraries living in the same `package.json`.
//!
//! Cheap to build: most signals are filename and `package.json` content
//! checks — no git history needed.

mod manifests;
mod multiple_date_libs;
mod multiple_http_clients;
mod multiple_lint_format_configs;
mod multiple_state_libs;
mod multiple_validation_libs;

use crate::audit::AuditContext;
use crate::finding::Finding;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(multiple_lint_format_configs::check(ctx));
    findings.extend(multiple_http_clients::check(ctx));
    findings.extend(multiple_validation_libs::check(ctx));
    findings.extend(multiple_state_libs::check(ctx));
    findings.extend(multiple_date_libs::check(ctx));
    findings
}
