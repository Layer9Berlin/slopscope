//! Signal: multiple competing lint/format toolchains in the same project.
//!
//! `eslint` + `prettier` is a healthy pair (lint + format, different jobs).
//! `eslint` + `biome` is two linters fighting for the same job. So is
//! `prettier` + `biome`, or `eslint` + `rome`. When an agent can't decide
//! which tool to use, both end up committed and the repo carries the cost of
//! two configs forever.

use crate::audit::AuditContext;
use crate::audit::util::basename;
use crate::finding::{Category, Finding, Severity};
use std::collections::BTreeSet;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    multiple_lint_format(&ctx.tracked).into_iter().collect()
}

/// Which tool a config file belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tool {
    Eslint,
    Prettier,
    Biome,
    Rome,
    StandardJs,
    XO,
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
        }
    }
}

/// Identify a tool from a config-file *basename*. `.eslintrc`,
/// `.eslintrc.json`, `eslint.config.js`, `eslint.config.mjs`, etc. all map to
/// `Eslint`. Returns `None` for non-config files.
fn tool_of(name: &str) -> Option<Tool> {
    let lower = name.to_ascii_lowercase();
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
    None
}

/// Pairs that overlap in responsibility — flagging one of these means the
/// repo carries two answers to the same question. Eslint + prettier is
/// deliberately not here: lint and format are separate jobs.
const COMPETING: &[(Tool, Tool)] = &[
    (Tool::Eslint, Tool::Biome),
    (Tool::Eslint, Tool::Rome),
    (Tool::Eslint, Tool::StandardJs),
    (Tool::Eslint, Tool::XO),
    (Tool::Prettier, Tool::Biome),
    (Tool::Prettier, Tool::Rome),
    (Tool::Biome, Tool::Rome),
    (Tool::StandardJs, Tool::XO),
];

fn multiple_lint_format(tracked: &[String]) -> Option<Finding> {
    let mut tools: BTreeSet<Tool> = BTreeSet::new();
    let mut evidence_by_tool: std::collections::BTreeMap<Tool, Vec<String>> = Default::default();
    for path in tracked {
        if let Some(t) = tool_of(basename(path)) {
            tools.insert(t);
            evidence_by_tool.entry(t).or_default().push(path.clone());
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
}
