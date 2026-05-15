//! Signal: an unusually large `.git` directory — often the ghost of
//! committed-then-deleted binaries still living in history.

use crate::audit::util::human_bytes;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use crate::git;

/// `.git` directories larger than this get an informational outlier finding.
const LARGE_GIT_DIR_BYTES: u64 = 100 * 1024 * 1024;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let Ok(size) = git::git_dir_size(&ctx.root) else {
        return Vec::new();
    };
    git_dir_finding(size).into_iter().collect()
}

/// Pure decision logic, split from the git call for testing.
fn git_dir_finding(size: u64) -> Option<Finding> {
    if size <= LARGE_GIT_DIR_BYTES {
        return None;
    }
    Some(Finding::new(
        Category::VcsHygiene,
        "git-dir-size-outlier",
        Severity::Info,
        format!(
            "The .git directory is {} — unusually large, often a sign of committed-then-deleted binaries",
            human_bytes(size)
        ),
        vec![format!(".git = {}", human_bytes(size))],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::one;

    #[test]
    fn quiet_under_threshold() {
        assert!(git_dir_finding(LARGE_GIT_DIR_BYTES).is_none());
        assert!(git_dir_finding(50 * 1024 * 1024).is_none());
    }

    #[test]
    fn fires_over_threshold() {
        let f = one(git_dir_finding(LARGE_GIT_DIR_BYTES + 1));
        assert_eq!(f.severity, Severity::Info);
        assert_eq!(f.check, "git-dir-size-outlier");
    }
}
