//! Source-file loader. Content-scanning signals (suppressed checks, stub
//! returns, swallowed errors, narrator comments, …) all need to read the
//! same set of files. Reading once up front keeps signals pure functions of
//! the [`AuditContext`] — same pattern as `tracked` / `commits` /
//! `blob_history`.
//!
//! Filtering is conservative:
//! - tracked-by-git (skip the untracked working tree)
//! - extension we recognize as source (so we know the comment / suppression
//!   syntax to look for; unknown extensions get [`Language::Other`] only when
//!   they're clearly text — currently nothing routes through `Other`)
//! - not generated / vendored / fixture (reuses [`is_generated_or_fixture`])
//! - under [`MAX_FILE_BYTES`] (huge files are almost always generated)
//! - no NUL byte in the first kilobyte (binary guard)
//! - valid UTF-8
//!
//! Tree-sitter trees are attached lazily on load for the five languages
//! whose grammars we ship (TS/TSX, JS/JSX, Python, Rust, Go). Structural
//! signals walk the tree; other signals stay on regex.

use crate::audit::util::is_generated_or_fixture;
use std::collections::HashMap;
use std::path::Path;
use tree_sitter::{Parser, Tree};

/// Skip files bigger than this. Hand-written source past 256 KB is rare;
/// past that it's bundles, minified JS, JSON dumps, fixtures.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Source language. Drives which suppression / stub / catch patterns a signal
/// applies. Comment-style alone isn't enough — `// @ts-ignore` is meaningless
/// in JS even though both use `//`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Ts,
    Js,
    Python,
    Rust,
    Go,
    Java,
    Kotlin,
    Scala,
    Ruby,
    Php,
    Csharp,
    Cpp,
    C,
    Swift,
    Shell,
}

impl Language {
    /// Map a lowercase extension to a language. `None` for unknown.
    pub fn from_ext(ext: &str) -> Option<Self> {
        Some(match ext {
            "ts" | "tsx" | "mts" | "cts" => Language::Ts,
            "js" | "jsx" | "mjs" | "cjs" => Language::Js,
            "py" | "pyi" => Language::Python,
            "rs" => Language::Rust,
            "go" => Language::Go,
            "java" => Language::Java,
            "kt" | "kts" => Language::Kotlin,
            "scala" | "sc" => Language::Scala,
            "rb" => Language::Ruby,
            "php" => Language::Php,
            "cs" => Language::Csharp,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Language::Cpp,
            "c" | "h" => Language::C,
            "swift" => Language::Swift,
            "sh" | "bash" | "zsh" => Language::Shell,
            _ => return None,
        })
    }

    /// True for ECMAScript-family languages (TS + JS variants). Many
    /// suppressions (`@ts-ignore`, `eslint-disable`) and idioms
    /// (`it.skip`, `try { … } catch (e) { }`) span both.
    pub fn is_ecmascript(self) -> bool {
        matches!(self, Language::Ts | Language::Js)
    }
}

/// A loaded source file. `content` is the raw UTF-8; signals walk it line by
/// line (most patterns are line-shaped). `tree` is the tree-sitter parse,
/// set for the five grammars we ship and `None` otherwise.
#[derive(Debug)]
pub struct SourceFile {
    pub path: String,
    pub content: String,
    pub language: Language,
    pub tree: Option<Tree>,
}

impl SourceFile {
    /// Iterate over `(1-based-line-number, line-content)`. Stripped of the
    /// trailing `\n` so callers don't have to.
    pub fn lines(&self) -> impl Iterator<Item = (usize, &str)> {
        self.content
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l))
    }

    /// Source bytes for tree-sitter `node.utf8_text(...)` lookups.
    pub fn bytes(&self) -> &[u8] {
        self.content.as_bytes()
    }
}

/// Tree-sitter [`tree_sitter::Language`] for the source-file language, or
/// `None` if we don't ship a grammar for it. Public so signals can ask
/// "would I have an AST here?" without depending on internal load code.
pub fn ts_language(lang: Language) -> Option<tree_sitter::Language> {
    Some(match lang {
        Language::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Js => tree_sitter_javascript::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    })
}

/// TSX vs TS: TSX needs a different grammar variant. We pick by file
/// extension once at load time.
fn ts_language_for_path(lang: Language, path: &str) -> Option<tree_sitter::Language> {
    match lang {
        Language::Ts if path.ends_with(".tsx") => {
            Some(tree_sitter_typescript::LANGUAGE_TSX.into())
        }
        Language::Js if path.ends_with(".jsx") => {
            // tree-sitter-javascript handles JSX in its single grammar.
            Some(tree_sitter_javascript::LANGUAGE.into())
        }
        other => ts_language(other),
    }
}

/// Read every recognized-source tracked file into memory.
///
/// Quiet on per-file failure: missing files, permission errors, oversize, and
/// invalid UTF-8 all silently skip rather than aborting the whole audit.
/// Slop signals are diagnostic — one unreadable file shouldn't kill the run.
pub fn load_all(root: &Path, tracked: &[String]) -> Vec<SourceFile> {
    let mut out = Vec::with_capacity(tracked.len() / 4);
    // One parser per (language, jsx/tsx-variant) reused across files. New
    // parsers are expensive enough that doing this saves real time on big
    // repos (~20% on 7000-file django).
    let mut parsers: HashMap<&'static str, Parser> = HashMap::new();
    for path in tracked {
        if is_generated_or_fixture(path) {
            continue;
        }
        let Some(ext) = ext_of(path) else {
            continue;
        };
        let Some(language) = Language::from_ext(&ext.to_ascii_lowercase()) else {
            continue;
        };

        let abs = root.join(path);
        let Ok(meta) = std::fs::metadata(&abs) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&abs) else {
            continue;
        };
        // Binary heuristic: a NUL in the first kilobyte. Text source never
        // has one; bundled binaries / sqlite dumps misnamed `.js` do.
        if bytes.iter().take(1024).any(|&b| b == 0) {
            continue;
        }
        let Ok(content) = String::from_utf8(bytes) else {
            continue;
        };

        let tree = parse_tree(&mut parsers, language, path, &content);
        out.push(SourceFile {
            path: path.clone(),
            content,
            language,
            tree,
        });
    }
    out
}

fn parse_tree(
    parsers: &mut HashMap<&'static str, Parser>,
    language: Language,
    path: &str,
    content: &str,
) -> Option<Tree> {
    let ts_lang = ts_language_for_path(language, path)?;
    // Key the parser cache by language+variant so TS and TSX get separate
    // parsers (they have different grammars internally).
    let key = parser_key(language, path);
    let parser = parsers.entry(key).or_insert_with(|| {
        let mut p = Parser::new();
        // Setting a known-good language can only fail if the parser ABI
        // mismatches the grammar — that's a build-time invariant, so we
        // unwrap here and let it panic loudly if it ever drifts.
        p.set_language(&ts_lang).expect("tree-sitter grammar ABI mismatch");
        p
    });
    parser.parse(content, None)
}

fn parser_key(lang: Language, path: &str) -> &'static str {
    match lang {
        Language::Ts if path.ends_with(".tsx") => "tsx",
        Language::Ts => "ts",
        Language::Js => "js",
        Language::Python => "python",
        Language::Rust => "rust",
        Language::Go => "go",
        _ => "other",
    }
}

fn ext_of(path: &str) -> Option<&str> {
    let last_slash = path.rfind('/').map(|i| i + 1).unwrap_or(0);
    let name = &path[last_slash..];
    name.rfind('.').map(|i| &name[i + 1..])
}

/// Helpers shared by signal unit tests. Builds in-memory [`SourceFile`]s
/// with their parse trees so AST-based signals can be exercised without
/// touching the filesystem.
#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;

    /// Build a [`SourceFile`] with parsed tree (if a grammar exists for the
    /// language) directly from a string. Used by signal tests.
    pub(crate) fn mk(path: &str, language: Language, content: &str) -> SourceFile {
        let tree = ts_language_for_path(language, path).and_then(|l| {
            let mut p = Parser::new();
            p.set_language(&l).expect("grammar ABI");
            p.parse(content, None)
        });
        SourceFile {
            path: path.to_string(),
            content: content.to_string(),
            language,
            tree,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn mk_tree(files: &[(&str, &[u8])]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (p, bytes) in files {
            let full = dir.path().join(p);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, bytes).unwrap();
        }
        dir
    }

    #[test]
    fn ext_of_handles_paths() {
        assert_eq!(ext_of("a/b/c.rs"), Some("rs"));
        assert_eq!(ext_of("foo.ts"), Some("ts"));
        assert_eq!(ext_of("Makefile"), None);
        assert_eq!(ext_of(""), None);
    }

    #[test]
    fn loads_recognized_source_files() {
        let dir = mk_tree(&[
            ("src/main.rs", b"fn main() {}"),
            ("lib/foo.ts", b"export const x = 1;"),
        ]);
        let tracked = vec!["src/main.rs".into(), "lib/foo.ts".into()];
        let loaded = load_all(dir.path(), &tracked);
        assert_eq!(loaded.len(), 2);
        let by_path: std::collections::HashMap<_, _> =
            loaded.iter().map(|s| (s.path.as_str(), s.language)).collect();
        assert_eq!(by_path["src/main.rs"], Language::Rust);
        assert_eq!(by_path["lib/foo.ts"], Language::Ts);
    }

    #[test]
    fn skips_generated_and_unknown_extensions() {
        let dir = mk_tree(&[
            ("node_modules/dep/index.js", b"x"),
            ("README.md", b"x"),
            ("src/main.rs", b"fn main() {}"),
        ]);
        let tracked = vec![
            "node_modules/dep/index.js".into(),
            "README.md".into(),
            "src/main.rs".into(),
        ];
        let loaded = load_all(dir.path(), &tracked);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, "src/main.rs");
    }

    #[test]
    fn skips_oversize_and_binary() {
        let huge = vec![b'a'; (MAX_FILE_BYTES + 1) as usize];
        let binary = b"some text\x00binary".to_vec();
        let dir = mk_tree(&[
            ("src/big.ts", &huge),
            ("src/blob.js", &binary),
            ("src/ok.py", b"print('hi')"),
        ]);
        let tracked = vec!["src/big.ts".into(), "src/blob.js".into(), "src/ok.py".into()];
        let loaded = load_all(dir.path(), &tracked);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, "src/ok.py");
    }

    #[test]
    fn missing_files_are_silently_skipped() {
        let dir = mk_tree(&[("src/main.rs", b"fn main() {}")]);
        // ghost.rs is "tracked" but doesn't exist on disk — load_all must
        // not panic or bail, it just drops it.
        let tracked = vec!["src/main.rs".into(), "src/ghost.rs".into()];
        let loaded = load_all(dir.path(), &tracked);
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn line_iter_is_one_based() {
        let f = SourceFile {
            path: "x.rs".into(),
            content: "first\nsecond\nthird\n".into(),
            language: Language::Rust,
            tree: None,
        };
        let lines: Vec<_> = f.lines().collect();
        assert_eq!(lines, vec![(1, "first"), (2, "second"), (3, "third")]);
    }

    #[test]
    fn loaded_files_have_a_tree_for_supported_languages() {
        let dir = mk_tree(&[
            ("src/main.rs", b"fn main() { let x = 1; }"),
            ("src/foo.ts", b"export const x: number = 1;"),
            ("src/foo.tsx", b"export const X = () => <div/>;"),
            ("src/foo.py", b"x = 1\n"),
            ("src/foo.go", b"package main\nfunc main() {}"),
        ]);
        let tracked: Vec<String> = ["src/main.rs", "src/foo.ts", "src/foo.tsx", "src/foo.py", "src/foo.go"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let loaded = load_all(dir.path(), &tracked);
        for f in &loaded {
            assert!(f.tree.is_some(), "{} should have a tree", f.path);
        }
        // The TS and TSX files use different grammars internally — the
        // cache should hold both.
        let tsx = loaded.iter().find(|f| f.path == "src/foo.tsx").unwrap();
        let tsx_root = tsx.tree.as_ref().unwrap().root_node();
        assert!(!tsx_root.has_error(), "TSX should parse cleanly");
    }
}
