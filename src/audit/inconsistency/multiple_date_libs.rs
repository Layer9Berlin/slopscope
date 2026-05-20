//! Signal: multiple date / time libraries as direct deps.
//!
//! `moment` + `dayjs` + `date-fns` in one `package.json` is the agent
//! reaching for whichever date lib the current prompt mentioned. Three is
//! hilariously common in vibe-coded JS/TS projects. Python equivalents
//! (`arrow`, `pendulum`, `maya`) compete with the stdlib `datetime` plus
//! one another.

use super::manifests::{detect_pile_up, EcosystemLibs, LibPile};
use crate::audit::AuditContext;
use crate::finding::Finding;

const SPEC: LibPile = LibPile {
    check_id: "multiple-date-libs",
    thing_name: "date / time libraries",
    libs: EcosystemLibs {
        js: &[
            "moment",
            "moment-timezone",
            "dayjs",
            "date-fns",
            "luxon",
            "js-joda",
            "@js-joda/core",
            "spacetime",
        ],
        // Python: stdlib `datetime` is universal, so we list only the
        // third-party ones. Two of these in one project is the tell.
        python: &["arrow", "pendulum", "maya", "delorean", "moment", "udatetime"],
        // Rust: `chrono` vs `time` is the big rivalry; `jiff` is the new
        // entrant. We list all three.
        rust: &["chrono", "time", "jiff"],
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
    use crate::finding::Severity;
    use std::collections::HashMap;
    use std::fs;
    

    fn run(rel: &str, content: &str) -> Option<Finding> {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
        let ctx = AuditContext {
            root: dir.path().to_path_buf(),
            tracked: vec![rel.into()],
            commits: Vec::new(),
            blob_history: HashMap::new(),
            source_files: Vec::new(),
        };
        check(&ctx).into_iter().next()
    }

    #[test]
    fn single_date_lib_is_quiet() {
        assert!(run("package.json", r#"{"dependencies":{"dayjs":"^1"}}"#).is_none());
    }

    #[test]
    fn moment_plus_dayjs_is_warn() {
        let f = run(
            "package.json",
            r#"{"dependencies":{"moment":"^2","dayjs":"^1"}}"#,
        )
        .unwrap();
        assert_eq!(f.check, "multiple-date-libs");
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn three_date_libs_is_critical() {
        let f = run(
            "package.json",
            r#"{"dependencies":{"moment":"^2","dayjs":"^1","date-fns":"^3"}}"#,
        )
        .unwrap();
        assert_eq!(f.severity, Severity::Critical);
    }

    #[test]
    fn rust_chrono_plus_time_flags() {
        let f = run(
            "Cargo.toml",
            r#"
[dependencies]
chrono = "0.4"
time = "0.3"
"#,
        );
        assert!(f.is_some());
    }

    #[test]
    fn python_arrow_plus_pendulum_flags() {
        let f = run(
            "pyproject.toml",
            r#"
[project]
dependencies = ["arrow>=1.3", "pendulum>=3"]
"#,
        );
        assert!(f.is_some());
    }
}
