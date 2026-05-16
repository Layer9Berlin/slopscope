//! Signal: too few test files relative to hand-authored source.
//!
//! Not measuring code coverage — that needs runtime data slopscope deliberately
//! doesn't have. Just: did anyone write tests at all? A 500-file repo with
//! zero `*.test.*` / `tests/` files is one tell of an agent that delivered
//! features and called it done. Tiny repos are exempt — a 20-file experiment
//! routinely has no tests and that is fine.
//!
//! Source = hand-authored files in known code languages (the usual ones).
//! Tests = files matching `.test.` / `.spec.` patterns OR living under
//! `tests/`, `test/`, `__tests__/`, `spec/`. Generated / vendored / fixture
//! paths don't count either way — they're not source *or* tests.

use crate::audit::util::{basename, is_generated_or_fixture};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Below this many source files, the ratio is too noisy to mean anything.
const MIN_SOURCE: usize = 30;
/// Flag when tests/source ratio falls below this.
const MIN_RATIO: f64 = 0.05;
/// At or below this ratio the repo has *almost* no tests — Critical territory.
const CRIT_RATIO: f64 = 0.005;

/// Code-language source extensions we know how to count. Anything not in this
/// set is ignored on both sides (docs, configs, assets) so the ratio measures
/// what it claims to.
const SOURCE_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte", "astro", "py", "rs", "go", "java",
    "kt", "kts", "scala", "rb", "php", "cs", "fs", "swift", "m", "mm", "c", "cc", "cpp", "cxx",
    "h", "hpp", "elm",
];

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    test_source_ratio(&ctx.tracked).into_iter().collect()
}

/// True if a path lives under a conventional test directory.
fn under_test_dir(path: &str) -> bool {
    const TEST_DIRS: &[&str] = &[
        "tests/", "test/", "__tests__/", "spec/", "specs/", "e2e/",
        // git uses a single-letter `t/` for its testsuite; the new known-good
        // corpus surfaced that without this any C project using git's
        // convention would show as testless.
        "t/",
    ];
    TEST_DIRS
        .iter()
        .any(|d| path.starts_with(d) || path.contains(&format!("/{d}")))
}

/// True if a *basename* says "this file is a test" (`foo.test.ts`,
/// `foo.spec.js`, `test_foo.py`, `foo_test.go`).
fn looks_like_test_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.starts_with("test_")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || lower.ends_with("_test.rs")
}

fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, e)| e)
}

fn test_source_ratio(tracked: &[String]) -> Option<Finding> {
    let mut source = 0usize;
    let mut tests = 0usize;

    for path in tracked {
        let name = basename(path);
        let is_test_path = under_test_dir(path);
        let is_test_name = looks_like_test_file(name);
        let is_test = is_test_path || is_test_name;

        if is_test {
            // Tests can be in any language — git's testsuite is `t/*.sh`,
            // neovim's is `test/*.lua`. Counting tests only when they hit
            // SOURCE_EXTS makes those repos look testless. So under a test
            // dir or with a test name, the file counts as a test regardless
            // of extension.
            tests += 1;
            continue;
        }
        // Source is gated on a known code extension so docs / configs /
        // assets don't inflate the denominator.
        let Some(ext) = ext_of(name).map(str::to_ascii_lowercase) else {
            continue;
        };
        if !SOURCE_EXTS.contains(&ext.as_str()) {
            continue;
        }
        if is_generated_or_fixture(path) {
            continue;
        }
        source += 1;
    }

    if source < MIN_SOURCE {
        return None;
    }
    let ratio = tests as f64 / source as f64;
    if ratio >= MIN_RATIO {
        return None;
    }

    let severity = if ratio <= CRIT_RATIO {
        Severity::Critical
    } else {
        Severity::Warn
    };
    let summary = if tests == 0 {
        format!(
            "{source} source files, 0 test files — nothing is being verified"
        )
    } else {
        format!(
            "{source} source files, only {tests} test file(s) ({:.1}% of source) — \
             not enough to verify anything meaningful",
            ratio * 100.0,
        )
    };
    Some(Finding::new(
        Category::Verification,
        "test-source-ratio",
        severity,
        summary,
        vec![format!("source: {source}, tests: {tests}, ratio: {:.3}", ratio)],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    fn n_sources(n: usize, ext: &str) -> Vec<String> {
        (0..n).map(|i| format!("src/f{i}.{ext}")).collect()
    }

    #[test]
    fn small_repo_is_quiet() {
        // 10 source files, no tests — below MIN_SOURCE.
        assert!(test_source_ratio(&n_sources(10, "ts")).is_none());
    }

    #[test]
    fn zero_tests_in_substantial_repo_is_critical() {
        let f = one(test_source_ratio(&n_sources(60, "ts")));
        assert_eq!(f.check, "test-source-ratio");
        assert_eq!(f.severity, Severity::Critical);
        assert!(f.summary.contains("0 test"));
    }

    #[test]
    fn healthy_test_count_is_quiet() {
        let mut paths = n_sources(60, "ts");
        for i in 0..10 {
            paths.push(format!("src/f{i}.test.ts"));
        }
        assert!(test_source_ratio(&paths).is_none());
    }

    #[test]
    fn handful_of_tests_is_warn() {
        // 60 source, 2 tests -> ratio 0.033, between CRIT_RATIO and MIN_RATIO.
        let mut paths = n_sources(60, "ts");
        paths.push("src/auth.test.ts".into());
        paths.push("src/user.test.ts".into());
        let f = one(test_source_ratio(&paths));
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn counts_tests_in_conventional_dirs() {
        // tests/ directory members are tests even with no `.test.` in name.
        let mut paths = n_sources(60, "py");
        for i in 0..10 {
            paths.push(format!("tests/test_f{i}.py"));
        }
        assert!(test_source_ratio(&paths).is_none());
    }

    #[test]
    fn tests_in_t_directory_with_sh_extension() {
        // Git's testsuite: `t/t0001-init.sh` etc. — non-source extension,
        // single-letter test dir. The corpus surfaced this.
        let mut paths = n_sources(60, "c");
        for i in 0..20 {
            paths.push(format!("t/t{i:04}-some-test.sh"));
        }
        assert!(test_source_ratio(&paths).is_none());
    }

    #[test]
    fn tests_in_test_directory_with_lua_extension() {
        // Neovim's testsuite: `test/functional/api/buffer_spec.lua`.
        let mut paths = n_sources(60, "c");
        for i in 0..15 {
            paths.push(format!("test/functional/spec_{i}.lua"));
        }
        assert!(test_source_ratio(&paths).is_none());
    }

    #[test]
    fn go_underscore_test_naming() {
        let mut paths = n_sources(60, "go");
        for i in 0..10 {
            paths.push(format!("internal/f{i}_test.go"));
        }
        assert!(test_source_ratio(&paths).is_none());
    }

    #[test]
    fn ignores_non_source_files() {
        // README.md, *.json etc. don't count as source or test.
        let mut paths = vec![
            "README.md".to_string(),
            "package.json".to_string(),
            "assets/logo.svg".to_string(),
        ];
        paths.extend(n_sources(60, "ts"));
        let f = test_source_ratio(&paths);
        // The .md/.json/.svg are dropped; only the 60 .ts files count.
        assert_eq!(f.unwrap().severity, Severity::Critical);
    }

    #[test]
    fn integrates_with_files_helper() {
        assert!(test_source_ratio(&files(&["src/main.rs"])).is_none());
    }
}
