//! Signal: AI-narrator comments — agent dictation leaking into source.
//!
//!     // Now let's add the error handling
//!     // Here we'll initialize the connection
//!     // As you can see, this needs to be cached
//!     // Step 1: parse the input
//!
//! These phrases are *tutorial register* — the agent narrating its own work
//! to itself. They don't belong in a codebase: code comments document the
//! *why*, not the *what's next*. A handful are easy to ignore; a pile is a
//! signature.
//!
//! The phrase set is deliberately narrow — we'd rather miss a borderline
//! case than flag `// Note: this is a workaround for …` (a legit engineering
//! comment that just starts with "Note").

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Below this total the signal is silent. Narrator-style prose appears at
/// some baseline rate in heavily commented OSS codebases (React's reconciler
/// has ~16 instances of "We're going to …" as legit explanatory comments),
/// so we need a real pile to call it.
const REPO_MIN: usize = 8;
const CRIT_TOTAL: usize = 40;
const EVIDENCE_CAP: usize = 25;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    narrator_comments(&ctx.source_files).into_iter().collect()
}

fn narrator_comments(files: &[SourceFile]) -> Option<Finding> {
    let mut hits: Vec<(String, usize, String)> = Vec::new();
    for file in files {
        let marker = line_comment_marker(file.language);
        for (lineno, line) in file.lines() {
            let Some(idx) = line.find(marker) else { continue };
            // The text after the marker, trimmed.
            let body = line[idx + marker.len()..].trim_start();
            if body.is_empty() {
                continue;
            }
            if is_narrator(body) {
                hits.push((file.path.clone(), lineno, trim_to(line, 100)));
            }
        }
    }
    if hits.len() < REPO_MIN {
        return None;
    }
    let total = hits.len();
    let severity = if total >= CRIT_TOTAL {
        Severity::Critical
    } else {
        Severity::Warn
    };
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let by_file = hits
        .iter()
        .map(|(p, _, _)| p)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let shown: Vec<String> = hits
        .into_iter()
        .take(EVIDENCE_CAP)
        .map(|(p, l, s)| format!("{p}:{l}: {s}"))
        .collect();
    Some(Finding::new(
        Category::CodeSmell,
        "narrator-comments",
        severity,
        format!(
            "{total} narrator-style comment(s) across {by_file} file(s) — \
             AI dictation register left in source ('Now let's…', 'Here we'll…', \
             'Step 1:…')"
        ),
        shown,
    ))
}

fn line_comment_marker(lang: Language) -> &'static str {
    match lang {
        Language::Python | Language::Ruby | Language::Shell => "#",
        _ => "//",
    }
}

/// Case-insensitive prefix match against the narrator phrase set. The
/// phrases are anchored at the start of the comment body — a comment whose
/// *middle* contains "let's" is almost always something else (a quoted
/// English fragment in a regex, a TODO note, a doc URL).
///
/// The phrase set is deliberately narrow. Earlier, broader entries
/// (`here, we…`, `first, we…`, `then, we…`, `let's …`, `step 1:`, bare
/// `we'll …` / `i'll …`) turned out to be standard engineering-comment
/// English (Django alone has ~12 of these in legacy code, all legit
/// explanations of multi-step pipelines). We keep only the phrases that
/// are *clearly* AI tutorial register — second-person address, first-person
/// future "I'll add"-style narration, or paired-action phrases that real
/// reviewers don't write.
fn is_narrator(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    const PREFIXES: &[&str] = &[
        // Tutorial register — the agent talking to itself / the user.
        "now let's",
        "now let us",
        "let's now",
        "let me ",
        "as you can see",
        "as we can see",
        "as i mentioned",
        // Future-tense agent narration with explicit action verbs.
        // Bare "we're going to" / "i'm going to" turned out to surface 16
        // legit explanatory comments in React; we keep only the
        // action-verb-paired forms, which engineering prose almost never uses.
        "i'll add",
        "i'll implement",
        "i'll create",
        "i'll build",
        "i'll start by",
        "i'll handle",
        "i'm going to add",
        "i'm going to implement",
        "we're going to add",
        "we're going to implement",
        // "Here we'll …" — the AI's "here is what I'll do next" tic.
        // `here we'll` / `here we will` (no comma) are AI-flavoured;
        // `here, we …` / `here we are …` are normal engineering prose.
        "here we'll",
        "here we will",
        // "This is where/how we'll …" — same explainer register.
        "this is where we'll",
        "this is how we'll",
    ];
    PREFIXES.iter().any(|p| lower.starts_with(p))
}

fn trim_to(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max - 1;
        while !s.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::audit::source::test_helpers::mk as f;

    #[test]
    fn classic_narrator_phrases() {
        let big = "// Now let's add the error handling\n\
                   // Here we'll initialize the database\n\
                   // As you can see, this is cached\n\
                   // Let me explain this\n\
                   // I'll add a fallback here\n\
                   // I'm going to implement the retry loop\n\
                   // We're going to add a backoff strategy\n\
                   // This is where we'll handle errors\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        let finding = narrator_comments(&files).expect("expected a finding");
        assert_eq!(finding.check, "narrator-comments");
        // All 8 lines matched.
        assert!(finding.summary.starts_with("8 narrator"));
    }

    #[test]
    fn python_hash_comments_caught() {
        let big = "# Now let's parse the JSON\n\
                   # Here we will set up logging\n\
                   # As you can see we cache responses\n\
                   # Let me show the request flow\n\
                   # I'll implement the retry loop next\n\
                   # I'll add the cache layer\n\
                   # I'll create the worker pool\n\
                   # I'll build the response shape\n";
        let files = vec![f("a.py", Language::Python, big)];
        assert!(narrator_comments(&files).is_some());
    }

    #[test]
    fn react_style_explanatory_prose_not_flagged() {
        // React's reconciler has ~16 comments like "We're going to find the
        // first row …" — legit engineering explanations of multi-step
        // pipelines. With our narrower prefix set (and a higher REPO_MIN)
        // they don't trip.
        let big = "// We're going to find the first row that has content.\n\
                   // We're going to render them separately in reverse order.\n\
                   // We're going to set the pending form status here.\n\
                   // We're going to delete it soon.\n\
                   // We're going to search forward into the tree.\n\
                   // We're going to cheat and intentionally not bind.\n\
                   // I'm going to walk the tree once and collect.\n\
                   // I'm going to skip the empty branches.\n\
                   // Let's see if the field is part of the parent chain.\n\
                   // First, we collect all the declared filters.\n";
        let files = vec![f("src/reconciler.js", Language::Js, big)];
        assert!(narrator_comments(&files).is_none());
    }

    #[test]
    fn engineering_comments_are_not_narrator() {
        // Real comments that mention surface words but aren't dictation.
        // These all surfaced as false positives on Django before we
        // narrowed the phrase set.
        let big = "// Note: this is a workaround for issue #421\n\
                   // FIXME: handle the case where x is null\n\
                   // The cache TTL must match the upstream Cache-Control header\n\
                   // Reverts the change from 2024-03-01 — see incident notes\n\
                   // Inspired by https://example.com/post (let's-encrypt rotation algo)\n\
                   // see also: docs/architecture.md\n\
                   // First, we collect all the declared list filters.\n\
                   // Then, we let every filter modify the queryset.\n\
                   // Finally, we apply the remaining lookup parameters.\n\
                   // Here, we distinguish between save types.\n\
                   // we'll ignore and serve a 403.\n\
                   // Step 1: Test the If-Match precondition.\n\
                   // Let's see if the field is part of the parent chain.\n"
            .repeat(2);
        let files = vec![f("src/a.ts", Language::Ts, &big)];
        assert!(narrator_comments(&files).is_none());
    }

    #[test]
    fn middle_of_comment_doesnt_trip() {
        // The URL contains "let's-encrypt", but that's not at the start of
        // the comment body, so it must not match.
        let big = "// fixes: https://letsencrypt.org cert issue\n".repeat(10);
        let files = vec![f("src/a.ts", Language::Ts, &big)];
        assert!(narrator_comments(&files).is_none());
    }

    #[test]
    fn below_threshold_is_quiet() {
        let big = "// Now let's do x\n// Here we'll do y\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(narrator_comments(&files).is_none());
    }

    #[test]
    fn critical_at_threshold() {
        let big = "// Now let's do step\n".repeat(CRIT_TOTAL);
        let files = vec![f("src/a.ts", Language::Ts, &big)];
        let finding = narrator_comments(&files).expect("expected a finding");
        assert_eq!(finding.severity, Severity::Critical);
    }

    #[test]
    fn case_insensitive() {
        let big = "// NOW LET'S add it\n// Here We'll handle this\n\
                   // I'll Implement the parser\n// Let Me describe the flow\n\
                   // I'M going to ADD a retry\n// I'll add caching\n\
                   // I'll create the worker\n// I'll build the report\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(narrator_comments(&files).is_some());
    }
}
