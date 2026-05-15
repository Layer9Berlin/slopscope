//! Signal: more than one package-manager lockfile at the repo root.

use crate::audit::util::{basename, is_root_level};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Lockfile names, one per package manager. Ecosystem-specific — lift into a
/// shared `ecosystems` table once a second ecosystem's worth accumulates.
const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "npm-shrinkwrap.json",
];

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    multiple_lockfiles(&ctx.tracked).into_iter().collect()
}

fn multiple_lockfiles(tracked: &[String]) -> Option<Finding> {
    let mut hits: Vec<String> = tracked
        .iter()
        .filter(|p| is_root_level(p) && LOCKFILES.contains(&basename(p)))
        .cloned()
        .collect();
    if hits.len() < 2 {
        return None;
    }
    hits.sort();
    Some(Finding::new(
        Category::VcsHygiene,
        "multiple-lockfiles",
        Severity::Warn,
        "Multiple package-manager lockfiles at repo root — the project can't decide on a package manager",
        hits,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    #[test]
    fn flagged_when_two_or_more_at_root() {
        let f = one(multiple_lockfiles(&files(&[
            "package-lock.json",
            "yarn.lock",
            "src/main.rs",
        ])));
        assert_eq!(f.severity, Severity::Warn);
        assert_eq!(f.evidence, vec!["package-lock.json", "yarn.lock"]);
    }

    #[test]
    fn single_is_fine() {
        assert!(multiple_lockfiles(&files(&["package-lock.json"])).is_none());
    }

    #[test]
    fn ignores_nested_lockfiles() {
        // lockfiles inside sub-packages are normal in monorepos
        assert!(multiple_lockfiles(&files(&[
            "package-lock.json",
            "packages/ui/yarn.lock",
        ]))
        .is_none());
    }
}
