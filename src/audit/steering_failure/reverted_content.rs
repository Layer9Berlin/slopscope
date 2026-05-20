//! Signal: files ping-ponging between content states.
//!
//! Content, not commit text. We don't look for the word "Revert" — we look at
//! blob hashes. If a file's content returns to an *exact prior state* three or
//! more times, the same change is being made, reverted, and remade — a thrash
//! loop. One return is healthy git usage ("oops, revert that"); three is not.

use crate::audit::util::is_generated_or_fixture;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use std::collections::HashMap;

/// A content state must recur at least this many times to count as thrash.
/// Two occurrences is one ordinary revert; three can still happen in a
/// well-run repo (revert, then revert the revert). Four is a real ping-pong.
const MIN_REPEAT: usize = 4;
/// At or above this many recurrences, the file is Critical.
const CRIT_REPEAT: usize = 6;
/// The repeated state must be at least this fraction of the file's *whole*
/// distinct-state history. In long-lived repos (hugo 9k commits, postgres,
/// rails 72k) random files cross MIN_REPEAT by noise — a real ping-pong
/// dominates the file's life. Same idea as churn-hotspots' burst-share guard.
const MIN_RECURRENCE_SHARE: f64 = 0.5;

/// If at least this fraction of eligible files all flag at once — and there are
/// at least `REPO_EVENT_MIN_HITS` of them — it is almost certainly a single
/// repo-wide history event (subtree merge, filter-repo rewrite, vendored-tree
/// re-import), not thousands of independent thrash loops. Report it once.
const REPO_EVENT_FRACTION: f64 = 0.30;
const REPO_EVENT_MIN_HITS: usize = 50;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    reverted_content(&ctx.blob_history).into_iter().collect()
}

fn reverted_content(blob_history: &HashMap<String, Vec<String>>) -> Option<Finding> {
    let mut hits: Vec<(String, usize)> = Vec::new();
    let mut eligible = 0usize;
    for (path, blobs) in blob_history {
        // Generated/vendored files (lockfiles especially) legitimately oscillate
        // between content states — that says nothing about steering.
        if is_generated_or_fixture(path) {
            continue;
        }
        eligible += 1;
        // Collapse consecutive identical blobs first. A file added, deleted,
        // and re-added with the same content (subtree merges, history replay)
        // shows the same blob many times in a row — that is not ping-ponging.
        // Real thrash *alternates* between distinct states: A -> B -> A -> B.
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut prev: Option<&str> = None;
        for blob in blobs {
            let b = blob.as_str();
            if prev != Some(b) {
                *counts.entry(b).or_default() += 1;
                prev = Some(b);
            }
        }
        let max_repeat = counts.values().copied().max().unwrap_or(0);
        let total_transitions: usize = counts.values().sum();
        let share = if total_transitions > 0 {
            max_repeat as f64 / total_transitions as f64
        } else {
            0.0
        };
        if max_repeat >= MIN_REPEAT && share >= MIN_RECURRENCE_SHARE {
            hits.push((path.clone(), max_repeat));
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort();

    // Repo-wide event guard: when a large fraction of all eligible files flag
    // simultaneously, the cause is one history operation, not real steering
    // thrash. Report it once as context instead of emitting thousands of
    // per-file findings the agent would have to wade through.
    if hits.len() >= REPO_EVENT_MIN_HITS
        && eligible > 0
        && (hits.len() as f64) / (eligible as f64) >= REPO_EVENT_FRACTION
    {
        return Some(Finding::new(
            Category::SteeringFailure,
            "reverted-content",
            Severity::Info,
            format!(
                "{} of {} files share recurring content states — a repo-wide history event \
                 (subtree merge / history rewrite / vendored re-import), not steering thrash",
                hits.len(),
                eligible
            ),
            vec![format!(
                "{} files affected — too broad to be independent thrash; \
                 inspect git history for a merge or rewrite",
                hits.len()
            )],
        ));
    }

    let severity = if hits.iter().any(|(_, r)| *r >= CRIT_REPEAT) {
        Severity::Critical
    } else {
        Severity::Warn
    };
    let evidence = hits
        .iter()
        .map(|(path, repeat)| {
            format!("{path}: returned to the same content state {repeat} times")
        })
        .collect();

    Some(Finding::new(
        Category::SteeringFailure,
        "reverted-content",
        severity,
        format!(
            "{} file(s) ping-ponging between content states — changes made, reverted, and remade",
            hits.len()
        ),
        evidence,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(entries: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(path, blobs)| {
                (
                    path.to_string(),
                    blobs.iter().map(|s| s.to_string()).collect(),
                )
            })
            .collect()
    }

    fn one(f: Option<Finding>) -> Finding {
        f.expect("expected a finding")
    }

    #[test]
    fn empty_history_is_quiet() {
        assert!(reverted_content(&history(&[])).is_none());
    }

    #[test]
    fn single_revert_is_healthy() {
        // A -> B -> A: the file returned to state A once. Two occurrences of a
        // blob is an ordinary revert, not thrash.
        assert!(reverted_content(&history(&[("f.rs", &["A", "B", "A"])])).is_none());
    }

    #[test]
    fn re_addition_of_same_content_is_not_thrash() {
        // A file added/deleted/re-added 39 times shows the same blob 39 times
        // in a row (deletions are skipped upstream). Consecutive dedup collapses
        // it to a single state — not a ping-pong. This is the colormass/platform
        // 6330-file bug.
        let blobs: Vec<&str> = vec!["A"; 39];
        assert!(reverted_content(&history(&[("config", &blobs)])).is_none());
    }

    #[test]
    fn three_recurrences_is_still_quiet() {
        // A -> B -> A -> B -> A: state A reached 3 times — a revert-of-a-revert
        // can still happen in a well-run repo. Below MIN_REPEAT.
        assert!(reverted_content(&history(&[("f.rs", &["A", "B", "A", "B", "A"])])).is_none());
    }

    #[test]
    fn linear_history_is_quiet() {
        assert!(reverted_content(&history(&[("f.rs", &["A", "B", "C", "D"])])).is_none());
    }

    #[test]
    fn ping_ponging_four_times_is_warn() {
        // state A reached 4 times.
        let f = one(reverted_content(&history(&[(
            "auth.ts",
            &["A", "B", "A", "B", "A", "B", "A"],
        )])));
        assert_eq!(f.check, "reverted-content");
        assert_eq!(f.severity, Severity::Warn);
        assert_eq!(f.count, 1);
        assert!(f.evidence[0].contains("auth.ts"));
        assert!(f.evidence[0].contains("4 times"));
    }

    #[test]
    fn six_recurrences_is_critical() {
        let f = one(reverted_content(&history(&[(
            "x.rs",
            &["A", "B", "A", "B", "A", "B", "A", "B", "A", "B", "A"],
        )])));
        assert_eq!(f.severity, Severity::Critical);
    }

    #[test]
    fn occasional_recurrence_in_long_history_is_quiet() {
        // The hugo/rails/postgres case: a file with 100+ commits where 4
        // happen to recur on one state — 4/100 = 4% share, below
        // MIN_RECURRENCE_SHARE. Real long-lived files cross MIN_REPEAT by
        // noise; a real ping-pong dominates the file's life.
        let mut blobs: Vec<String> = (0..50).map(|i| format!("s{i}")).collect();
        // Sprinkle 4 returns to "A" across the history, separated by other
        // states (so dedup keeps each).
        for i in [5, 15, 25, 35].iter() {
            blobs[*i] = "A".to_string();
        }
        assert!(reverted_content(&history(&[("core.go", &blobs.iter().map(String::as_str).collect::<Vec<_>>())])).is_none());
    }

    #[test]
    fn repo_wide_recurrence_reports_once_as_info() {
        // 60 files all genuinely ping-ponging A->B->A->B->A->B->A. Per-file that
        // would be 60 Critical findings; but a thrash event hitting *every* file
        // at once is a history rewrite, not 60 independent steering failures.
        let entries: Vec<(String, Vec<String>)> = (0..60)
            .map(|i| {
                (
                    format!("f{i}.rs"),
                    ["A", "B", "A", "B", "A", "B", "A"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )
            })
            .collect();
        let hist: HashMap<String, Vec<String>> = entries.into_iter().collect();
        let f = one(reverted_content(&hist));
        assert_eq!(f.severity, Severity::Info);
        assert_eq!(f.count, 1);
        assert!(f.summary.contains("repo-wide history event"));
    }

    #[test]
    fn many_hits_below_min_count_still_report_per_file() {
        // 40 flagged files but each is a real ping-pong and 40 < REPO_EVENT_MIN_HITS:
        // not broad enough to assume a single history event.
        let entries: Vec<(String, Vec<String>)> = (0..40)
            .map(|i| {
                (
                    format!("f{i}.rs"),
                    ["A", "B", "A", "B", "A", "B", "A"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )
            })
            .collect();
        let hist: HashMap<String, Vec<String>> = entries.into_iter().collect();
        let f = one(reverted_content(&hist));
        assert_eq!(f.count, 40);
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn reports_every_affected_file_sorted() {
        let f = one(reverted_content(&history(&[
            ("z.rs", &["A", "B", "A", "B", "A", "B", "A"]),
            ("a.rs", &["X", "Y", "X", "Y", "X", "Y", "X"]),
            ("ok.rs", &["P", "Q", "P"]), // not flagged
        ])));
        assert_eq!(f.count, 2);
        assert!(f.evidence[0].starts_with("a.rs"));
        assert!(f.evidence[1].starts_with("z.rs"));
    }
}
