use regex::Regex;

use crate::diff::DiffResult;
use crate::watch::{Filter, FilterTarget};

/// Context for evaluating filters
pub struct FilterContext<'a> {
    pub old_content: &'a str,
    pub new_content: &'a str,
    pub diff: &'a DiffResult,
}

/// Evaluate a single filter against the context
pub fn evaluate_filter(filter: &Filter, ctx: &FilterContext) -> bool {
    let target_content = match filter.on {
        FilterTarget::New => ctx.new_content,
        FilterTarget::Old => ctx.old_content,
        FilterTarget::Diff => &ctx.diff.diff_text,
    };

    // Check contains
    if let Some(ref text) = filter.contains {
        if !target_content.contains(text) {
            return false;
        }
    }

    // Check not_contains
    if let Some(ref text) = filter.not_contains {
        if target_content.contains(text) {
            return false;
        }
    }

    // Check regex matches
    if let Some(ref pattern) = filter.matches {
        match Regex::new(pattern) {
            Ok(re) => {
                if !re.is_match(target_content) {
                    return false;
                }
            }
            Err(_) => {
                // Invalid regex, treat as not matching
                return false;
            }
        }
    }

    // Check size_gt (applies to the target based on filter.on)
    if let Some(min_size) = filter.size_gt {
        let size = match filter.on {
            FilterTarget::New => ctx.new_content.len(),
            FilterTarget::Old => ctx.old_content.len(),
            FilterTarget::Diff => ctx.diff.diff_size,
        };
        if size <= min_size {
            return false;
        }
    }

    true
}

/// Evaluate all filters (AND logic - all must pass)
pub fn evaluate_filters(filters: &[Filter], ctx: &FilterContext) -> bool {
    if filters.is_empty() {
        // No filters = any change passes
        return true;
    }

    filters.iter().all(|f| evaluate_filter(f, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff;

    fn make_context<'a>(
        old: &'a str,
        new: &'a str,
        diff_result: &'a DiffResult,
    ) -> FilterContext<'a> {
        FilterContext {
            old_content: old,
            new_content: new,
            diff: diff_result,
        }
    }

    #[test]
    fn test_contains_filter() {
        let old = "Price: $100";
        let new = "Price: $80 (Sale!)";
        let diff_result = diff(old, new);
        let ctx = make_context(old, new, &diff_result);

        let filter = Filter {
            on: FilterTarget::New,
            contains: Some("Sale".to_string()),
            not_contains: None,
            matches: None,
            size_gt: None,
        };

        assert!(evaluate_filter(&filter, &ctx));
    }

    #[test]
    fn test_not_contains_filter() {
        let old = "Status: In Stock";
        let new = "Status: Out of Stock";
        let diff_result = diff(old, new);
        let ctx = make_context(old, new, &diff_result);

        let filter = Filter {
            on: FilterTarget::New,
            contains: None,
            not_contains: Some("In Stock".to_string()),
            matches: None,
            size_gt: None,
        };

        assert!(evaluate_filter(&filter, &ctx));
    }

    #[test]
    fn test_regex_filter() {
        let old = "Price: $100.00";
        let new = "Price: $79.99";
        let diff_result = diff(old, new);
        let ctx = make_context(old, new, &diff_result);

        let filter = Filter {
            on: FilterTarget::New,
            contains: None,
            not_contains: None,
            matches: Some(r"\$\d+\.\d{2}".to_string()),
            size_gt: None,
        };

        assert!(evaluate_filter(&filter, &ctx));
    }

    #[test]
    fn test_size_filter() {
        let old = "Hello";
        let new = "Hello World, this is a much longer text with significant changes";
        let diff_result = diff(old, new);
        let ctx = make_context(old, new, &diff_result);

        let filter_small = Filter {
            on: FilterTarget::Diff,
            contains: None,
            not_contains: None,
            matches: None,
            size_gt: Some(10),
        };

        let filter_large = Filter {
            on: FilterTarget::Diff,
            contains: None,
            not_contains: None,
            matches: None,
            size_gt: Some(1000),
        };

        assert!(evaluate_filter(&filter_small, &ctx));
        assert!(!evaluate_filter(&filter_large, &ctx));
    }

    #[test]
    fn test_empty_filters() {
        let old = "Hello";
        let new = "World";
        let diff_result = diff(old, new);
        let ctx = make_context(old, new, &diff_result);

        assert!(evaluate_filters(&[], &ctx));
    }
}
