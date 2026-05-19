//! Signal: function-level LOC outliers — single functions that grew past
//! the point a coder can hold in their head.
//!
//! File-level signals (`oversized_growing_files`) miss the most common shape
//! of agent slop: one 600-line `handle()` / `process()` / `main()` function
//! buried in an otherwise-normal file. The fix is "extract small functions";
//! the agent never did because it didn't see itself nesting.
//!
//! AST-driven across the 5 grammars we ship. The body span is measured by
//! tree-sitter (start row → end row of the function's body block), so it's
//! immune to inline-string formatting and macro tricks.

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use tree_sitter::Node;

/// Below this LOC, a function isn't worth surfacing.
const WARN_LOC: usize = 120;
/// Minimum repo-wide count of large functions before we say anything.
/// One 130-line function in an otherwise-fine repo isn't a signal.
const REPO_MIN: usize = 3;
/// Fraction of source files containing a god function below which we stay
/// silent. Django's worst 53 ÷ ~3000 source files ≈ 0.018 is dense enough
/// to be a real long-tail of historical 100+ LOC functions — but no agent
/// piled them all in one session, so it's a known-good shape. Slop repos
/// land at 0.2–1.0+.
const MIN_DENSITY: f64 = 0.02;
/// Density at which the signal escalates to Critical.
const CRIT_DENSITY: f64 = 0.10;
const EVIDENCE_CAP: usize = 25;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    god_function_loc(&ctx.source_files).into_iter().collect()
}

fn god_function_loc(files: &[SourceFile]) -> Option<Finding> {
    let mut hits: Vec<Hit> = Vec::new();
    for file in files {
        let Some(tree) = file.tree.as_ref() else {
            continue;
        };
        walk(tree.root_node(), &mut |node| {
            if let Some(span) = function_body_span(file.language, node) {
                let loc = span.lines;
                if loc >= WARN_LOC {
                    hits.push(Hit {
                        path: file.path.clone(),
                        line: span.start_line,
                        name: function_name(file.language, node, file).unwrap_or("<anon>".into()),
                        loc,
                    });
                }
            }
        });
    }
    if hits.len() < REPO_MIN {
        return None;
    }
    // Density gate: how many source files have at least one god function?
    // Mature OSS accumulates 100+ LOC functions over decades — the slop
    // signature is *concentration*, not raw count.
    let by_file_count = hits
        .iter()
        .map(|h| h.path.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let density = by_file_count as f64 / files.len().max(1) as f64;
    if density < MIN_DENSITY {
        return None;
    }
    hits.sort_by(|a, b| b.loc.cmp(&a.loc).then(a.path.cmp(&b.path)));
    let severity = if density >= CRIT_DENSITY {
        Severity::Critical
    } else {
        Severity::Warn
    };
    let total = hits.len();
    let by_file = by_file_count;
    let shown: Vec<String> = hits
        .into_iter()
        .take(EVIDENCE_CAP)
        .map(|h| format!("{}:{} {} ({} lines)", h.path, h.line, h.name, h.loc))
        .collect();
    Some(Finding::new(
        Category::Complexity,
        "god-function-loc",
        severity,
        format!(
            "{total} function(s) over {WARN_LOC} lines across {by_file} file(s) — \
             not refactored; agents pile features into the same function",
        ),
        shown,
    ))
}

struct Hit {
    path: String,
    line: usize,
    name: String,
    loc: usize,
}

struct Span {
    start_line: usize,
    lines: usize,
}

/// If `node` is a function-like declaration with a body, return the body's
/// span. Otherwise `None`.
fn function_body_span(lang: Language, node: Node) -> Option<Span> {
    if !is_function_node(lang, node.kind()) {
        return None;
    }
    let body = function_body(node)?;
    let start = body.start_position().row;
    let end = body.end_position().row;
    // `end >= start` always for valid AST; saturating_sub keeps the maths
    // honest if the grammar ever ships a degenerate node.
    Some(Span {
        start_line: node.start_position().row + 1,
        lines: end.saturating_sub(start) + 1,
    })
}

fn is_function_node(lang: Language, kind: &str) -> bool {
    match lang {
        Language::Ts | Language::Js => matches!(
            kind,
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "generator_function_declaration"
                | "generator_function"
        ),
        Language::Python => kind == "function_definition",
        Language::Rust => kind == "function_item",
        Language::Go => matches!(kind, "function_declaration" | "method_declaration"),
        _ => false,
    }
}

fn function_body(node: Node) -> Option<Node> {
    node.child_by_field_name("body")
}

fn function_name(lang: Language, node: Node, file: &SourceFile) -> Option<String> {
    // Most grammars expose `name` as a field on the function declaration.
    // Arrow functions in TS/JS don't have a name field — for those we walk
    // up to the variable_declarator (one parent up).
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(file.bytes()).ok().map(String::from);
    }
    if matches!(lang, Language::Ts | Language::Js) {
        // Anonymous arrow: `const foo = () => { … }` — the name is on the
        // declarator.
        let parent = node.parent()?;
        if matches!(
            parent.kind(),
            "variable_declarator" | "pair" | "assignment_expression"
        ) {
            let n = parent
                .child_by_field_name("name")
                .or_else(|| parent.child_by_field_name("key"))
                .or_else(|| parent.child_by_field_name("left"))?;
            return n.utf8_text(file.bytes()).ok().map(String::from);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::source::test_helpers::mk as f;

    fn long_function(lang: Language, lines: usize) -> String {
        match lang {
            Language::Ts | Language::Js => {
                let body = "  let x = 1;\n".repeat(lines.saturating_sub(2));
                format!("function huge() {{\n{body}}}\n")
            }
            Language::Python => {
                let body = "    x = 1\n".repeat(lines.saturating_sub(1));
                format!("def huge():\n{body}\n")
            }
            Language::Rust => {
                let body = "    let x = 1;\n".repeat(lines.saturating_sub(2));
                format!("fn huge() {{\n{body}}}\n")
            }
            Language::Go => {
                let body = "    x := 1\n".repeat(lines.saturating_sub(2));
                format!("func huge() {{\n{body}}}\n")
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn three_huge_functions_flags() {
        let big = long_function(Language::Ts, 150);
        let files = vec![
            f("src/a.ts", Language::Ts, &big),
            f("src/b.ts", Language::Ts, &big),
            f("src/c.ts", Language::Ts, &big),
        ];
        let finding = god_function_loc(&files).expect("expected a finding");
        assert_eq!(finding.check, "god-function-loc");
    }

    #[test]
    fn small_functions_are_quiet() {
        let small = "function ok() {\n  return 1;\n}\n".repeat(20);
        let files = vec![f("src/a.ts", Language::Ts, &small)];
        assert!(god_function_loc(&files).is_none());
    }

    #[test]
    fn python_huge_def_flags() {
        let big = long_function(Language::Python, 200);
        let files = vec![
            f("a.py", Language::Python, &big),
            f("b.py", Language::Python, &big),
            f("c.py", Language::Python, &big),
        ];
        assert!(god_function_loc(&files).is_some());
    }

    #[test]
    fn rust_huge_fn_flags() {
        let big = long_function(Language::Rust, 150);
        let files = vec![
            f("src/a.rs", Language::Rust, &big),
            f("src/b.rs", Language::Rust, &big),
            f("src/c.rs", Language::Rust, &big),
        ];
        assert!(god_function_loc(&files).is_some());
    }

    #[test]
    fn go_huge_func_flags() {
        let big = long_function(Language::Go, 150);
        let big = format!("package main\n{big}");
        let files = vec![
            f("a.go", Language::Go, &big),
            f("b.go", Language::Go, &big),
            f("c.go", Language::Go, &big),
        ];
        assert!(god_function_loc(&files).is_some());
    }

    #[test]
    fn critical_at_250_lines() {
        let big = long_function(Language::Ts, 280);
        let files = vec![
            f("src/a.ts", Language::Ts, &big),
            f("src/b.ts", Language::Ts, &big),
            f("src/c.ts", Language::Ts, &big),
        ];
        let finding = god_function_loc(&files).expect("expected a finding");
        assert_eq!(finding.severity, Severity::Critical);
    }

    #[test]
    fn languages_without_a_grammar_are_skipped() {
        let big = "void huge() {\n  /* 200 lines */\n}\n".repeat(50);
        let files = vec![f("Foo.java", Language::Java, &big)];
        assert!(god_function_loc(&files).is_none());
    }

    #[test]
    fn anonymous_arrow_uses_declarator_name() {
        let big = format!(
            "const huge = () => {{\n{}\n}};\n",
            "  let x = 1;\n".repeat(140)
        );
        let files = vec![
            f("src/a.ts", Language::Ts, &big),
            f("src/b.ts", Language::Ts, &big),
            f("src/c.ts", Language::Ts, &big),
        ];
        let finding = god_function_loc(&files).expect("expected a finding");
        // Evidence should pick up the const-name "huge", not "<anon>".
        assert!(finding.evidence[0].contains("huge"));
    }
}
