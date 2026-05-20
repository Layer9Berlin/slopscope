//! Signal: multiple client-side state-management libraries as direct deps.
//!
//! `redux` + `zustand` + `jotai` in one `package.json` is the agent picking
//! a different answer every time a feature needed shared state. JS / TS
//! ecosystem only — Python / Rust / Go aren't structured around this kind
//! of library choice.

use super::manifests::{detect_pile_up, EcosystemLibs, LibPile};
use crate::audit::AuditContext;
use crate::finding::Finding;

const SPEC: LibPile = LibPile {
    check_id: "multiple-state-libs",
    thing_name: "state-management libraries",
    libs: EcosystemLibs {
        js: &[
            // Redux family — `@reduxjs/toolkit` lives under @reduxjs/, so
            // we include both the bare name and the toolkit package.
            "redux",
            "@reduxjs/toolkit",
            // Atom-style
            "jotai",
            "recoil",
            // Hook stores
            "zustand",
            "valtio",
            // Reactive / observable
            "mobx",
            "@legendapp/state",
            "xstate",
            // Vue (kept here so a polyglot monorepo's package.json with
            // both vuex *and* pinia trips this signal)
            "vuex",
            "pinia",
        ],
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
    

    fn run(rel: &str, json: &str) -> Option<Finding> {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(rel), json).unwrap();
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
    fn single_state_lib_is_quiet() {
        assert!(run("package.json", r#"{"dependencies":{"zustand":"^4"}}"#).is_none());
    }

    #[test]
    fn redux_plus_zustand_is_warn() {
        let f = run("package.json", r#"{"dependencies":{"redux":"^5","zustand":"^4"}}"#).unwrap();
        assert_eq!(f.check, "multiple-state-libs");
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn redux_plus_jotai_plus_zustand_is_critical() {
        let f = run(
            "package.json",
            r#"{"dependencies":{"redux":"^5","zustand":"^4","jotai":"^2"}}"#,
        )
        .unwrap();
        assert_eq!(f.severity, Severity::Critical);
    }

    #[test]
    fn vuex_and_pinia_in_same_repo_flags() {
        let f = run("package.json", r#"{"dependencies":{"vuex":"^4","pinia":"^2"}}"#);
        assert!(f.is_some());
    }
}
