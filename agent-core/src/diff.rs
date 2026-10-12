//! Minimal line-level diff used for edit_file artifact previews/metadata.
//!
//! A full LCS over lines is used; inputs are small (a single replacement
//! region), so O(n*m) memory is acceptable. This is intentionally the only
//! diff computation in core: richer formats (PDF/docx) stay out (see
//! PLAN.md — core only defines Artifact shapes).

/// Result of `line_diff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDiff {
    /// Lines added by the change
    pub insertions: usize,
    /// Lines removed by the change
    pub deletions: usize,
    /// Rendered diff (`- old` / `+ new` lines, in order); `None` when the
    /// texts are identical.
    pub preview: Option<String>,
}

/// Compare two texts line by line and produce changed-line stats plus a
/// condensed preview containing only the changed lines.
pub fn line_diff(old: &str, new: &str) -> LineDiff {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let (m, n) = (old_lines.len(), new_lines.len());

    // table[i][j] = LCS length of old_lines[i..] and new_lines[j..]
    let mut table = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            table[i][j] = if old_lines[i] == new_lines[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut ops: Vec<String> = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut insertions, mut deletions) = (0usize, 0usize);
    while i < m && j < n {
        if old_lines[i] == new_lines[j] {
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push(format!("- {}", old_lines[i]));
            deletions += 1;
            i += 1;
        } else {
            ops.push(format!("+ {}", new_lines[j]));
            insertions += 1;
            j += 1;
        }
    }
    while i < m {
        ops.push(format!("- {}", old_lines[i]));
        deletions += 1;
        i += 1;
    }
    while j < n {
        ops.push(format!("+ {}", new_lines[j]));
        insertions += 1;
        j += 1;
    }

    LineDiff {
        insertions,
        deletions,
        preview: if ops.is_empty() {
            None
        } else {
            Some(ops.join("\n"))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_texts_produce_empty_diff() {
        let d = line_diff("a\nb\n", "a\nb\n");
        assert_eq!(d.insertions, 0);
        assert_eq!(d.deletions, 0);
        assert!(d.preview.is_none());
    }

    #[test]
    fn single_line_change_is_one_deletion_one_insertion() {
        let d = line_diff("项目周期12个月", "项目周期18个月");
        assert_eq!((d.insertions, d.deletions), (1, 1));
        let preview = d.preview.unwrap();
        assert!(preview.starts_with("- "));
        assert!(preview.contains("+ 项目周期18个月"));
    }

    #[test]
    fn multi_line_replacement_keeps_shared_lines() {
        let d = line_diff("header\nold-1\nold-2\nfooter", "header\nnew-1\nfooter");
        assert_eq!(d.insertions, 1);
        assert_eq!(d.deletions, 2);
        let preview = d.preview.unwrap();
        assert!(preview.contains("- old-1"));
        assert!(preview.contains("- old-2"));
        assert!(preview.contains("+ new-1"));
        // unchanged context lines must not appear in the condensed preview
        assert!(!preview.contains("header"));
        assert!(!preview.contains("footer"));
    }

    #[test]
    fn pure_addition_and_pure_deletion() {
        let add = line_diff("a", "a\nb\nc");
        assert_eq!(add.insertions, 2);
        assert_eq!(add.deletions, 0);

        let del = line_diff("a\nb\nc", "a");
        assert_eq!(del.insertions, 0);
        assert_eq!(del.deletions, 2);
    }
}
