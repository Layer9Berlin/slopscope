use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// One commit, newest-first ordering when returned in a Vec.
#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    /// Author date, unix epoch seconds. Author date (not committer date) so a
    /// rebase doesn't collapse the whole history into one timestamp cluster.
    pub timestamp: i64,
    /// Author email.
    pub author: String,
    pub subject: String,
    /// Per-file line churn introduced by this commit.
    pub files: Vec<FileDelta>,
}

/// One file's line churn within a single commit.
#[derive(Debug, Clone)]
pub struct FileDelta {
    pub path: String,
    pub added: u32,
    pub deleted: u32,
    /// Git reports binary files as `-` / `-` — no line counts available.
    pub binary: bool,
}

/// The all-zero SHA git uses for an absent blob (file added or deleted).
const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))
}

pub fn is_git_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Files tracked by git (the deterministic "what was committed" set).
pub fn tracked_files(root: &Path) -> Result<Vec<String>> {
    let out = git(root, &["ls-files", "-z"])?;
    if !out.status.success() {
        bail!("git ls-files failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Full commit log with per-file churn, newest first. Empty if the repo has no
/// commits yet.
pub fn commit_log(root: &Path) -> Result<Vec<Commit>> {
    // %x1f (unit separator) delimits header fields; subjects never contain it.
    // --numstat appends `<added>\t<deleted>\t<path>` lines per changed file.
    let out = git(
        root,
        &[
            "log",
            "--no-merges",
            "--no-renames",
            "--numstat",
            "--format=%H%x1f%at%x1f%ae%x1f%s",
        ],
    )?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("does not have any commits") {
            return Ok(Vec::new());
        }
        bail!("git log failed: {}", err);
    }
    Ok(parse_numstat_log(&String::from_utf8_lossy(&out.stdout)))
}

/// Per-path sequence of content blob hashes, oldest commit first. A hash that
/// recurs means the file returned to an exact prior content state.
pub fn blob_history(root: &Path) -> Result<HashMap<String, Vec<String>>> {
    let out = git(
        root,
        &[
            "log",
            "--no-merges",
            "--no-renames",
            "--reverse",
            "--raw",
            "--no-abbrev",
            "--format=%H",
        ],
    )?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("does not have any commits") {
            return Ok(HashMap::new());
        }
        bail!("git log --raw failed: {}", err);
    }
    Ok(parse_raw_log(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse `git log --numstat --format=%H%x1f%ct%x1f%ae%x1f%s` output.
fn parse_numstat_log(stdout: &str) -> Vec<Commit> {
    let mut commits: Vec<Commit> = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(commit) = parse_header(line) {
            commits.push(commit);
        } else if let Some(delta) = parse_numstat_line(line) {
            if let Some(last) = commits.last_mut() {
                last.files.push(delta);
            }
        }
    }
    commits
}

/// A header line: `<hash>\x1f<author-unix-ts>\x1f<author-email>\x1f<subject>`.
fn parse_header(line: &str) -> Option<Commit> {
    let mut parts = line.splitn(4, '\u{1f}');
    let hash = parts.next()?;
    let ts = parts.next()?;
    let author = parts.next()?;
    let subject = parts.next()?;
    // A real header always has all four fields; bail if the split came short.
    Some(Commit {
        hash: hash.to_string(),
        timestamp: ts.parse().unwrap_or(0),
        author: author.to_string(),
        subject: subject.to_string(),
        files: Vec::new(),
    })
}

/// A numstat line: `<added>\t<deleted>\t<path>`, where added/deleted are `-`
/// for binary files.
fn parse_numstat_line(line: &str) -> Option<FileDelta> {
    let mut parts = line.splitn(3, '\t');
    let added = parts.next()?;
    let deleted = parts.next()?;
    let path = parts.next()?;
    // Reject anything that isn't shaped like a numstat row.
    if path.is_empty() {
        return None;
    }
    let binary = added == "-" || deleted == "-";
    Some(FileDelta {
        path: path.to_string(),
        added: if binary { 0 } else { added.parse().ok()? },
        deleted: if binary { 0 } else { deleted.parse().ok()? },
        binary,
    })
}

/// Parse `git log --raw --no-abbrev --reverse --format=%H` output into a
/// per-path list of content blob hashes (oldest first). Deletions (all-zero
/// new SHA) are skipped — we track content states the file actually held.
fn parse_raw_log(stdout: &str) -> HashMap<String, Vec<String>> {
    let mut history: HashMap<String, Vec<String>> = HashMap::new();
    for line in stdout.lines() {
        // Raw entry: `:<omode> <nmode> <osha> <nsha> <status>\t<path>`.
        let Some(rest) = line.strip_prefix(':') else {
            continue; // header (%H) or blank line
        };
        let Some((meta, path)) = rest.split_once('\t') else {
            continue;
        };
        let fields: Vec<&str> = meta.split_whitespace().collect();
        let Some(new_sha) = fields.get(3) else {
            continue;
        };
        if *new_sha == ZERO_SHA {
            continue; // deletion — not a content state
        }
        history
            .entry(path.to_string())
            .or_default()
            .push(new_sha.to_string());
    }
    history
}

/// Size of the `.git` directory in bytes.
pub fn git_dir_size(root: &Path) -> Result<u64> {
    let out = git(root, &["rev-parse", "--absolute-git-dir"])?;
    if !out.status.success() {
        bail!("could not locate .git dir");
    }
    let git_dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(dir_size(Path::new(&git_dir)))
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                total += dir_size(&entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_numstat_log_basic() {
        let raw = "abc123\u{1f}1700000000\u{1f}dev@example.com\u{1f}Add feature\n\
                   10\t2\tsrc/main.rs\n\
                   0\t5\tREADME.md\n\
                   \n\
                   def456\u{1f}1700001000\u{1f}other@example.com\u{1f}fix: bug\n\
                   3\t3\tsrc/main.rs\n";
        let commits = parse_numstat_log(raw);
        assert_eq!(commits.len(), 2);

        let c0 = &commits[0];
        assert_eq!(c0.hash, "abc123");
        assert_eq!(c0.timestamp, 1_700_000_000);
        assert_eq!(c0.author, "dev@example.com");
        assert_eq!(c0.subject, "Add feature");
        assert_eq!(c0.files.len(), 2);
        assert_eq!(c0.files[0].path, "src/main.rs");
        assert_eq!(c0.files[0].added, 10);
        assert_eq!(c0.files[0].deleted, 2);
        assert!(!c0.files[0].binary);

        assert_eq!(commits[1].files.len(), 1);
        assert_eq!(commits[1].subject, "fix: bug");
    }

    #[test]
    fn parse_numstat_log_handles_binary_and_empty_commit() {
        let raw = "h1\u{1f}1\u{1f}a@b.c\u{1f}add image\n\
                   -\t-\tassets/logo.png\n\
                   \n\
                   h2\u{1f}2\u{1f}a@b.c\u{1f}empty commit\n";
        let commits = parse_numstat_log(raw);
        assert_eq!(commits.len(), 2);
        assert!(commits[0].files[0].binary);
        assert_eq!(commits[0].files[0].added, 0);
        assert_eq!(commits[0].files[0].deleted, 0);
        assert!(commits[1].files.is_empty());
    }

    #[test]
    fn parse_numstat_log_handles_paths_with_spaces() {
        let raw = "h1\u{1f}1\u{1f}a@b.c\u{1f}s\n4\t1\tsome dir/a file.rs\n";
        let commits = parse_numstat_log(raw);
        assert_eq!(commits[0].files[0].path, "some dir/a file.rs");
        assert_eq!(commits[0].files[0].added, 4);
    }

    #[test]
    fn parse_numstat_log_empty_input() {
        assert!(parse_numstat_log("").is_empty());
    }

    #[test]
    fn parse_raw_log_builds_blob_history_oldest_first() {
        // file.rs: blob A, then B, then back to A (a revert).
        let raw = format!(
            "h1\n:100644 100644 {z} aaa M\tfile.rs\n\
             h2\n:100644 100644 aaa bbb M\tfile.rs\n\
             h3\n:100644 100644 bbb aaa M\tfile.rs\n",
            z = ZERO_SHA
        );
        let hist = parse_raw_log(&raw);
        assert_eq!(hist["file.rs"], vec!["aaa", "bbb", "aaa"]);
    }

    #[test]
    fn parse_raw_log_skips_deletions() {
        let raw = format!(
            "h1\n:100644 100644 {z} aaa A\tx.rs\n\
             h2\n:100644 100644 aaa {z} D\tx.rs\n",
            z = ZERO_SHA
        );
        let hist = parse_raw_log(&raw);
        // The deletion (new SHA = zero) is not recorded as a content state.
        assert_eq!(hist["x.rs"], vec!["aaa"]);
    }

    #[test]
    fn parse_raw_log_empty_input() {
        assert!(parse_raw_log("").is_empty());
    }
}
