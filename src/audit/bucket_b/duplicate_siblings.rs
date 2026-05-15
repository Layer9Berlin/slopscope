//! Signal: near-duplicate sibling files — `parser.ts`, `parser-new.ts`,
//! `parser-final.ts`, `parser-copy.ts` living side by side.
//!
//! When an agent can't make a change in place, it clones the file and edits the
//! copy — leaving a trail of variant siblings instead of one evolving file. A
//! human refactors or deletes the old one; the variant pile is the residue of
//! an agent that couldn't. Pure filename signal, no history needed.
//!
//! Calibration note: an earlier version also stripped a bare trailing digit
//! (`foo2` -> `foo`), but the known-good corpus showed that pattern is
//! dominated by *semantic* numbers in real code — `http2`/`http3`,
//! `ipv4`/`ipv6`, `sslv2`/`sslv3`, `FindLibssh2.cmake`, version-numbered
//! release notes. So we now key only on explicit copy-words (`-new`, `_copy`,
//! `-final`, …) and case-collisions (`Skeleton.tsx` next to `skeleton.tsx`),
//! which had zero false positives on the corpus and still caught the real
//! agent clones.
//!
//! We only group files in the *same directory* with the *same extension* whose
//! stems collapse to the same base once a copy-word suffix is stripped.

use crate::audit::util::{basename, is_generated_or_fixture};
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};
use std::collections::BTreeMap;

/// This many variant groups escalates the finding to Critical.
const CRIT_GROUP_COUNT: usize = 5;

/// Media / asset extensions. This signal is about *code* clones ("refactored
/// in place"); a pile of `image_0.png`, `image_1.png` is an AI-asset dump — a
/// real tell, but a different one — so it does not belong here.
const ASSET_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp", "avif", "mp4", "mov", "webm", "avi",
    "mp3", "wav", "ogg", "pdf", "woff", "woff2", "ttf", "otf", "eot",
];

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    duplicate_siblings(&ctx.tracked).into_iter().collect()
}

/// Split a path into `(dir, stem, ext)`. `ext` is the last `.`-segment of the
/// basename; a dotfile like `.env` has an empty stem and `env` ext, which is
/// fine — it just won't collide with anything.
fn split(path: &str) -> (&str, &str, &str) {
    let dir = match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    };
    let name = basename(path);
    match name.rfind('.') {
        Some(i) if i > 0 => (dir, &name[..i], &name[i + 1..]),
        _ => (dir, name, ""),
    }
}

/// What kind of variant a filename is, w.r.t. the base name of its group.
#[derive(Clone, PartialEq)]
enum Kind {
    /// The stem carries no copy-word suffix — it is itself the potential base.
    Base,
    /// A dash/space copy-word variant (`parser-new`, `spec copy`) — agents
    /// reliably clone with these separators, so a single one is enough.
    Dash,
    /// An underscore copy-word variant (`basebackup_copy`). Underscore is the
    /// standard C / Python / Go module separator, so a *single* underscore
    /// variant is typically a normal module name (postgres `basebackup.c` +
    /// `basebackup_copy.c`). We carry the word so a group can count *distinct*
    /// underscore copy-words and only flag when there are ≥2.
    Underscore(String),
}

/// Collapse a stem to `(base, kind)` — the base it is a variant of, and how it
/// was derived. A stem with no copy-word suffix is its own base; a case-only
/// collision (`Skeleton` vs `skeleton`) falls together because we lowercase.
fn classify(stem: &str) -> (String, Kind) {
    let lower = stem.to_ascii_lowercase();

    // Copy-words: `-new`, `_copy`, ` copy`, `-final`, `-bak`, … Deliberately
    // excludes `-test` (colocated test files), `-v2` (legit API versioning),
    // and `draft` (`gnus-draft.el`, `save-draft.xpm` are semantic "draft
    // email/message" subsystems, not clones — surfaced by the emacs control).
    const WORDS: &[&str] = &[
        "new", "old", "copy", "final", "fixed", "updated", "backup", "bak", "orig", "original",
        "temp", "tmp", "wip",
    ];
    for &(sep, kind_is_dash) in &[('-', true), (' ', true), ('_', false)] {
        if let Some(i) = lower.rfind(sep) {
            let suffix = &lower[i + 1..];
            let base = &lower[..i];
            if !base.is_empty() && WORDS.contains(&suffix) {
                let kind = if kind_is_dash {
                    Kind::Dash
                } else {
                    Kind::Underscore(suffix.to_string())
                };
                return (base.to_string(), kind);
            }
        }
    }
    (lower, Kind::Base)
}

fn duplicate_siblings(tracked: &[String]) -> Option<Finding> {
    // Key: (dir, base-stem, ext) -> members (path, kind).
    let mut groups: BTreeMap<(String, String, String), Vec<(String, Kind)>> = BTreeMap::new();
    for path in tracked {
        // Generated / vendored / fixture paths churn out numbered files by
        // design — that is not an agent failing to refactor.
        if is_generated_or_fixture(path) {
            continue;
        }
        let (dir, stem, ext) = split(path);
        if ASSET_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        let (base, kind) = classify(stem);
        if base.is_empty() {
            continue;
        }
        groups
            .entry((dir.to_string(), base, ext.to_string()))
            .or_default()
            .push((path.clone(), kind));
    }

    let mut hits: Vec<(String, Vec<String>)> = groups
        .into_iter()
        .filter_map(|((dir, base, ext), mut members)| {
            members.sort_by(|a, b| a.0.cmp(&b.0));
            members.dedup_by(|a, b| a.0 == b.0);
            if members.len() < 2 {
                return None;
            }
            // A group is real slop when any of these hold:
            //   * a dash/space copy-word variant exists (`parser-new.ts`),
            //   * ≥2 distinct underscore copy-words (`logs_final.txt` +
            //     `logs_new.txt`),
            //   * ≥2 Base members in the same dir+stem+ext bucket — a
            //     case-collision (`Skeleton.tsx` + `skeleton.tsx`).
            // A lone underscore variant (`basebackup_copy.c`) is a normal
            // C/Python/Go module name, not slop.
            let has_dash_variant = members.iter().any(|(_, k)| *k == Kind::Dash);
            let base_count = members.iter().filter(|(_, k)| *k == Kind::Base).count();
            let distinct_underscore_words: std::collections::HashSet<&str> = members
                .iter()
                .filter_map(|(_, k)| match k {
                    Kind::Underscore(w) => Some(w.as_str()),
                    _ => None,
                })
                .collect();
            if !has_dash_variant && distinct_underscore_words.len() < 2 && base_count < 2 {
                return None;
            }
            let label = if dir.is_empty() {
                format!("{base}.{ext}")
            } else {
                format!("{dir}/{base}.{ext}")
            };
            let files: Vec<String> = members.into_iter().map(|(p, _)| p).collect();
            Some((label, files))
        })
        .collect();

    if hits.is_empty() {
        return None;
    }
    hits.sort();

    let severity = if hits.len() >= CRIT_GROUP_COUNT {
        Severity::Critical
    } else {
        Severity::Warn
    };
    let variant_files: usize = hits.iter().map(|(_, f)| f.len()).sum();
    let evidence: Vec<String> = hits
        .iter()
        .map(|(label, files)| format!("{label}: {}", files.join(", ")))
        .collect();

    Some(Finding::new(
        Category::SteeringFailure,
        "duplicate-siblings",
        severity,
        format!(
            "{} group(s) of near-duplicate sibling files ({variant_files} files) — \
             cloned-and-edited instead of refactored in place",
            hits.len()
        ),
        evidence,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::{files, one};

    #[test]
    fn distinct_files_are_quiet() {
        assert!(duplicate_siblings(&files(&[
            "src/auth.ts",
            "src/user.ts",
            "src/db.ts",
        ]))
        .is_none());
    }

    #[test]
    fn semantic_digits_are_not_clones() {
        // The corpus killer: http2/http3, ipv4/ipv6, sslv2/sslv3 are different
        // things, not copies. We no longer strip bare digits at all.
        assert!(duplicate_siblings(&files(&[
            "docs/http.md",
            "docs/http2.md",
            "docs/http3.md",
            "docs/ipv4.md",
            "docs/ipv6.md",
            "cmake/FindLibssh.cmake",
            "cmake/FindLibssh2.cmake",
        ]))
        .is_none());
    }

    #[test]
    fn word_variants_collapse_to_one_group() {
        let f = one(duplicate_siblings(&files(&[
            "lib/parser.ts",
            "lib/parser-new.ts",
            "lib/parser-final.ts",
            "lib/parser-copy.ts",
        ])));
        assert_eq!(f.check, "duplicate-siblings");
        assert_eq!(f.severity, Severity::Warn);
        assert_eq!(f.count, 1);
        assert!(f.evidence[0].contains("parser.ts"));
        assert!(f.evidence[0].contains("parser-final.ts"));
    }

    #[test]
    fn space_copy_suffix_is_caught() {
        let f = one(duplicate_siblings(&files(&[
            "docs/spec.md",
            "docs/spec copy.md",
        ])));
        assert_eq!(f.count, 1);
    }

    #[test]
    fn case_collision_is_caught() {
        // `Skeleton.tsx` next to `skeleton.tsx` — the agent re-created a file it
        // couldn't see was already there (case-insensitive filesystem tell).
        let f = one(duplicate_siblings(&files(&[
            "src/ui/Skeleton.tsx",
            "src/ui/skeleton.tsx",
        ])));
        assert_eq!(f.count, 1);
    }

    #[test]
    fn different_directories_do_not_collide() {
        // Same name in two dirs is normal project structure, not a clone.
        assert!(duplicate_siblings(&files(&[
            "a/index.ts",
            "b/index.ts",
            "c/index.ts",
        ]))
        .is_none());
    }

    #[test]
    fn different_extensions_do_not_collide() {
        // foo.ts + foo.css is a component pair, not a duplicate.
        assert!(duplicate_siblings(&files(&["ui/button.ts", "ui/button.css"])).is_none());
    }

    #[test]
    fn five_groups_is_critical() {
        let bases = ["alpha", "beta", "gamma", "delta", "omega"];
        let paths: Vec<String> = bases
            .iter()
            .flat_map(|b| [format!("src/{b}.ts"), format!("src/{b}-copy.ts")])
            .collect();
        let f = one(duplicate_siblings(&paths));
        assert_eq!(f.severity, Severity::Critical);
        assert_eq!(f.count, 5);
    }

    #[test]
    fn lone_underscore_variant_is_not_a_clone() {
        // Postgres surfaced this: `basebackup.c` + `basebackup_copy.c` is a
        // normal C module-naming convention, not an agent clone. Same with
        // hugo's `commands/hugo.md` + `commands/hugo_new.md`.
        assert!(duplicate_siblings(&files(&[
            "src/backend/backup/basebackup.c",
            "src/backend/backup/basebackup_copy.c",
            "src/include/optimizer/geqo.h",
            "src/include/optimizer/geqo_copy.h",
            "docs/commands/hugo.md",
            "docs/commands/hugo_new.md",
        ]))
        .is_none());
    }

    #[test]
    fn multiple_underscore_copy_words_still_flag() {
        // liquid-ai's `backend_logs.txt` + `backend_logs_final.txt` +
        // `backend_logs_new.txt`: two distinct underscore copy-words (final +
        // new) → a real "I made copies, picked one" pile.
        let f = one(duplicate_siblings(&files(&[
            "logs/backend_logs.txt",
            "logs/backend_logs_final.txt",
            "logs/backend_logs_new.txt",
        ])));
        assert_eq!(f.count, 1);
    }

    #[test]
    fn draft_is_not_a_copy_word() {
        // Emacs surfaced this: `gnus-draft.el` is a subsystem of gnus (draft
        // emails), not a clone of `gnus.el`. Same with `save-draft.xpm`.
        assert!(duplicate_siblings(&files(&[
            "lisp/gnus/gnus.el",
            "lisp/gnus/gnus-draft.el",
            "etc/images/mail/save.xpm",
            "etc/images/mail/save-draft.xpm",
        ]))
        .is_none());
    }

    #[test]
    fn fixture_paths_are_skipped() {
        assert!(duplicate_siblings(&files(&[
            "tests/data/sample.json",
            "tests/data/sample-copy.json",
        ]))
        .is_none());
    }

    #[test]
    fn media_asset_variations_are_skipped() {
        // AI-generated image dumps are a real tell, but a different one — not
        // "couldn't refactor code in place".
        assert!(duplicate_siblings(&files(&[
            "media/hero.png",
            "media/hero-copy.png",
            "screenshots/view.mp4",
            "screenshots/view-final.mp4",
        ]))
        .is_none());
    }
}
