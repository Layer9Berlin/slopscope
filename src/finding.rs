use serde::Serialize;

/// Severity of a finding. Ordered: Info < Warn < Critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Critical,
}

/// Which detection bucket a finding belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Bucket A — committed files that should never have been committed.
    VcsHygiene,
    /// Bucket B — git-history signature of an agent that couldn't be steered.
    SteeringFailure,
}

/// A single deterministic finding. `evidence` is the ground truth — file
/// paths, commit hashes, counts — not an opinion.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub category: Category,
    /// Stable machine id, e.g. "committed-env-file". Never changes once shipped.
    pub check: String,
    pub severity: Severity,
    pub summary: String,
    pub count: usize,
    pub evidence: Vec<String>,
}

impl Finding {
    pub fn new(
        category: Category,
        check: &str,
        severity: Severity,
        summary: impl Into<String>,
        evidence: Vec<String>,
    ) -> Self {
        Finding {
            category,
            check: check.to_string(),
            severity,
            summary: summary.into(),
            count: evidence.len(),
            evidence,
        }
    }
}
