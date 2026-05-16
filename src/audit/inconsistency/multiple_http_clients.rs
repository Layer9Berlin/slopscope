//! Signal: multiple HTTP client libraries listed as direct dependencies.
//!
//! `axios` *and* `node-fetch` *and* `ky` in one `package.json` is the
//! fingerprint of an agent not remembering which client it used last time
//! and reaching for whichever the prompt suggested. One is normal; two or
//! more direct deps doing the same job is inconsistency frozen into the
//! dependency tree.
//!
//! Reads `package.json` files from disk (only `dependencies` and
//! `devDependencies` — transitive doesn't count). Multi-package repos with
//! several `package.json` files report the worst offender plus a count.

use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

/// Direct HTTP-client dependencies we know about. `fetch` is built-in to
/// modern node + browser, so absence isn't tracked — but if `node-fetch`
/// is added *on top of* one of these, that's the inconsistency tell.
const HTTP_CLIENTS: &[&str] = &[
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

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    multiple_http_clients(&ctx.tracked, &ctx.root)
        .into_iter()
        .collect()
}

/// One offending package.json and which clients it lists.
struct Offender {
    path: String,
    clients: Vec<String>,
}

fn multiple_http_clients(tracked: &[String], root: &Path) -> Option<Finding> {
    let mut offenders: Vec<Offender> = Vec::new();
    for path in tracked {
        if !is_package_json(path) {
            continue;
        }
        // Skip vendored / fixture package.json files — agent-clone hygiene
        // applies to the project's own manifests only.
        if crate::audit::util::is_generated_or_fixture(path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let mut clients: BTreeSet<&str> = BTreeSet::new();
        for field in ["dependencies", "devDependencies"] {
            if let Some(deps) = json.get(field).and_then(Value::as_object) {
                for name in deps.keys() {
                    if let Some(known) = HTTP_CLIENTS.iter().find(|c| **c == name) {
                        clients.insert(*known);
                    }
                }
            }
        }
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
            "{} package.json file(s) list multiple HTTP clients as direct deps — \
             pick one; carrying both is an agent that didn't remember its previous answer",
            offenders.len()
        ),
        evidence,
    ))
}

fn is_package_json(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == "package.json"
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
}
