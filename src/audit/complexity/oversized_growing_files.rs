//! Signal: large source files that *kept growing* in recent activity.
//!
//! Big files alone are not slop. postgres has many .c files >2000 lines that
//! are stable, well-maintained, and edited once a month — those are fine.
//! The agent-amplified shape we're after is: file is already large AND has
//! been heavily edited in the recent window AND has *net grown* — features
//! piled on instead of factored out. Pure thrash (added == deleted) lives
//! in steering-failure's churn-hotspots; this signal is about accumulation.
//!
//! Recency is measured from the latest commit timestamp, not wallclock now,
//! so a historical repo cloned years later gets a fair read.
//!
//! Generated / fixture / manifest / non-source paths are excluded so docs,
//! configs, lockfiles, vendored code don't inflate the count.

use crate::audit::util::{is_generated_or_fixture, is_manifest};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use crate::git::Commit;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Minimum file size to be considered. Below this nothing reads as a "god
/// file" regardless of churn. Set high enough that ordinary modules in
/// well-run repos (React's reconciler files, postgres .c files) clear the
/// bar by activity, not by accident.
const MIN_LINES: usize = 800;
/// File size at which the file *itself* is large enough to escalate the
/// finding to Critical even at lower recent commit counts.
const CRIT_LINES: usize = 2000;
/// Width of the "recent activity" window, measured from the repo's latest
/// commit timestamp.
const RECENT_SECS: i64 = 90 * 24 * 3600;
/// Minimum recent commits touching the file. Below this the file isn't
/// "actively grown" — it's just large.
const MIN_RECENT_COMMITS: usize = 10;
/// At or above this many recent commits the file escalates to Critical.
const CRIT_RECENT_COMMITS: usize = 20;
/// Recent edits must net-add at least this many lines. Pure thrash (gross
/// churn cancels out) is steering-failure territory; here we want growth.
const MIN_RECENT_NET_GROWTH: i64 = 200;

/// Source extensions only — measuring growth in docs/configs/assets would
/// be meaningless, and they would dominate any large-file ranking.
const SOURCE_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte", "astro", "py", "rs", "go", "java",
    "kt", "kts", "scala", "rb", "php", "cs", "fs", "swift", "m", "mm", "c", "cc", "cpp", "cxx",
    "h", "hpp", "elm", "ex", "exs", "erl", "lua", "ml", "mli",
];

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let recent = recent_activity(&ctx.commits);
    let line_counts = read_line_counts(&ctx.root, &ctx.tracked, &recent);
    oversized_growing_files(&ctx.tracked, &recent, &line_counts)
        .into_iter()
        .collect()
}

struct Activity {
    commits: usize,
    added: i64,
    deleted: i64,
}

struct Spot {
    path: String,
    lines: usize,
    recent_commits: usize,
    recent_net_growth: i64,
    severity: Severity,
}

fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, e)| e)
}

fn is_source_path(path: &str) -> bool {
    if is_generated_or_fixture(path) || is_manifest(path) {
        return false;
    }
    let Some(ext) = ext_of(path).map(str::to_ascii_lowercase) else {
        return false;
    };
    SOURCE_EXTS.contains(&ext.as_str())
}

/// Aggregate per-file activity within the recent window. Window is anchored
/// to the latest commit in the log, not wallclock now.
fn recent_activity(commits: &[Commit]) -> HashMap<String, Activity> {
    let Some(latest_ts) = commits.iter().map(|c| c.timestamp).max() else {
        return HashMap::new();
    };
    let cutoff = latest_ts - RECENT_SECS;
    let mut recent: HashMap<String, Activity> = HashMap::new();
    for c in commits {
        if c.timestamp < cutoff {
            continue;
        }
        for d in &c.files {
            if d.binary {
                continue;
            }
            let a = recent.entry(d.path.clone()).or_insert(Activity {
                commits: 0,
                added: 0,
                deleted: 0,
            });
            a.commits += 1;
            a.added += d.added as i64;
            a.deleted += d.deleted as i64;
        }
    }
    recent
}

/// Read line counts from disk only for files that already pass the recent-
/// activity gate. Avoids `fs::read_to_string` on thousands of cold files.
fn read_line_counts(
    root: &Path,
    tracked: &[String],
    recent: &HashMap<String, Activity>,
) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for path in tracked {
        if !is_source_path(path) {
            continue;
        }
        let Some(a) = recent.get(path) else { continue };
        if a.commits < MIN_RECENT_COMMITS || (a.added - a.deleted) < MIN_RECENT_NET_GROWTH {
            continue;
        }
        if let Ok(content) = fs::read_to_string(root.join(path)) {
            counts.insert(path.clone(), content.lines().count());
        }
    }
    counts
}

fn oversized_growing_files(
    tracked: &[String],
    recent: &HashMap<String, Activity>,
    line_counts: &HashMap<String, usize>,
) -> Option<Finding> {
    let mut spots: Vec<Spot> = Vec::new();
    for path in tracked {
        if !is_source_path(path) {
            continue;
        }
        let Some(a) = recent.get(path) else { continue };
        if a.commits < MIN_RECENT_COMMITS {
            continue;
        }
        let net = a.added - a.deleted;
        if net < MIN_RECENT_NET_GROWTH {
            continue;
        }
        let Some(&lines) = line_counts.get(path) else {
            continue;
        };
        if lines < MIN_LINES {
            continue;
        }
        let severity = if lines >= CRIT_LINES || a.commits >= CRIT_RECENT_COMMITS {
            Severity::Critical
        } else {
            Severity::Warn
        };
        spots.push(Spot {
            path: path.clone(),
            lines,
            recent_commits: a.commits,
            recent_net_growth: net,
            severity,
        });
    }

    if spots.is_empty() {
        return None;
    }
    // Worst-first by line count, ties broken by path for stable output.
    spots.sort_by(|a, b| b.lines.cmp(&a.lines).then(a.path.cmp(&b.path)));

    let severity = spots.iter().map(|s| s.severity).max().unwrap();
    let evidence: Vec<String> = spots
        .iter()
        .map(|s| {
            format!(
                "{}: {} lines, {} recent commit(s), +{} net in last 90 day(s)",
                s.path, s.lines, s.recent_commits, s.recent_net_growth
            )
        })
        .collect();
    let summary = format!(
        "{} large source file(s) kept growing without refactor in recent activity \
         — features piled on instead of factored out",
        spots.len()
    );
    Some(Finding::new(
        Category::Complexity,
        "oversized-growing-files",
        severity,
        summary,
        evidence,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileDelta;

    const DAY: i64 = 86_400;

    fn touch(ts: i64, path: &str, added: u32, deleted: u32) -> Commit {
        Commit {
            hash: format!("{ts:08x}"),
            timestamp: ts,
            author: "dev@example.com".to_string(),
            subject: "edit".to_string(),
            files: vec![FileDelta {
                path: path.to_string(),
                added,
                deleted,
                binary: false,
            }],
        }
    }

    fn run(
        tracked: &[&str],
        commits: &[Commit],
        line_counts: &[(&str, usize)],
    ) -> Option<Finding> {
        let tracked: Vec<String> = tracked.iter().map(|s| s.to_string()).collect();
        let recent = recent_activity(commits);
        let counts: HashMap<String, usize> = line_counts
            .iter()
            .map(|(p, n)| (p.to_string(), *n))
            .collect();
        oversized_growing_files(&tracked, &recent, &counts)
    }

    #[test]
    fn small_file_is_quiet() {
        // Tons of recent commits, big net growth, but only 200 lines.
        let commits: Vec<Commit> = (0..15)
            .map(|i| touch(i * DAY, "src/a.ts", 80, 10))
            .collect();
        assert!(run(&["src/a.ts"], &commits, &[("src/a.ts", 200)]).is_none());
    }

    #[test]
    fn big_file_with_no_recent_activity_is_quiet() {
        // 2500 lines but the only activity was years ago — current "recent"
        // window is empty.
        let commits = vec![touch(0, "src/old.ts", 2500, 0)];
        let later = touch(5000 * DAY, "src/other.ts", 5, 0);
        let commits = vec![commits[0].clone(), later];
        assert!(run(&["src/old.ts"], &commits, &[("src/old.ts", 2500)]).is_none());
    }

    #[test]
    fn big_file_with_pure_thrash_is_quiet() {
        // 1500 lines, 15 recent commits, but added == deleted — zero net
        // growth. churn-hotspots' territory, not ours.
        let commits: Vec<Commit> = (0..15)
            .map(|i| touch(i * DAY, "src/big.ts", 100, 100))
            .collect();
        assert!(run(&["src/big.ts"], &commits, &[("src/big.ts", 1500)]).is_none());
    }

    #[test]
    fn big_growing_file_is_warn() {
        // 1200 lines, 12 recent commits, +480 net growth.
        let commits: Vec<Commit> = (0..12)
            .map(|i| touch(i * DAY, "src/grow.ts", 60, 20))
            .collect();
        let f = run(&["src/grow.ts"], &commits, &[("src/grow.ts", 1200)]).unwrap();
        assert_eq!(f.check, "oversized-growing-files");
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn huge_growing_file_is_critical_by_size() {
        // 2500 lines, only 12 recent commits but the file size alone escalates.
        let commits: Vec<Commit> = (0..12)
            .map(|i| touch(i * DAY, "src/god.ts", 60, 20))
            .collect();
        let f = run(&["src/god.ts"], &commits, &[("src/god.ts", 2500)]).unwrap();
        assert_eq!(f.severity, Severity::Critical);
    }

    #[test]
    fn many_recent_commits_escalate_to_critical() {
        // 1100 lines, 22 recent commits — under CRIT_LINES but over CRIT_RECENT_COMMITS.
        let commits: Vec<Commit> = (0..22)
            .map(|i| touch(i * (DAY / 2), "src/hot.ts", 30, 8))
            .collect();
        let f = run(&["src/hot.ts"], &commits, &[("src/hot.ts", 1100)]).unwrap();
        assert_eq!(f.severity, Severity::Critical);
    }

    #[test]
    fn non_source_extension_is_quiet() {
        // README.md is huge and gets a lot of edits — but it's not source.
        let commits: Vec<Commit> = (0..15)
            .map(|i| touch(i * DAY, "README.md", 80, 10))
            .collect();
        assert!(run(&["README.md"], &commits, &[("README.md", 2000)]).is_none());
    }

    #[test]
    fn generated_path_is_quiet() {
        // dist/bundle.js: matches a SOURCE_EXT but is generated.
        let commits: Vec<Commit> = (0..15)
            .map(|i| touch(i * DAY, "dist/bundle.js", 80, 10))
            .collect();
        assert!(run(&["dist/bundle.js"], &commits, &[("dist/bundle.js", 5000)]).is_none());
    }

    #[test]
    fn manifest_is_quiet() {
        // package.json grows over time as deps accumulate — not a code file.
        let commits: Vec<Commit> = (0..15)
            .map(|i| touch(i * DAY, "package.json", 80, 10))
            .collect();
        assert!(run(&["package.json"], &commits, &[("package.json", 1500)]).is_none());
    }

    #[test]
    fn old_recent_activity_outside_window_is_excluded() {
        // 12 commits, all in a 14-day cluster, but the cluster is far older
        // than the latest commit in the repo.
        let mut commits: Vec<Commit> = (0..12)
            .map(|i| touch(i * DAY, "src/old-hot.ts", 60, 20))
            .collect();
        // Latest commit is 200 days after the cluster — old-hot is outside the
        // 90-day window.
        commits.push(touch(200 * DAY, "src/other.ts", 5, 0));
        assert!(run(&["src/old-hot.ts"], &commits, &[("src/old-hot.ts", 1200)]).is_none());
    }
}
