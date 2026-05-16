//! Signal: large binary files tracked by git.

use crate::audit::util::human_bytes;
use crate::audit::AuditContext;
use crate::finding::{Category, Finding, Severity};

/// Tracked files larger than this are flagged as committed large binaries.
const LARGE_FILE_BYTES: u64 = 5 * 1024 * 1024;

pub(crate) fn check(ctx: &AuditContext) -> Vec<Finding> {
    let sized: Vec<(String, u64)> = ctx
        .tracked
        .iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(ctx.root.join(path)).ok()?;
            meta.is_file().then(|| (path.clone(), meta.len()))
        })
        .collect();
    large_files_finding(&sized).into_iter().collect()
}

/// Pure decision logic, split from the filesystem walk so it can be tested
/// without touching disk.
fn large_files_finding(sized: &[(String, u64)]) -> Option<Finding> {
    let mut hits: Vec<String> = sized
        .iter()
        .filter(|(_, len)| *len > LARGE_FILE_BYTES)
        .map(|(path, len)| format!("{} ({})", path, human_bytes(*len)))
        .collect();
    if hits.is_empty() {
        return None;
    }
    hits.sort();
    Some(Finding::new(
        Category::VcsHygiene,
        "committed-large-files",
        Severity::Warn,
        "Large binary files (>5 MB) are tracked by git",
        hits,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::util::test_helpers::one;

    #[test]
    fn flags_above_threshold_only() {
        let sized = vec![
            ("small.png".to_string(), 1024),
            ("exact.bin".to_string(), LARGE_FILE_BYTES), // not '>' threshold
            ("big.mp4".to_string(), LARGE_FILE_BYTES + 1),
        ];
        let f = one(large_files_finding(&sized));
        assert_eq!(f.count, 1);
        assert!(f.evidence[0].starts_with("big.mp4 ("));
    }

    #[test]
    fn none_when_all_small() {
        let sized = vec![("a".to_string(), 10), ("b".to_string(), 20)];
        assert!(large_files_finding(&sized).is_none());
    }

    #[test]
    fn evidence_sorted_and_sized() {
        let sized = vec![
            ("z.bin".to_string(), 20 * 1024 * 1024),
            ("a.bin".to_string(), 10 * 1024 * 1024),
        ];
        let f = one(large_files_finding(&sized));
        assert_eq!(f.evidence, vec!["a.bin (10.0 MB)", "z.bin (20.0 MB)"]);
    }
}
