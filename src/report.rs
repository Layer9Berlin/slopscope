use crate::audit::AuditReport;
use crate::finding::{Category, Severity};

/// Stable JSON for the agent / for piping into other tools.
pub fn json(report: &AuditReport) -> String {
    serde_json::to_string_pretty(report).expect("AuditReport is always serializable")
}

/// Human-readable report for the terminal.
pub fn human(report: &AuditReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("slopscope audit — {}\n", report.root));
    out.push_str(&format!(
        "{} commits, {} tracked files\n\n",
        report.commits_analyzed, report.tracked_files
    ));

    if report.findings.is_empty() {
        out.push_str("No slop signals found. Clean repo.\n");
        return out;
    }

    let crit = count(report, Severity::Critical);
    let warn = count(report, Severity::Warn);
    let info = count(report, Severity::Info);
    out.push_str(&format!(
        "{} finding(s): {crit} critical, {warn} warning, {info} info\n",
        report.findings.len()
    ));
    out.push('\n');

    for f in &report.findings {
        out.push_str(&format!(
            "{}  [{}] {}\n",
            tag(f.severity),
            bucket(f.category),
            f.summary
        ));
        out.push_str(&format!("    check: {}  ({} item(s))\n", f.check, f.count));
        for (i, e) in f.evidence.iter().enumerate() {
            if i == 8 {
                out.push_str(&format!("      … and {} more\n", f.evidence.len() - 8));
                break;
            }
            out.push_str(&format!("      - {e}\n"));
        }
        out.push('\n');
    }
    out
}

fn count(report: &AuditReport, sev: Severity) -> usize {
    report.findings.iter().filter(|f| f.severity == sev).count()
}

fn tag(sev: Severity) -> &'static str {
    match sev {
        Severity::Critical => "CRIT",
        Severity::Warn => "WARN",
        Severity::Info => "INFO",
    }
}

fn bucket(cat: Category) -> &'static str {
    match cat {
        Category::VcsHygiene => "vcs-hygiene",
        Category::SteeringFailure => "steering-failure",
        Category::Inconsistency => "inconsistency",
        Category::Verification => "verification",
        Category::Complexity => "complexity",
        Category::CodeSmell => "code-smell",
    }
}
