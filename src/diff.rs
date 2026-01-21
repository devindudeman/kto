use similar::{ChangeTag, TextDiff};

/// Result of comparing two pieces of content
#[derive(Debug, Clone)]
pub struct DiffResult {
    /// Whether content has changed
    pub changed: bool,
    /// Human-readable diff (word-level)
    pub diff_text: String,
    /// Size of the diff in characters
    pub diff_size: usize,
    /// Number of additions
    pub additions: usize,
    /// Number of deletions
    pub deletions: usize,
    /// Simple summary of the change (e.g., "+5 / -3 changes")
    pub summary: Option<String>,
}

/// Compare old and new content, producing a diff
pub fn diff(old: &str, new: &str) -> DiffResult {
    if old == new {
        return DiffResult {
            changed: false,
            diff_text: String::new(),
            diff_size: 0,
            additions: 0,
            deletions: 0,
            summary: None,
        };
    }

    let text_diff = TextDiff::from_words(old, new);

    let mut diff_text = String::new();
    let mut additions = 0;
    let mut deletions = 0;

    for change in text_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                diff_text.push_str(&format!("[-{}]", change.value()));
                deletions += 1;
            }
            ChangeTag::Insert => {
                diff_text.push_str(&format!("[+{}]", change.value()));
                additions += 1;
            }
            ChangeTag::Equal => {
                // Include some context
                let value = change.value();
                if value.len() <= 20 {
                    diff_text.push_str(value);
                }
            }
        }
    }

    let diff_size = diff_text.len();

    // Generate simple summary
    let summary = generate_summary(additions, deletions);

    DiffResult {
        changed: true,
        diff_text,
        diff_size,
        additions,
        deletions,
        summary,
    }
}

/// Generate a simple summary from change counts
fn generate_summary(additions: usize, deletions: usize) -> Option<String> {
    if additions > 0 && deletions > 0 {
        Some(format!("+{} / -{} changes", additions, deletions))
    } else if additions > 0 {
        Some(format!("+{} additions", additions))
    } else if deletions > 0 {
        Some(format!("-{} removals", deletions))
    } else {
        None
    }
}

/// Generate a unified diff format (for display)
pub fn unified_diff(old: &str, new: &str, context_lines: usize) -> String {
    let text_diff = TextDiff::from_lines(old, new);

    text_diff
        .unified_diff()
        .context_radius(context_lines)
        .header("old", "new")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_change() {
        let result = diff("Hello World", "Hello World");
        assert!(!result.changed);
        assert_eq!(result.diff_size, 0);
    }

    #[test]
    fn test_simple_change() {
        let result = diff("Hello World", "Hello Universe");
        assert!(result.changed);
        assert!(result.diff_text.contains("[-World]"));
        assert!(result.diff_text.contains("[+Universe]"));
    }

    #[test]
    fn test_additions_deletions() {
        let result = diff("one two three", "one four three");
        assert!(result.changed);
        assert_eq!(result.deletions, 1);
        assert_eq!(result.additions, 1);
    }

    #[test]
    fn test_unified_diff() {
        let old = "line1\nline2\nline3";
        let new = "line1\nmodified\nline3";
        let unified = unified_diff(old, new, 1);
        assert!(unified.contains("-line2"));
        assert!(unified.contains("+modified"));
    }

    #[test]
    fn test_generic_summary() {
        let result = diff("foo bar baz", "foo qux baz");
        assert!(result.changed);
        assert!(result.summary.is_some());
        let summary = result.summary.unwrap();
        // Should show +/- format
        assert!(summary.contains("+") || summary.contains("-"));
    }
}
