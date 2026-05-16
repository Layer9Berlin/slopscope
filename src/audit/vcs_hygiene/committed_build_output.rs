//! Signal: generated build output / dependencies tracked by git.

use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Directory names that hold generated output. Ecosystem-specific — when a
/// second ecosystem's worth of these accumulates, lift this into a shared
/// `ecosystems` table.
///
/// `build/` is deliberately *not* here: the known-good corpus showed it is
/// genuinely ambiguous — `scripts/build/` (prettier) and root `build/` (bat)
/// are hand-authored build *tooling*, not generated output — so it produced
/// nothing but false positives. `dist/`, `out/`, `node_modules/` etc. are
/// unambiguous.
const BUILD_DIRS: &[&str] = &[
    "node_modules/",
    "dist/",
    "out/",
    "target/",
    ".next/",
    ".nuxt/",
    ".output/",
    "coverage/",
    "__pycache__/",
];

/// Path segments that mark a deliberately-committed test fixture. A
/// `node_modules/` *under* one of these (vite's `playground/`, esbuild's
/// fixtures, anything under `tests/`) is an intentional fixture, not a
/// `.gitignore` failure.
const FIXTURE_MARKERS: &[&str] = &[
    "/tests/",
    "/test/",
    "/__tests__/",
    "/fixtures/",
    "/__fixtures__/",
    "/testdata/",
    "/e2e/",
    "/examples/",
    "/playground/",
];

fn is_test_fixture(path: &str) -> bool {
    let p = format!("/{path}");
    FIXTURE_MARKERS.iter().any(|m| p.contains(m))
}

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    committed_build_output(&ctx.tracked).into_iter().collect()
}

fn committed_build_output(tracked: &[String]) -> Option<Finding> {
    let mut hits: Vec<String> = tracked
        .iter()
        .filter(|p| {
            BUILD_DIRS
                .iter()
                .any(|d| p.starts_with(d) || p.contains(&format!("/{d}")))
                && !is_test_fixture(p)
        })
        .cloned()
        .collect();
    if hits.is_empty() {
        return None;
    }
    hits.sort();
    // node_modules being tracked is categorically worse than a stray dist file.
    let severity = if hits.iter().any(|p| p.contains("node_modules/")) {
        Severity::Critical
    } else {
        Severity::Warn
    };
    Some(Finding::new(
        Category::VcsHygiene,
        "committed-build-output",
        severity,
        "Generated build output / dependencies are tracked by git",
        hits,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    #[test]
    fn node_modules_is_critical() {
        let f = one(committed_build_output(&files(&["node_modules/react/index.js"])));
        assert_eq!(f.severity, Severity::Critical);
    }

    #[test]
    fn dist_only_is_warn() {
        let f = one(committed_build_output(&files(&["dist/bundle.js", "out/app.o"])));
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn matches_nested_dirs() {
        let f = one(committed_build_output(&files(&["packages/ui/dist/index.js"])));
        assert_eq!(f.count, 1);
    }

    #[test]
    fn does_not_match_lookalike_dirs() {
        // "distribution/" must not trip the "dist/" rule.
        assert!(committed_build_output(&files(&["distribution/notes.md"])).is_none());
    }

    #[test]
    fn build_dir_is_not_flagged() {
        // `build/` is hand-authored tooling in real repos (prettier's
        // scripts/build/, bat's root build/) — too ambiguous to flag.
        assert!(committed_build_output(&files(&[
            "scripts/build/build.js",
            "build/application.rs",
        ]))
        .is_none());
    }

    #[test]
    fn node_modules_in_a_test_fixture_is_not_flagged() {
        // Deliberately-committed fixtures: vite's playground/, anything under
        // tests/ — an intentional fixture, not a .gitignore failure.
        assert!(committed_build_output(&files(&[
            "playground/nested-deps/pkg/node_modules/dep/index.js",
            "packages/vite/src/node/__tests__/fixture/node_modules/framework/x.js",
        ]))
        .is_none());
    }

    #[test]
    fn none_when_clean() {
        assert!(committed_build_output(&files(&["src/main.rs"])).is_none());
    }
}
