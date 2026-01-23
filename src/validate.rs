//! Preflight Validation - Test configurations before creating watches
//!
//! This module provides validation of watch configurations to ensure they
//! will work correctly before committing them to the database.

use crate::error::Result;
use crate::extract;
use crate::fetch::{self, PageContent};
use crate::page_facts::PageFacts;
use crate::platform::Strategy;
use crate::transforms::Intent;
use crate::watch::{Engine, Extraction};
use std::collections::HashMap;
use std::time::Instant;

/// Result of validating a configuration
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether validation passed
    pub success: bool,
    /// Extracted content (if successful)
    pub extracted_content: Option<String>,
    /// Data quality assessment
    pub data_quality: DataQuality,
    /// Error message (if failed)
    pub error: Option<String>,
    /// How long the validation took (ms)
    pub runtime_ms: u64,
    /// Warnings (non-fatal issues)
    pub warnings: Vec<String>,
}

impl ValidationResult {
    /// Create a successful result
    pub fn success(content: String, quality: DataQuality, runtime_ms: u64) -> Self {
        Self {
            success: true,
            extracted_content: Some(content),
            data_quality: quality,
            error: None,
            runtime_ms,
            warnings: Vec::new(),
        }
    }

    /// Create a failed result
    pub fn failure(error: String, runtime_ms: u64) -> Self {
        Self {
            success: false,
            extracted_content: None,
            data_quality: DataQuality::default(),
            error: Some(error),
            runtime_ms,
            warnings: Vec::new(),
        }
    }

    /// Add a warning
    pub fn with_warning(mut self, warning: String) -> Self {
        self.warnings.push(warning);
        self
    }

    /// Overall confidence score (0.0 - 1.0)
    pub fn confidence(&self) -> f32 {
        if !self.success {
            return 0.0;
        }
        self.data_quality.score()
    }
}

/// Assessment of extracted data quality
#[derive(Debug, Clone, Default)]
pub struct DataQuality {
    /// Content is not empty (> 100 chars)
    pub not_empty: bool,
    /// Content has expected type for the intent
    pub has_expected_type: bool,
    /// Content is not just template placeholders
    pub not_template: bool,
    /// Selector exists in page (if selector-based)
    pub selector_found: bool,
    /// Stability hint based on extraction method
    pub stability_hint: f32,
    /// Additional notes about quality
    pub notes: Vec<String>,
}

impl DataQuality {
    /// Calculate overall quality score (0.0 - 1.0)
    pub fn score(&self) -> f32 {
        let mut score = 0.0;
        let mut weights = 0.0;

        // Not empty is critical (weight: 0.4)
        if self.not_empty {
            score += 0.4;
        }
        weights += 0.4;

        // Expected type for intent (weight: 0.2)
        if self.has_expected_type {
            score += 0.2;
        }
        weights += 0.2;

        // Not a template (weight: 0.2)
        if self.not_template {
            score += 0.2;
        }
        weights += 0.2;

        // Selector found (weight: 0.1) - only if relevant
        if self.selector_found {
            score += 0.1;
        }
        weights += 0.1;

        // Stability (weight: 0.1)
        score += self.stability_hint * 0.1;
        weights += 0.1;

        if weights > 0.0 {
            score / weights
        } else {
            0.0
        }
    }

    /// Human-readable summary
    pub fn summary(&self) -> String {
        let mut issues = Vec::new();

        if !self.not_empty {
            issues.push("content too short");
        }
        if !self.has_expected_type {
            issues.push("content type mismatch");
        }
        if !self.not_template {
            issues.push("may be template/placeholder");
        }
        if !self.selector_found {
            issues.push("selector not found");
        }

        if issues.is_empty() {
            format!("Good quality (stability: {:.0}%)", self.stability_hint * 100.0)
        } else {
            format!("Issues: {}", issues.join(", "))
        }
    }
}

/// Validate a strategy against page facts
pub fn validate_strategy(
    url: &str,
    strategy: &Strategy,
    intent: Intent,
    facts: &PageFacts,
) -> ValidationResult {
    let start = Instant::now();

    // Determine what content to use based on engine
    let html = match &strategy.engine {
        Engine::Playwright => {
            // Prefer JS-rendered content if available
            facts.js_rendered_html.as_deref().unwrap_or(&facts.html)
        }
        _ => &facts.html,
    };

    // Try to extract content
    let extracted = match extract_for_validation(html, &strategy.extraction, url) {
        Ok(content) => content,
        Err(e) => {
            return ValidationResult::failure(
                format!("Extraction failed: {}", e),
                start.elapsed().as_millis() as u64,
            );
        }
    };

    // Assess quality
    let quality = assess_quality(&extracted, &strategy.extraction, intent);

    let runtime_ms = start.elapsed().as_millis() as u64;

    // Add warnings first (before moving quality)
    let stability_warning = quality.stability_hint < 0.5;
    let type_warning = !quality.has_expected_type;

    // Build result
    let mut result = if quality.not_empty && quality.not_template {
        ValidationResult::success(extracted, quality, runtime_ms)
    } else {
        let error = if !quality.not_empty {
            "Extracted content too short (< 100 chars)"
        } else {
            "Content appears to be template/placeholder"
        };
        ValidationResult::failure(error.to_string(), runtime_ms)
    };

    // Add warnings
    if type_warning {
        result = result.with_warning("Content may not match expected type for intent".to_string());
    }
    if stability_warning {
        result = result.with_warning("Extraction method may be unstable".to_string());
    }

    result
}

/// Extract content for validation
fn extract_for_validation(html: &str, extraction: &Extraction, url: &str) -> Result<String> {
    // Build minimal page content
    let page_content = PageContent {
        url: url.to_string(),
        title: None,
        html: html.to_string(),
        text: None,
    };

    extract::extract(&page_content, extraction)
}

/// Assess the quality of extracted content
fn assess_quality(content: &str, extraction: &Extraction, intent: Intent) -> DataQuality {
    let mut quality = DataQuality {
        not_empty: content.len() >= 100,
        has_expected_type: check_expected_type(content, intent),
        not_template: !is_template_content(content),
        selector_found: true, // Assume true unless selector-based
        stability_hint: stability_for_extraction(extraction),
        notes: Vec::new(),
    };

    // Check selector-specific issues
    if let Extraction::Selector { selector } = extraction {
        if content.is_empty() || content.trim().len() < 10 {
            quality.selector_found = false;
            quality.notes.push(format!("Selector '{}' may not exist", selector));
        }
    }

    // Add notes based on content analysis
    if content.contains("{{") || content.contains("${") {
        quality.notes.push("Contains template syntax".to_string());
    }

    if content.to_lowercase().contains("loading") && content.len() < 500 {
        quality.notes.push("May be loading placeholder".to_string());
    }

    quality
}

/// Check if content matches expected type for intent
fn check_expected_type(content: &str, intent: Intent) -> bool {
    let lower = content.to_lowercase();

    match intent {
        Intent::Price => {
            // Should contain price-like patterns
            has_price_pattern(content)
        }
        Intent::Stock => {
            // Should contain availability indicators
            lower.contains("stock")
                || lower.contains("available")
                || lower.contains("sold out")
                || lower.contains("add to cart")
                || lower.contains("buy now")
                || lower.contains("out of stock")
                || lower.contains("in stock")
                || lower.contains("notify me")
        }
        Intent::Release => {
            // Should contain version/release indicators
            lower.contains("version")
                || lower.contains("release")
                || lower.contains("changelog")
                || lower.contains("v1")
                || lower.contains("v2")
                || content.contains(".")  // version numbers
        }
        Intent::Jobs => {
            // Should contain job-related content
            lower.contains("job")
                || lower.contains("position")
                || lower.contains("apply")
                || lower.contains("hiring")
                || lower.contains("career")
                || lower.contains("salary")
        }
        Intent::News => {
            // News is generic, most content qualifies
            content.len() >= 200
        }
        Intent::Generic => {
            // No specific expectation
            true
        }
    }
}

/// Check if content looks like a price
fn has_price_pattern(content: &str) -> bool {
    // Check for currency symbols followed by numbers
    let patterns = [
        r"\$\d", r"€\d", r"£\d", r"¥\d",
        r"\d+\.\d{2}", // decimal prices
        r"USD", r"EUR", r"GBP",
    ];

    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(content) {
                return true;
            }
        }
    }

    // Also check for common price phrases
    let lower = content.to_lowercase();
    lower.contains("price") || lower.contains("cost") || lower.contains("$")
}

/// Check if content appears to be template/placeholder
fn is_template_content(content: &str) -> bool {
    let lower = content.to_lowercase();

    // Check for common template patterns
    if content.contains("{{") && content.contains("}}") {
        return true;
    }
    if content.contains("${") && content.contains("}") {
        return true;
    }
    if content.contains("__") && content.matches("__").count() >= 2 {
        // Could be Python dunder or template
        if lower.contains("placeholder") || lower.contains("template") {
            return true;
        }
    }

    // Check for placeholder prices
    if lower.contains("$0.00") || lower.contains("$0") || lower.contains("price: 0") {
        return true;
    }

    // Check for loading states
    if (lower.contains("loading") || lower.contains("please wait"))
        && content.len() < 300
    {
        return true;
    }

    // Check for common placeholder text
    let placeholders = [
        "lorem ipsum",
        "sample text",
        "placeholder",
        "coming soon",
        "to be announced",
        "tba",
        "tbd",
    ];

    for placeholder in placeholders {
        if lower.contains(placeholder) {
            return true;
        }
    }

    false
}

/// Get stability hint for an extraction method
fn stability_for_extraction(extraction: &Extraction) -> f32 {
    match extraction {
        Extraction::Rss => 0.95, // RSS is very stable
        Extraction::JsonLd { .. } => 0.9, // JSON-LD is stable
        Extraction::Meta { .. } => 0.85, // Meta tags are fairly stable
        Extraction::Auto => 0.7, // Auto is moderate
        Extraction::Selector { selector } => {
            // Selector stability depends on specificity
            selector_stability(selector)
        }
        Extraction::Full => 0.5, // Full page changes frequently
    }
}

/// Estimate selector stability based on its specificity
fn selector_stability(selector: &str) -> f32 {
    let mut score: f32 = 0.7; // Base score

    // ID selectors are more stable
    if selector.contains('#') {
        score += 0.15;
    }

    // Data attributes are often stable
    if selector.contains("[data-") {
        score += 0.1;
    }

    // Class selectors are moderately stable
    if selector.contains('.') {
        score += 0.05;
    }

    // Very long/specific selectors are less stable
    if selector.len() > 50 {
        score -= 0.1;
    }

    // Tag-only selectors are unstable
    if !selector.contains('#') && !selector.contains('.') && !selector.contains('[') {
        score -= 0.2;
    }

    score.clamp(0.0, 1.0)
}

/// Try multiple strategies and return the best one with its validation result
pub fn try_strategies_with_fallback(
    url: &str,
    strategies: Vec<Strategy>,
    intent: Intent,
    facts: &PageFacts,
) -> (Strategy, ValidationResult) {
    let mut best_strategy: Option<Strategy> = None;
    let mut best_result: Option<ValidationResult> = None;
    let mut best_score: f32 = -1.0;

    for strategy in strategies {
        let result = validate_strategy(url, &strategy, intent, facts);

        if result.success {
            let score = result.confidence();
            if score > best_score {
                best_score = score;
                best_strategy = Some(strategy.clone());
                best_result = Some(result);
            }

            // If we found a very good result, stop early
            if score >= 0.85 {
                break;
            }
        } else if best_strategy.is_none() {
            // Keep track of first failed attempt as fallback
            best_strategy = Some(strategy.clone());
            best_result = Some(result);
        }
    }

    // Return best result or fallback
    let strategy = best_strategy.unwrap_or_else(|| Strategy {
        engine: Engine::Http,
        extraction: Extraction::Auto,
        reason: "Default fallback".to_string(),
        confidence: 0.3,
        notes: None,
    });

    let result = best_result.unwrap_or_else(|| {
        ValidationResult::failure("No strategies available".to_string(), 0)
    });

    (strategy, result)
}

/// Validate a URL with a specific configuration
pub fn validate_url_config(
    url: &str,
    engine: Engine,
    extraction: Extraction,
    intent: Intent,
) -> Result<ValidationResult> {
    let start = Instant::now();

    // Fetch content
    let headers = HashMap::new();
    let content = match fetch::fetch(url, engine.clone(), &headers) {
        Ok(c) => c,
        Err(e) => {
            return Ok(ValidationResult::failure(
                format!("Fetch failed: {}", e),
                start.elapsed().as_millis() as u64,
            ));
        }
    };

    // Extract
    let extracted = match extract::extract(&content, &extraction) {
        Ok(e) => e,
        Err(e) => {
            return Ok(ValidationResult::failure(
                format!("Extraction failed: {}", e),
                start.elapsed().as_millis() as u64,
            ));
        }
    };

    // Assess quality
    let quality = assess_quality(&extracted, &extraction, intent);
    let runtime_ms = start.elapsed().as_millis() as u64;

    let result = if quality.not_empty && quality.not_template {
        ValidationResult::success(extracted, quality, runtime_ms)
    } else {
        let error = if !quality.not_empty {
            "Extracted content too short"
        } else {
            "Content appears to be template/placeholder"
        };
        ValidationResult::failure(error.to_string(), runtime_ms)
    };

    Ok(result)
}

/// Quick validation check without full fetch (uses provided content)
pub fn quick_validate(content: &str, extraction: &Extraction, intent: Intent) -> DataQuality {
    let page_content = PageContent {
        url: String::new(),
        title: None,
        html: content.to_string(),
        text: None,
    };

    let extracted = extract::extract(&page_content, extraction).unwrap_or_default();
    assess_quality(&extracted, extraction, intent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_price_detection() {
        assert!(has_price_pattern("$99.99"));
        assert!(has_price_pattern("Price: $50"));
        assert!(has_price_pattern("€49.00"));
        assert!(has_price_pattern("USD 100"));
        assert!(!has_price_pattern("Hello world"));
    }

    #[test]
    fn test_template_detection() {
        assert!(is_template_content("Price: {{price}}"));
        assert!(is_template_content("${product.name}"));
        assert!(is_template_content("$0.00"));
        assert!(is_template_content("Loading..."));
        assert!(is_template_content("Lorem ipsum dolor sit amet"));
        assert!(!is_template_content("Nike Air Max - $149.99 - In Stock"));
    }

    #[test]
    fn test_intent_type_checking() {
        assert!(check_expected_type("$99.99", Intent::Price));
        assert!(check_expected_type("In Stock - Add to Cart", Intent::Stock));
        assert!(check_expected_type("Version 2.0 released", Intent::Release));
        assert!(check_expected_type("Software Engineer - Apply Now", Intent::Jobs));
    }

    #[test]
    fn test_selector_stability() {
        assert!(selector_stability("#main-content") > 0.8);
        assert!(selector_stability("[data-product-id]") > 0.7);
        assert!(selector_stability(".product-price") > 0.6);
        assert!(selector_stability("div") < 0.6);
    }

    #[test]
    fn test_quality_score() {
        let good_quality = DataQuality {
            not_empty: true,
            has_expected_type: true,
            not_template: true,
            selector_found: true,
            stability_hint: 0.9,
            notes: vec![],
        };
        assert!(good_quality.score() > 0.8);

        let poor_quality = DataQuality {
            not_empty: false,
            has_expected_type: false,
            not_template: false,
            selector_found: false,
            stability_hint: 0.3,
            notes: vec![],
        };
        assert!(poor_quality.score() < 0.2);
    }
}
