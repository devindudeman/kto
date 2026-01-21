use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::watch::Normalization;

// Pre-compiled regex for whitespace normalization (compile once, use many times)
static WHITESPACE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\s+").expect("Invalid whitespace regex pattern")
});

/// Normalize content according to the specified options
pub fn normalize(content: &str, options: &Normalization) -> String {
    let mut result = content.to_string();

    if options.strip_whitespace {
        result = normalize_whitespace(&result);
    }

    if options.strip_dates {
        result = strip_dates(&result);
    }

    if options.strip_random_ids {
        result = strip_random_ids(&result);
    }

    for pattern in &options.ignore_patterns {
        if let Ok(re) = Regex::new(pattern) {
            result = re.replace_all(&result, "").to_string();
        }
    }

    result.trim().to_string()
}

/// Normalize whitespace: collapse multiple spaces/newlines into single space
fn normalize_whitespace(content: &str) -> String {
    WHITESPACE_RE.replace_all(content, " ").to_string()
}

/// Strip common date patterns
fn strip_dates(content: &str) -> String {
    // Common date patterns
    let patterns = [
        // ISO dates: 2024-01-15
        r"\d{4}-\d{2}-\d{2}",
        // US dates: 01/15/2024, 1/15/24
        r"\d{1,2}/\d{1,2}/\d{2,4}",
        // Written dates: January 15, 2024
        r"(?i)(january|february|march|april|may|june|july|august|september|october|november|december)\s+\d{1,2},?\s*\d{4}",
        // Relative times: 2 hours ago, 3 days ago
        r"\d+\s+(second|minute|hour|day|week|month|year)s?\s+ago",
        // Times: 10:30 AM, 14:30:00
        r"\d{1,2}:\d{2}(:\d{2})?\s*(AM|PM|am|pm)?",
    ];

    let mut result = content.to_string();
    for pattern in patterns {
        if let Ok(re) = Regex::new(pattern) {
            result = re.replace_all(&result, "").to_string();
        }
    }
    result
}

/// Strip random IDs and cache-busters
fn strip_random_ids(content: &str) -> String {
    let patterns = [
        // Long hex strings (likely cache busters or session IDs)
        r"[a-f0-9]{32,}",
        // UUIDs
        r"[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}",
        // Numeric IDs in URLs
        r"[?&](id|session|token|cache|v|_)=[a-zA-Z0-9]+",
    ];

    let mut result = content.to_string();
    for pattern in patterns {
        if let Ok(re) = Regex::new(pattern) {
            result = re.replace_all(&result, "").to_string();
        }
    }
    result
}

/// Compute SHA-256 hash of content
pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_whitespace() {
        let input = "Hello   World\n\n\nTest";
        let result = normalize_whitespace(input);
        assert_eq!(result, "Hello World Test");
    }

    #[test]
    fn test_strip_dates() {
        let input = "Updated on 2024-01-15 at 10:30 AM";
        let result = strip_dates(input);
        assert!(!result.contains("2024-01-15"));
        assert!(!result.contains("10:30"));
    }

    #[test]
    fn test_hash_content() {
        let hash1 = hash_content("Hello World");
        let hash2 = hash_content("Hello World");
        let hash3 = hash_content("Hello World!");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 64); // SHA-256 produces 64 hex chars
    }

    #[test]
    fn test_normalize_full() {
        let options = Normalization {
            strip_whitespace: true,
            strip_dates: true,
            strip_random_ids: false,
            ignore_patterns: vec![],
        };

        let input = "Updated   on 2024-01-15\n\nPrice: $99";
        let result = normalize(input, &options);
        assert!(result.contains("Price: $99"));
        assert!(!result.contains("2024-01-15"));
    }
}
