//! Intent Parsing System - Structured representation of user monitoring intent
//!
//! This module extracts and preserves user intent from natural language input,
//! including filtering criteria like thresholds ("below $50", "v2.0+"), scope,
//! and goal type.

use std::fmt;

use crate::transforms::Intent;

/// A threshold value extracted from user input
#[derive(Debug, Clone, PartialEq)]
pub enum Threshold {
    /// Price threshold (e.g., "$50", "50 dollars")
    Price {
        value: f64,
        comparison: Comparison,
        currency: Option<String>,
    },
    /// Version threshold (e.g., "v2.0", "version 3")
    Version {
        version: String,
        comparison: Comparison,
    },
    /// Percentage threshold (e.g., "10%", "50 percent")
    Percentage {
        value: f64,
        comparison: Comparison,
    },
    /// Numeric threshold (e.g., "100 units", "5 items")
    Numeric {
        value: f64,
        comparison: Comparison,
        unit: Option<String>,
    },
    /// Raw text threshold that couldn't be parsed structurally
    Raw(String),
}

impl fmt::Display for Threshold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Threshold::Price { value, comparison, currency } => {
                let curr = currency.as_deref().unwrap_or("$");
                write!(f, "{} {}{}", comparison, curr, value)
            }
            Threshold::Version { version, comparison } => {
                write!(f, "{} {}", comparison, version)
            }
            Threshold::Percentage { value, comparison } => {
                write!(f, "{} {}%", comparison, value)
            }
            Threshold::Numeric { value, comparison, unit } => {
                let u = unit.as_deref().unwrap_or("");
                write!(f, "{} {}{}", comparison, value, if u.is_empty() { "".to_string() } else { format!(" {}", u) })
            }
            Threshold::Raw(s) => write!(f, "{}", s),
        }
    }
}

/// Comparison operator for thresholds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Below,      // less than
    Above,      // greater than
    AtMost,     // less than or equal
    AtLeast,    // greater than or equal
    Exactly,    // equal to
    Any,        // any change (no specific threshold)
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Comparison::Below => write!(f, "below"),
            Comparison::Above => write!(f, "above"),
            Comparison::AtMost => write!(f, "at most"),
            Comparison::AtLeast => write!(f, "at least"),
            Comparison::Exactly => write!(f, "exactly"),
            Comparison::Any => write!(f, "any"),
        }
    }
}

/// Scope of the user's intent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IntentScope {
    /// Monitor any matching changes
    #[default]
    Any,
    /// Narrow focus on specific criteria
    Narrow,
    /// Exclude certain items from monitoring
    Exclude,
    /// Only major changes (e.g., major versions)
    MajorOnly,
}

impl fmt::Display for IntentScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntentScope::Any => write!(f, "any"),
            IntentScope::Narrow => write!(f, "specific"),
            IntentScope::Exclude => write!(f, "excluding"),
            IntentScope::MajorOnly => write!(f, "major only"),
        }
    }
}

/// Structured representation of parsed user intent
#[derive(Debug, Clone)]
pub struct ParsedIntent {
    /// Primary goal (release, price, stock, etc.)
    pub goal: Intent,
    /// Optional threshold for filtering (e.g., "below $50")
    pub threshold: Option<Threshold>,
    /// Scope of the intent
    pub scope: IntentScope,
    /// Original raw input (preserved for AI context)
    pub raw_input: String,
    /// Specific keywords found that indicate intent
    pub keywords_found: Vec<String>,
    /// Target item if specified (e.g., "19 inch", "blue variant")
    pub target_item: Option<String>,
}

impl ParsedIntent {
    /// Create a new ParsedIntent from raw input
    pub fn new(raw_input: &str) -> Self {
        parse_intent(raw_input)
    }

    /// Check if this intent has filtering criteria that should enable AI agent
    pub fn has_filtering_criteria(&self) -> bool {
        self.threshold.is_some()
            || self.scope != IntentScope::Any
            || self.target_item.is_some()
    }

    /// Generate AI instructions from this intent
    pub fn to_instructions(&self) -> String {
        intent_to_instructions(self)
    }

    /// Merge this intent with variant selection
    pub fn with_variant(&self, variant_name: &str, variant_status: Option<&str>) -> Self {
        merge_intents(self, variant_name, variant_status)
    }

    /// Check if the raw input mentions specific terms
    pub fn mentions(&self, term: &str) -> bool {
        self.raw_input.to_lowercase().contains(&term.to_lowercase())
    }

    /// Get a brief description of the intent
    pub fn brief_description(&self) -> String {
        let goal_str = match self.goal {
            Intent::Release => "releases",
            Intent::Price => "price changes",
            Intent::Stock => "stock/availability",
            Intent::Jobs => "job postings",
            Intent::News => "news/updates",
            Intent::Generic => "changes",
        };

        if let Some(ref threshold) = self.threshold {
            format!("{} {}", goal_str, threshold)
        } else if let Some(ref target) = self.target_item {
            format!("{} for {}", goal_str, target)
        } else {
            goal_str.to_string()
        }
    }
}

impl Default for ParsedIntent {
    fn default() -> Self {
        Self {
            goal: Intent::Generic,
            threshold: None,
            scope: IntentScope::Any,
            raw_input: String::new(),
            keywords_found: vec![],
            target_item: None,
        }
    }
}

/// Parse user input into a structured intent
pub fn parse_intent(input: &str) -> ParsedIntent {
    let lower = input.to_lowercase();
    let mut keywords_found = Vec::new();

    // Detect goal using existing Intent::detect and add our own keywords
    let goal = Intent::detect(input);

    // Track keywords found
    let release_keywords = ["release", "changelog", "version", "update", "new version"];
    let price_keywords = ["price", "deal", "discount", "sale", "cost", "$"];
    let stock_keywords = ["stock", "available", "availability", "back in", "restock", "inventory", "sold out"];
    let job_keywords = ["job", "career", "hiring", "position", "opening"];
    let news_keywords = ["news", "article", "blog", "post", "feed"];

    for kw in release_keywords.iter() {
        if lower.contains(kw) {
            keywords_found.push(kw.to_string());
        }
    }
    for kw in price_keywords.iter() {
        if lower.contains(kw) {
            keywords_found.push(kw.to_string());
        }
    }
    for kw in stock_keywords.iter() {
        if lower.contains(kw) {
            keywords_found.push(kw.to_string());
        }
    }
    for kw in job_keywords.iter() {
        if lower.contains(kw) {
            keywords_found.push(kw.to_string());
        }
    }
    for kw in news_keywords.iter() {
        if lower.contains(kw) {
            keywords_found.push(kw.to_string());
        }
    }

    // Parse threshold
    let threshold = parse_threshold(input);

    // Detect scope
    let scope = detect_scope(input);

    // Detect target item (e.g., "19 inch", "blue variant", "XL size")
    let target_item = detect_target_item(input);

    ParsedIntent {
        goal,
        threshold,
        scope,
        raw_input: input.to_string(),
        keywords_found,
        target_item,
    }
}

/// Parse threshold from user input
fn parse_threshold(input: &str) -> Option<Threshold> {
    let lower = input.to_lowercase();

    // Detect comparison operator
    let comparison = if lower.contains("below") || lower.contains("under") || lower.contains("less than") {
        Comparison::Below
    } else if lower.contains("above") || lower.contains("over") || lower.contains("more than") {
        Comparison::Above
    } else if lower.contains("at most") || lower.contains("no more than") || lower.contains("max") {
        Comparison::AtMost
    } else if lower.contains("at least") || lower.contains("minimum") || lower.contains("min") {
        Comparison::AtLeast
    } else if lower.contains("exactly") || lower.contains("equal to") {
        Comparison::Exactly
    } else {
        Comparison::Any
    };

    // Try to parse price threshold (e.g., "$50", "50 dollars", "50€")
    if let Some(price) = parse_price(input) {
        return Some(Threshold::Price {
            value: price.0,
            comparison: if comparison == Comparison::Any { Comparison::Below } else { comparison },
            currency: price.1,
        });
    }

    // Try to parse version threshold (e.g., "v2.0", "version 3", "2.0+")
    if let Some(version) = parse_version(input) {
        let comp = if input.contains('+') || lower.contains("and above") || lower.contains("or higher") {
            Comparison::AtLeast
        } else if comparison == Comparison::Any {
            Comparison::Below
        } else {
            comparison
        };
        return Some(Threshold::Version {
            version,
            comparison: comp,
        });
    }

    // Try to parse percentage (e.g., "10%", "50 percent")
    if let Some(pct) = parse_percentage(input) {
        return Some(Threshold::Percentage {
            value: pct,
            comparison: if comparison == Comparison::Any { Comparison::AtLeast } else { comparison },
        });
    }

    // If we have a comparison word but couldn't parse a structured threshold,
    // extract the raw threshold text
    if comparison != Comparison::Any {
        // Try to extract the threshold phrase
        let patterns = [
            "below ", "under ", "less than ",
            "above ", "over ", "more than ",
            "at most ", "no more than ", "max ",
            "at least ", "minimum ", "min ",
            "exactly ", "equal to ",
        ];
        for pattern in patterns {
            if let Some(idx) = lower.find(pattern) {
                let start = idx + pattern.len();
                let rest = &input[start..];
                // Extract until end of word or common delimiters
                let end = rest.find(|c: char| c == ',' || c == '.' || c == '!' || c == '?' || (c.is_whitespace() && rest[..rest.find(c).unwrap_or(rest.len())].contains(' ')))
                    .unwrap_or(rest.len());
                let threshold_text = rest[..end].trim();
                if !threshold_text.is_empty() {
                    return Some(Threshold::Raw(format!("{} {}", pattern.trim(), threshold_text)));
                }
            }
        }
    }

    None
}

/// Parse price from input (e.g., "$50", "50 dollars", "€100")
fn parse_price(input: &str) -> Option<(f64, Option<String>)> {
    use regex::Regex;

    // Match patterns like "$50", "$50.99", "50$", "50 dollars", "€100"
    let patterns = [
        (r"\$\s*(\d+(?:\.\d{1,2})?)", Some("$")),       // $50 or $50.99
        (r"(\d+(?:\.\d{1,2})?)\s*\$", Some("$")),       // 50$
        (r"€\s*(\d+(?:\.\d{1,2})?)", Some("€")),        // €50
        (r"(\d+(?:\.\d{1,2})?)\s*€", Some("€")),        // 50€
        (r"£\s*(\d+(?:\.\d{1,2})?)", Some("£")),        // £50
        (r"(\d+(?:\.\d{1,2})?)\s*(?:dollars?|usd)", Some("$")), // 50 dollars
        (r"(\d+(?:\.\d{1,2})?)\s*(?:euros?|eur)", Some("€")),   // 50 euros
    ];

    for (pattern, currency) in patterns {
        if let Ok(re) = Regex::new(&format!("(?i){}", pattern)) {
            if let Some(caps) = re.captures(input) {
                if let Some(value_str) = caps.get(1) {
                    if let Ok(value) = value_str.as_str().parse::<f64>() {
                        return Some((value, currency.map(|s| s.to_string())));
                    }
                }
            }
        }
    }

    None
}

/// Parse version from input (e.g., "v2.0", "version 3", "2.0+")
fn parse_version(input: &str) -> Option<String> {
    use regex::Regex;

    let patterns = [
        r"v(\d+(?:\.\d+)*)\+?",                    // v2.0, v2.0+
        r"version\s*(\d+(?:\.\d+)*)",              // version 2.0
        r"(\d+\.\d+(?:\.\d+)?)\s*(?:and above|or higher|\+)?", // 2.0 and above
    ];

    for pattern in patterns {
        if let Ok(re) = Regex::new(&format!("(?i){}", pattern)) {
            if let Some(caps) = re.captures(input) {
                if let Some(version) = caps.get(1) {
                    return Some(format!("v{}", version.as_str()));
                }
            }
        }
    }

    None
}

/// Parse percentage from input (e.g., "10%", "50 percent")
fn parse_percentage(input: &str) -> Option<f64> {
    use regex::Regex;

    let patterns = [
        r"(\d+(?:\.\d+)?)\s*%",           // 10%
        r"(\d+(?:\.\d+)?)\s*percent",     // 10 percent
    ];

    for pattern in patterns {
        if let Ok(re) = Regex::new(&format!("(?i){}", pattern)) {
            if let Some(caps) = re.captures(input) {
                if let Some(value_str) = caps.get(1) {
                    if let Ok(value) = value_str.as_str().parse::<f64>() {
                        return Some(value);
                    }
                }
            }
        }
    }

    None
}

/// Detect scope from input
fn detect_scope(input: &str) -> IntentScope {
    let lower = input.to_lowercase();

    if lower.contains("major") || lower.contains("significant") {
        IntentScope::MajorOnly
    } else if lower.contains("only") || lower.contains("specific") || lower.contains("just") {
        IntentScope::Narrow
    } else if lower.contains("except") || lower.contains("exclude") || lower.contains("not ") || lower.contains("ignore") {
        IntentScope::Exclude
    } else {
        IntentScope::Any
    }
}

/// Detect target item from input (e.g., "19 inch", "blue", "XL")
fn detect_target_item(input: &str) -> Option<String> {
    use regex::Regex;

    // Common size/variant patterns (with word boundaries to avoid partial matches)
    let patterns = [
        r#"(\d+)\s*(?:inch|in|")"#,                   // 19 inch, 19in, 19"
        r"(\d+)\s*(?:gb|tb|mb)\b",                    // 256GB, 1TB
        r"\b(xs|small|xm|medium|xl|large|xxl)\b",    // XS, Small, M, Medium, L, Large, XL, XXL
        r"\b(black|white|blue|red|green|silver|gold|gray|grey|pink|purple)\b", // colors
    ];

    for pattern in patterns {
        if let Ok(re) = Regex::new(&format!("(?i){}", pattern)) {
            if let Some(caps) = re.captures(input) {
                if let Some(matched) = caps.get(0) {
                    return Some(matched.as_str().to_string());
                }
            }
        }
    }

    // Check for "variant" mentions with specific names
    if let Ok(re) = Regex::new(r"(?i)(\w+)\s+variant") {
        if let Some(caps) = re.captures(input) {
            if let Some(variant_name) = caps.get(1) {
                return Some(variant_name.as_str().to_string());
            }
        }
    }

    None
}

/// Generate AI agent instructions from a parsed intent
pub fn intent_to_instructions(intent: &ParsedIntent) -> String {
    let mut instructions = Vec::new();

    // Add the primary monitoring goal
    match intent.goal {
        Intent::Release => {
            instructions.push("Monitor for new releases and version updates.".to_string());
        }
        Intent::Price => {
            instructions.push("Monitor for price changes.".to_string());
        }
        Intent::Stock => {
            instructions.push("Monitor for stock/availability changes.".to_string());
        }
        Intent::Jobs => {
            instructions.push("Monitor for new job postings. Ignore updates to existing positions.".to_string());
        }
        Intent::News => {
            instructions.push("Monitor for new articles and news updates.".to_string());
        }
        Intent::Generic => {
            instructions.push("Monitor for significant changes.".to_string());
        }
    }

    // Add threshold-based filtering
    if let Some(ref threshold) = intent.threshold {
        let threshold_instruction = match threshold {
            Threshold::Price { value, comparison, currency } => {
                let curr = currency.as_deref().unwrap_or("$");
                format!("ONLY alert when price is {} {}{:.2}.", comparison, curr, value)
            }
            Threshold::Version { version, comparison } => {
                format!("ONLY alert for versions {} {}.", comparison, version)
            }
            Threshold::Percentage { value, comparison } => {
                format!("ONLY alert when change is {} {:.0}%.", comparison, value)
            }
            Threshold::Numeric { value, comparison, unit } => {
                let u = unit.as_deref().unwrap_or("");
                format!("ONLY alert when value is {} {:.0}{}.", comparison, value, if u.is_empty() { "".to_string() } else { format!(" {}", u) })
            }
            Threshold::Raw(s) => {
                format!("ONLY alert when condition matches: {}.", s)
            }
        };
        instructions.push(threshold_instruction);
    }

    // Add scope-based filtering
    match intent.scope {
        IntentScope::MajorOnly => {
            instructions.push("Focus on MAJOR changes only (e.g., major version bumps like v1.0 → v2.0, not v1.0 → v1.1).".to_string());
        }
        IntentScope::Narrow => {
            instructions.push("Be selective - only alert on changes that directly match the criteria.".to_string());
        }
        IntentScope::Exclude => {
            // Look for exclusion patterns in raw input
            let lower = intent.raw_input.to_lowercase();
            if let Some(idx) = lower.find("except") {
                let rest = &intent.raw_input[idx + 6..];
                instructions.push(format!("Exclude: {}", rest.trim()));
            } else if let Some(idx) = lower.find("ignore") {
                let rest = &intent.raw_input[idx + 6..];
                instructions.push(format!("Ignore: {}", rest.trim()));
            }
        }
        IntentScope::Any => {}
    }

    // Add target item filtering
    if let Some(ref target) = intent.target_item {
        instructions.push(format!("Focus specifically on the '{}' variant/option.", target));
    }

    // Preserve raw input for AI context
    if !intent.raw_input.is_empty() {
        instructions.push(format!("\nOriginal request: \"{}\"", intent.raw_input));
    }

    instructions.join(" ")
}

/// Merge a base intent with a selected variant
pub fn merge_intents(base: &ParsedIntent, variant_name: &str, variant_status: Option<&str>) -> ParsedIntent {
    let mut merged = base.clone();

    // Set the target item to the variant
    merged.target_item = Some(variant_name.to_string());

    // Update raw input to include variant context
    let status_context = variant_status
        .map(|s| format!(" (currently: {})", s))
        .unwrap_or_default();
    merged.raw_input = format!(
        "{} [Selected variant: {}{}]",
        base.raw_input,
        variant_name,
        status_context
    );

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_intent_with_price_threshold() {
        let intent = parse_intent("alert me when price drops below $50");
        assert_eq!(intent.goal, Intent::Price);
        assert!(intent.threshold.is_some());
        if let Some(Threshold::Price { value, comparison, .. }) = intent.threshold {
            assert_eq!(value, 50.0);
            assert_eq!(comparison, Comparison::Below);
        } else {
            panic!("Expected price threshold");
        }
    }

    #[test]
    fn test_parse_intent_with_version_threshold() {
        let intent = parse_intent("github.com/org/repo releases below v2.0");
        assert_eq!(intent.goal, Intent::Release);
        assert!(intent.threshold.is_some());
        if let Some(Threshold::Version { version, comparison }) = intent.threshold {
            assert_eq!(version, "v2.0");
            assert_eq!(comparison, Comparison::Below);
        } else {
            panic!("Expected version threshold");
        }
    }

    #[test]
    fn test_parse_intent_major_only_scope() {
        let intent = parse_intent("notify on major version updates only");
        assert_eq!(intent.scope, IntentScope::MajorOnly);
    }

    #[test]
    fn test_parse_intent_with_target_item() {
        let intent = parse_intent("monitor the 19 inch variant for price drops");
        assert!(intent.target_item.is_some());
        assert!(intent.target_item.as_ref().unwrap().contains("19"));
    }

    #[test]
    fn test_has_filtering_criteria() {
        let basic = parse_intent("watch this page");
        assert!(!basic.has_filtering_criteria());

        let with_threshold = parse_intent("alert when price below $50");
        assert!(with_threshold.has_filtering_criteria());

        let with_scope = parse_intent("major updates only");
        assert!(with_scope.has_filtering_criteria());
    }

    #[test]
    fn test_merge_intent_preserves_threshold() {
        let base = parse_intent("monitor price drops below $50");
        let merged = merge_intents(&base, "19 inch", Some("in stock"));

        // Original threshold should be preserved
        assert!(merged.threshold.is_some());
        if let Some(Threshold::Price { value, .. }) = merged.threshold {
            assert_eq!(value, 50.0);
        } else {
            panic!("Threshold should be preserved");
        }

        // Variant should be added
        assert_eq!(merged.target_item, Some("19 inch".to_string()));

        // Raw input should include variant context
        assert!(merged.raw_input.contains("19 inch"));
    }

    #[test]
    fn test_intent_to_instructions_includes_threshold() {
        let intent = parse_intent("alert when price below $50");
        let instructions = intent_to_instructions(&intent);

        assert!(instructions.contains("50"));
        assert!(instructions.contains("below"));
        assert!(instructions.contains("ONLY alert"));
    }

    #[test]
    fn test_parse_percentage_threshold() {
        let intent = parse_intent("alert when discount is at least 20%");
        assert!(intent.threshold.is_some());
        if let Some(Threshold::Percentage { value, comparison }) = intent.threshold {
            assert_eq!(value, 20.0);
            assert_eq!(comparison, Comparison::AtLeast);
        } else {
            panic!("Expected percentage threshold");
        }
    }
}
