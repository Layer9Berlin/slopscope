//! Signal: tests that don't assert anything.
//!
//! A file under `tests/` with `describe`/`it`/`test`/`def test_*` but no
//! `expect` / `assert` / `should` keyword is a test that only proves the
//! code under test *doesn't throw*. The agent wrote tests until the runner
//! reported green; the green is hollow.
//!
//! Per-file decision (each test file is independently classified), repo-wide
//! threshold on how many qualify.

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Below this many assertion-free test files, the signal is silent — one or
/// two could be smoke tests or scaffolding.
const REPO_MIN: usize = 3;
/// At or above this ratio of test-files-without-assertions to total test
/// files, escalate to Critical.
const CRIT_RATIO: f64 = 0.30;
/// Cap evidence list.
const EVIDENCE_CAP: usize = 30;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    assertion_free_tests(&ctx.source_files).into_iter().collect()
}

fn assertion_free_tests(files: &[SourceFile]) -> Option<Finding> {
    let mut empty: Vec<String> = Vec::new();
    let mut total_tests = 0usize;

    for file in files {
        if !is_test_file(&file.path) {
            continue;
        }
        // Heuristic: must look like a *test file* — has at least one
        // recognizable test-runner-introduced block. A `.test.ts` file with
        // no `describe`/`it`/`test` is probably a test helper, not a test.
        if !has_test_block(file) {
            continue;
        }
        total_tests += 1;
        if !has_assertion(file) {
            empty.push(file.path.clone());
        }
    }

    if empty.len() < REPO_MIN {
        return None;
    }
    let ratio = empty.len() as f64 / total_tests.max(1) as f64;
    let severity = if ratio >= CRIT_RATIO {
        Severity::Critical
    } else {
        Severity::Warn
    };
    empty.sort();
    let shown: Vec<String> = empty.iter().take(EVIDENCE_CAP).cloned().collect();
    Some(Finding::new(
        Category::Verification,
        "assertion-free-tests",
        severity,
        format!(
            "{} test file(s) of {total_tests} contain no assertion — \
             they only prove the code under test doesn't throw",
            empty.len()
        ),
        shown,
    ))
}

fn is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    const DIRS: &[&str] = &[
        "tests/",
        "test/",
        "__tests__/",
        "spec/",
        "specs/",
        "e2e/",
        "t/",
    ];
    if DIRS
        .iter()
        .any(|d| lower.starts_with(d) || lower.contains(&format!("/{d}")))
    {
        return true;
    }
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    name.contains(".test.")
        || name.contains(".spec.")
        || name.starts_with("test_")
        || name.ends_with("_test.go")
        || name.ends_with("_test.py")
        || name.ends_with("_test.rs")
}

/// True if the file declares at least one test — `describe`/`it`/`test` for
/// ECMAScript, `def test_*` for Python, `func Test*` for Go, `#[test]` for
/// Rust, etc. Catches the most common runners.
fn has_test_block(file: &SourceFile) -> bool {
    let content = &file.content;
    match file.language {
        l if l.is_ecmascript() => {
            has_call(content, "describe")
                || has_call(content, "it")
                || has_call(content, "test")
        }
        Language::Python => content.contains("def test_") || content.contains("class Test"),
        Language::Rust => content.contains("#[test]") || content.contains("#[tokio::test]"),
        Language::Go => content.contains("func Test")
            || content.contains("func Benchmark")
            || content.contains("func Example"),
        Language::Java | Language::Kotlin | Language::Scala => {
            content.contains("@Test") || content.contains("@ParameterizedTest")
        }
        Language::Ruby => content.contains("describe ") || content.contains("def test_"),
        Language::Php => content.contains("public function test") || content.contains("@test"),
        Language::Csharp => content.contains("[Test]") || content.contains("[Fact]") || content.contains("[Theory]"),
        Language::Swift => content.contains("func test"),
        _ => false,
    }
}

/// True if the file contains a recognizable assertion call.
fn has_assertion(file: &SourceFile) -> bool {
    let content = &file.content;
    match file.language {
        l if l.is_ecmascript() => {
            // expect(…), assert(…), assert.X(…), should*, chai, sinon spies
            // with .calledWith style. A line containing "expect" is the
            // overwhelming common case.
            content.contains("expect(")
                || has_call(content, "assert")
                || content.contains("assert.")
                || content.contains(".should.")
                || content.contains(".to.equal")
                || content.contains(".toBe(")
                || content.contains(".toEqual(")
                || content.contains(".toMatch")
                || content.contains(".toHave")
                || content.contains(".toThrow")
                || content.contains(".toContain")
        }
        Language::Python => {
            // `assert …`, `self.assert*`, `pytest.raises`, `np.testing.assert_*`.
            content.contains("assert ")
                || content.contains("self.assert")
                || content.contains("self.fail(")
                || content.contains("pytest.raises")
                || content.contains("pytest.warns")
                || content.contains("np.testing.assert")
        }
        Language::Rust => {
            content.contains("assert!")
                || content.contains("assert_eq!")
                || content.contains("assert_ne!")
                || content.contains("debug_assert!")
                || content.contains(".unwrap()")
                || content.contains(".expect(")
                || content.contains("matches!(")
        }
        Language::Go => {
            // testing.T.Error / Fatal / Helper, plus testify family.
            content.contains("t.Error")
                || content.contains("t.Fatal")
                || content.contains("t.Fail")
                || content.contains("assert.")
                || content.contains("require.")
                || content.contains(".Equal(")
        }
        Language::Java | Language::Kotlin | Language::Scala => {
            content.contains("assertEquals")
                || content.contains("assertThat")
                || content.contains("assertTrue")
                || content.contains("assertFalse")
                || content.contains("assertNull")
                || content.contains("assertNotNull")
                || content.contains("assertThrows")
                || content.contains("Assertions.")
                || content.contains("fail(")
        }
        Language::Ruby => {
            content.contains("expect(")
                || content.contains("assert_")
                || content.contains(".should ")
                || content.contains(".to eq")
                || content.contains(".to_not")
        }
        Language::Php => {
            content.contains("$this->assert")
                || content.contains("self::assert")
                || content.contains("static::assert")
        }
        Language::Csharp => {
            content.contains("Assert.")
                || content.contains(".Should().")
                || content.contains("StringAssert.")
                || content.contains("CollectionAssert.")
        }
        Language::Swift => {
            content.contains("XCTAssert")
                || content.contains("XCTFail")
                || content.contains("#expect(")
        }
        _ => true, // unknown language → don't accuse it of missing assertions
    }
}

/// Word-boundary check for an identifier followed by `(`. Avoids matching
/// `submit(` as `it(`.
fn has_call(haystack: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    let bytes = haystack.as_bytes();
    let n = needle.as_bytes();
    let mut i = 0;
    while i + n.len() <= bytes.len() {
        if bytes[i..i + n.len()] == *n {
            let left_ok = i == 0
                || (!bytes[i - 1].is_ascii_alphanumeric()
                    && bytes[i - 1] != b'_'
                    && bytes[i - 1] != b'.');
            if left_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::audit::source::test_helpers::mk as f;

    #[test]
    fn detects_tests_without_expect() {
        let empty_test = "describe('thing', () => { it('does it', () => { doIt(); }); });\n";
        let files = vec![
            f("src/a.test.ts", Language::Ts, empty_test),
            f("src/b.test.ts", Language::Ts, empty_test),
            f("src/c.test.ts", Language::Ts, empty_test),
        ];
        let finding = assertion_free_tests(&files).expect("expected a finding");
        assert_eq!(finding.check, "assertion-free-tests");
        assert_eq!(finding.count, 3);
    }

    #[test]
    fn tests_with_expect_pass() {
        let good_test =
            "describe('thing', () => { it('does it', () => { expect(doIt()).toBe(true); }); });\n";
        let files = vec![
            f("src/a.test.ts", Language::Ts, good_test),
            f("src/b.test.ts", Language::Ts, good_test),
            f("src/c.test.ts", Language::Ts, good_test),
        ];
        assert!(assertion_free_tests(&files).is_none());
    }

    #[test]
    fn python_assert_keyword_counts() {
        let good = "def test_x():\n    assert 1 == 1\n";
        let bad = "def test_y():\n    do_something()\n";
        let files = vec![
            f("tests/test_a.py", Language::Python, good),
            f("tests/test_b.py", Language::Python, bad),
            f("tests/test_c.py", Language::Python, bad),
            f("tests/test_d.py", Language::Python, bad),
        ];
        let finding = assertion_free_tests(&files).expect("expected a finding");
        assert_eq!(finding.count, 3);
    }

    #[test]
    fn go_t_error_counts_as_assertion() {
        let good = "func TestX(t *testing.T) { if x != 1 { t.Errorf(\"bad\") } }\n";
        let bad = "func TestY(t *testing.T) { doIt() }\n";
        let files = vec![
            f("a_test.go", Language::Go, good),
            f("b_test.go", Language::Go, bad),
            f("c_test.go", Language::Go, bad),
            f("d_test.go", Language::Go, bad),
        ];
        let finding = assertion_free_tests(&files).expect("expected a finding");
        assert_eq!(finding.count, 3);
    }

    #[test]
    fn rust_assert_macros_count() {
        let good = "#[test]\nfn x() { assert_eq!(1, 1); }\n";
        let bad = "#[test]\nfn y() { do_it(); }\n";
        let files = vec![
            f("src/lib.rs", Language::Rust, good),
            f("tests/a.rs", Language::Rust, bad),
            f("tests/b.rs", Language::Rust, bad),
            f("tests/c.rs", Language::Rust, bad),
        ];
        let finding = assertion_free_tests(&files).expect("expected a finding");
        assert!(finding.count >= 3);
    }

    #[test]
    fn non_test_files_dont_count() {
        let bad = "function thing() { return 1; }\n";
        let files = vec![
            f("src/a.ts", Language::Ts, bad),
            f("src/b.ts", Language::Ts, bad),
            f("src/c.ts", Language::Ts, bad),
        ];
        assert!(assertion_free_tests(&files).is_none());
    }

    #[test]
    fn test_file_without_test_blocks_is_skipped() {
        // A `.test.ts` file that's actually just a helper — no it/describe.
        let helper = "export function mkUser() { return { id: 1 }; }\n";
        let files = vec![
            f("src/util.test.ts", Language::Ts, helper),
            f("src/other.test.ts", Language::Ts, helper),
            f("src/again.test.ts", Language::Ts, helper),
        ];
        assert!(assertion_free_tests(&files).is_none());
    }

    #[test]
    fn ratio_drives_critical_severity() {
        let bad = "describe('x', () => { it('does', () => { go(); }); });\n";
        // 3 of 4 test files are empty → ratio 0.75, well above CRIT_RATIO.
        let good = "describe('x', () => { it('does', () => { expect(1).toBe(1); }); });\n";
        let files = vec![
            f("src/a.test.ts", Language::Ts, bad),
            f("src/b.test.ts", Language::Ts, bad),
            f("src/c.test.ts", Language::Ts, bad),
            f("src/d.test.ts", Language::Ts, good),
        ];
        let finding = assertion_free_tests(&files).expect("expected a finding");
        assert_eq!(finding.severity, Severity::Critical);
    }
}
