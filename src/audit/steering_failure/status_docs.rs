//! Signal: proliferation of agent-generated status / phase / summary docs —
//! FIX_*, DEBUG_*, CHECK_*, PHASE*_COMPLETE, *_SUMMARY. The breadcrumbs an
//! agent drops when it narrates its own flailing into files.

use crate::audit::util::basename;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    status_doc_proliferation(&ctx.tracked).into_iter().collect()
}

/// Status docs are *narration* files — `.md` notes, `.txt` logs, occasionally
/// `.sql` "fix" scripts. The prefix patterns (`FIX_`, `DEBUG_`, `CHECK_`, …)
/// alone aren't enough: postgres's `check_decls.m4`, django's
/// `.github/workflows/check_*.yml` and rails's `debug_exceptions.rb` all match
/// the prefix but are ordinary source, so we require a doc-shaped extension.
const STATUS_DOC_EXTS: &[&str] = &["md", "txt", "sql"];

fn status_doc_proliferation(tracked: &[String]) -> Option<Finding> {
    let mut hits = Vec::new();
    for path in tracked {
        let name = basename(path);
        let upper = name.to_ascii_uppercase();
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let is_doc_ext = ext.as_deref().is_some_and(|e| STATUS_DOC_EXTS.contains(&e));

        // Prefix patterns gate on doc extension — keeps `debug_exceptions.rb`
        // and `check_btree.out` out, lets `FIX_LOGIN.md` and `fix_signup.sql`
        // through.
        let prefix_hit = is_doc_ext
            && (upper.starts_with("FIX_")
                || upper.starts_with("FIXED_")
                || upper.starts_with("DEBUG_")
                || upper.starts_with("CHECK_")
                || upper.starts_with("TODO_"));

        // PHASE*_COMPLETE / PHASE*_DONE and the *_SUMMARY/_COMPLETE/_NOTES/
        // _STATUS suffixes already hard-code `.md`, so no extension gate.
        // `release_notes` is a universal docs convention (rails has 19 of
        // them) so we exclude it from `_NOTES.MD` matches.
        let is_release_notes = upper.contains("RELEASE_NOTES");
        let suffix_hit = (upper.starts_with("PHASE")
            && (upper.contains("_COMPLETE") || upper.contains("_DONE")))
            || upper.ends_with("_SUMMARY.MD")
            || upper.ends_with("_COMPLETE.MD")
            || (upper.ends_with("_NOTES.MD") && !is_release_notes)
            || upper.ends_with("_STATUS.MD");

        if prefix_hit || suffix_hit {
            hits.push(path.clone());
        }
    }
    if hits.len() < 3 {
        return None;
    }
    hits.sort();
    let severity = if hits.len() >= 10 {
        Severity::Critical
    } else {
        Severity::Warn
    };
    Some(Finding::new(
        Category::SteeringFailure,
        "status-doc-proliferation",
        severity,
        format!(
            "{} agent-generated status / phase / summary docs committed — the repo is narrating its own flailing",
            hits.len()
        ),
        hits,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    #[test]
    fn below_three_is_quiet() {
        assert!(status_doc_proliferation(&files(&["FIX_auth.md", "DEBUG_x.md"])).is_none());
    }

    #[test]
    fn matches_known_patterns() {
        let f = one(status_doc_proliferation(&files(&[
            "FIX_login.md",
            "FIXED_race.md",
            "DEBUG_api.md",
            "CHECK_perms.sql",
            "TODO_cleanup.md",
            "PHASE_2_COMPLETE.md",
            "PHASE3_DONE.md",
            "MIGRATION_SUMMARY.md",
            "AUTH_STATUS.md",
            "deploy_notes.md",
        ])));
        assert_eq!(f.count, 10);
        assert_eq!(f.severity, Severity::Critical); // >= 10
    }

    #[test]
    fn three_to_nine_is_warn() {
        let f = one(status_doc_proliferation(&files(&[
            "FIX_a.md",
            "DEBUG_b.md",
            "CHECK_c.md",
        ])));
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn release_notes_are_not_status_docs() {
        // Rails surfaced this: `guides/source/X_Y_release_notes.md` is a
        // universal docs convention, not agent narration.
        assert!(status_doc_proliferation(&files(&[
            "guides/source/2_2_release_notes.md",
            "guides/source/2_3_release_notes.md",
            "guides/source/3_0_release_notes.md",
            "guides/source/3_1_release_notes.md",
        ]))
        .is_none());
    }

    #[test]
    fn prefix_only_matches_doc_extensions() {
        // The new known-good corpus surfaced this: source files matching the
        // prefix patterns (`debug_exceptions.rb`, `check_decls.m4`,
        // `.github/workflows/check_*.yml`) are *not* status docs.
        assert!(status_doc_proliferation(&files(&[
            "actionpack/lib/action_dispatch/middleware/debug_exceptions.rb",
            "actionpack/lib/action_dispatch/middleware/debug_locks.rb",
            "config/check_decls.m4",
            "config/check_modules.pl",
            "contrib/amcheck/expected/check_btree.out",
            ".github/workflows/check_commit_messages.yml",
            ".github/workflows/check_pr_quality.yml",
        ]))
        .is_none());
    }

    #[test]
    fn rejects_lookalikes() {
        // FIXTURES.md starts with FIX but not "FIX_"; PHASE1.md has no _COMPLETE.
        assert!(status_doc_proliferation(&files(&[
            "FIXTURES.md",
            "PHASE1.md",
            "README.md",
            "SUMMARY.md", // no leading "_"
        ]))
        .is_none());
    }

    #[test]
    fn is_case_insensitive_and_uses_basename() {
        let f = one(status_doc_proliferation(&files(&[
            "docs/fix_a.md",
            "nested/dir/debug_b.md",
            "check_c.md",
        ])));
        assert_eq!(f.count, 3);
    }
}
