//! Signal: the agent reached for a suppression instead of fixing the cause.
//! `@ts-ignore`, `# type: ignore`, `eslint-disable`, `#[allow(...)]`,
//! `it.skip`, … — each one is a place a check fired and got silenced.
//!
//! A handful are normal (a stubborn type, a known-flaky test). A pile of them
//! concentrated in a few files is the signature of an agent making red lights
//! green. The signal reports per-file counts so the coder gets a concrete list
//! of files to revisit.

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use std::collections::BTreeMap;

/// Per-file count above which the file makes the evidence list.
const PER_FILE_MIN: usize = 1;
/// Total count below this, the signal is quiet — a few suppressions in a
/// large repo say nothing.
const REPO_MIN: usize = 25;
/// At or above this many in a single file, escalate to Critical regardless
/// of the total — one file weaponizing `@ts-nocheck` is enough. React's
/// largest suppression-pile is 21 in ReactFlightServer.js (legit
/// performance-track / Flow types), so a 25+ pile is the actual tell.
const CRIT_PER_FILE: usize = 25;
/// Cap evidence list at the worst offenders.
const EVIDENCE_CAP: usize = 30;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    suppressed_checks(&ctx.source_files).into_iter().collect()
}

fn suppressed_checks(files: &[SourceFile]) -> Option<Finding> {
    let mut per_file: BTreeMap<String, usize> = BTreeMap::new();
    let mut total: usize = 0;

    for f in files {
        let n = count_in_file(f);
        if n >= PER_FILE_MIN {
            per_file.insert(f.path.clone(), n);
            total += n;
        }
    }

    if total < REPO_MIN {
        return None;
    }

    let worst_per_file = per_file.values().copied().max().unwrap_or(0);
    let severity = if worst_per_file >= CRIT_PER_FILE {
        Severity::Critical
    } else {
        Severity::Warn
    };

    // Sort evidence by count desc, then path, so the coder sees the worst
    // offenders first.
    let mut ranked: Vec<(String, usize)> = per_file.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let shown: Vec<String> = ranked
        .iter()
        .take(EVIDENCE_CAP)
        .map(|(p, n)| format!("{p} ({n})"))
        .collect();

    Some(Finding::new(
        Category::CodeSmell,
        "suppressed-checks",
        severity,
        format!(
            "{total} suppression(s) across {} file(s) — type/lint/test checks silenced \
             instead of fixed",
            ranked.len()
        ),
        shown,
    ))
}

/// Count suppressions in one file. Patterns are matched as substrings of a
/// line — overcounts by one if a line carries both (`// @ts-ignore eslint-disable-next-line`),
/// which is fine for a "how loud is this file" metric.
fn count_in_file(f: &SourceFile) -> usize {
    let mut n = 0;
    for line in f.content.lines() {
        n += matches_for(f.language, line);
    }
    n
}

fn matches_for(lang: Language, line: &str) -> usize {
    let mut n = 0;
    // ECMAScript family: @ts-ignore / @ts-nocheck / @ts-expect-error live in
    // `.ts` only, but eslint-disable / it.skip span both.
    if lang.is_ecmascript() {
        if line.contains("@ts-ignore")
            || line.contains("@ts-nocheck")
            || line.contains("@ts-expect-error")
        {
            n += 1;
        }
        if line.contains("eslint-disable") {
            n += 1;
        }
        // Vitest / Jest / Mocha skip APIs. `xit` / `xdescribe` are the
        // BDD-style skips; `it.skip` / `test.skip` / `describe.skip` are the
        // method form. Match word-boundary-ish so `exit(` doesn't trip `xit`.
        if has_word(line, "it.skip")
            || has_word(line, "test.skip")
            || has_word(line, "describe.skip")
            || has_word(line, "xit(")
            || has_word(line, "xdescribe(")
        {
            n += 1;
        }
    }
    if lang == Language::Python {
        // `# type: ignore`, `# noqa`, `# pylint: disable=...`,
        // `# pyright: ignore`, `# fmt: off`.
        if line.contains("# type: ignore")
            || line.contains("# noqa")
            || line.contains("# pylint: disable")
            || line.contains("# pyright: ignore")
            || line.contains("# fmt: off")
        {
            n += 1;
        }
        // pytest skip decorators / markers.
        if line.contains("@pytest.mark.skip")
            || line.contains("pytest.skip(")
            || line.contains("unittest.skip")
        {
            n += 1;
        }
    }
    if lang == Language::Rust {
        // `#[allow(...)]` is sometimes legitimate, but agents reach for it
        // to silence dead-code / unused-import warnings rather than delete
        // the dead code. `#[ignore]` skips a test.
        if line.contains("#[allow(") || line.contains("#![allow(") {
            n += 1;
        }
        if has_word(line, "#[ignore]") || line.contains("#[ignore =") {
            n += 1;
        }
    }
    if lang == Language::Go {
        // `//nolint:...` is the golangci-lint suppression; `t.Skip(` is the
        // testing.T skip. `//nolint` may have a space or not.
        if line.contains("//nolint") || line.contains("// nolint") {
            n += 1;
        }
        if line.contains("t.Skip(") || line.contains("t.Skipf(") || line.contains("t.SkipNow(") {
            n += 1;
        }
    }
    if matches!(lang, Language::Java | Language::Kotlin | Language::Scala) {
        if line.contains("@SuppressWarnings") {
            n += 1;
        }
        // JUnit 4 `@Ignore`, JUnit 5 `@Disabled`, Kotlin test `@Ignore`.
        if has_word(line, "@Ignore") || has_word(line, "@Disabled") {
            n += 1;
        }
    }
    if matches!(lang, Language::Csharp) {
        // `#pragma warning disable`, `[SuppressMessage(...)]`,
        // `[Ignore]` / `[Fact(Skip = ...)]` xUnit, `[Test(Skip = ...)]`.
        if line.contains("#pragma warning disable") || line.contains("[SuppressMessage(") {
            n += 1;
        }
        if has_word(line, "[Ignore]") || line.contains("(Skip =") {
            n += 1;
        }
    }
    if matches!(lang, Language::Cpp | Language::C) {
        // Clang/GCC diagnostic ignored pragmas — agents reach for these to
        // silence warnings.
        if line.contains("#pragma GCC diagnostic ignored")
            || line.contains("#pragma clang diagnostic ignored")
            || line.contains("#pragma warning(disable")
        {
            n += 1;
        }
    }
    if lang == Language::Ruby {
        if line.contains("# rubocop:disable") {
            n += 1;
        }
        // RSpec skip variants: `xit`, `xdescribe`, `skip "...".
        if has_word(line, "xit ")
            || has_word(line, "xdescribe ")
            || has_word(line, "skip(")
            || line.trim_start().starts_with("skip ")
        {
            n += 1;
        }
    }
    if lang == Language::Php {
        if line.contains("@phpstan-ignore")
            || line.contains("@psalm-suppress")
            || line.contains("@SuppressWarnings(")
        {
            n += 1;
        }
        if line.contains("$this->markTestSkipped(") {
            n += 1;
        }
    }
    if lang == Language::Swift {
        // Swift has no first-class suppression, but `@available(*,
        // unavailable)` and `XCTSkip(` / `try XCTSkipIf` are the equivalents.
        if line.contains("XCTSkip(") || line.contains("XCTSkipIf(") || line.contains("XCTSkipUnless(") {
            n += 1;
        }
    }
    n
}

/// True if `needle` appears in `haystack` with non-alphanumeric / non-underscore
/// boundary on the left. The left side is the only one that matters for these
/// patterns — they all carry their own right-side delimiter (`(`, `.`, `]`).
fn has_word(haystack: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(i) = haystack[start..].find(needle) {
        let pos = start + i;
        let prev_ok = pos == 0
            || !haystack.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && haystack.as_bytes()[pos - 1] != b'_';
        if prev_ok {
            return true;
        }
        start = pos + needle.len();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::audit::source::test_helpers::mk as f;

    #[test]
    fn under_threshold_is_quiet() {
        // 2 suppressions, well below REPO_MIN.
        let files = vec![f(
            "src/a.ts",
            Language::Ts,
            "// @ts-ignore\nconst x = 1;\n// eslint-disable-next-line\nconst y = 2;\n",
        )];
        assert!(suppressed_checks(&files).is_none());
    }

    #[test]
    fn ts_suppressions_counted() {
        // Each line in `content` is one suppression. Repeating it three
        // times gives us 30 total hits — comfortably over REPO_MIN.
        let one_each = "// @ts-ignore\n// @ts-nocheck\n// @ts-expect-error\n\
             // eslint-disable-next-line\n// eslint-disable\n\
             it.skip('a', () => {});\ntest.skip('b', () => {});\n\
             describe.skip('c', () => {});\nxit('d', () => {});\nxdescribe('e', () => {});\n";
        let content = one_each.repeat(3); // 30 hits
        let files = vec![
            f("src/a.ts", Language::Ts, &content),
            f("src/b.ts", Language::Ts, "// @ts-ignore\n// eslint-disable\n"),
        ];
        let finding = suppressed_checks(&files).expect("expected a finding");
        assert_eq!(finding.check, "suppressed-checks");
        // Two files in evidence, worst first.
        assert!(finding.evidence[0].starts_with("src/a.ts"));
    }

    #[test]
    fn pile_in_one_file_is_critical() {
        // 25 suppressions in one file → CRIT_PER_FILE threshold tripped.
        let lines = "// @ts-ignore\n".repeat(CRIT_PER_FILE);
        let files = vec![f("src/bad.ts", Language::Ts, &lines)];
        let finding = suppressed_checks(&files).expect("expected a finding");
        assert_eq!(finding.severity, Severity::Critical);
    }

    #[test]
    fn react_scale_concentration_stays_warn() {
        // React's worst suppression-pile is 21 in one file. With our gate
        // at 25+, that stays WARN — the signal is still reported, it just
        // isn't called a fire.
        let lines = "// @ts-ignore\n".repeat(21);
        // Boost total above REPO_MIN by spreading across multiple files.
        let mut files = vec![f("src/main.ts", Language::Ts, &lines)];
        for i in 0..5 {
            files.push(f(
                &format!("src/x{i}.ts"),
                Language::Ts,
                "// @ts-ignore\n// @ts-ignore\n",
            ));
        }
        let finding = suppressed_checks(&files).expect("expected a finding");
        assert_eq!(finding.severity, Severity::Warn);
    }

    #[test]
    fn python_patterns() {
        let content = "x = 1  # type: ignore\n\
                       y = 2  # noqa: E501\n\
                       z = 3  # pylint: disable=invalid-name\n\
                       w = 4  # pyright: ignore[reportGeneralTypeIssues]\n\
                       # fmt: off\n\
                       @pytest.mark.skip(reason='flaky')\n\
                       def test_foo(): pass\n\
                       @unittest.skip('broken')\n\
                       def test_bar(): pass\n\
                       pytest.skip('not ready')\n";
        // Generate enough hits to clear REPO_MIN.
        let big = content.repeat(4);
        let files = vec![f("t/a.py", Language::Python, &big)];
        let finding = suppressed_checks(&files).expect("expected a finding");
        assert!(finding.count >= 1);
        assert!(finding.summary.contains("suppression"));
    }

    #[test]
    fn rust_allow_and_ignore() {
        let big = "#[allow(dead_code)]\n#![allow(unused)]\n#[ignore]\n#[ignore = \"flaky\"]\n"
            .repeat(7);
        let files = vec![f("src/lib.rs", Language::Rust, &big)];
        let finding = suppressed_checks(&files).expect("expected a finding");
        assert_eq!(finding.evidence.len(), 1);
    }

    #[test]
    fn go_nolint_and_t_skip() {
        let big = "//nolint:gosec\n// nolint:errcheck\nt.Skip(\"flaky\")\nt.Skipf(\"x\")\n".repeat(7);
        let files = vec![f("foo_test.go", Language::Go, &big)];
        let finding = suppressed_checks(&files).expect("expected a finding");
        assert!(finding.evidence[0].starts_with("foo_test.go"));
    }

    #[test]
    fn java_suppress_and_ignore() {
        let big = "@SuppressWarnings(\"unchecked\")\n@Ignore\n@Disabled\n".repeat(9);
        let files = vec![f("Foo.java", Language::Java, &big)];
        assert!(suppressed_checks(&files).is_some());
    }

    #[test]
    fn has_word_respects_left_boundary() {
        // `exit(` must not trip `xit`.
        assert!(!has_word("process.exit(1)", "xit("));
        assert!(has_word("xit('a', ...)", "xit("));
        assert!(has_word("  xit('a', ...)", "xit("));
        // Underscore should also be a non-boundary.
        assert!(!has_word("my_xit_helper(", "xit("));
    }

    #[test]
    fn unrelated_keywords_in_other_languages_are_ignored() {
        // A Rust file containing the literal text `@ts-ignore` shouldn't
        // count — patterns are language-scoped.
        let big = "// @ts-ignore\n".repeat(20);
        let files = vec![f("src/a.rs", Language::Rust, &big)];
        assert!(suppressed_checks(&files).is_none());
    }

    #[test]
    fn evidence_sorted_by_severity_desc() {
        // Build two files with different counts; the heavier one comes first.
        let many = "// @ts-ignore\n".repeat(20);
        let few = "// @ts-ignore\n".repeat(8);
        let files = vec![f("src/a.ts", Language::Ts, &few), f("src/b.ts", Language::Ts, &many)];
        let finding = suppressed_checks(&files).expect("expected a finding");
        assert!(finding.evidence[0].starts_with("src/b.ts"));
        assert!(finding.evidence[1].starts_with("src/a.ts"));
    }
}
