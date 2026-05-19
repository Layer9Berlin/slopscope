//! Signal: swallowed errors — the agent caught an exception and did nothing.
//! `catch (e) {}`, `except: pass`, `if err != nil {}`, `Err(_) => {}`.
//!
//! AST-driven: every `catch_clause` / `except_clause` / `match_arm` /
//! `if_statement` is inspected by walking its body. A body is a *swallow*
//! when it's empty, or when every statement in it is a `pass` / `...` /
//! log call (`console.*`, `print`, `log.*`, `fmt.Println`, `System.out.*`).
//! A body that contains `throw` / `raise` / `return` / `continue` / any
//! real call / variable binding is *not* a swallow.
//!
//! The AST removes a class of regex false positives: `catch` inside a
//! string literal, comment, or template no longer trips, and an empty body
//! followed by chained statements outside the catch can't fool the matcher.

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use tree_sitter::Node;

/// Below this total the signal is silent.
const REPO_MIN: usize = 2;
const CRIT_TOTAL: usize = 20;
const EVIDENCE_CAP: usize = 25;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    swallowed_errors(&ctx.source_files).into_iter().collect()
}

fn swallowed_errors(files: &[SourceFile]) -> Option<Finding> {
    let mut hits: Vec<(String, usize, String)> = Vec::new();
    for file in files {
        let Some(tree) = file.tree.as_ref() else {
            continue;
        };
        walk(tree.root_node(), &mut |node| {
            if let Some(line) = swallow_site(file.language, node, file) {
                hits.push((file.path.clone(), line, snippet_at(node, file)));
            }
        });
    }
    if hits.len() < REPO_MIN {
        return None;
    }
    let total = hits.len();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let by_file = hits
        .iter()
        .map(|(p, _, _)| p.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    // Concentration drives severity. Django has 143 swallows over 91 files
    // — historical idiom, not agent flailing. 8+ in a single file is the
    // tell of a controller / handler the agent panic-wrapped everywhere.
    let max_per_file = max_per_file(&hits);
    let avg_per_file = total as f64 / by_file.max(1) as f64;
    let severity = if max_per_file >= 8 || (total >= CRIT_TOTAL && avg_per_file >= 4.0) {
        Severity::Critical
    } else {
        Severity::Warn
    };
    let shown: Vec<String> = hits
        .into_iter()
        .take(EVIDENCE_CAP)
        .map(|(p, l, s)| format!("{p}:{l}: {s}"))
        .collect();
    Some(Finding::new(
        Category::CodeSmell,
        "swallowed-errors",
        severity,
        format!(
            "{total} swallowed error site(s) across {by_file} file(s) — \
             exceptions caught and discarded, faults will be silent"
        ),
        shown,
    ))
}

/// If `node` is a swallow site, return the 1-based line. Otherwise `None`.
fn swallow_site(lang: Language, node: Node, file: &SourceFile) -> Option<usize> {
    match lang {
        Language::Ts | Language::Js => ecma_catch_swallow(node, file),
        Language::Python => python_except_swallow(node, file),
        Language::Rust => rust_err_arm_swallow(node, file),
        Language::Go => go_err_branch_swallow(node, file),
        _ => None,
    }
}

/// JS / TS: `catch_clause` whose body is empty or all-log.
fn ecma_catch_swallow(node: Node, file: &SourceFile) -> Option<usize> {
    if node.kind() != "catch_clause" {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    if is_swallow_block(body, file, EcmaSwallow) {
        Some(node.start_position().row + 1)
    } else {
        None
    }
}

/// Python: `except_clause` whose body is empty or all-`pass`/`...`/log.
fn python_except_swallow(node: Node, file: &SourceFile) -> Option<usize> {
    if node.kind() != "except_clause" {
        return None;
    }
    // tree-sitter-python: except_clause's body is an unnamed `block` child
    // (no field name). It's always the last named child.
    let mut cursor = node.walk();
    let body = node.named_children(&mut cursor).last()?;
    if body.kind() != "block" {
        return None;
    }
    if is_swallow_block(body, file, PythonSwallow) {
        Some(node.start_position().row + 1)
    } else {
        None
    }
}

/// Rust: a `match_arm` whose pattern is `Err(_)` (or `Err(_)`-like) and
/// whose body is `{}`, `()`, or an empty block.
fn rust_err_arm_swallow(node: Node, file: &SourceFile) -> Option<usize> {
    if node.kind() != "match_arm" {
        return None;
    }
    let pattern = node.child_by_field_name("pattern")?;
    if !is_err_underscore_pattern(pattern, file) {
        return None;
    }
    let value = node.child_by_field_name("value")?;
    if is_rust_empty_value(value, file) {
        Some(node.start_position().row + 1)
    } else {
        None
    }
}

fn is_err_underscore_pattern(node: Node, file: &SourceFile) -> bool {
    // tree-sitter-rust wraps each arm pattern in `match_pattern`. The real
    // pattern is its first named child.
    let inner = if node.kind() == "match_pattern" {
        node.named_child(0).unwrap_or(node)
    } else {
        node
    };
    if inner.kind() != "tuple_struct_pattern" {
        return false;
    }
    let type_node = inner.child_by_field_name("type");
    type_node.and_then(|t| t.utf8_text(file.bytes()).ok()) == Some("Err")
}

fn is_rust_empty_value(node: Node, file: &SourceFile) -> bool {
    match node.kind() {
        "unit_expression" => true,
        "block" => {
            // Empty block `{}` has no named children.
            let mut cursor = node.walk();
            let empty = node.named_children(&mut cursor).next().is_none();
            empty
        }
        _ => {
            // `Err(_) => 0` — the trailing 0 isn't empty.
            let _ = file; // kept for symmetry with other handlers
            false
        }
    }
}

/// Go: `if err != nil { … }` whose block is empty.
fn go_err_branch_swallow(node: Node, file: &SourceFile) -> Option<usize> {
    if node.kind() != "if_statement" {
        return None;
    }
    let cond = node.child_by_field_name("condition")?;
    if !is_go_err_not_nil(cond, file) {
        return None;
    }
    let consequence = node.child_by_field_name("consequence")?;
    if consequence.kind() != "block" {
        return None;
    }
    if is_swallow_block(consequence, file, GoSwallow) {
        Some(node.start_position().row + 1)
    } else {
        None
    }
}

fn is_go_err_not_nil(cond: Node, file: &SourceFile) -> bool {
    if cond.kind() != "binary_expression" {
        return false;
    }
    let op = cond
        .child_by_field_name("operator")
        .and_then(|o| o.utf8_text(file.bytes()).ok());
    if op != Some("!=") {
        return false;
    }
    let lhs = cond.named_child(0).and_then(|n| n.utf8_text(file.bytes()).ok());
    let rhs = cond.named_child(1).and_then(|n| n.utf8_text(file.bytes()).ok());
    matches!((lhs, rhs), (Some("err"), Some("nil")))
}

/// Family-specific knowledge for "is this statement a no-op?"
trait SwallowFamily {
    fn statement_is_no_op(stmt: Node, file: &SourceFile) -> bool;
}
struct EcmaSwallow;
struct PythonSwallow;
struct GoSwallow;

impl SwallowFamily for EcmaSwallow {
    fn statement_is_no_op(stmt: Node, file: &SourceFile) -> bool {
        match stmt.kind() {
            "empty_statement" => true,
            "expression_statement" => {
                let expr = stmt.named_child(0);
                expr.is_some_and(|e| is_ecma_log_call(e, file))
            }
            _ => false,
        }
    }
}

impl SwallowFamily for PythonSwallow {
    fn statement_is_no_op(stmt: Node, file: &SourceFile) -> bool {
        match stmt.kind() {
            "pass_statement" => true,
            "expression_statement" => {
                // Lone `...` (Ellipsis) — the Python equivalent of pass for
                // type stubs and stub bodies.
                let expr = stmt.named_child(0);
                expr.is_some_and(|e| {
                    e.kind() == "ellipsis"
                        || (e.kind() == "string"
                            && stmt
                                .next_named_sibling()
                                .map(|n| n.kind() != "expression_statement")
                                .unwrap_or(true))
                        || is_python_log_call(e, file)
                })
            }
            _ => false,
        }
    }
}

impl SwallowFamily for GoSwallow {
    fn statement_is_no_op(stmt: Node, file: &SourceFile) -> bool {
        match stmt.kind() {
            "expression_statement" | "short_var_declaration" => {
                let expr = stmt.named_child(0);
                expr.is_some_and(|e| is_go_log_call(e, file))
            }
            _ => false,
        }
    }
}

/// True if every named statement inside `block` is a no-op. Comment nodes
/// are skipped — a body that's only `// nothing` is still empty.
fn is_swallow_block<F: SwallowFamily>(block: Node, file: &SourceFile, _family: F) -> bool {
    let mut cursor = block.walk();
    for child in block.named_children(&mut cursor) {
        if child.kind() == "comment" || child.kind() == "line_comment" || child.kind() == "block_comment" {
            continue;
        }
        if !F::statement_is_no_op(child, file) {
            return false;
        }
    }
    true
}

fn is_ecma_log_call(node: Node, file: &SourceFile) -> bool {
    // call_expression where the function is `console.*` / `logger.*` /
    // `log.*` / `print` / `println`.
    if node.kind() != "call_expression" {
        return false;
    }
    let func = node
        .child_by_field_name("function")
        .and_then(|n| n.utf8_text(file.bytes()).ok())
        .unwrap_or("");
    let lower = func.to_ascii_lowercase();
    lower.starts_with("console.")
        || lower.starts_with("logger.")
        || lower.starts_with("log.")
        || lower == "print"
        || lower == "println"
}

fn is_python_log_call(node: Node, file: &SourceFile) -> bool {
    if node.kind() != "call" {
        return false;
    }
    let func = node
        .child_by_field_name("function")
        .and_then(|n| n.utf8_text(file.bytes()).ok())
        .unwrap_or("");
    let lower = func.to_ascii_lowercase();
    lower == "print"
        || lower.starts_with("log.")
        || lower.starts_with("logger.")
        || lower.starts_with("logging.")
}

fn is_go_log_call(node: Node, file: &SourceFile) -> bool {
    if node.kind() != "call_expression" {
        return false;
    }
    let func = node
        .child_by_field_name("function")
        .and_then(|n| n.utf8_text(file.bytes()).ok())
        .unwrap_or("");
    func.starts_with("fmt.Print")
        || func.starts_with("log.")
        || func.starts_with("logger.")
        || func.ends_with(".Println")
        || func.ends_with(".Printf")
}

fn walk<'a>(node: Node<'a>, f: &mut impl FnMut(Node<'a>)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, f);
    }
}

fn max_per_file(hits: &[(String, usize, String)]) -> usize {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (p, _, _) in hits {
        *counts.entry(p.as_str()).or_insert(0) += 1;
    }
    counts.values().copied().max().unwrap_or(0)
}

fn snippet_at(node: Node, file: &SourceFile) -> String {
    let start = node.start_position().row;
    let line = file.content.lines().nth(start).unwrap_or("").trim();
    trim_to(line, 80)
}

fn trim_to(s: &str, max: usize) -> String {
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
    fn empty_catch_same_line() {
        let big = "try { a(); } catch (e) {}\n\
                   try { b(); } catch {}\n\
                   try { c(); } catch (err) { }\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        let finding = swallowed_errors(&files).expect("expected a finding");
        assert_eq!(finding.check, "swallowed-errors");
        assert!(finding.summary.contains("3 swallowed"));
    }

    #[test]
    fn empty_catch_multi_line() {
        let big = "try {\n  a();\n} catch (e) {\n}\n\
                   try {\n  b();\n} catch (e) {\n  // nothing\n}\n\
                   try {\n  c();\n} catch (e) {\n  console.log(e);\n}\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        let finding = swallowed_errors(&files).expect("expected a finding");
        // All 3 swallows: empty, comment-only, console.log-only.
        assert!(finding.summary.contains("3 swallowed"));
    }

    #[test]
    fn rethrowing_or_returning_doesnt_count() {
        let big = "try { a(); } catch (e) { throw e; }\n\
                   try { b(); } catch (e) { return null; }\n\
                   try { c(); } catch (e) { console.log(e); throw e; }\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(swallowed_errors(&files).is_none());
    }

    #[test]
    fn python_except_pass() {
        let big = "try:\n  x()\nexcept:\n  pass\n\
                   try:\n  y()\nexcept Exception:\n  pass\n\
                   try:\n  z()\nexcept (A, B) as e:\n  pass\n";
        let files = vec![f("a.py", Language::Python, big)];
        let finding = swallowed_errors(&files).expect("expected a finding");
        assert!(finding.count >= 2);
    }

    #[test]
    fn python_except_with_real_handler_is_ok() {
        let big = "try:\n  x()\nexcept Exception as e:\n  log(e)\n  raise\n";
        let files = vec![f("a.py", Language::Python, big)];
        assert!(swallowed_errors(&files).is_none());
    }

    #[test]
    fn go_empty_err_branch() {
        let big = "package main\nfunc main() {\n\
                   _, err := f()\nif err != nil {}\n\
                   x, err := g()\nif err != nil {\n}\n\
                   y, err := h()\nif err != nil { log.Println(err) }\n}\n";
        let files = vec![f("main.go", Language::Go, big)];
        let finding = swallowed_errors(&files).expect("expected a finding");
        assert!(finding.count >= 2);
    }

    #[test]
    fn rust_err_arm_with_unit_body() {
        let big = "fn x() -> i32 {\n\
                     match r() {\n  Ok(x) => x,\n  Err(_) => 0,\n  }\n}\n\
                   fn y() -> i32 {\n\
                     match s() {\n  Ok(x) => x,\n  Err(_) => { 0 }\n  }\n}\n\
                   fn z() {\n\
                     match t() {\n  Ok(_) => (),\n  Err(_) => (),\n  }\n}\n\
                   fn w() {\n\
                     match u() {\n  Ok(_) => (),\n  Err(_) => {},\n  }\n}\n";
        // Of the four matches:
        //   x: Err(_) => 0           → NOT a swallow (has value)
        //   y: Err(_) => { 0 }       → NOT a swallow (block has a value)
        //   z: Err(_) => ()          → swallow
        //   w: Err(_) => {}          → swallow
        let files = vec![f("src/lib.rs", Language::Rust, big)];
        let finding = swallowed_errors(&files).expect("expected a finding");
        assert_eq!(finding.count, 2);
    }

    #[test]
    fn under_threshold_is_quiet() {
        let big = "try { a(); } catch (e) {}\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        assert!(swallowed_errors(&files).is_none());
    }

    #[test]
    fn catch_in_comment_or_string_doesnt_trip() {
        // Both occurrences are inside string / comment — AST won't see
        // them as catch_clause nodes.
        let big = "const a = \"try { x } catch (e) {}\";\n\
                   const b = `try { y } catch (e) {}`;\n\
                   // try { z } catch (e) {}\n"
            .repeat(10);
        let files = vec![f("src/a.ts", Language::Ts, &big)];
        assert!(swallowed_errors(&files).is_none());
    }

    #[test]
    fn evidence_has_path_and_line() {
        let big = "\n\ntry { a(); } catch (e) {}\ntry { b(); } catch (e) {}\n";
        let files = vec![f("src/a.ts", Language::Ts, big)];
        let finding = swallowed_errors(&files).expect("expected a finding");
        assert!(finding.evidence[0].contains("src/a.ts:"));
        assert!(finding.evidence[0].contains("catch"));
    }

    #[test]
    fn languages_without_a_grammar_are_skipped() {
        let big = "try { a(); } catch (e) {}\n".repeat(10);
        let files = vec![f("Foo.java", Language::Java, &big)];
        assert!(swallowed_errors(&files).is_none());
    }
}
