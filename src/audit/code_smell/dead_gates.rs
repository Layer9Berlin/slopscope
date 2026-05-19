//! Signal: dead conditional gates — `if (false)`, `if (true)`, `if (1 === 1)`.
//! Agents reach for these to skip / force a branch when they can't make the
//! real condition work.
//!
//! AST-driven: we walk the tree-sitter parse for each file and inspect the
//! *condition* node of every `if_statement` / `if_expression`. That means we
//! never trip on `if (false)` inside a comment, a regex, or a string literal
//! — the parser has already classified those for us.
//!
//! Idiomatic infinite loops (`while (true)`, `while True:`) are deliberately
//! not flagged: they're how every long-running process / event loop is
//! written in every supported language. Only *if-gated* constants are tells.
//!
//! Only languages with a tree-sitter grammar (TS/TSX, JS/JSX, Python, Rust,
//! Go) participate — the rest are silently skipped.

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use tree_sitter::Node;

const REPO_MIN: usize = 2;
const CRIT_TOTAL: usize = 15;
const EVIDENCE_CAP: usize = 25;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    dead_gates(&ctx.source_files).into_iter().collect()
}

fn dead_gates(files: &[SourceFile]) -> Option<Finding> {
    let mut hits: Vec<(String, usize, String)> = Vec::new();
    for file in files {
        let Some(tree) = file.tree.as_ref() else {
            continue;
        };
        walk(tree.root_node(), &mut |node| {
            if let Some(cond) = condition_if_dead(node, file) {
                let line = cond.start_position().row + 1;
                let snippet = node
                    .utf8_text(file.bytes())
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim();
                hits.push((file.path.clone(), line, trim_to(snippet, 80)));
            }
        });
    }
    if hits.len() < REPO_MIN {
        return None;
    }
    let total = hits.len();
    let severity = if total >= CRIT_TOTAL {
        Severity::Critical
    } else {
        Severity::Warn
    };
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let by_file = hits
        .iter()
        .map(|(p, _, _)| p.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let shown: Vec<String> = hits
        .into_iter()
        .take(EVIDENCE_CAP)
        .map(|(p, l, s)| format!("{p}:{l}: {s}"))
        .collect();
    Some(Finding::new(
        Category::CodeSmell,
        "dead-gates",
        severity,
        format!(
            "{total} hardcoded-condition gate(s) across {by_file} file(s) — \
             `if (false)` / `if (true)` branches the agent left in"
        ),
        shown,
    ))
}

/// If `node` is an `if_*` statement/expression whose condition is constant
/// or trivially reflexive, return the condition node. The condition is
/// returned (not the outer if) so we can report the precise location.
fn condition_if_dead<'a>(node: Node<'a>, file: &SourceFile) -> Option<Node<'a>> {
    let kind = node.kind();
    if !is_if_node(file.language, kind) {
        return None;
    }
    let cond = node.child_by_field_name("condition")?;
    let inner = unwrap_parenthesized(cond);
    if is_constant_truthy_or_falsy(file.language, inner, file)
        || is_reflexive_comparison(inner, file)
    {
        Some(cond)
    } else {
        None
    }
}

fn is_if_node(lang: Language, kind: &str) -> bool {
    match lang {
        Language::Rust => kind == "if_expression",
        _ => kind == "if_statement",
    }
}

/// JS/TS wraps its condition in `parenthesized_expression`; Python / Rust /
/// Go don't. Unwrap one level so the constant check can ignore the wrapper.
fn unwrap_parenthesized<'a>(node: Node<'a>) -> Node<'a> {
    if node.kind() == "parenthesized_expression" {
        if let Some(inner) = node.named_child(0) {
            return inner;
        }
    }
    node
}

/// Is this node the literal `true`/`false` (or Python `True`/`False`/`1`/`0`)?
fn is_constant_truthy_or_falsy(lang: Language, node: Node, file: &SourceFile) -> bool {
    let kind = node.kind();
    match lang {
        Language::Ts | Language::Js => kind == "true" || kind == "false",
        Language::Python => {
            kind == "true"
                || kind == "false"
                || (kind == "integer"
                    && matches!(node.utf8_text(file.bytes()).unwrap_or(""), "0" | "1"))
        }
        Language::Rust => kind == "boolean_literal",
        Language::Go => {
            // Go represents `true`/`false` as identifiers, not literals.
            kind == "true"
                || kind == "false"
                || (kind == "identifier"
                    && matches!(
                        node.utf8_text(file.bytes()).unwrap_or(""),
                        "true" | "false"
                    ))
        }
        _ => false,
    }
}

/// `1 === 1`, `1 == 1`, `x == x` (where both sides are the same literal /
/// identifier). Reflexive on names produces false positives on NaN
/// detection (`x !== x` is the canonical NaN test) — but we only flag
/// equality, not inequality, so that's fine.
fn is_reflexive_comparison(node: Node, file: &SourceFile) -> bool {
    let kind = node.kind();
    let is_compare = matches!(
        kind,
        "binary_expression" | "comparison_operator" | "comparison"
    );
    if !is_compare {
        return false;
    }
    // Find the operator. Different grammars expose it differently:
    //  - JS/TS/Rust/Go: `operator` field of binary_expression
    //  - Python: children include comparison_operator nodes
    let op_text = operator_of(node, file);
    if !matches!(op_text.as_deref(), Some("==" | "===")) {
        return false;
    }
    let lhs = node.named_child(0);
    let rhs = node.named_child(1);
    match (lhs, rhs) {
        (Some(l), Some(r)) => {
            let lt = l.utf8_text(file.bytes()).unwrap_or("").trim();
            let rt = r.utf8_text(file.bytes()).unwrap_or("").trim();
            !lt.is_empty() && lt == rt
        }
        _ => false,
    }
}

fn operator_of(node: Node, file: &SourceFile) -> Option<String> {
    if let Some(op) = node.child_by_field_name("operator") {
        return op.utf8_text(file.bytes()).ok().map(String::from);
    }
    // Python: walk children to find a non-named operator token.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            let t = child.utf8_text(file.bytes()).unwrap_or("");
            if matches!(t, "==" | "===" | "!=" | "!==" | "<" | ">" | "<=" | ">=") {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn walk<'a>(node: Node<'a>, f: &mut impl FnMut(Node<'a>)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, f);
    }
}

fn trim_to(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max - 1;
        while !s.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::source::test_helpers::mk as f;

    #[test]
    fn ecmascript_dead_gates() {
        // 4 conditional constant gates — `while (true)` is deliberately
        // excluded (the idiomatic infinite loop).
        let big = "if (false) { skipMe(); }\n\
                   if (true) { alwaysRun(); }\n\
                   if (1 === 1) { tautology(); }\n\
                   if (1 == 1) { again(); }\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        let finding = dead_gates(&files).expect("expected a finding");
        assert_eq!(finding.check, "dead-gates");
        assert!(finding.summary.contains("4 hardcoded"));
    }

    #[test]
    fn javascript_while_true_is_not_a_dead_gate() {
        let big = "function f() { while (true) { if (done()) break; } }\n\
                   function g() { while(true) { step(); } }\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(dead_gates(&files).is_none());
    }

    #[test]
    fn intentional_flags_are_not_flagged() {
        let big = "if (DEBUG) { /* … */ }\n\
                   if (FEATURE_X) { /* … */ }\n\
                   if (this.ready) { /* … */ }\n\
                   if (x === 1) { /* … */ }\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(dead_gates(&files).is_none());
    }

    #[test]
    fn python_dead() {
        let big = "if True:\n    pass\nif False:\n    pass\nif 1:\n    pass\nif 0:\n    pass\n";
        let files = vec![f("a.py", Language::Python, big)];
        let finding = dead_gates(&files).expect("expected a finding");
        assert_eq!(finding.count.min(4), 4);
    }

    #[test]
    fn python_while_true_is_not_a_dead_gate() {
        let big = "while True:\n    do_work()\nwhile True:\n    do_more()\n";
        let files = vec![f("a.py", Language::Python, big)];
        assert!(dead_gates(&files).is_none());
    }

    #[test]
    fn rust_dead() {
        let big = "fn a() { if true { 1 } else { 2 }; }\n\
                   fn b() { if false { unreachable!() } }\n";
        let files = vec![f("src/lib.rs", Language::Rust, big)];
        assert!(dead_gates(&files).is_some());
    }

    #[test]
    fn go_dead() {
        let big = "package main\nfunc a() { if true { println(\"\") } }\n\
                   func b() { if false { println(\"\") } }\n";
        let files = vec![f("main.go", Language::Go, big)];
        assert!(dead_gates(&files).is_some());
    }

    #[test]
    fn strings_and_comments_are_safe() {
        // The AST won't see this `if (false)` since it's inside a string
        // literal and a comment. Regex-based detection would false-positive;
        // tree-sitter doesn't.
        let big = "// if (false) { skip(); }\n\
                   const s = \"if (false) { skip(); }\";\n\
                   const t = `if (true) { run(); }`;\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(dead_gates(&files).is_none());
    }

    #[test]
    fn below_threshold_is_quiet() {
        let big = "if (false) {}\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(dead_gates(&files).is_none());
    }

    #[test]
    fn languages_without_a_grammar_are_skipped() {
        // Java has no tree; the signal can't see anything.
        let big = "if (false) { skip(); }\n".repeat(20);
        let files = vec![f("Foo.java", Language::Java, &big)];
        assert!(dead_gates(&files).is_none());
    }
}
