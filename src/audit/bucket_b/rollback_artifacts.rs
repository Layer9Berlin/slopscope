//! Signal: committed backup / rollback files — manual save-points instead of
//! trusting version control.

use crate::audit::util::basename;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Extensions that make a `backup_`-prefixed file look like a real data dump.
const BACKUP_EXTS: &[&str] = &[
    ".sql", ".gz", ".zip", ".tar", ".dump", ".bak", ".db", ".sqlite", ".json", ".csv",
];

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    rollback_artifacts(&ctx.tracked).into_iter().collect()
}

fn rollback_artifacts(tracked: &[String]) -> Option<Finding> {
    let mut hits = Vec::new();
    for path in tracked {
        let name = basename(path).to_ascii_lowercase();
        // `backup_` / `backup-` prefix only counts as a manual save-point if it
        // also looks like a data dump — has a date/number or a backup-ish
        // extension. Guards against legit config like Android's backup_rules.xml.
        let backup_prefixed = name.starts_with("backup_") || name.starts_with("backup-");
        let looks_like_dump = name.chars().any(|c| c.is_ascii_digit())
            || BACKUP_EXTS.iter().any(|e| name.ends_with(e));
        // `rollback` is a real SQL keyword — postgres ships `rollback.sgml`,
        // `rollback_prepared.sgml` etc. as command docs. The agent-narration
        // form is always a markdown/text file (`ROLLBACK_GUIDE.md`,
        // `_ROLLBACK_SUMMARY.md`).
        let rollback_doc =
            name.contains("rollback") && (name.ends_with(".md") || name.ends_with(".txt"));
        let is_rollback = (backup_prefixed && looks_like_dump)
            || rollback_doc
            || name.contains("_old.")
            || name.starts_with("old_");
        if is_rollback {
            hits.push(path.clone());
        }
    }
    if hits.len() < 2 {
        return None;
    }
    hits.sort();
    Some(Finding::new(
        Category::SteeringFailure,
        "rollback-artifacts",
        Severity::Warn,
        format!(
            "{} backup / rollback files committed — manual save-points instead of trusting version control",
            hits.len()
        ),
        hits,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    #[test]
    fn below_two_is_quiet() {
        assert!(rollback_artifacts(&files(&["backup_2025.sql"])).is_none());
    }

    #[test]
    fn flags_dated_or_dump_backups() {
        let f = one(rollback_artifacts(&files(&[
            "backup_20251127.sql",
            "backup-1.txt",  // has a digit
            "backup_db.sql", // backup-ish extension
        ])));
        assert_eq!(f.severity, Severity::Warn);
        assert_eq!(f.count, 3);
    }

    #[test]
    fn does_not_flag_android_backup_rules() {
        // Regression: backup_rules.xml has no digit and a non-dump extension.
        assert!(rollback_artifacts(&files(&[
            "android/app/src/main/res/xml/backup_rules.xml",
            "src/backup_handler.ts",
        ]))
        .is_none());
    }

    #[test]
    fn postgres_sql_rollback_docs_are_not_flagged() {
        // Postgres surfaced this: `doc/.../rollback.sgml` is documentation for
        // the SQL ROLLBACK command, not a rollback artifact.
        assert!(rollback_artifacts(&files(&[
            "doc/src/sgml/ref/rollback.sgml",
            "doc/src/sgml/ref/rollback_prepared.sgml",
            "doc/src/sgml/ref/rollback_to.sgml",
        ]))
        .is_none());
    }

    #[test]
    fn flags_rollback_and_old_patterns() {
        let f = one(rollback_artifacts(&files(&[
            "ROLLBACK_GUIDE.md",
            "config_old.json",
            "old_schema.sql",
        ])));
        assert_eq!(f.count, 3);
    }
}
