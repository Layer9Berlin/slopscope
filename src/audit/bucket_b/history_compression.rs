//! Signal: a git history too compressed to be a real development record.
//!
//! Two shapes of the same tell — git was populated *after* the code existed,
//! not alongside it:
//! - **Compressed timespan:** a substantial repo whose entire history spans
//!   less than a day. Real projects of any size accrete over weeks; a 200-file
//!   repo born in 40 minutes was built elsewhere and committed in a sitting.
//! - **LOC firehose:** very few commits, each carrying an enormous payload.
//!   A handful of 5000-line commits is not iterative work — it is paste-drops.
//!
//! Both are weak alone (a genuine hackathon repo is young; a vendored import
//! is one huge commit) so thresholds are deliberately conservative.

use crate::audit::util::is_generated_or_fixture;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use crate::git::Commit;

/// "Substantial" repo: below this many tracked files, a young history is just
/// a young small project, not a tell.
const MIN_TRACKED: usize = 30;
/// Whole-history span under this counts as compressed.
const COMPRESSED_SPAN_SECS: i64 = 24 * 3600;
/// LOC-firehose: at most this many commits...
const FIREHOSE_MAX_COMMITS: usize = 6;
/// ...carrying at least this much hand-authored added churn in total.
const FIREHOSE_MIN_ADDED: u32 = 5000;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    history_compression(&ctx.commits, ctx.tracked.len())
        .into_iter()
        .collect()
}

fn history_compression(commits: &[Commit], tracked: usize) -> Option<Finding> {
    if commits.len() < 2 {
        return None; // need a span to measure
    }

    let min_ts = commits.iter().map(|c| c.timestamp).min().unwrap();
    let max_ts = commits.iter().map(|c| c.timestamp).max().unwrap();
    let span = max_ts - min_ts;

    let total_added: u32 = commits
        .iter()
        .flat_map(|c| &c.files)
        .filter(|f| !is_generated_or_fixture(&f.path))
        .map(|f| f.added)
        .sum();

    let mut reasons: Vec<String> = Vec::new();

    if tracked >= MIN_TRACKED && span < COMPRESSED_SPAN_SECS {
        reasons.push(format!(
            "{} files, but the entire history spans {} — built elsewhere, committed in one sitting",
            tracked,
            fmt_span(span),
        ));
    }

    if commits.len() <= FIREHOSE_MAX_COMMITS && total_added >= FIREHOSE_MIN_ADDED {
        reasons.push(format!(
            "{} commit(s) carrying {} added lines — {} lines/commit, paste-drops not iteration",
            commits.len(),
            total_added,
            total_added / commits.len() as u32,
        ));
    }

    if reasons.is_empty() {
        return None;
    }

    // Both shapes firing at once is a strong combined signal.
    let severity = if reasons.len() > 1 {
        Severity::Critical
    } else {
        Severity::Warn
    };

    Some(Finding::new(
        Category::SteeringFailure,
        "history-compression",
        severity,
        "git history is too compressed to be a real development record — \
         code was populated into git after the fact"
            .to_string(),
        reasons,
    ))
}

fn fmt_span(secs: i64) -> String {
    if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 24 * 3600 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::one;
    use crate::git::{Commit, FileDelta};

    const DAY: i64 = 86_400;

    fn commit(ts: i64, added: u32) -> Commit {
        Commit {
            hash: format!("{ts:08x}"),
            timestamp: ts,
            author: "dev@example.com".to_string(),
            subject: "edit".to_string(),
            files: vec![FileDelta {
                path: "src/main.rs".to_string(),
                added,
                deleted: 0,
                binary: false,
            }],
        }
    }

    #[test]
    fn single_commit_is_quiet() {
        assert!(history_compression(&[commit(0, 100)], 100).is_none());
    }

    #[test]
    fn healthy_long_history_is_quiet() {
        // 30 commits over 30 days, modest churn each.
        let commits: Vec<Commit> = (0..30).map(|i| commit(i * DAY, 100)).collect();
        assert!(history_compression(&commits, 200).is_none());
    }

    #[test]
    fn small_young_repo_is_quiet() {
        // A 10-file repo committed in an hour — just a small new project.
        let commits: Vec<Commit> = (0..5).map(|i| commit(i * 600, 50)).collect();
        assert!(history_compression(&commits, 10).is_none());
    }

    #[test]
    fn substantial_repo_in_under_a_day_is_flagged() {
        // 200-file repo, 10 commits all within ~40 minutes.
        let commits: Vec<Commit> = (0..10).map(|i| commit(i * 240, 50)).collect();
        let f = one(history_compression(&commits, 200));
        assert_eq!(f.check, "history-compression");
        assert_eq!(f.severity, Severity::Warn);
        assert!(f.evidence[0].contains("entire history spans"));
    }

    #[test]
    fn loc_firehose_is_flagged() {
        // 3 commits, ~3000 added lines each, spread over months (span is fine).
        let commits: Vec<Commit> = (0..3).map(|i| commit(i * 60 * DAY, 3000)).collect();
        let f = one(history_compression(&commits, 50));
        assert_eq!(f.severity, Severity::Warn);
        assert!(f.evidence[0].contains("paste-drops"));
    }

    #[test]
    fn both_shapes_at_once_is_critical() {
        // 3 commits, huge payload, all within an hour, big repo.
        let commits: Vec<Commit> = (0..3).map(|i| commit(i * 600, 4000)).collect();
        let f = one(history_compression(&commits, 100));
        assert_eq!(f.severity, Severity::Critical);
        assert_eq!(f.count, 2);
    }

    #[test]
    fn vendored_churn_does_not_feed_the_firehose() {
        // Few commits, but the added lines are all vendored — not paste-dropped
        // hand-authored code.
        let commits: Vec<Commit> = (0..3)
            .map(|i| Commit {
                hash: format!("h{i}"),
                timestamp: i * 60 * DAY,
                author: "dev@example.com".to_string(),
                subject: "deps".to_string(),
                files: vec![FileDelta {
                    path: "node_modules/pkg/index.js".to_string(),
                    added: 9000,
                    deleted: 0,
                    binary: false,
                }],
            })
            .collect();
        assert!(history_compression(&commits, 50).is_none());
    }
}
