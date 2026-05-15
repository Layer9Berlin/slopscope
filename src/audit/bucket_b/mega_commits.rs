//! Signal: mega-commits — a single commit touching a huge fraction of the repo.
//!
//! When code is vibe-coded somewhere else and then dropped in wholesale —
//! `git add . && git commit -m "initial"` — one commit touches hundreds of
//! files at once. Real development lands in reviewable increments; a commit
//! that rewrites half the repo in one shot is a tell that git was used as a
//! delivery mechanism, not a development record.
//!
//! The repo's *first* commit legitimately introduces everything, so it is held
//! to a higher bar (and always labelled) — but it is not exempt: a giant
//! initial commit is exactly the "coded elsewhere, then imported" pattern.
//!
//! A repo with a long, dense history is — by construction — a real development
//! record, even if it has done the occasional huge import (neovim's vim-patch
//! syncs, redis auto-generating a command table). So in a large history we
//! only flag mega-commits when they are a *recurring* shape, not a rare blip.
//!
//! Two legitimate patterns also touch the whole repo at once, and we exclude
//! both:
//! - A **directory restructure** ("move all source into crates/") — the
//!   *opposite* of a delivery drop: almost no new lines, just files moving.
//!   Caught by a tiny net-to-gross ratio.
//! - A **mechanical sweep** (add a license header to every file, a doc-format
//!   migration) — touches thousands of files but only a line or two each. A
//!   real paste-drop carries substantial code per file. Caught by a low
//!   average added-lines-per-file.

use crate::audit::util::is_generated_or_fixture;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use crate::git::Commit;

/// Below this many tracked files, "fraction of the repo" is meaningless — a
/// 5-file repo where one commit touches 4 files is just a small repo.
const MIN_TRACKED: usize = 25;
/// A non-initial commit touching at least this fraction of tracked files.
const MEGA_FRACTION: f64 = 0.50;
/// The initial commit is held to a higher bar — most repos start substantial.
const INITIAL_MEGA_FRACTION: f64 = 0.90;
/// Absolute floor: any commit touching this many files is mega regardless of
/// repo size.
const MEGA_ABSOLUTE: usize = 300;
/// A commit at or above this fraction is Critical; so is >1 mega-commit.
const CRIT_FRACTION: f64 = 0.80;
/// A repo-wide commit whose net line change is below this fraction of its
/// gross churn is a restructure / move, not a paste-drop of new code.
const RESTRUCTURE_NET_RATIO: f64 = 0.15;
/// A real delivery drop carries substantial code per file. Below this many
/// added lines per touched file on average, the commit is a mechanical sweep
/// (license headers, a format migration), not vibe-coded content.
const MEGA_MIN_AVG_ADDED: f64 = 15.0;
/// A history longer than this is demonstrably a real development record.
const LARGE_HISTORY: usize = 1000;
/// In a large history, mega-commits must be at least this fraction of all
/// commits to count — a rare huge import (neovim's vim-patch sync) is a blip,
/// not "git used as a delivery drop".
const RECURRING_SHARE: f64 = 0.05;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    mega_commits(&ctx.commits, ctx.tracked.len())
        .into_iter()
        .collect()
}

struct Mega {
    hash: String,
    subject: String,
    hand_files: usize,
    fraction: f64,
    is_initial: bool,
}

fn mega_commits(commits: &[Commit], tracked: usize) -> Option<Finding> {
    if tracked < MIN_TRACKED || commits.is_empty() {
        return None;
    }
    // `commits` is newest-first, so the last element is the initial commit.
    let initial_hash = commits.last().map(|c| c.hash.as_str());

    let mut megas: Vec<Mega> = commits
        .iter()
        .filter_map(|c| {
            // Count only hand-authored files — a commit that vendors
            // node_modules touches thousands of files but that is a .gitignore
            // failure (bucket A territory), not a mega-commit of *code*.
            let hand: Vec<_> = c
                .files
                .iter()
                .filter(|f| !is_generated_or_fixture(&f.path) && !f.binary)
                .collect();
            let hand_files = hand.len();
            if hand_files == 0 {
                return None;
            }
            // A restructure moves files without writing new code: gross churn
            // is large but net is near zero. A paste-drop is nearly all adds.
            let added: u64 = hand.iter().map(|f| f.added as u64).sum();
            let deleted: u64 = hand.iter().map(|f| f.deleted as u64).sum();
            let gross = added + deleted;
            let net = added.abs_diff(deleted);
            if gross > 0 && (net as f64) / (gross as f64) < RESTRUCTURE_NET_RATIO {
                return None;
            }
            // A mechanical sweep touches every file but adds almost nothing to
            // each. A real delivery drop carries substantial code per file.
            if (added as f64) / (hand_files as f64) < MEGA_MIN_AVG_ADDED {
                return None;
            }
            let fraction = hand_files as f64 / tracked as f64;
            let is_initial = Some(c.hash.as_str()) == initial_hash;
            let bar = if is_initial {
                INITIAL_MEGA_FRACTION
            } else {
                MEGA_FRACTION
            };
            if fraction >= bar || hand_files >= MEGA_ABSOLUTE {
                Some(Mega {
                    hash: c.hash.clone(),
                    subject: c.subject.clone(),
                    hand_files,
                    fraction,
                    is_initial,
                })
            } else {
                None
            }
        })
        .collect();

    if megas.is_empty() {
        return None;
    }
    // In a long, dense history, a rare huge commit is a blip, not a habit. Only
    // flag when mega-commits are a *recurring* shape of how the repo is built.
    if commits.len() > LARGE_HISTORY
        && (megas.len() as f64) / (commits.len() as f64) < RECURRING_SHARE
    {
        return None;
    }
    megas.sort_by_key(|m| std::cmp::Reverse(m.hand_files));

    let worst_fraction = megas.iter().fold(0.0_f64, |m, x| m.max(x.fraction));
    let severity = if worst_fraction >= CRIT_FRACTION || megas.len() > 1 {
        Severity::Critical
    } else {
        Severity::Warn
    };

    let evidence: Vec<String> = megas
        .iter()
        .map(|m| {
            let tag = if m.is_initial { " (initial commit)" } else { "" };
            format!(
                "{}{}: {} hand-authored files ({}% of repo) — \"{}\"",
                &m.hash[..m.hash.len().min(12)],
                tag,
                m.hand_files,
                (m.fraction * 100.0).round() as u32,
                truncate(&m.subject, 60),
            )
        })
        .collect();

    Some(Finding::new(
        Category::SteeringFailure,
        "mega-commits",
        severity,
        format!(
            "{} commit(s) each touching a large fraction of the repo at once — \
             git used as a delivery drop, not a development record",
            megas.len()
        ),
        evidence,
    ))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max).collect();
        format!("{kept}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::one;
    use crate::git::{Commit, FileDelta};

    /// A commit touching `n` distinct hand-authored files.
    fn commit(hash: &str, subject: &str, n: usize) -> Commit {
        commit_with_prefix(hash, subject, n, "src/f")
    }

    fn commit_with_prefix(hash: &str, subject: &str, n: usize, prefix: &str) -> Commit {
        Commit {
            hash: hash.to_string(),
            timestamp: 0,
            author: "dev@example.com".to_string(),
            subject: subject.to_string(),
            // 30 added lines per file: a real paste-drop, clears MEGA_MIN_AVG_ADDED.
            files: (0..n)
                .map(|i| FileDelta {
                    path: format!("{prefix}{i}.rs"),
                    added: 30,
                    deleted: 0,
                    binary: false,
                })
                .collect(),
        }
    }

    #[test]
    fn tiny_repo_is_quiet() {
        // Below MIN_TRACKED — "fraction of repo" is meaningless.
        let commits = vec![commit("h1", "init", 10)];
        assert!(mega_commits(&commits, 10).is_none());
    }

    #[test]
    fn incremental_history_is_quiet() {
        // 100-file repo built in 20 commits of ~5 files each.
        let commits: Vec<Commit> = (0..20)
            .map(|i| commit(&format!("h{i}"), "feature", 5))
            .collect();
        assert!(mega_commits(&commits, 100).is_none());
    }

    #[test]
    fn mid_history_mega_commit_is_flagged() {
        // newest-first: a 60-file commit lands mid-history in a 100-file repo.
        let mut commits = vec![
            commit("hnew", "small", 3),
            commit("hmega", "rewrite everything", 60),
        ];
        commits.push(commit("hinit", "init", 10)); // initial, small
        let f = one(mega_commits(&commits, 100));
        assert_eq!(f.check, "mega-commits");
        assert_eq!(f.count, 1);
        assert!(f.evidence[0].starts_with("hmega"));
        assert!(!f.evidence[0].contains("initial commit"));
    }

    #[test]
    fn huge_initial_commit_is_flagged_and_labelled() {
        // A 95-file initial commit in a 100-file repo: coded elsewhere, imported.
        let commits = vec![
            commit("hnew", "tweak", 2),
            commit("hinit", "initial commit", 95),
        ];
        let f = one(mega_commits(&commits, 100));
        assert!(f.evidence[0].contains("(initial commit)"));
    }

    #[test]
    fn modest_initial_commit_is_quiet() {
        // A 55-file initial commit in a 100-file repo clears the non-initial
        // bar but not the higher INITIAL bar — most repos start substantial.
        let commits = vec![
            commit("hnew", "work", 5),
            commit("hinit", "initial commit", 55),
        ];
        assert!(mega_commits(&commits, 100).is_none());
    }

    #[test]
    fn two_mega_commits_is_critical() {
        let commits = vec![
            commit("ha", "big rewrite", 60),
            commit("hb", "another rewrite", 65),
            commit("hinit", "init", 10),
        ];
        let f = one(mega_commits(&commits, 100));
        assert_eq!(f.severity, Severity::Critical);
        assert_eq!(f.count, 2);
    }

    #[test]
    fn vendored_files_do_not_inflate_a_commit() {
        // 80 files but all under node_modules/ — a .gitignore failure, not a
        // mega-commit of hand-authored code.
        let commits = vec![
            commit("hnew", "work", 3),
            commit_with_prefix("hvendor", "add deps", 80, "node_modules/pkg/f"),
            commit("hinit", "init", 10),
        ];
        assert!(mega_commits(&commits, 100).is_none());
    }

    #[test]
    fn directory_restructure_is_not_a_mega_commit() {
        // "move all source into crates/": 80 files touched, but it is a move —
        // gross churn is large while net line change is ~zero.
        let restructure = Commit {
            hash: "hmove".to_string(),
            timestamp: 0,
            author: "dev@example.com".to_string(),
            subject: "repo: move all source into crates/".to_string(),
            files: (0..80)
                .map(|i| FileDelta {
                    path: format!("crates/core/f{i}.rs"),
                    added: 50,
                    deleted: 50,
                    binary: false,
                })
                .collect(),
        };
        let commits = vec![
            commit("hnew", "work", 3),
            restructure,
            commit("hinit", "init", 10),
        ];
        assert!(mega_commits(&commits, 100).is_none());
    }

    #[test]
    fn mechanical_sweep_is_not_a_mega_commit() {
        // "add a license header to every file": 200 files touched, but only
        // ~2 added lines each — a mechanical sweep, not a vibe-coded drop.
        let sweep = Commit {
            hash: "hsweep".to_string(),
            timestamp: 0,
            author: "dev@example.com".to_string(),
            subject: "copyright: add SPDX header to all files".to_string(),
            files: (0..200)
                .map(|i| FileDelta {
                    path: format!("src/f{i}.rs"),
                    added: 2,
                    deleted: 0,
                    binary: false,
                })
                .collect(),
        };
        let commits = vec![
            commit("hnew", "work", 3),
            sweep,
            commit("hinit", "init", 10),
        ];
        assert!(mega_commits(&commits, 300).is_none());
    }

    #[test]
    fn rare_mega_commit_in_a_long_history_is_quiet() {
        // A 1200-commit repo that did 3 huge imports (neovim's vim-patch sync):
        // 3 / 1203 < RECURRING_SHARE, so it reads as a real dev record.
        let mut commits: Vec<Commit> = (0..1200)
            .map(|i| commit(&format!("h{i}"), "normal work", 3))
            .collect();
        commits.insert(0, commit("hmega1", "import", 90));
        commits.insert(0, commit("hmega2", "import", 90));
        commits.insert(0, commit("hmega3", "import", 90));
        assert!(mega_commits(&commits, 150).is_none());
    }

    #[test]
    fn recurring_mega_commits_in_a_long_history_still_flag() {
        // Same long history, but mega-commits are a *habit* (>= RECURRING_SHARE).
        let mut commits: Vec<Commit> = (0..1100)
            .map(|i| commit(&format!("h{i}"), "normal work", 3))
            .collect();
        for i in 0..100 {
            commits.insert(0, commit(&format!("hm{i}"), "drop", 90));
        }
        assert!(mega_commits(&commits, 150).is_some());
    }

    #[test]
    fn absolute_floor_flags_huge_commit_in_huge_repo() {
        // 350 files in one commit, but the repo has 5000 — fraction is only 7%,
        // yet 350 files in one commit is mega by the absolute floor.
        let commits = vec![
            commit("hnew", "work", 3),
            commit("hbig", "import", 350),
            commit("hinit", "init", 50),
        ];
        let f = one(mega_commits(&commits, 5000));
        assert_eq!(f.count, 1);
        assert!(f.evidence[0].starts_with("hbig"));
    }
}
