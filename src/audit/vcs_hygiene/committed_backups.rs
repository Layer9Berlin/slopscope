//! Signal: backup / `.orig` / `.old` files and log files tracked by git.

use crate::audit::util::{basename, is_generated_or_fixture};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    committed_backups_and_logs(&ctx.tracked)
}

fn committed_backups_and_logs(tracked: &[String]) -> Vec<Finding> {
    let mut backups = Vec::new();
    let mut logs = Vec::new();

    for path in tracked {
        // A `.bak` / `.log` under tests/fixtures/vendor is a test fixture or a
        // dependency artifact, not a carelessly committed backup.
        if is_generated_or_fixture(path) {
            continue;
        }
        let name = basename(path).to_ascii_lowercase();
        // Extension-based only: `backup_*` prefixes are handled by steering-failure's
        // rollback-artifacts check, where the date/extension guard avoids
        // false-positiving on legit files like Android's backup_rules.xml.
        let is_backup = name.ends_with(".bak")
            || name.ends_with(".backup")
            || name.ends_with(".old")
            || name.ends_with(".orig")
            || name.ends_with('~')
            || name.contains(".backup.");
        let is_log = name.ends_with(".log");

        if is_backup {
            backups.push(path.clone());
        } else if is_log {
            logs.push(path.clone());
        }
    }

    let mut findings = Vec::new();
    if !backups.is_empty() {
        backups.sort();
        findings.push(Finding::new(
            Category::VcsHygiene,
            "committed-backup-files",
            Severity::Warn,
            "Backup / .orig / .old files are tracked by git",
            backups,
        ));
    }
    if !logs.is_empty() {
        logs.sort();
        findings.push(Finding::new(
            Category::VcsHygiene,
            "committed-log-files",
            Severity::Warn,
            "Log files are tracked by git",
            logs,
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::files;

    #[test]
    fn flags_extension_based_artifacts() {
        let f = committed_backups_and_logs(&files(&[
            "db.sql.bak",
            "config.backup",
            "main.rs.old",
            "patch.orig",
            "notes.txt~",
            "dump.backup.gz",
        ]));
        let backup = f.iter().find(|x| x.check == "committed-backup-files").unwrap();
        assert_eq!(backup.severity, Severity::Warn);
        assert_eq!(backup.count, 6);
    }

    #[test]
    fn does_not_flag_android_backup_rules() {
        // Regression: backup_rules.xml is legit Android config, not a backup.
        let f = committed_backups_and_logs(&files(&[
            "android/app/src/main/res/xml/backup_rules.xml",
            "backup_20251127.sql", // backup_ prefix, but vcs-hygiene is extension-only
        ]));
        assert!(
            f.iter().all(|x| x.check != "committed-backup-files"),
            "vcs-hygiene must not flag backup_-prefixed files"
        );
    }

    #[test]
    fn logs_flagged_separately_from_backups() {
        let f = committed_backups_and_logs(&files(&["app.log", "error.LOG", "x.bak"]));
        let logs = f.iter().find(|x| x.check == "committed-log-files").unwrap();
        assert_eq!(logs.count, 2);
        assert!(f.iter().any(|x| x.check == "committed-backup-files"));
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn ignores_backups_and_logs_under_test_and_vendor_paths() {
        // bat ships .bak/.orig/.log files under tests/ as syntax fixtures.
        let fixtures = &[
            "tests/syntax-tests/source/Ignored suffixes/test.rs.bak",
            "tests/syntax-tests/highlighted/Log/example.log",
            "node_modules/dep/old.orig",
        ];
        assert!(committed_backups_and_logs(&files(fixtures)).is_empty());
    }

    #[test]
    fn empty_when_clean() {
        assert!(committed_backups_and_logs(&files(&["main.rs", "lib.rs"])).is_empty());
    }
}
