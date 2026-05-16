//! Signal: files that *still exist* churned heavily in a concentrated burst
//! without the churn going anywhere.
//!
//! Content, not commit text. Concentrated churn alone isn't slop — a healthy
//! sprint also concentrates churn. The tell is **wasted** churn: a productive
//! sprint adds ~1000 lines for ~900 net; a thrash loop adds 1000 and deletes
//! 950, so gross churn is huge but *net* change is near zero. We measure the
//! densest 14-day window per file and flag bursts where gross churn dwarfs net.
//!
//! Guards that keep this honest against well-run repos:
//! - Only files that still exist are considered. A deleted/moved file has
//!   net ≈ 0 by construction (everything added was eventually removed) — that
//!   is an artifact of deletion, not thrash.
//! - Generated / vendored / fixture / CI paths are skipped.
//! - Package / build manifests are skipped — dependency churn is +1/-1 by
//!   nature, structurally high-waste even when the repo is healthy.
//!
//! Escalation language in the burst's commit messages ("actually fix", "STILL
//! broken") is a *side-note corroborator* — it bumps severity on a file already
//! flagged for wasted churn, never fires on its own.

use crate::audit::util::{is_generated_or_fixture, is_manifest};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use crate::git::Commit;
use std::collections::{HashMap, HashSet};

/// Width of the sliding window used to find a churn burst.
const BURST_WINDOW_SECS: i64 = 14 * 24 * 3600;
/// A burst needs at least this many commits touching one file...
const MIN_BURST_TOUCHES: usize = 6;
/// ...the burst must be at least this fraction of the file's *entire* commit
/// history. This is what separates a thrash loop from an ordinary hot file in
/// a mature repo: over a 40-year history, every core file has *some* dense
/// 14-day window, but only a handful of commits out of hundreds. A vibe-coded
/// thrash file is different — its whole short life *is* the burst.
const MIN_BURST_SHARE: f64 = 0.5;
/// ...at least this much gross churn (added + deleted lines)...
const MIN_BURST_GROSS: u32 = 300;
/// ...and gross churn at least this many times the net line change. A ratio of
/// 12 means ~92% of the line edits cancelled out. Hot files under healthy
/// active development sit around 5-8x; 12x+ on a *living* file is the signal.
const MIN_WASTE_RATIO: u32 = 12;
/// Either of these escalates a hotspot to Critical on its own. Tuned against
/// the known-good corpus: a *new* file under intense legitimate iteration
/// (React's experimental view-transitions file hit ~40x over 12 days) must
/// stay at Warn, while real agent thrash sits far higher — spy-search and
/// liquid-ai files in the slop corpus run 200x–1000x.
const CRIT_WASTE_RATIO: u32 = 60;
const CRIT_BURST_GROSS: u32 = 3000;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let tracked: HashSet<&str> = ctx.tracked.iter().map(String::as_str).collect();
    churn_hotspots(&ctx.commits, &tracked).into_iter().collect()
}

/// One commit's contribution to a single file.
struct Touch {
    ts: i64,
    added: u32,
    deleted: u32,
    subject: String,
}

/// A flagged file and the stats of its worst burst.
struct Hotspot {
    path: String,
    touches: usize,
    gross: u32,
    net: u32,
    waste: u32,
    span_days: i64,
    escalation: bool,
    severity: Severity,
}

fn churn_hotspots(commits: &[Commit], tracked: &HashSet<&str>) -> Option<Finding> {
    // Gather every hand-authored, non-binary touch to a still-existing file.
    let mut by_file: HashMap<&str, Vec<Touch>> = HashMap::new();
    for commit in commits {
        for delta in &commit.files {
            // Manifests (package.json, Cargo.toml, …) churn high-waste by
            // nature — every dependency bump is a +1/-1 line edit — so they
            // read as "thrashed" even in healthy repos.
            if delta.binary
                || is_generated_or_fixture(&delta.path)
                || is_manifest(&delta.path)
                || !tracked.contains(delta.path.as_str())
            {
                continue;
            }
            by_file.entry(&delta.path).or_default().push(Touch {
                ts: commit.timestamp,
                added: delta.added,
                deleted: delta.deleted,
                subject: commit.subject.clone(),
            });
        }
    }

    let mut hotspots: Vec<Hotspot> = by_file
        .into_iter()
        .filter_map(|(path, mut touches)| {
            touches.sort_by_key(|t| t.ts);
            let total_touches = touches.len();
            let burst = densest_window(&touches)?;

            let gross = burst.added + burst.deleted;
            let net = burst.added.abs_diff(burst.deleted);
            let waste = gross / net.max(1);
            let burst_share = burst.touches as f64 / total_touches as f64;
            if burst.touches < MIN_BURST_TOUCHES
                || burst_share < MIN_BURST_SHARE
                || gross < MIN_BURST_GROSS
                || waste < MIN_WASTE_RATIO
            {
                return None;
            }

            let escalation = touches[burst.start..=burst.end]
                .iter()
                .any(|t| has_escalation_language(&t.subject));
            let mut severity = if waste >= CRIT_WASTE_RATIO || gross >= CRIT_BURST_GROSS {
                Severity::Critical
            } else {
                Severity::Warn
            };
            if escalation {
                severity = bump(severity);
            }

            Some(Hotspot {
                path: path.to_string(),
                touches: burst.touches,
                gross,
                net,
                waste,
                span_days: (touches[burst.end].ts - touches[burst.start].ts) / 86_400,
                escalation,
                severity,
            })
        })
        .collect();

    if hotspots.is_empty() {
        return None;
    }
    hotspots.sort_by(|a, b| a.path.cmp(&b.path));

    let severity = hotspots.iter().map(|h| h.severity).max().unwrap();
    let any_escalation = hotspots.iter().any(|h| h.escalation);
    let evidence: Vec<String> = hotspots
        .iter()
        .map(|h| {
            let esc = if h.escalation {
                " [escalation language in burst]"
            } else {
                ""
            };
            format!(
                "{}: {} commits, {} lines changed for {} net ({}x churn) within {} day(s){}",
                h.path, h.touches, h.gross, h.net, h.waste, h.span_days, esc
            )
        })
        .collect();

    let mut summary = format!(
        "{} file(s) churned heavily without the changes landing — effort an agent couldn't resolve",
        hotspots.len()
    );
    if any_escalation {
        summary.push_str("; burst commit messages show escalation language");
    }

    Some(Finding::new(
        Category::SteeringFailure,
        "churn-hotspots",
        severity,
        summary,
        evidence,
    ))
}

struct Window {
    touches: usize,
    added: u32,
    deleted: u32,
    start: usize,
    end: usize,
}

/// The densest `BURST_WINDOW_SECS` window over timestamp-sorted touches,
/// chosen by touch count (ties keep the earliest window).
fn densest_window(touches: &[Touch]) -> Option<Window> {
    if touches.is_empty() {
        return None;
    }
    let mut lo = 0;
    let mut best: Option<Window> = None;
    for hi in 0..touches.len() {
        while touches[hi].ts - touches[lo].ts > BURST_WINDOW_SECS {
            lo += 1;
        }
        let count = hi - lo + 1;
        if best.as_ref().is_none_or(|b| count > b.touches) {
            best = Some(Window {
                touches: count,
                added: touches[lo..=hi].iter().map(|t| t.added).sum(),
                deleted: touches[lo..=hi].iter().map(|t| t.deleted).sum(),
                start: lo,
                end: hi,
            });
        }
    }
    best
}

/// Frustration tells in a commit subject. Tight on purpose — only unambiguous
/// "I'm stuck" phrasing, since this only ever *corroborates* a churn finding.
fn has_escalation_language(subject: &str) -> bool {
    let s = subject.to_ascii_lowercase();
    s.contains("actually fix")
        || s.contains("really fix")
        || s.contains("final fix")
        || s.contains("fix again")
        || s.contains("still broken")
        || s.contains("still not")
        || s.contains("for real")
        || s.contains("this time")
        || s.contains("one more")
        || s.contains("please work")
}

fn bump(s: Severity) -> Severity {
    match s {
        Severity::Info => Severity::Warn,
        Severity::Warn => Severity::Critical,
        Severity::Critical => Severity::Critical,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileDelta;

    const DAY: i64 = 86_400;

    /// Build a commit touching one file with given add/delete churn at a time.
    fn commit(ts: i64, subject: &str, path: &str, added: u32, deleted: u32) -> Commit {
        Commit {
            hash: format!("{ts:08x}"),
            timestamp: ts,
            author: "dev@example.com".to_string(),
            subject: subject.to_string(),
            files: vec![FileDelta {
                path: path.to_string(),
                added,
                deleted,
                binary: false,
            }],
        }
    }

    /// Run the signal treating every file touched in `commits` as still tracked.
    fn run(commits: &[Commit]) -> Option<Finding> {
        let tracked: HashSet<&str> = commits
            .iter()
            .flat_map(|c| c.files.iter().map(|f| f.path.as_str()))
            .collect();
        churn_hotspots(commits, &tracked)
    }

    fn one(f: Option<Finding>) -> Finding {
        f.expect("expected a finding")
    }

    #[test]
    fn quiet_when_no_burst() {
        // Only 3 touches — below MIN_BURST_TOUCHES.
        let commits: Vec<Commit> = (0..3)
            .map(|i| commit(i * DAY, "edit", "a.rs", 200, 200))
            .collect();
        assert!(run(&commits).is_none());
    }

    #[test]
    fn quiet_when_churn_too_small() {
        // 8 touches but tiny edits each — below MIN_BURST_GROSS.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "tweak", "a.rs", 2, 1))
            .collect();
        assert!(run(&commits).is_none());
    }

    #[test]
    fn quiet_for_a_productive_sprint() {
        // 8 heavy commits, but the churn LANDS: mostly additions, big net gain.
        // This is the false-positive that killed v1 — a healthy feature sprint.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "build feature", "feature.rs", 300, 20))
            .collect();
        assert!(run(&commits).is_none());
    }

    #[test]
    fn quiet_for_moderate_hot_file_iteration() {
        // 8 commits, gross 1840, net ~720 -> ~2.5x. Normal hot-file churn.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "iterate", "hot.rs", 160, 70))
            .collect();
        assert!(run(&commits).is_none());
    }

    #[test]
    fn quiet_when_churn_is_spread_over_time() {
        // 8 wasteful touches, but one per month — never 6 in a 14-day window.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * 30 * DAY, "rework", "core.rs", 100, 100))
            .collect();
        assert!(run(&commits).is_none());
    }

    #[test]
    fn deleted_files_are_ignored() {
        // A heavily-thrashed file that no longer exists: net is ~0 only because
        // everything added was eventually deleted. Not thrash — an artifact.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "edit", "gone.rs", 300, 300))
            .collect();
        let empty: HashSet<&str> = HashSet::new();
        assert!(churn_hotspots(&commits, &empty).is_none());
    }

    #[test]
    fn mature_hot_file_with_a_brief_burst_is_quiet() {
        // A core file in a long-lived repo: 8 wasteful commits in one 14-day
        // window, but 100 ordinary commits across years around them. The burst
        // is a blip in the file's life, not its whole story — not a thrash loop.
        let mut commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "edit", "core.rs", 120, 110))
            .collect();
        commits.extend(
            (0..100).map(|i| commit((100 + i * 30) * DAY, "normal work", "core.rs", 20, 5)),
        );
        assert!(run(&commits).is_none());
    }

    #[test]
    fn flags_wasteful_concentrated_burst() {
        // 8 commits in a week, churn cancels out — gross huge, net ~zero.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "edit auth", "auth.ts", 120, 110))
            .collect();
        let f = one(run(&commits));
        assert_eq!(f.check, "churn-hotspots");
        assert_eq!(f.count, 1);
        assert!(f.evidence[0].starts_with("auth.ts: 8 commits"));
    }

    #[test]
    fn very_wasteful_burst_is_critical() {
        // Gross churn dwarfs net by far more than CRIT_WASTE_RATIO.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "rewrite", "big.rs", 400, 400))
            .collect();
        assert_eq!(one(run(&commits)).severity, Severity::Critical);
    }

    #[test]
    fn escalation_language_bumps_warn_to_critical() {
        // Tuned to land at Warn: each commit +64 -56 over 6 commits ->
        // gross 720, net 48, waste 15 (in [MIN, CRIT)).
        let subjects = [
            "fix auth",
            "fix auth",
            "actually fix auth",
            "fix auth",
            "still broken",
            "fix auth for real this time",
        ];
        let commits: Vec<Commit> = subjects
            .iter()
            .enumerate()
            .map(|(i, s)| commit(i as i64 * DAY, s, "auth.ts", 64, 56))
            .collect();
        let f = one(run(&commits));
        assert_eq!(f.severity, Severity::Critical);
        assert!(f.summary.contains("escalation language"));
        assert!(f.evidence[0].contains("[escalation language in burst]"));
    }

    #[test]
    fn generated_and_fixture_files_are_ignored() {
        // Heavy wasteful churn, but on a lockfile and a test fixture.
        let mut commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "deps", "Cargo.lock", 200, 200))
            .collect();
        commits.extend(
            (0..8).map(|i| commit(i * DAY, "fixture", "tests/data/big.json", 200, 200)),
        );
        assert!(run(&commits).is_none());
    }

    #[test]
    fn manifest_files_are_ignored() {
        // Heavy wasteful churn on package.json — every dep bump is +1/-1, so
        // this is structurally high-waste even in a perfectly healthy repo.
        let commits: Vec<Commit> = (0..8)
            .map(|i| commit(i * DAY, "bump deps", "packages/blitz/package.json", 200, 200))
            .collect();
        assert!(run(&commits).is_none());
    }

    #[test]
    fn binary_files_are_ignored() {
        let commits: Vec<Commit> = (0..8)
            .map(|i| {
                let mut c = commit(i * DAY, "asset", "logo.png", 0, 0);
                c.files[0].binary = true;
                c
            })
            .collect();
        assert!(run(&commits).is_none());
    }

    #[test]
    fn densest_window_picks_the_tightest_cluster() {
        // Two early touches, then a tight cluster of 6 a year later.
        let mut touches: Vec<Touch> = vec![
            Touch { ts: 0, added: 10, deleted: 0, subject: String::new() },
            Touch { ts: 5 * DAY, added: 10, deleted: 0, subject: String::new() },
        ];
        for i in 0..6 {
            touches.push(Touch {
                ts: 400 * DAY + i * DAY,
                added: 60,
                deleted: 40,
                subject: String::new(),
            });
        }
        let w = densest_window(&touches).unwrap();
        assert_eq!(w.touches, 6);
        assert_eq!(w.added, 360);
        assert_eq!(w.deleted, 240);
    }
}
