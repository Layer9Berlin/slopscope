//! Small helpers shared across signal modules.

/// The final path component (git always uses `/`, even on Windows).
pub(crate) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// True if the path has no directory component — i.e. it lives at the repo root.
pub(crate) fn is_root_level(path: &str) -> bool {
    !path.contains('/')
}

/// True for paths that aren't hand-authored source: generated output, vendored
/// dependencies, lockfiles, test fixtures. Signals that reason about how code
/// *evolved* (churn, reverts) must skip these — a churned lockfile or test
/// fixture says nothing about steering.
pub(crate) fn is_generated_or_fixture(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();

    const EXCLUDED_DIRS: &[&str] = &[
        "node_modules/",
        "vendor/",
        "dist/",
        "build/",
        "out/",
        "target/",
        ".next/",
        ".nuxt/",
        ".output/",
        "coverage/",
        "__pycache__/",
        ".github/",
        "tests/",
        "test/",
        "__tests__/",
        "testdata/",
        "fixtures/",
        "__fixtures__/",
        "e2e/",
        "examples/",
    ];
    if EXCLUDED_DIRS
        .iter()
        .any(|d| lower.starts_with(d) || lower.contains(&format!("/{d}")))
    {
        return true;
    }

    const LOCKFILES: &[&str] = &[
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lockb",
        "cargo.lock",
        "composer.lock",
        "gemfile.lock",
        "poetry.lock",
        "go.sum",
    ];
    // Autotools-generated / autoconf-style files: not hand-authored. Surfaced
    // by the postgres + emacs controls — `configure` oscillates by design, m4
    // macros under `config/`/`m4/` are imported, etc.
    const AUTOTOOLS_NAMES: &[&str] = &[
        "configure",
        "autogen.sh",
        "aclocal.m4",
        "config.guess",
        "config.sub",
        "ltmain.sh",
        "depcomp",
        "install-sh",
        "missing",
    ];
    let name = basename(&lower);
    if AUTOTOOLS_NAMES.contains(&name) {
        return true;
    }
    // .m4 macros under config/ or m4/ dirs are autoconf sources.
    if name.ends_with(".m4") && (lower.starts_with("config/") || lower.contains("/config/")
        || lower.starts_with("m4/") || lower.contains("/m4/"))
    {
        return true;
    }
    LOCKFILES.contains(&name)
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".map")
}

/// True for package / build manifests. Their churn is structurally high-waste
/// — every dependency bump is a +1/-1 line edit — so they read as "thrashed"
/// to churn analysis even in healthy repos. Signals about *code* evolution
/// should skip them.
pub(crate) fn is_manifest(path: &str) -> bool {
    const MANIFESTS: &[&str] = &[
        "package.json",
        "cargo.toml",
        "go.mod",
        "pyproject.toml",
        "setup.py",
        "requirements.txt",
        "composer.json",
        "gemfile",
        "pom.xml",
        "build.gradle",
        "pubspec.yaml",
    ];
    let name = basename(path).to_ascii_lowercase();
    MANIFESTS.contains(&name.as_str()) || name.ends_with(".csproj")
}

/// True for automated commit authors — dependabot, renovate, CI bots. Their
/// commit cadence and volume say nothing about who *steered* the repo, so
/// history-shape signals (commit bursts, author concentration) skip them.
pub(crate) fn is_bot_author(email: &str) -> bool {
    let lower = email.to_ascii_lowercase();
    lower.contains("[bot]")
        || lower.contains("dependabot")
        || lower.contains("renovate")
        || lower.contains("github-actions")
        || lower.ends_with("@users.noreply.github.com") && lower.contains("bot")
}

/// Human-readable byte size, e.g. `1.5 KB`, `19.7 MB`.
pub(crate) fn human_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = n as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// Helpers shared by signal unit tests. Compiled only under `cfg(test)`.
#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::finding::Finding;

    /// Build a tracked-file list from string literals.
    pub(crate) fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    /// Unwrap the single finding a check is expected to produce.
    pub(crate) fn one(f: Option<Finding>) -> Finding {
        f.expect("expected a finding")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_strips_directories() {
        assert_eq!(basename("a/b/c.txt"), "c.txt");
        assert_eq!(basename("c.txt"), "c.txt");
        assert_eq!(basename(""), "");
        assert_eq!(basename("a/b/"), "");
    }

    #[test]
    fn is_root_level_detects_top_level_paths() {
        assert!(is_root_level("README.md"));
        assert!(!is_root_level("src/main.rs"));
        assert!(!is_root_level("a/b"));
    }

    #[test]
    fn human_bytes_scales_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn is_generated_or_fixture_flags_non_source() {
        for p in [
            "node_modules/react/index.js",
            "packages/ui/dist/bundle.js",
            ".github/workflows/ci.yml",
            "tests/integration/big.rs",
            "src/__tests__/foo.test.ts",
            "fixtures/sample.json",
            "Cargo.lock",
            "frontend/package-lock.json",
            "vendor/lib.go",
            "app.min.js",
            "styles.min.css",
            "bundle.js.map",
            // autotools (surfaced by postgres + emacs controls)
            "configure",
            "autogen.sh",
            "aclocal.m4",
            "config.guess",
            "config/python.m4",
            "m4/ax_python.m4",
        ] {
            assert!(is_generated_or_fixture(p), "{p} should be excluded");
        }
    }

    #[test]
    fn is_generated_or_fixture_keeps_real_source() {
        for p in [
            "src/main.rs",
            "src/audit/steering_failure/churn_hotspots.rs",
            "lib/auth.ts",
            "cmd/server/main.go",
            "Cargo.toml", // manifest, not lockfile
        ] {
            assert!(!is_generated_or_fixture(p), "{p} should be kept");
        }
    }

    #[test]
    fn is_bot_author_flags_automation() {
        for e in [
            "49699333+dependabot[bot]@users.noreply.github.com",
            "dependabot[bot]@users.noreply.github.com",
            "bot@renovateapp.com",
            "41898282+github-actions[bot]@users.noreply.github.com",
        ] {
            assert!(is_bot_author(e), "{e} should be a bot");
        }
        for e in ["davidpeter@web.de", "dev@example.com", "jane@users.noreply.github.com"] {
            assert!(!is_bot_author(e), "{e} should not be a bot");
        }
    }

    #[test]
    fn is_manifest_flags_package_manifests() {
        for p in [
            "package.json",
            "frontend/package.json",
            "Cargo.toml",
            "go.mod",
            "pyproject.toml",
            "Server.csproj",
        ] {
            assert!(is_manifest(p), "{p} should be a manifest");
        }
        for p in ["src/main.rs", "package-lock.json", "config.json"] {
            assert!(!is_manifest(p), "{p} should not be a manifest");
        }
    }
}
