//! Signal: author concentration — context, not a verdict.
//!
//! Every other steering-failure signal assumes a *solo-driver* repo: one person (or
//! one person + one agent) steering. That assumption is what lets us read
//! churn loops and reverted content as "the agent couldn't be steered". In a
//! repo with dozens of contributors, the same patterns can be ordinary
//! distributed development — merges, parallel work, reverts across teams.
//!
//! So this signal never flags slop. It reports *how concentrated* authorship
//! is, so a reader knows whether to trust the rest of steering-failure at face value
//! or treat it as lower-confidence. Solo-driver repos produce no finding —
//! they need no caveat; the default assumption already holds.

use crate::audit::util::is_bot_author;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use crate::git::Commit;
use std::collections::HashMap;

/// A "substantial" contributor has at least this many commits. Drive-by
/// authors with one or two commits don't change who *steered* the repo, so
/// they don't count toward the solo-driver question — this is what keeps the
/// extremely common "one maintainer + hundreds of drive-by PRs" OSS shape from
/// reading as multi-author.
const MIN_SUBSTANTIAL_COMMITS: usize = 10;
/// At or above this many *substantial* authors, steering-failure's solo-driver
/// assumption is worth caveating.
const MULTI_AUTHOR_THRESHOLD: usize = 5;
/// ...unless one author still dominates this share of all commits — then it is
/// effectively solo-driven and the caveat would be misleading.
const SOLO_DOMINANCE_SHARE: f64 = 0.60;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    author_concentration(&ctx.commits).into_iter().collect()
}

fn author_concentration(commits: &[Commit]) -> Option<Finding> {
    // Bots (dependabot, CI) commit volume but don't *steer* — exclude them so
    // a maintainer + dependabot doesn't read as a two-driver repo.
    let mut by_author: HashMap<&str, usize> = HashMap::new();
    for c in commits {
        if is_bot_author(&c.author) {
            continue;
        }
        *by_author.entry(c.author.as_str()).or_default() += 1;
    }
    if by_author.is_empty() {
        return None;
    }

    let total: usize = by_author.values().sum();
    let top_count = by_author.values().copied().max().unwrap();
    let top_share = top_count as f64 / total as f64;
    let substantial = by_author
        .values()
        .filter(|&&n| n >= MIN_SUBSTANTIAL_COMMITS)
        .count();

    // Solo-driven (few substantial contributors, or one dominant author): the
    // default assumption holds, no caveat needed.
    if substantial < MULTI_AUTHOR_THRESHOLD || top_share >= SOLO_DOMINANCE_SHARE {
        return None;
    }

    let mut ranked: Vec<(&str, usize)> = by_author.iter().map(|(k, v)| (*k, *v)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let evidence: Vec<String> = ranked
        .iter()
        .take(5)
        .map(|(author, n)| {
            format!(
                "{author}: {n} commit(s) ({}%)",
                (*n as f64 / total as f64 * 100.0).round() as u32
            )
        })
        .collect();

    Some(Finding::new(
        Category::SteeringFailure,
        "author-concentration",
        Severity::Info,
        format!(
            "{substantial} substantial contributors (>= {MIN_SUBSTANTIAL_COMMITS} commits), \
             top at {}% of commits — steering-failure signals assume a solo-driver \
             repo, so treat them as lower-confidence here",
            (top_share * 100.0).round() as u32
        ),
        evidence,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::one;
    use crate::git::Commit;

    fn commit(author: &str) -> Commit {
        Commit {
            hash: "h".to_string(),
            timestamp: 0,
            author: author.to_string(),
            subject: "edit".to_string(),
            files: Vec::new(),
        }
    }

    fn log(authors: &[&str]) -> Vec<Commit> {
        authors.iter().map(|a| commit(a)).collect()
    }

    #[test]
    fn empty_history_is_quiet() {
        assert!(author_concentration(&[]).is_none());
    }

    #[test]
    fn solo_author_is_quiet() {
        assert!(author_concentration(&log(&["a@x.com"; 20])).is_none());
    }

    /// `n` commits each from the given authors, concatenated.
    fn balanced(authors: &[&str], n: usize) -> Vec<Commit> {
        authors
            .iter()
            .flat_map(|a| std::iter::repeat(*a).take(n))
            .map(commit)
            .collect()
    }

    #[test]
    fn few_substantial_authors_is_quiet() {
        // 3 substantial authors (>= 10 commits each) — below the threshold.
        assert!(author_concentration(&balanced(&["a@x.com", "b@x.com", "c@x.com"], 12)).is_none());
    }

    #[test]
    fn maintainer_plus_drive_by_contributors_is_quiet() {
        // The classic OSS shape: one maintainer with 200 commits, plus 50
        // drive-by authors with one commit each. Only one *substantial* author,
        // and the maintainer dominates — effectively solo-driven.
        let mut authors: Vec<&str> = vec!["lead@x.com"; 200];
        let drive_by: Vec<String> = (0..50).map(|i| format!("c{i}@x.com")).collect();
        authors.extend(drive_by.iter().map(String::as_str));
        assert!(author_concentration(&log(&authors)).is_none());
    }

    #[test]
    fn dominant_author_among_many_is_quiet() {
        // 6 substantial authors, but the lead holds > SOLO_DOMINANCE_SHARE.
        let mut authors: Vec<&str> = vec!["lead@x.com"; 100];
        for a in ["b@x.com", "c@x.com", "d@x.com", "e@x.com", "f@x.com"] {
            authors.extend(std::iter::repeat(a).take(11));
        }
        // lead 100 / 155 ~= 65% >= 60%.
        assert!(author_concentration(&log(&authors)).is_none());
    }

    #[test]
    fn many_balanced_substantial_authors_emit_info_caveat() {
        // 5 authors, 12 commits each — genuinely multi-driver, no dominance.
        let f = one(author_concentration(&balanced(
            &["a@x.com", "b@x.com", "c@x.com", "d@x.com", "e@x.com"],
            12,
        )));
        assert_eq!(f.check, "author-concentration");
        assert_eq!(f.severity, Severity::Info);
        assert!(f.summary.contains("substantial contributors"));
        assert!(!f.evidence.is_empty());
    }
}
