//! Signal: secret / credential files tracked by git.

use crate::audit::util::{basename, is_generated_or_fixture};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    committed_secrets(&ctx.tracked).into_iter().collect()
}

fn committed_secrets(tracked: &[String]) -> Option<Finding> {
    let mut critical = Vec::new();
    let mut warn = Vec::new();

    for path in tracked {
        // A secret-looking file under tests/fixtures/vendor isn't a leaked
        // credential — it's a test fixture or a dependency's own file.
        if is_generated_or_fixture(path) {
            continue;
        }
        let name = basename(path).to_ascii_lowercase();
        // Key material / credentials — assume the worst.
        let is_key = name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
            || name == "id_rsa"
            || name == "id_dsa"
            || name == "id_ecdsa"
            || name == "id_ed25519"
            || name == "credentials.json"
            || name.starts_with("service-account")
            || name.starts_with("serviceaccount");
        // .env family, excluding the harmless templates.
        let is_env = (name == ".env" || name.starts_with(".env."))
            && !name.ends_with(".example")
            && !name.ends_with(".sample")
            && !name.ends_with(".template")
            && !name.ends_with(".dist");

        if is_key {
            critical.push(path.clone());
        } else if is_env {
            warn.push(path.clone());
        }
    }

    if critical.is_empty() && warn.is_empty() {
        return None;
    }
    // Critical key material sets the finding's severity; .env files ride along
    // in the evidence so nothing is hidden, but they don't downgrade the tier.
    let severity = if critical.is_empty() {
        Severity::Warn
    } else {
        Severity::Critical
    };
    let mut evidence = critical;
    evidence.extend(warn);
    evidence.sort();
    Some(Finding::new(
        Category::VcsHygiene,
        "committed-secrets",
        severity,
        "Secret or credential files are tracked by git",
        evidence,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    #[test]
    fn none_when_clean() {
        assert!(committed_secrets(&files(&["src/main.rs", "README.md"])).is_none());
    }

    #[test]
    fn flags_bare_env_as_warn() {
        let f = one(committed_secrets(&files(&[".env"])));
        assert_eq!(f.check, "committed-secrets");
        assert_eq!(f.severity, Severity::Warn);
        assert_eq!(f.evidence, vec![".env"]);
        assert_eq!(f.count, 1);
    }

    #[test]
    fn flags_env_variants() {
        for name in [".env.local", ".env.production", ".ENV", "config/.env"] {
            let f = committed_secrets(&files(&[name]));
            assert!(f.is_some(), "{name} should be flagged");
            assert_eq!(f.unwrap().severity, Severity::Warn, "{name}");
        }
    }

    #[test]
    fn excludes_env_templates() {
        let clean = &[
            ".env.example",
            ".env.sample",
            ".env.template",
            ".env.dist",
            ".environment", // not an env file at all
        ];
        assert!(committed_secrets(&files(clean)).is_none());
    }

    #[test]
    fn flags_key_material_as_critical() {
        for name in [
            "server.pem",
            "private.key",
            "cert.p12",
            "cert.pfx",
            "id_rsa",
            "id_ed25519",
            "credentials.json",
            "service-account.json",
            "serviceaccount-prod.json",
        ] {
            let f = one(committed_secrets(&files(&[name])));
            assert_eq!(f.severity, Severity::Critical, "{name} must be critical");
        }
    }

    #[test]
    fn ignores_secrets_under_test_and_vendor_paths() {
        // A syntax-highlighter's test fixture, not a leaked credential.
        let fixtures = &[
            "tests/syntax-tests/source/DotENV/.env",
            "fixtures/sample.pem",
            "node_modules/some-dep/.env",
        ];
        assert!(committed_secrets(&files(fixtures)).is_none());
    }

    #[test]
    fn critical_wins_severity_but_keeps_all_evidence() {
        let f = one(committed_secrets(&files(&[".env", "id_rsa", "deploy.key"])));
        assert_eq!(f.severity, Severity::Critical);
        assert_eq!(f.count, 3);
        assert_eq!(f.evidence, vec![".env", "deploy.key", "id_rsa"]);
    }
}
