//! Signal: loose `.md` / `.sql` files piling up at the repo root —
//! documentation sprawl and ad-hoc migrations, usually agent-generated.

use crate::audit::util::is_root_level;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    root_file_sprawl(&ctx.tracked)
}

fn root_file_sprawl(tracked: &[String]) -> Vec<Finding> {
    let mut md = Vec::new();
    let mut sql = Vec::new();
    for path in tracked {
        if !is_root_level(path) {
            continue;
        }
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".md") {
            md.push(path.clone());
        } else if lower.ends_with(".sql") {
            sql.push(path.clone());
        }
    }
    let mut findings = Vec::new();
    if md.len() > 10 {
        md.sort();
        let severity = if md.len() > 25 {
            Severity::Warn
        } else {
            Severity::Info
        };
        findings.push(Finding::new(
            Category::SteeringFailure,
            "root-markdown-sprawl",
            severity,
            format!(
                "{} markdown files at repo root — documentation sprawl, usually agent-generated",
                md.len()
            ),
            md,
        ));
    }
    if sql.len() > 3 {
        sql.sort();
        findings.push(Finding::new(
            Category::SteeringFailure,
            "root-sql-sprawl",
            Severity::Info,
            format!(
                "{} loose .sql files at repo root — ad-hoc migrations outside any migration tool",
                sql.len()
            ),
            sql,
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n_root_files(n: usize, ext: &str) -> Vec<String> {
        (0..n).map(|i| format!("doc{i}.{ext}")).collect()
    }

    #[test]
    fn markdown_threshold_and_severity() {
        // <= 10 is quiet
        assert!(root_file_sprawl(&n_root_files(10, "md")).is_empty());
        // 11..=25 is Info
        let f = &root_file_sprawl(&n_root_files(20, "md"))[0];
        assert_eq!(f.check, "root-markdown-sprawl");
        assert_eq!(f.severity, Severity::Info);
        // > 25 is Warn
        let f = &root_file_sprawl(&n_root_files(30, "md"))[0];
        assert_eq!(f.severity, Severity::Warn);
    }

    #[test]
    fn sql_threshold() {
        assert!(root_file_sprawl(&n_root_files(3, "sql")).is_empty());
        let findings = root_file_sprawl(&n_root_files(4, "sql"));
        assert_eq!(findings[0].check, "root-sql-sprawl");
        assert_eq!(findings[0].severity, Severity::Info);
    }

    #[test]
    fn ignores_nested_files() {
        // 30 markdown files, but all nested — repo root is clean
        let nested: Vec<String> = (0..30).map(|i| format!("docs/d{i}.md")).collect();
        assert!(root_file_sprawl(&nested).is_empty());
    }

    #[test]
    fn emits_both_findings() {
        let mut paths = n_root_files(15, "md");
        paths.extend(n_root_files(5, "sql"));
        let findings = root_file_sprawl(&paths);
        assert_eq!(findings.len(), 2);
    }
}
