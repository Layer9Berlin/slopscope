//! Signal: multiple schema-validation libraries as direct dependencies.
//!
//! `zod` + `yup` + `joi` in one `package.json` is the agent picking
//! "whichever the prompt mentioned" each time it wrote a validator.
//! Almost every vibe-coded TS/JS repo has at least two of these — same
//! shape as the HTTP-client pile, different library set.

use super::manifests::{detect_pile_up, EcosystemLibs, LibPile};
use crate::audit::AuditContext;
use crate::finding::Finding;

const SPEC: LibPile = LibPile {
    check_id: "multiple-validation-libs",
    thing_name: "schema-validation libraries",
    libs: EcosystemLibs {
        js: &[
            "zod",
            "yup",
            "joi",
            "ajv",
            "io-ts",
            "superstruct",
            "valibot",
            "vest",
            "class-validator",
            "myzod",
            "runtypes",
        ],
        // Python: pydantic is the dominant choice; marshmallow / cerberus /
        // voluptuous / schema are legacy or niche alternatives.
        python: &[
            "pydantic",
            "marshmallow",
            "cerberus",
            "voluptuous",
            "schema",
            "schematics",
            "jsonschema",
        ],
        // Rust validation libs are niche; we include `validator` + `garde`
        // (the two common derive-based ones).
        rust: &["validator", "garde", "valico"],
        ..EcosystemLibs::empty()
    },
};

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    detect_pile_up(ctx, &SPEC).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditContext;
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

    fn run(dir: &TempDir, tracked: &[&str]) -> Option<Finding> {
        let ctx = AuditContext {
            root: dir.path().to_path_buf(),
            tracked: tracked.iter().map(|s| s.to_string()).collect(),
            commits: Vec::new(),
            blob_history: HashMap::new(),
            source_files: Vec::new(),
        };
        check(&ctx).into_iter().next()
    }

    #[test]
    fn single_validator_is_quiet() {
        let dir = setup(&[("package.json", r#"{"dependencies":{"zod":"^3"}}"#)]);
        assert!(run(&dir, &["package.json"]).is_none());
    }

    #[test]
    fn js_zod_and_yup_conflict() {
        let dir = setup(&[(
            "package.json",
            r#"{"dependencies":{"zod":"^3","yup":"^1"}}"#,
        )]);
        let f = run(&dir, &["package.json"]).unwrap();
        assert_eq!(f.check, "multiple-validation-libs");
        assert!(f.evidence[0].contains("zod"));
        assert!(f.evidence[0].contains("yup"));
    }

    #[test]
    fn three_validators_is_critical() {
        use crate::finding::Severity;
        let dir = setup(&[(
            "package.json",
            r#"{"dependencies":{"zod":"^3","yup":"^1","joi":"^17"}}"#,
        )]);
        assert_eq!(
            run(&dir, &["package.json"]).unwrap().severity,
            Severity::Critical
        );
    }

    #[test]
    fn python_pydantic_and_marshmallow_conflict() {
        let dir = setup(&[(
            "pyproject.toml",
            r#"
[project]
dependencies = ["pydantic>=2", "marshmallow>=3"]
"#,
        )]);
        assert!(run(&dir, &["pyproject.toml"]).is_some());
    }
}
