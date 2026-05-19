//! Shared dependency-manifest parsers for inconsistency signals.
//!
//! Each `deps_from_*` reads a single manifest file off disk and returns the
//! subset of `known` library names it lists as a *direct* dependency. Used
//! by every "agent piled multiple libraries doing the same job" signal:
//! HTTP clients, validation libs, state libs, date libs, …
//!
//! Quiet on read / parse failure — a malformed manifest isn't this
//! category's problem.

use serde_json::Value as JsonValue;
use std::collections::BTreeSet;
use std::path::Path;

/// `package.json` — checks `dependencies` + `devDependencies` +
/// `peerDependencies`. Transitive doesn't count.
pub(crate) fn deps_from_package_json<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(json) = serde_json::from_str::<JsonValue>(&text) else {
        return BTreeSet::new();
    };
    let mut found = BTreeSet::new();
    for field in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(deps) = json.get(field).and_then(JsonValue::as_object) {
            for name in deps.keys() {
                if let Some(k) = known.iter().find(|c| **c == name) {
                    found.insert(*k);
                }
            }
        }
    }
    found
}

/// `pyproject.toml` — handles both PEP 621 (`[project] dependencies = […]`)
/// and Poetry (`[tool.poetry.dependencies] foo = "…"`).
pub(crate) fn deps_from_pyproject<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return BTreeSet::new();
    };
    let mut found = BTreeSet::new();
    if let Some(arr) = value
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|v| v.as_array())
    {
        for d in arr {
            if let Some(s) = d.as_str() {
                if let Some(k) = match_pep_dep(s, known) {
                    found.insert(k);
                }
            }
        }
    }
    if let Some(table) = value
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for name in table.keys() {
            if let Some(k) = known.iter().find(|c| **c == name) {
                found.insert(*k);
            }
        }
    }
    found
}

/// `requirements.txt` — one dep per line, version specifiers ignored.
pub(crate) fn deps_from_requirements<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let mut found = BTreeSet::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('-') {
            continue;
        }
        if let Some(k) = match_pep_dep(line, known) {
            found.insert(k);
        }
    }
    found
}

/// `Cargo.toml` — `[dependencies]`, `[dev-dependencies]`,
/// `[build-dependencies]`.
pub(crate) fn deps_from_cargo<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return BTreeSet::new();
    };
    let mut found = BTreeSet::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = value.get(section).and_then(|v| v.as_table()) {
            for name in table.keys() {
                if let Some(k) = known.iter().find(|c| **c == name) {
                    found.insert(*k);
                }
            }
        }
    }
    found
}

/// `go.mod` — `require <module> <version>` lines, plus `require ( … )`
/// blocks. Matches by *prefix* so `github.com/foo/bar/v2` collides with
/// `github.com/foo/bar`.
pub(crate) fn deps_from_gomod<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let mut found = BTreeSet::new();
    let mut in_block = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("require (") {
            in_block = true;
            continue;
        }
        if in_block && line == ")" {
            in_block = false;
            continue;
        }
        let dep_text = if let Some(rest) = line.strip_prefix("require ") {
            rest
        } else if in_block {
            line
        } else {
            continue;
        };
        let module = dep_text.split_whitespace().next().unwrap_or("");
        for k in known {
            if module.starts_with(k) {
                found.insert(*k);
            }
        }
    }
    found
}

/// `pom.xml` — non-strict XML; we substring-match for known fragments in
/// `<artifactId>…</artifactId>`. Maven artifact names vary too much for a
/// clean parser to be worth it.
pub(crate) fn deps_from_pom<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let lower = text.to_ascii_lowercase();
    let mut found = BTreeSet::new();
    for k in known {
        if lower.contains(&format!("<artifactid>{k}")) || lower.contains(&format!(">{k}<")) {
            found.insert(*k);
        }
    }
    found
}

/// `build.gradle` / `build.gradle.kts` — Gradle DSL is too varied for a
/// strict parser; we substring-match against the lowercased file.
pub(crate) fn deps_from_gradle<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let lower = text.to_ascii_lowercase();
    let mut found = BTreeSet::new();
    for k in known {
        if lower.contains(k) {
            found.insert(*k);
        }
    }
    found
}

/// Parse a PEP 508-ish dep spec (`requests>=2.31`, `httpx[http2]==0.25`)
/// and return the known name if it matches.
fn match_pep_dep<'a>(spec: &str, known: &'a [&'a str]) -> Option<&'a str> {
    let name: String = spec
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect();
    known.iter().find(|c| **c == name.as_str()).copied()
}

/// What kind of manifest a tracked file is. Drives which parser to call.
pub(crate) enum Manifest {
    PackageJson,
    Pyproject,
    Requirements,
    Cargo,
    GoMod,
    Pom,
    Gradle,
}

impl Manifest {
    /// Identify a manifest from its *basename*. Returns `None` for files
    /// that aren't a manifest.
    pub(crate) fn from_basename(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "package.json" => Some(Self::PackageJson),
            "pyproject.toml" => Some(Self::Pyproject),
            "requirements.txt" => Some(Self::Requirements),
            "cargo.toml" => Some(Self::Cargo),
            "go.mod" => Some(Self::GoMod),
            "pom.xml" => Some(Self::Pom),
            "build.gradle" | "build.gradle.kts" => Some(Self::Gradle),
            _ => None,
        }
    }

    /// Run the right parser for this manifest and return the subset of
    /// `known` it lists as a direct dependency. Each library-pile signal
    /// must hand in the per-ecosystem name list, since a `package.json`
    /// asking about validation libs is "zod" / "yup" / …, not "axios" /
    /// "ky".
    pub(crate) fn deps<'a>(
        &self,
        path: &Path,
        ecosystem_libs: &EcosystemLibs<'a>,
    ) -> BTreeSet<&'a str> {
        match self {
            Self::PackageJson => deps_from_package_json(path, ecosystem_libs.js),
            Self::Pyproject => deps_from_pyproject(path, ecosystem_libs.python),
            Self::Requirements => deps_from_requirements(path, ecosystem_libs.python),
            Self::Cargo => deps_from_cargo(path, ecosystem_libs.rust),
            Self::GoMod => deps_from_gomod(path, ecosystem_libs.go),
            Self::Pom | Self::Gradle => {
                if let Self::Pom = self {
                    deps_from_pom(path, ecosystem_libs.java)
                } else {
                    deps_from_gradle(path, ecosystem_libs.java)
                }
            }
        }
    }
}

/// Per-ecosystem name lists for one library-pile signal. Each list may be
/// empty if the signal doesn't apply to that ecosystem.
pub(crate) struct EcosystemLibs<'a> {
    pub js: &'a [&'a str],
    pub python: &'a [&'a str],
    pub rust: &'a [&'a str],
    pub go: &'a [&'a str],
    pub java: &'a [&'a str],
}

impl<'a> EcosystemLibs<'a> {
    /// Empty lists everywhere — building block for ecosystem-specific
    /// signals that fill in only one or two.
    pub(crate) const fn empty() -> Self {
        Self {
            js: &[],
            python: &[],
            rust: &[],
            go: &[],
            java: &[],
        }
    }
}

/// Specification of one "agent piled N libraries doing the same job"
/// signal. The four library-pile checks (HTTP clients, validation,
/// state, date) all share the same loop and severity logic; only the
/// known-name lists and the thing being talked about differ.
pub(crate) struct LibPile {
    /// Stable machine id of the finding, e.g. `"multiple-http-clients"`.
    pub check_id: &'static str,
    /// Per-ecosystem name lists. Any ecosystem with an empty list is
    /// skipped silently — useful for JS-only signals.
    pub libs: EcosystemLibs<'static>,
    /// Phrase for the summary — e.g. `"HTTP clients"` produces "list
    /// multiple HTTP clients as direct deps".
    pub thing_name: &'static str,
}

/// Walk every tracked manifest, find ones that list ≥2 of the libs we
/// know about, and roll them into a [`Finding`]. Returns `None` if no
/// manifest has a pile-up.
pub(crate) fn detect_pile_up(
    ctx: &crate::audit::AuditContext,
    pile: &LibPile,
) -> Option<crate::finding::Finding> {
    use crate::finding::{Category, Finding, Severity};

    struct Offender {
        path: String,
        names: Vec<String>,
    }

    let mut offenders: Vec<Offender> = Vec::new();
    for path in &ctx.tracked {
        // Vendored / fixture manifests don't reflect *this* project's
        // choices — `node_modules/<dep>/package.json` is the dep's deps,
        // not ours.
        if crate::audit::util::is_generated_or_fixture(path) {
            continue;
        }
        let name = crate::audit::util::basename(path).to_ascii_lowercase();
        let Some(manifest) = Manifest::from_basename(&name) else {
            continue;
        };
        let names = manifest.deps(&ctx.root.join(path), &pile.libs);
        if names.len() >= 2 {
            offenders.push(Offender {
                path: path.clone(),
                names: names.into_iter().map(String::from).collect(),
            });
        }
    }
    if offenders.is_empty() {
        return None;
    }
    offenders.sort_by(|a, b| b.names.len().cmp(&a.names.len()).then(a.path.cmp(&b.path)));
    let worst_count = offenders[0].names.len();
    let severity = if worst_count >= 3 {
        Severity::Critical
    } else {
        Severity::Warn
    };
    let evidence: Vec<String> = offenders
        .iter()
        .map(|o| format!("{}: {}", o.path, o.names.join(", ")))
        .collect();
    Some(Finding::new(
        Category::Inconsistency,
        pile.check_id,
        severity,
        format!(
            "{} manifest(s) list multiple {} as direct deps — \
             pick one; carrying both is an agent that didn't remember its previous answer",
            offenders.len(),
            pile.thing_name
        ),
        evidence,
    ))
}
