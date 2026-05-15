//! Signal: files whose names look like shell fragments committed by accident
//! — e.g. `=1.0.0` from `pip install pkg =1.0.0`, or `nul`.

use crate::audit::util::{basename, is_generated_or_fixture};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    accidental_files(&ctx.tracked).into_iter().collect()
}

fn accidental_files(tracked: &[String]) -> Option<Finding> {
    let mut hits = Vec::new();
    for path in tracked {
        // Test fixtures deliberately use weird-looking names — django's
        // `tests/migrations/test_migrations_private/~util.py` is a real
        // example. Surfaced by the django known-good control.
        if is_generated_or_fixture(path) {
            continue;
        }
        let name = basename(path);
        let looks_accidental = name.starts_with('=')
            || name.starts_with('~')
            || name == "nul"
            || name == "con"
            || name.eq_ignore_ascii_case("aux")
            || name.contains('|')
            || name.contains(';')
            || name.contains('&')
            || name.contains('>')
            || name.contains('<')
            || name.starts_with('"')
            || name.starts_with('\'')
            || name.starts_with("-- ")
            || name.trim() != name;
        if looks_accidental {
            hits.push(path.clone());
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort();
    Some(Finding::new(
        Category::VcsHygiene,
        "accidental-files",
        Severity::Warn,
        "Files whose names look like shell fragments were committed by accident",
        hits,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::files;

    #[test]
    fn flags_shell_fragments() {
        for name in [
            "=1.0.0", // pip install pkg =1.0.0
            "nul",    // windows redirect
            "con",
            "AUX",
            "a;rm",
            "x&y",
            "a|b",
            "out>file",
            "in<file",
            "\"quoted",
            "'quoted",
            "-- comment",
            " leadingspace",
            "trailingspace ",
        ] {
            assert!(
                accidental_files(&files(&[name])).is_some(),
                "{name:?} should be flagged accidental"
            );
        }
    }

    #[test]
    fn ignores_normal_names() {
        let clean = &["main.rs", "README.md", "my-file_2.txt", ".gitignore", "a.b.c"];
        assert!(accidental_files(&files(clean)).is_none());
    }

    #[test]
    fn test_fixture_weird_names_are_skipped() {
        // Django's `tests/migrations/.../~util.py` is a deliberate fixture,
        // not an accidentally committed shell fragment.
        assert!(accidental_files(&files(&[
            "tests/migrations/test_migrations_private/~util.py",
        ]))
        .is_none());
    }

    #[test]
    fn uses_basename_not_full_path() {
        // a directory component is allowed to contain '-'; only basename matters
        assert!(accidental_files(&files(&["my-dir/main.rs"])).is_none());
    }
}
