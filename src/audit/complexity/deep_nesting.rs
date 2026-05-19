//! Signal: deeply nested control flow — `if` inside `for` inside `while`
//! inside `try` inside `if`. The agent kept adding edge cases on top of
//! edge cases instead of extracting helpers.
//!
//! AST-driven. For each function body we walk down, incrementing depth on
//! any block-bearing control-flow node, and remember the deepest path. A
//! function whose deepest path exceeds the threshold makes the evidence
//! list.
//!
//! Five-or-more levels of nesting is a recognised smell across most style
//! guides (SonarQube's cognitive-complexity rule defaults to 4). We warn
//! at 5, crit at 7. Excluded constructs: function definitions reset the
//! depth count (a nested function isn't part of the outer function's
//! cognitive load).

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use tree_sitter::Node;

/// At or above this nesting depth, a function makes the list. 5 levels is
/// the SonarQube-equivalent "cognitive complexity" threshold.
const WARN_DEPTH: u32 = 5;
const REPO_MIN: usize = 3;
/// Same density-based severity as `god_function_loc`. React + Django have
/// real 5+ deep functions because real engineering sometimes requires
/// them; what marks slop is having a high *fraction* of source files with
/// one.
const MIN_DENSITY: f64 = 0.02;
const CRIT_DENSITY: f64 = 0.10;
const EVIDENCE_CAP: usize = 25;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    deep_nesting(&ctx.source_files).into_iter().collect()
}

fn deep_nesting(files: &[SourceFile]) -> Option<Finding> {
    let mut hits: Vec<Hit> = Vec::new();
    for file in files {
        let Some(tree) = file.tree.as_ref() else {
            continue;
        };
        find_functions(tree.root_node(), file, &mut |func_node, name| {
            let depth = max_depth(func_node, file.language);
            if depth >= WARN_DEPTH {
                hits.push(Hit {
                    path: file.path.clone(),
                    line: func_node.start_position().row + 1,
                    name,
                    depth,
                });
            }
        });
    }
    if hits.len() < REPO_MIN {
        return None;
    }
    let by_file = hits
        .iter()
        .map(|h| h.path.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let density = by_file as f64 / files.len().max(1) as f64;
    if density < MIN_DENSITY {
        return None;
    }
    hits.sort_by(|a, b| b.depth.cmp(&a.depth).then(a.path.cmp(&b.path)));
    let severity = if density >= CRIT_DENSITY {
        Severity::Critical
    } else {
        Severity::Warn
    };
    let total = hits.len();
    let shown: Vec<String> = hits
        .into_iter()
        .take(EVIDENCE_CAP)
        .map(|h| format!("{}:{} {} (depth {})", h.path, h.line, h.name, h.depth))
        .collect();
    Some(Finding::new(
        Category::Complexity,
        "deep-nesting",
        severity,
        format!(
            "{total} function(s) with nesting depth ≥{WARN_DEPTH} across {by_file} \
             file(s) — control flow that nobody can read"
        ),
        shown,
    ))
}

struct Hit {
    path: String,
    line: usize,
    name: String,
    depth: u32,
}

/// Walk the tree, find every function-like node, and pass it to `cb` with
/// its declared name (or `<anon>`).
fn find_functions<'a>(node: Node<'a>, file: &SourceFile, cb: &mut impl FnMut(Node<'a>, String)) {
    if is_function_node(file.language, node.kind()) {
        let name = function_name(file.language, node, file).unwrap_or_else(|| "<anon>".into());
        cb(node, name);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_functions(child, file, cb);
    }
}

/// Maximum nesting depth of block-bearing control-flow nodes inside the
/// function's body. The function header itself is depth 0; its body's
/// direct `if`/`for`/`while`/… are depth 1; nested constructs are depth
/// 2+. Nested function definitions reset the count (they're a different
/// reader's-load).
fn max_depth(func_node: Node, lang: Language) -> u32 {
    let Some(body) = func_node.child_by_field_name("body") else {
        return 0;
    };
    descend(body, lang, 0)
}

fn descend(node: Node, lang: Language, current: u32) -> u32 {
    let mut max_seen = current;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // A nested function is its own scope — don't blame the outer
        // function for whatever depth happens inside.
        if is_function_node(lang, child.kind()) {
            continue;
        }
        let next = if is_nesting_node(lang, child.kind()) {
            current + 1
        } else {
            current
        };
        let d = descend(child, lang, next);
        if d > max_seen {
            max_seen = d;
        }
    }
    max_seen
}

fn is_nesting_node(lang: Language, kind: &str) -> bool {
    match lang {
        Language::Ts | Language::Js => matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "for_in_statement"
                | "for_of_statement"
                | "while_statement"
                | "do_statement"
                | "try_statement"
                | "switch_statement"
        ),
        Language::Python => matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "while_statement"
                | "try_statement"
                | "with_statement"
                | "match_statement"
        ),
        Language::Rust => matches!(
            kind,
            "if_expression"
                | "for_expression"
                | "while_expression"
                | "loop_expression"
                | "match_expression"
                | "while_let_expression"
        ),
        Language::Go => matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "expression_switch_statement"
                | "type_switch_statement"
                | "select_statement"
        ),
        _ => false,
    }
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

fn function_name(lang: Language, node: Node, file: &SourceFile) -> Option<String> {
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(file.bytes()).ok().map(String::from);
    }
    if matches!(lang, Language::Ts | Language::Js) {
        let parent = node.parent()?;
        if matches!(parent.kind(), "variable_declarator" | "pair") {
            let n = parent
                .child_by_field_name("name")
                .or_else(|| parent.child_by_field_name("key"))?;
            return n.utf8_text(file.bytes()).ok().map(String::from);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::source::test_helpers::mk as f;

    #[test]
    fn shallow_function_is_quiet() {
        let big = "function ok(x) {\n  if (x > 0) {\n    return x;\n  }\n  return 0;\n}\n"
            .repeat(20);
        let files = vec![f("src/a.ts", Language::Ts, &big)];
        assert!(deep_nesting(&files).is_none());
    }

    #[test]
    fn ts_deeply_nested_flags() {
        // 5 levels: function body → if → for → while → try → if.
        let src = "function bad(xs: number[]) {\n\
                       if (xs.length > 0) {\n\
                         for (const x of xs) {\n\
                           while (x > 0) {\n\
                             try {\n\
                               if (x % 2 === 0) {\n\
                                 console.log(x);\n\
                               }\n\
                             } catch {}\n\
                           }\n\
                         }\n\
                       }\n\
                     }\n";
        let files = vec![
            f("src/a.ts", Language::Ts, src),
            f("src/b.ts", Language::Ts, src),
            f("src/c.ts", Language::Ts, src),
        ];
        let finding = deep_nesting(&files).expect("expected a finding");
        assert_eq!(finding.check, "deep-nesting");
    }

    #[test]
    fn python_deeply_nested_flags() {
        let src = "def bad(xs):\n\
                   \x20\x20if xs:\n\
                   \x20\x20\x20\x20for x in xs:\n\
                   \x20\x20\x20\x20\x20\x20while x > 0:\n\
                   \x20\x20\x20\x20\x20\x20\x20\x20try:\n\
                   \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if x % 2 == 0:\n\
                   \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20print(x)\n\
                   \x20\x20\x20\x20\x20\x20\x20\x20except Exception:\n\
                   \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20pass\n";
        let files = vec![
            f("a.py", Language::Python, src),
            f("b.py", Language::Python, src),
            f("c.py", Language::Python, src),
        ];
        assert!(deep_nesting(&files).is_some());
    }

    #[test]
    fn rust_deeply_nested_flags() {
        let src = "fn bad(xs: &[i32]) {\n\
                       if !xs.is_empty() {\n\
                         for x in xs {\n\
                           while *x > 0 {\n\
                             match x % 2 {\n\
                               0 => if true { let _ = x; },\n\
                               _ => {},\n\
                             }\n\
                           }\n\
                         }\n\
                       }\n\
                     }\n";
        let files = vec![
            f("src/a.rs", Language::Rust, src),
            f("src/b.rs", Language::Rust, src),
            f("src/c.rs", Language::Rust, src),
        ];
        assert!(deep_nesting(&files).is_some());
    }

    #[test]
    fn nested_function_does_not_count() {
        // Outer function's body contains a single nested function which is
        // itself deeply nested. The OUTER function's depth should be 0
        // (we crossed into a new function scope).
        let src = "function outer() {\n\
                     function inner() {\n\
                       if (a) {\n\
                         for (x of xs) {\n\
                           while (y) {\n\
                             try {\n\
                               if (z) {\n\
                                 console.log();\n\
                               }\n\
                             } catch {}\n\
                           }\n\
                         }\n\
                       }\n\
                     }\n\
                   }\n";
        let files = vec![f("src/a.ts", Language::Ts, src)];
        // Only the `inner` function is deeply nested — that's 1 hit, below
        // REPO_MIN, so no finding.
        let only_inner = deep_nesting(&files);
        assert!(only_inner.is_none());
    }

    #[test]
    fn critical_at_depth_7() {
        let src = "function bad() {\n\
                     if (a) {\n\
                       if (b) {\n\
                         if (c) {\n\
                           if (d) {\n\
                             if (e) {\n\
                               if (f) {\n\
                                 if (g) {\n\
                                   console.log();\n\
                                 }\n\
                               }\n\
                             }\n\
                           }\n\
                         }\n\
                       }\n\
                     }\n\
                   }\n";
        let files = vec![
            f("src/a.ts", Language::Ts, src),
            f("src/b.ts", Language::Ts, src),
            f("src/c.ts", Language::Ts, src),
        ];
        let finding = deep_nesting(&files).expect("expected a finding");
        assert_eq!(finding.severity, Severity::Critical);
    }
}
