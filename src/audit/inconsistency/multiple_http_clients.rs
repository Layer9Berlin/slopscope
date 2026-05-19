//! Signal: multiple HTTP client libraries listed as direct dependencies.
//!
//! `axios` *and* `node-fetch` *and* `ky` in one `package.json`, or
//! `requests` + `httpx` in one `pyproject.toml`, or `reqwest` + `ureq` in
//! `Cargo.toml` — the fingerprint of an agent not remembering which client
//! it used last time and reaching for whichever the prompt suggested. One
//! is normal; two or more direct deps doing the same job is inconsistency
//! frozen into the dependency tree.
//!
//! Reads manifests from disk and looks at direct deps only — transitive
//! doesn't count, since you don't choose those. Each ecosystem has its own
//! manifest format and its own client set.

use crate::audit::AuditContext;
use crate::audit::util::basename;
use crate::finding::{Category, Finding, Severity};
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    multiple_http_clients(&ctx.tracked, &ctx.root)
        .into_iter()
        .collect()
}

/// One offending manifest and which clients it lists.
struct Offender {
    path: String,
    clients: Vec<String>,
}

fn multiple_http_clients(tracked: &[String], root: &Path) -> Option<Finding> {
    let mut offenders: Vec<Offender> = Vec::new();
    for path in tracked {
        // Skip vendored / fixture manifests — agent-clone hygiene applies
        // to the project's own deps only.
        if crate::audit::util::is_generated_or_fixture(path) {
            continue;
        }
        let name = basename(path).to_ascii_lowercase();
        let clients: BTreeSet<&str> = match name.as_str() {
            "package.json" => deps_from_package_json(&root.join(path), JS_HTTP_CLIENTS),
            "pyproject.toml" => deps_from_pyproject(&root.join(path), PY_HTTP_CLIENTS),
            "requirements.txt" => deps_from_requirements(&root.join(path), PY_HTTP_CLIENTS),
            "cargo.toml" => deps_from_cargo(&root.join(path), RUST_HTTP_CLIENTS),
            "go.mod" => deps_from_gomod(&root.join(path), GO_HTTP_CLIENTS),
            "pom.xml" => deps_from_pom(&root.join(path), JAVA_HTTP_CLIENTS),
            "build.gradle" | "build.gradle.kts" => {
                deps_from_gradle(&root.join(path), JAVA_HTTP_CLIENTS)
            }
            _ => continue,
        };
        if clients.len() >= 2 {
            offenders.push(Offender {
                path: path.clone(),
                clients: clients.into_iter().map(String::from).collect(),
            });
        }
    }

    if offenders.is_empty() {
        return None;
    }
    offenders.sort_by(|a, b| b.clients.len().cmp(&a.clients.len()).then(a.path.cmp(&b.path)));
    let worst_count = offenders[0].clients.len();
    let severity = if worst_count >= 3 {
        Severity::Critical
    } else {
        Severity::Warn
    };

    let evidence: Vec<String> = offenders
        .iter()
        .map(|o| format!("{}: {}", o.path, o.clients.join(", ")))
        .collect();

    Some(Finding::new(
        Category::Inconsistency,
        "multiple-http-clients",
        severity,
        format!(
            "{} manifest(s) list multiple HTTP clients as direct deps — \
             pick one; carrying both is an agent that didn't remember its previous answer",
            offenders.len()
        ),
        evidence,
    ))
}

const JS_HTTP_CLIENTS: &[&str] = &[
    "axios",
    "node-fetch",
    "isomorphic-fetch",
    "cross-fetch",
    "ky",
    "got",
    "undici",
    "superagent",
    "request",
    "phin",
    "needle",
    "wretch",
    "redaxios",
];

/// Python: `requests` and `urllib3` are sometimes both present because
/// requests *uses* urllib3. We deliberately list urllib3 only as a
/// stand-alone client — adding it on top of requests + httpx still flags
/// (the agent piled them on), but a project that only declares urllib3
/// stays quiet.
const PY_HTTP_CLIENTS: &[&str] = &[
    "requests",
    "httpx",
    "aiohttp",
    "urllib3",
    "httplib2",
    "tornado",
    "pycurl",
    "treq",
    "niquests",
];

const RUST_HTTP_CLIENTS: &[&str] = &[
    "reqwest",
    "ureq",
    "surf",
    "isahc",
    "curl",
    "hyper",
    "attohttpc",
    "minreq",
];

const GO_HTTP_CLIENTS: &[&str] = &[
    "github.com/go-resty/resty",
    "github.com/valyala/fasthttp",
    "github.com/imroc/req",
    "github.com/parnurzeal/gorequest",
    "github.com/gojek/heimdall",
];

const JAVA_HTTP_CLIENTS: &[&str] = &[
    "okhttp",
    "okhttp3",
    "apache.httpcomponents",
    "httpclient",
    "retrofit",
    "spring-webclient",
    "spring-webflux",
    "feign",
    "unirest",
];

// ---------------- manifest parsers ----------------

fn deps_from_package_json<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
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

fn deps_from_pyproject<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return BTreeSet::new();
    };
    let mut found = BTreeSet::new();
    // PEP 621 / standard `[project] dependencies = [...]`
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
    // Poetry-style: `[tool.poetry.dependencies] requests = "..."`
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

fn match_pep_dep<'a>(spec: &str, known: &'a [&'a str]) -> Option<&'a str> {
    // "requests>=2.31" / "httpx[http2]==0.25" — name is the prefix before
    // any non-name character.
    let name: String = spec
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect();
    known.iter().find(|c| **c == name.as_str()).copied()
}

fn deps_from_requirements<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
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

fn deps_from_cargo<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
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

fn deps_from_gomod<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    // go.mod isn't TOML — we walk line by line. A `require <module>
    // <version>` line names one dep; a `require ( … )` block names many.
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
        // Match by *prefix* — `github.com/go-resty/resty/v2` should match
        // `github.com/go-resty/resty`.
        for k in known {
            if module.starts_with(k) {
                found.insert(*k);
            }
        }
    }
    found
}

fn deps_from_pom<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
    // We don't parse XML — just grep for <artifactId> / <groupId>
    // containing a known fragment. Maven artifact names vary enough that
    // a substring check across both fields is the simplest robust thing.
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

fn deps_from_gradle<'a>(path: &Path, known: &'a [&'a str]) -> BTreeSet<&'a str> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup(files: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, content) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, content).unwrap();
        }
        dir
    }

    fn run(dir: &TempDir, tracked: &[&str]) -> Option<Finding> {
        let tracked: Vec<String> = tracked.iter().map(|s| s.to_string()).collect();
        multiple_http_clients(&tracked, dir.path())
    }

    // ---------- JS / TS (legacy tests, kept) ----------

    #[test]
    fn single_client_is_quiet() {
        let dir = setup(&[(
            "package.json",
            r#"{"dependencies":{"axios":"^1.0.0","react":"^19"}}"#,
        )]);
        assert!(run(&dir, &["package.json"]).is_none());
    }

    #[test]
    fn two_clients_is_warn() {
        let dir = setup(&[(
            "package.json",
            r#"{"dependencies":{"axios":"^1.0.0","ky":"^1"}}"#,
        )]);
        let f = run(&dir, &["package.json"]).unwrap();
        assert_eq!(f.check, "multiple-http-clients");
        assert_eq!(f.severity, Severity::Warn);
        assert!(f.evidence[0].contains("axios"));
        assert!(f.evidence[0].contains("ky"));
    }

    #[test]
    fn three_clients_is_critical() {
        let dir = setup(&[(
            "package.json",
            r#"{"dependencies":{"axios":"^1","node-fetch":"^3","ky":"^1"}}"#,
        )]);
        assert_eq!(run(&dir, &["package.json"]).unwrap().severity, Severity::Critical);
    }

    #[test]
    fn dev_dependencies_count_too() {
        let dir = setup(&[(
            "package.json",
            r#"{"dependencies":{"axios":"^1"},"devDependencies":{"node-fetch":"^3"}}"#,
        )]);
        assert!(run(&dir, &["package.json"]).is_some());
    }

    #[test]
    fn ignores_fixture_paths() {
        let dir = setup(&[(
            "tests/fixtures/package.json",
            r#"{"dependencies":{"axios":"^1","ky":"^1"}}"#,
        )]);
        assert!(run(&dir, &["tests/fixtures/package.json"]).is_none());
    }

    #[test]
    fn malformed_json_is_skipped_silently() {
        let dir = setup(&[("package.json", r#"{not valid json"#)]);
        assert!(run(&dir, &["package.json"]).is_none());
    }

    // ---------- Python ----------

    #[test]
    fn python_pyproject_pep621_two_clients() {
        let dir = setup(&[(
            "pyproject.toml",
            r#"
[project]
name = "x"
dependencies = ["requests>=2.0", "httpx[http2]==0.25", "click"]
"#,
        )]);
        let f = run(&dir, &["pyproject.toml"]).unwrap();
        assert!(f.evidence[0].contains("requests"));
        assert!(f.evidence[0].contains("httpx"));
    }

    #[test]
    fn python_pyproject_poetry_two_clients() {
        let dir = setup(&[(
            "pyproject.toml",
            r#"
[tool.poetry.dependencies]
python = "^3.10"
requests = "^2.0"
aiohttp = "^3.9"
"#,
        )]);
        assert!(run(&dir, &["pyproject.toml"]).is_some());
    }

    #[test]
    fn python_requirements_two_clients() {
        let dir = setup(&[(
            "requirements.txt",
            "# top-level requirements\nrequests>=2.31\nhttpx==0.25\n-e .\n",
        )]);
        assert!(run(&dir, &["requirements.txt"]).is_some());
    }

    #[test]
    fn python_single_client_is_quiet() {
        let dir = setup(&[(
            "pyproject.toml",
            r#"
[project]
dependencies = ["requests"]
"#,
        )]);
        assert!(run(&dir, &["pyproject.toml"]).is_none());
    }

    // ---------- Rust ----------

    #[test]
    fn rust_cargo_two_clients() {
        let dir = setup(&[(
            "Cargo.toml",
            r#"
[package]
name = "x"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = "0.12"
ureq = "2"
serde = "1"
"#,
        )]);
        let f = run(&dir, &["Cargo.toml"]).unwrap();
        assert!(f.evidence[0].contains("reqwest"));
        assert!(f.evidence[0].contains("ureq"));
    }

    #[test]
    fn rust_dev_deps_count_too() {
        let dir = setup(&[(
            "Cargo.toml",
            r#"
[dependencies]
reqwest = "0.12"

[dev-dependencies]
ureq = "2"
"#,
        )]);
        assert!(run(&dir, &["Cargo.toml"]).is_some());
    }

    // ---------- Go ----------

    #[test]
    fn go_module_two_clients() {
        let dir = setup(&[(
            "go.mod",
            r#"
module example.com/x

go 1.21

require (
    github.com/go-resty/resty/v2 v2.10.0
    github.com/valyala/fasthttp v1.50.0
    github.com/spf13/cobra v1.7.0
)
"#,
        )]);
        let f = run(&dir, &["go.mod"]).unwrap();
        assert!(f.evidence[0].contains("resty"));
        assert!(f.evidence[0].contains("fasthttp"));
    }

    // ---------- Java ----------

    #[test]
    fn java_pom_two_clients() {
        let dir = setup(&[(
            "pom.xml",
            r#"<project>
              <dependencies>
                <dependency><groupId>com.squareup.okhttp3</groupId><artifactId>okhttp</artifactId></dependency>
                <dependency><groupId>com.squareup.retrofit2</groupId><artifactId>retrofit</artifactId></dependency>
              </dependencies>
            </project>"#,
        )]);
        let f = run(&dir, &["pom.xml"]).unwrap();
        assert!(f.evidence[0].contains("okhttp"));
        assert!(f.evidence[0].contains("retrofit"));
    }

    #[test]
    fn java_gradle_two_clients() {
        let dir = setup(&[(
            "build.gradle",
            r#"
dependencies {
    implementation "com.squareup.okhttp3:okhttp:4.12.0"
    implementation "com.squareup.retrofit2:retrofit:2.9.0"
}
"#,
        )]);
        assert!(run(&dir, &["build.gradle"]).is_some());
    }
}
