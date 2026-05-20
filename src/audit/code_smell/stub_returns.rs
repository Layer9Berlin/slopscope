//! Signal: `NotImplemented` landmines — functions that compile but explode at
//! runtime. `throw new Error("TODO")`, `raise NotImplementedError("TODO …")`,
//! `unimplemented!()`, `todo!()`, `panic!("TODO")`. Each one is a place the
//! agent stopped before the work was done.
//!
//! AST-driven: we walk every `throw_statement` / `raise_statement` /
//! `macro_invocation` / `panic(…)` call and decide structurally whether the
//! shape is a *stub* or an *abstract method idiom* (which is legitimate and
//! shouldn't fire). Abstract-method classification is done by inspecting the
//! literal message argument; markers like TODO / FIXME / WIP / "not yet"
//! mean "I'm not done", everything else (including `Subclasses must …`)
//! reads as an intentional abstract declaration.
//!
//! Test files are excluded by path — `unimplemented!()` legitimately marks
//! unreachable match arms there.

use crate::audit::source::{Language, SourceFile};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use tree_sitter::Node;

const REPO_MIN: usize = 5;
const CRIT_TOTAL: usize = 25;
const EVIDENCE_CAP: usize = 25;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    stub_returns(&ctx.source_files).into_iter().collect()
}

fn stub_returns(files: &[SourceFile]) -> Option<Finding> {
    let mut hits: Vec<(String, usize, String)> = Vec::new();
    for file in files {
        if is_test_path(&file.path) {
            continue;
        }
        let Some(tree) = file.tree.as_ref() else {
            continue;
        };
        walk(tree.root_node(), &mut |node| {
            if let Some(snippet) = stub_for(file.language, node, file) {
                let line = node.start_position().row + 1;
                hits.push((file.path.clone(), line, trim_to(&snippet, 80)));
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
        "stub-returns",
        severity,
        format!(
            "{total} stub / not-implemented landmine(s) across {by_file} file(s) — \
             functions that compile but throw at runtime"
        ),
        shown,
    ))
}

/// If `node` matches a stub-shaped throw / raise / panic / macro, return the
/// source text of the matching line; otherwise `None`.
fn stub_for(lang: Language, node: Node, file: &SourceFile) -> Option<String> {
    match lang {
        Language::Ts | Language::Js => ecma_stub(node, file),
        Language::Python => python_stub(node, file),
        Language::Rust => rust_stub(node, file),
        Language::Go => go_stub(node, file),
        _ => None,
    }
}

/// `throw new Error("TODO …")` — the message must signal incomplete work.
/// `throw new Error("Not implemented.")` on its own is the abstract-method
/// idiom (React has 16 of these in noop renderers / legacy fallbacks).
fn ecma_stub(node: Node, file: &SourceFile) -> Option<String> {
    if node.kind() != "throw_statement" {
        return None;
    }
    // First named child is the thrown expression.
    let thrown = node.named_child(0)?;
    if thrown.kind() != "new_expression" {
        return None;
    }
    // The constructor — `Error`, `TypeError`, …
    let ctor = thrown.child_by_field_name("constructor")?;
    let ctor_name = ctor.utf8_text(file.bytes()).ok()?;
    if !ctor_name.contains("Error") {
        return None;
    }
    let args = thrown.child_by_field_name("arguments")?;
    let msg = first_string_literal(args, file)?;
    if !has_stub_marker(&msg) {
        return None;
    }
    Some(line_text(node, file))
}

/// Python: `raise NotImplementedError("TODO …")` or `raise NotImplemented`
/// (typo bug — `NotImplemented` is a comparison sentinel, not an exception).
fn python_stub(node: Node, file: &SourceFile) -> Option<String> {
    if node.kind() != "raise_statement" {
        return None;
    }
    // The first named child is the thing being raised (call or identifier).
    let raised = node.named_child(0)?;
    match raised.kind() {
        "identifier" => {
            let name = raised.utf8_text(file.bytes()).ok()?;
            // `raise NotImplemented` (no `Error` suffix) is always a bug.
            if name == "NotImplemented" {
                Some(line_text(node, file))
            } else {
                None
            }
        }
        "call" => {
            let func = raised.child_by_field_name("function")?;
            let name = func.utf8_text(file.bytes()).ok()?;
            if name != "NotImplementedError" {
                return None;
            }
            // Bare `NotImplementedError()` with no message is the abstract
            // method idiom — skip. With a TODO/FIXME/WIP message → stub.
            let args = raised.child_by_field_name("arguments")?;
            let msg = first_string_literal(args, file)?;
            if has_stub_marker(&msg) {
                Some(line_text(node, file))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Rust: `todo!()` / `unimplemented!()` always count; `panic!("TODO …")`
/// counts only with a stub-marker message. tree-sitter-rust doesn't expose
/// `macro` / `arguments` fields on `macro_invocation`; the macro name and
/// the `token_tree` argument list are anonymous named children.
fn rust_stub(node: Node, file: &SourceFile) -> Option<String> {
    if node.kind() != "macro_invocation" {
        return None;
    }
    let mut cursor = node.walk();
    let mut macro_name: Option<String> = None;
    let mut token_tree: Option<Node> = None;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" | "scoped_identifier" if macro_name.is_none() => {
                let raw = child.utf8_text(file.bytes()).unwrap_or("");
                // `std::todo` / `core::panic` → just the trailing name.
                let last = raw.rsplit("::").next().unwrap_or(raw);
                macro_name = Some(last.to_string());
            }
            "token_tree" => {
                token_tree = Some(child);
            }
            _ => {}
        }
    }
    let name = macro_name?;
    match name.as_str() {
        "todo" | "unimplemented" => Some(line_text(node, file)),
        "panic" => {
            let tt = token_tree?;
            let msg = first_string_literal(tt, file)?;
            if has_stub_marker(&msg) {
                Some(line_text(node, file))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Go: `panic("TODO …")` with a stub-marker message.
fn go_stub(node: Node, file: &SourceFile) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    let name = func.utf8_text(file.bytes()).ok()?;
    if name != "panic" {
        return None;
    }
    let args = node.child_by_field_name("arguments")?;
    let msg = first_string_literal(args, file)?;
    if has_stub_marker(&msg) {
        Some(line_text(node, file))
    } else {
        None
    }
}

/// First string literal under `args` (walks one level: the arguments node's
/// direct named children). Returns the *content* with surrounding quotes
/// stripped so marker matching is straightforward.
fn first_string_literal(args: Node, file: &SourceFile) -> Option<String> {
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
        if is_string_node(child.kind()) {
            let raw = child.utf8_text(file.bytes()).ok()?;
            return Some(strip_quotes(raw));
        }
        // Template strings / f-strings / call inside Error(...) — keep
        // walking the immediate children's subtree at depth 2 for the
        // common `new Error(`…`)` case where the template literal is the
        // arg itself but parsed as `template_string`.
        if let Some(inner) = first_string_literal(child, file) {
            return Some(inner);
        }
    }
    None
}

fn is_string_node(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "string_literal"
            | "template_string"
            | "interpreted_string_literal"
            | "raw_string_literal"
            | "concatenated_string"
    )
}

fn strip_quotes(s: &str) -> String {
    // Python strings may have prefix (b, r, f, rb, …); JS template strings
    // are backtick-bounded; Rust strings use double quotes. Be liberal.
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return s.to_string();
    }
    let (start, end) = (bytes[0], bytes[bytes.len() - 1]);
    if matches!(start, b'"' | b'\'' | b'`') && start == end {
        return s[1..s.len() - 1].to_string();
    }
    // Python string with prefix?  `r"…"`, `f"…"`, `rb"…"` — drop everything
    // up to the first quote then drop the trailing quote.
    if let Some(q_pos) = s.find(['"', '\'']) {
        let q = bytes[q_pos];
        if s.ends_with(q as char) {
            return s[q_pos + 1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

fn has_stub_marker(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("todo")
        || lower.contains("fixme")
        || lower.contains("wip")
        || lower.contains("not yet")
        || lower.contains("not done")
        || lower.contains("not implemented yet")
}

fn line_text(node: Node, file: &SourceFile) -> String {
    let start = node.start_position().row;
    file.content
        .lines()
        .nth(start)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn walk<'a>(node: Node<'a>, f: &mut impl FnMut(Node<'a>)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, f);
    }
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    const DIRS: &[&str] = &[
        "tests/",
        "test/",
        "__tests__/",
        "spec/",
        "specs/",
        "e2e/",
        "t/",
    ];
    if DIRS
        .iter()
        .any(|d| lower.starts_with(d) || lower.contains(&format!("/{d}")))
    {
        return true;
    }
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    name.contains(".test.")
        || name.contains(".spec.")
        || name.starts_with("test_")
        || name.ends_with("_test.go")
        || name.ends_with("_test.py")
        || name.ends_with("_test.rs")
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
    fn below_threshold_is_quiet() {
        let files = vec![f(
            "src/a.rs",
            Language::Rust,
            "fn x() { todo!() }\nfn y() { todo!() }\n",
        )];
        assert!(stub_returns(&files).is_none());
    }

    #[test]
    fn rust_macros_caught() {
        let big = "fn a() { todo!() }\n\
                   fn b() { unimplemented!() }\n\
                   fn c() { panic!(\"not implemented yet\") }\n\
                   fn d() { panic!(\"TODO\") }\n\
                   fn e() { todo!(\"feature X\") }\n";
        let files = vec![f("src/lib.rs", Language::Rust, big)];
        let finding = stub_returns(&files).expect("expected a finding");
        // 5 stubs across 1 file.
        assert!(finding.summary.starts_with("5 stub"));
    }

    #[test]
    fn rust_plain_panic_is_not_a_stub() {
        // `panic!("invariant violated")` is not a TODO landmine, just a
        // hard assertion. Without a stub marker we ignore it.
        let big = "fn a() { panic!(\"invariant violated\") }\n".repeat(10);
        let files = vec![f("src/lib.rs", Language::Rust, &big)];
        assert!(stub_returns(&files).is_none());
    }

    #[test]
    fn ts_js_todo_shaped_throws_only() {
        let big = "function a() { throw new Error('TODO: implement this'); }\n\
                   function b() { throw new Error(\"TODO\"); }\n\
                   function c() { throw new Error('FIXME: handle empty case'); }\n\
                   const d = () => { throw new Error('not implemented yet'); };\n\
                   class X { foo() { throw new Error('WIP'); } }\n";
        let files = vec![f("src/api.ts", Language::Ts, big)];
        let finding = stub_returns(&files).expect("expected a finding");
        assert!(finding.evidence.iter().any(|e| e.contains("src/api.ts:")));
    }

    #[test]
    fn ts_js_abstract_throws_not_flagged() {
        // React's `throw new Error('Not implemented.')` pattern.
        let big = "class Base { foo() { throw new Error('Not implemented.'); } }\n\
                   class Other { bar() { throw new Error('NotImplemented'); } }\n\
                   class Third { baz() { throw new Error('unimplemented'); } }\n\
                   function legacy() { throw new Error('Not implemented'); }\n\
                   function abstractFn() { throw new Error('Subclasses must override'); }\n"
            .repeat(3);
        let files = vec![f("src/abstract.ts", Language::Ts, &big)];
        assert!(stub_returns(&files).is_none());
    }

    #[test]
    fn python_stub_only_flags_todo_shaped_not_implemented() {
        let big = "def a(): raise NotImplementedError('TODO: implement this')\n\
                   def b(): raise NotImplementedError('FIXME: handle the empty case')\n\
                   def c(): raise NotImplementedError('not yet implemented')\n\
                   def d(): raise NotImplementedError('WIP')\n\
                   def e(): raise NotImplemented\n";
        let files = vec![f("api.py", Language::Python, big)];
        let finding = stub_returns(&files).expect("expected a finding");
        assert!(finding.summary.starts_with("5 stub"));
    }

    #[test]
    fn python_abstract_methods_are_not_stubs() {
        // The Django / DRF idiom.
        let big = "def a(self):\n    raise NotImplementedError('Subclasses must override foo()')\n\
                   def b(self):\n    raise NotImplementedError('subclasses of Base must provide a get_x method')\n\
                   def c(self):\n    raise NotImplementedError('Override this in your storage backend')\n\
                   def d(self):\n    raise NotImplementedError()\n\
                   def e(self):\n    raise NotImplementedError\n"
            .repeat(3);
        let files = vec![f("base.py", Language::Python, &big)];
        assert!(stub_returns(&files).is_none());
    }

    #[test]
    fn go_panic_not_implemented() {
        let big = "package main\n\
                   func A() { panic(\"TODO\") }\n\
                   func B() { panic(\"TODO: implement\") }\n\
                   func C() { panic(\"not implemented yet\") }\n\
                   func D() { panic(\"WIP\") }\n\
                   func E() { panic(\"FIXME: handle me\") }\n";
        let files = vec![f("api.go", Language::Go, big)];
        let finding = stub_returns(&files).expect("expected a finding");
        assert!(finding.summary.starts_with("5 stub"));
    }

    #[test]
    fn go_panic_real_invariant_is_not_a_stub() {
        let big = "package main\nfunc x() { panic(\"unreachable\") }\n".repeat(10);
        let files = vec![f("api.go", Language::Go, &big)];
        assert!(stub_returns(&files).is_none());
    }

    #[test]
    fn test_paths_are_excluded() {
        let big = "fn a() { todo!() }\n".repeat(10);
        let files = vec![f("tests/api.rs", Language::Rust, &big)];
        assert!(stub_returns(&files).is_none());
    }

    #[test]
    fn comments_and_strings_dont_match() {
        // The text `todo!()` inside a string or comment is parsed as data,
        // not as a macro invocation. Regex would false-positive; AST won't.
        let big = "fn x() { let s = \"todo!() — note\"; s }\n\
                   // todo!() — also a note\n"
            .repeat(10);
        let files = vec![f("src/lib.rs", Language::Rust, &big)];
        assert!(stub_returns(&files).is_none());
    }

    #[test]
    fn critical_threshold_reached() {
        let big = "fn x() { todo!() }\n".repeat(CRIT_TOTAL);
        let files = vec![f("src/lib.rs", Language::Rust, &big)];
        let finding = stub_returns(&files).expect("expected a finding");
        assert_eq!(finding.severity, Severity::Critical);
    }

    #[test]
    fn languages_without_a_grammar_are_skipped() {
        let big = "throw new Error('TODO');\n".repeat(20);
        let files = vec![f("Foo.java", Language::Java, &big)];
        assert!(stub_returns(&files).is_none());
    }
}
