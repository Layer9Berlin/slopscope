use serde::Serialize;

/// Severity of a finding. Ordered: Info < Warn < Critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Critical,
}

/// Which category a finding belongs to. Categories are themed by what they
/// diagnose, not lettered by spec order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Committed files that should never have been committed (secrets,
    /// build output, accidental shell-fragment filenames).
    VcsHygiene,
    /// Git-history signature of an agent that couldn't be steered (churn
    /// loops, ping-ponging content, dump-commits, narration-doc piles).
    SteeringFailure,
    /// Pile-up of tools doing the same job — `eslint` + `biome`, two HTTP
    /// clients, two state libraries. Agent indecision frozen into deps.
    Inconsistency,
    /// Whether the repo has tests, a CI gate, a typecheck step — the
    /// scaffolding that makes a project verifiable at all.
    Verification,
    /// Structural complexity an agent kept piling onto instead of factoring
    /// out — god files that grow without refactor. Not generic LOC opinions:
    /// cross-referenced against recent git activity so only *living* big
    /// files are flagged.
    Complexity,
    /// Places the agent gave up or hand-waved — suppressed type/lint checks,
    /// `NotImplemented` stubs, swallowed errors, dead `if (false)` gates,
    /// AI-narrator comments left in code. The "got it working" without
    /// actually solving the problem signature.
    CodeSmell,
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
