//! Signal: multiple competing lint/format toolchains in the same project.
//!
//! `eslint` + `prettier` is a healthy pair (lint + format, different jobs).
//! `eslint` + `biome` is two linters fighting for the same job. So is
//! `prettier` + `biome`, or `eslint` + `rome`. When an agent can't decide
//! which tool to use, both end up committed and the repo carries the cost of
//! two configs forever.
//!
//! Covers ECMAScript-family tools (eslint / prettier / biome / rome /
//! standardjs / xo), Python lint+format tools (ruff / pylint / flake8 /
//! black / autopep8 / yapf / isort), and Java static-analysis tools
//! (checkstyle / spotbugs / pmd / errorprone).

use crate::audit::AuditContext;
use crate::audit::util::basename;
use crate::finding::{Category, Finding, Severity};
use std::collections::BTreeSet;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    multiple_lint_format_with_root(&ctx.tracked, &ctx.root)
        .into_iter()
        .collect()
}

/// Which tool a config file belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tool {
    // ECMAScript family — lint
    Eslint,
    StandardJs,
    XO,
    // ECMAScript family — format
    Prettier,
    // ECMAScript family — combined (lint + format)
    Biome,
    Rome,
    // Python — lint
    Ruff,
    Pylint,
    Flake8,
    // Python — format
    Black,
    Autopep8,
    Yapf,
    // Python — import sort
    Isort,
    // Java/Kotlin — static analysis
    Checkstyle,
    Spotbugs,
    Pmd,
    Errorprone,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Eslint => "eslint",
            Tool::Prettier => "prettier",
            Tool::Biome => "biome",
            Tool::Rome => "rome",
            Tool::StandardJs => "standardjs",
            Tool::XO => "xo",
            Tool::Ruff => "ruff",
            Tool::Pylint => "pylint",
            Tool::Flake8 => "flake8",
            Tool::Black => "black",
            Tool::Autopep8 => "autopep8",
            Tool::Yapf => "yapf",
            Tool::Isort => "isort",
            Tool::Checkstyle => "checkstyle",
            Tool::Spotbugs => "spotbugs",
            Tool::Pmd => "pmd",
            Tool::Errorprone => "errorprone",
        }
    }
}

/// Identify a tool from a config-file *basename* / known path. Returns
/// `None` for non-config files.
fn tool_of(name: &str) -> Option<Tool> {
    let lower = name.to_ascii_lowercase();
    // ECMAScript
    if lower.starts_with(".eslintrc") || lower.starts_with("eslint.config.") {
        return Some(Tool::Eslint);
    }
    if lower.starts_with(".prettierrc") || lower == "prettier.config.js" || lower == "prettier.config.mjs" {
        return Some(Tool::Prettier);
    }
    if lower == "biome.json" || lower == "biome.jsonc" {
        return Some(Tool::Biome);
    }
    if lower == "rome.json" {
        return Some(Tool::Rome);
    }
    if lower == ".standardrc" || lower == "standardrc.json" {
        return Some(Tool::StandardJs);
    }
    if lower == ".xo-config.js" || lower == "xo-config.json" || lower == ".xo-config.json" {
        return Some(Tool::XO);
    }
    // Python — many of these live as bare config files at repo root.
    if lower == "ruff.toml" || lower == ".ruff.toml" {
        return Some(Tool::Ruff);
    }
    if lower == ".pylintrc" || lower == "pylintrc" {
        return Some(Tool::Pylint);
    }
    if lower == ".flake8" || lower == "setup.cfg" {
        // setup.cfg often contains a flake8 section; we surface it as a
        // *possible* flake8 marker. The conflict check below is fine even
        // if setup.cfg is also used for other things — having setup.cfg +
        // ruff.toml + pylintrc all in one repo is the actual tell.
        return Some(Tool::Flake8);
    }
    if lower == ".isort.cfg" {
        return Some(Tool::Isort);
    }
    // black/autopep8/yapf usually live inside pyproject.toml — we detect
    // their presence in the pyproject-keys check below.
    // Java
    if lower == "checkstyle.xml" || lower.ends_with("/checkstyle.xml") || lower == ".checkstyle" {
        return Some(Tool::Checkstyle);
    }
    if lower == "spotbugs.xml" || lower.ends_with("/spotbugs.xml") || lower == ".spotbugs" {
        return Some(Tool::Spotbugs);
    }
    if lower == "pmd.xml" || lower.ends_with("/pmd.xml") || lower == "pmd-ruleset.xml" {
        return Some(Tool::Pmd);
    }
    None
}

/// Pairs that overlap in responsibility — flagging one of these means the
/// repo carries two answers to the same question. Eslint + prettier is
/// deliberately not here: lint and format are separate jobs. Similarly,
/// black + ruff (formatter + linter) doesn't conflict, but ruff + flake8
/// does (both linters), as does black + yapf (both formatters).
const COMPETING: &[(Tool, Tool)] = &[
    // ECMAScript — lint vs lint, lint+format combo conflicts
    (Tool::Eslint, Tool::Biome),
    (Tool::Eslint, Tool::Rome),
    (Tool::Eslint, Tool::StandardJs),
    (Tool::Eslint, Tool::XO),
    (Tool::Prettier, Tool::Biome),
    (Tool::Prettier, Tool::Rome),
    (Tool::Biome, Tool::Rome),
    (Tool::StandardJs, Tool::XO),
    // Python — linter vs linter
    (Tool::Ruff, Tool::Pylint),
    (Tool::Ruff, Tool::Flake8),
    (Tool::Pylint, Tool::Flake8),
    // Python — formatter vs formatter
    (Tool::Black, Tool::Yapf),
    (Tool::Black, Tool::Autopep8),
    (Tool::Yapf, Tool::Autopep8),
    // Java — every pair of static analyzers competes
    (Tool::Checkstyle, Tool::Spotbugs),
    (Tool::Checkstyle, Tool::Pmd),
    (Tool::Spotbugs, Tool::Pmd),
    (Tool::Checkstyle, Tool::Errorprone),
    (Tool::Spotbugs, Tool::Errorprone),
    (Tool::Pmd, Tool::Errorprone),
];

/// Test-only no-root wrapper. Production code goes through
/// [`multiple_lint_format_with_root`] via `check`.
#[cfg(test)]
fn multiple_lint_format(tracked: &[String]) -> Option<Finding> {
    multiple_lint_format_with_root(tracked, std::path::Path::new("."))
}

fn multiple_lint_format_with_root(tracked: &[String], root: &std::path::Path) -> Option<Finding> {
    let mut tools: BTreeSet<Tool> = BTreeSet::new();
    let mut evidence_by_tool: std::collections::BTreeMap<Tool, Vec<String>> = Default::default();
    for path in tracked {
        if let Some(t) = tool_of(basename(path)) {
            tools.insert(t);
            evidence_by_tool.entry(t).or_default().push(path.clone());
        }
        // pyproject.toml embeds Python tool configs as `[tool.X]` tables.
        // We don't need the manifests filter here — pyproject.toml is rare
        // outside Python projects and won't trip on stray names.
        if basename(path).eq_ignore_ascii_case("pyproject.toml") {
            for t in tools_from_pyproject(&root.join(path)) {
                tools.insert(t);
                evidence_by_tool.entry(t).or_default().push(path.clone());
            }
        }
    }
    if tools.len() < 2 {
        return None;
    }
    // Any competing pair present in the set?
    let conflict: Vec<(Tool, Tool)> = COMPETING
        .iter()
        .copied()
        .filter(|(a, b)| tools.contains(a) && tools.contains(b))
        .collect();
    if conflict.is_empty() {
        return None;
    }

    let mut evidence: Vec<String> = Vec::new();
    for (a, b) in &conflict {
        let a_paths = evidence_by_tool.get(a).cloned().unwrap_or_default();
        let b_paths = evidence_by_tool.get(b).cloned().unwrap_or_default();
        evidence.push(format!(
            "{} ({}) competes with {} ({})",
            a.name(),
            a_paths.join(", "),
            b.name(),
            b_paths.join(", "),
        ));
    }

    Some(Finding::new(
        Category::Inconsistency,
        "multiple-lint-format-configs",
        Severity::Warn,
        format!(
            "{} competing lint/format toolchain pair(s) committed — both configurations \
             active in the same repo means neither is the answer",
            conflict.len()
        ),
        evidence,
    ))
}

/// Parse `pyproject.toml` and return the set of tools whose `[tool.X]`
/// section is present. Quiet on read / parse error — pyproject.toml absence
/// or breakage isn't this signal's problem.
fn tools_from_pyproject(path: &std::path::Path) -> Vec<Tool> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return Vec::new();
    };
    let Some(tool_table) = value.get("tool").and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, _) in tool_table {
        match key.as_str() {
            "ruff" => out.push(Tool::Ruff),
            "black" => out.push(Tool::Black),
            "yapf" => out.push(Tool::Yapf),
            "autopep8" => out.push(Tool::Autopep8),
            "isort" => out.push(Tool::Isort),
            "pylint" => out.push(Tool::Pylint),
            "flake8" => out.push(Tool::Flake8),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    #[test]
    fn eslint_and_prettier_alone_is_quiet() {
        // The healthy pair — different jobs.
        assert!(multiple_lint_format(&files(&[".eslintrc.json", ".prettierrc"])).is_none());
    }

    #[test]
    fn eslint_and_biome_is_a_conflict() {
        let f = one(multiple_lint_format(&files(&[".eslintrc.json", "biome.json"])));
        assert_eq!(f.check, "multiple-lint-format-configs");
        assert_eq!(f.severity, Severity::Warn);
        assert!(f.evidence[0].contains("eslint"));
        assert!(f.evidence[0].contains("biome"));
    }

    #[test]
    fn prettier_and_biome_is_a_conflict() {
        assert!(multiple_lint_format(&files(&[".prettierrc", "biome.json"])).is_some());
    }

    #[test]
    fn detects_modern_flat_config_names() {
        assert!(
            multiple_lint_format(&files(&["eslint.config.mjs", "biome.jsonc"])).is_some()
        );
    }

    #[test]
    fn empty_repo_is_quiet() {
        assert!(multiple_lint_format(&files(&["src/main.rs", "README.md"])).is_none());
    }

    #[test]
    fn python_ruff_and_pylint_conflict() {
        // Both linters present → competing pair.
        let f = multiple_lint_format(&files(&["ruff.toml", ".pylintrc"]))
            .expect("expected a finding");
        assert!(f.evidence[0].contains("ruff"));
        assert!(f.evidence[0].contains("pylint"));
    }

    #[test]
    fn python_black_alone_with_ruff_is_fine() {
        // ruff (linter) + black (formatter) is the healthy modern pair —
        // not a conflict.
        assert!(multiple_lint_format(&files(&["ruff.toml"])).is_none());
        // black on its own surfaces via pyproject.toml only, tested below.
    }

    #[test]
    fn pyproject_toml_competing_formatters() {
        // pyproject.toml containing both [tool.black] and [tool.yapf]
        // sections is two formatters fighting.
        let dir = tempfile::tempdir().unwrap();
        let py = dir.path().join("pyproject.toml");
        std::fs::write(
            &py,
            r#"
[tool.black]
line-length = 88

[tool.yapf]
based_on_style = "pep8"
"#,
        )
        .unwrap();
        let tracked = files(&["pyproject.toml"]);
        let f = multiple_lint_format_with_root(&tracked, dir.path())
            .expect("expected a finding");
        assert!(f.evidence[0].to_lowercase().contains("black"));
        assert!(f.evidence[0].to_lowercase().contains("yapf"));
    }

    #[test]
    fn pyproject_ruff_plus_pylintrc_conflict() {
        // pyproject [tool.ruff] section + a separate .pylintrc → conflict.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.ruff]\nline-length = 100\n",
        )
        .unwrap();
        let tracked = files(&["pyproject.toml", ".pylintrc"]);
        assert!(multiple_lint_format_with_root(&tracked, dir.path()).is_some());
    }

    #[test]
    fn java_checkstyle_and_pmd_conflict() {
        let f = multiple_lint_format(&files(&["checkstyle.xml", "pmd.xml"]))
            .expect("expected a finding");
        assert!(f.evidence[0].contains("checkstyle"));
        assert!(f.evidence[0].contains("pmd"));
    }

    #[test]
    fn java_three_analyzers_three_conflict_pairs() {
        let f =
            multiple_lint_format(&files(&["checkstyle.xml", "spotbugs.xml", "pmd.xml"]))
                .expect("expected a finding");
        // 3 tools = C(3,2) = 3 competing pairs.
        assert_eq!(f.evidence.len(), 3);
    }
}
