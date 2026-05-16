//! Signal: no CI configuration in a repo big enough that absence is a tell.
//!
//! A real project has *something* — GitHub Actions, GitLab CI, Circle, a
//! Jenkinsfile, an Azure pipeline — gating merges. A vibe-coded repo
//! frequently has none: the agent ships code without ever wiring up a
//! verifier. We don't flag tiny experimental repos (too noisy); below
//! `MIN_TRACKED`, "no CI" is just "small project".

use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Below this many tracked files, absence-of-CI is uninformative — small
/// experiments routinely have no CI and that is fine.
const MIN_TRACKED: usize = 30;

/// Path *prefixes* (or basenames at root) that indicate a CI provider. We
/// match on exact filename or `dir/` prefix because checking arbitrary
/// substring would catch `.github/ISSUE_TEMPLATE/` and similar non-CI paths.
const CI_MARKERS: &[&str] = &[
    ".github/workflows/",
    ".gitlab-ci.yml",
    ".gitlab/ci.yml",
    ".circleci/",
    "azure-pipelines.yml",
    "Jenkinsfile",
    ".jenkinsfile",
    "bitbucket-pipelines.yml",
    ".woodpecker.yml",
    ".woodpecker/",
    ".drone.yml",
    "appveyor.yml",
    ".appveyor.yml",
    ".travis.yml",
    "wercker.yml",
    "buildkite.yml",
    ".buildkite/",
    "cloudbuild.yaml",
    "cloudbuild.yml",
    ".cirrus.yml",
];

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    ci_presence(&ctx.tracked).into_iter().collect()
}

fn ci_presence(tracked: &[String]) -> Option<Finding> {
    if tracked.len() < MIN_TRACKED {
        return None;
    }
    let has_ci = tracked.iter().any(|p| {
        CI_MARKERS
            .iter()
            .any(|m| p == m || p.starts_with(m))
    });
    if has_ci {
        return None;
    }
    Some(Finding::new(
        Category::Verification,
        "no-ci-config",
        Severity::Warn,
        format!(
            "{} tracked files, but no CI configuration found — no automated check \
             gates merges; nothing prevents broken code from landing",
            tracked.len()
        ),
        vec!["no .github/workflows, .gitlab-ci.yml, .circleci/, Jenkinsfile, or peer".to_string()],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    fn many(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("src/f{i}.rs")).collect()
    }

    #[test]
    fn small_repo_is_quiet() {
        // 10 files, no CI — but too small to indict.
        assert!(ci_presence(&many(10)).is_none());
    }

    #[test]
    fn substantial_repo_without_ci_is_warn() {
        let f = one(ci_presence(&many(50)));
        assert_eq!(f.check, "no-ci-config");
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn github_actions_satisfies() {
        let mut paths = many(50);
        paths.push(".github/workflows/ci.yml".into());
        assert!(ci_presence(&paths).is_none());
    }

    #[test]
    fn gitlab_ci_satisfies() {
        let mut paths = many(50);
        paths.push(".gitlab-ci.yml".into());
        assert!(ci_presence(&paths).is_none());
    }

    #[test]
    fn circleci_dir_satisfies() {
        let mut paths = many(50);
        paths.push(".circleci/config.yml".into());
        assert!(ci_presence(&paths).is_none());
    }

    #[test]
    fn jenkinsfile_at_root_satisfies() {
        let mut paths = many(50);
        paths.push("Jenkinsfile".into());
        assert!(ci_presence(&paths).is_none());
    }

    #[test]
    fn non_ci_github_paths_do_not_satisfy() {
        // .github/ISSUE_TEMPLATE/, .github/CODEOWNERS — common but not CI.
        let mut paths = many(50);
        paths.push(".github/ISSUE_TEMPLATE/bug.md".into());
        paths.push(".github/CODEOWNERS".into());
        assert!(ci_presence(&paths).is_some());
    }

    #[test]
    fn integrates_with_files_helper() {
        // Sanity check that the test_helpers::files() path still works.
        assert!(ci_presence(&files(&["README.md"])).is_none());
    }
}
