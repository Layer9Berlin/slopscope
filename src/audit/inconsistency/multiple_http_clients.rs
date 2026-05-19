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

use super::manifests::{detect_pile_up, EcosystemLibs, LibPile};
use crate::audit::AuditContext;
use crate::finding::Finding;

const SPEC: LibPile = LibPile {
    check_id: "multiple-http-clients",
    thing_name: "HTTP clients",
    libs: EcosystemLibs {
        js: &[
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
        ],
        // Python: `requests` and `urllib3` are sometimes both present
        // because requests *uses* urllib3. We list urllib3 only as a
        // stand-alone client — adding it on top of requests + httpx
        // still flags (the agent piled them on), but a project that
        // only declares urllib3 stays quiet.
        python: &[
            "requests",
            "httpx",
            "aiohttp",
            "urllib3",
            "httplib2",
            "tornado",
            "pycurl",
            "treq",
            "niquests",
        ],
        rust: &[
            "reqwest",
            "ureq",
            "surf",
            "isahc",
            "curl",
            "hyper",
            "attohttpc",
            "minreq",
        ],
        go: &[
            "github.com/go-resty/resty",
            "github.com/valyala/fasthttp",
            "github.com/imroc/req",
            "github.com/parnurzeal/gorequest",
            "github.com/gojek/heimdall",
        ],
        java: &[
            "okhttp",
            "okhttp3",
            "apache.httpcomponents",
            "httpclient",
            "retrofit",
            "spring-webclient",
            "spring-webflux",
            "feign",
            "unirest",
        ],
    },
};

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    detect_pile_up(ctx, &SPEC).into_iter().collect()
}

// Tests are deliberately end-to-end here: they build a temp dir, write
// manifests, populate an [`AuditContext`], and run `check`. That exercises
// the full path through `manifests::detect_pile_up` rather than the
// per-parser helpers, so it stays valuable even as the implementation
// moves around.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditContext;
    use crate::finding::Severity;
    use std::collections::HashMap;
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

    fn ctx_of(dir: &TempDir, tracked: &[&str]) -> AuditContext {
        AuditContext {
            root: dir.path().to_path_buf(),
            tracked: tracked.iter().map(|s| s.to_string()).collect(),
            commits: Vec::new(),
            blob_history: HashMap::new(),
            source_files: Vec::new(),
        }
    }

    fn run(dir: &TempDir, tracked: &[&str]) -> Option<Finding> {
        let ctx = ctx_of(dir, tracked);
        check(&ctx).into_iter().next()
    }

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
