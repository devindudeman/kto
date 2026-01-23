//! Platform Detection Integration
//!
//! This module integrates the platform detection system with the watch creation flow.
//! It provides functions to analyze URLs using the platform KB and select optimal strategies.

use std::collections::HashMap;

use colored::Colorize;
use kto::error::Result;
use kto::fetch::PageContent;
use kto::page_facts::PageFacts;
use kto::platform::{self, PlatformKB, PlatformMatch, Strategy};
use kto::transforms::Intent;
use kto::validate::{self, ValidationResult};
use kto::watch::{Engine, Extraction};

use crate::utils;

/// Result of platform-based URL analysis
#[derive(Debug, Clone)]
pub struct PlatformAnalysis {
    /// Detected platform (if any)
    pub platform_match: Option<PlatformMatch>,
    /// Page facts extracted from the URL
    pub page_facts: PageFacts,
    /// Recommended strategies from KB
    pub kb_strategies: Vec<Strategy>,
    /// Best strategy after validation
    pub best_strategy: Option<Strategy>,
    /// Validation result for best strategy
    pub validation: Option<ValidationResult>,
    /// Human-readable analysis summary
    pub summary: String,
    /// Overall confidence (0.0 - 1.0)
    pub confidence: f32,
}

impl PlatformAnalysis {
    /// Check if a platform was detected
    pub fn has_platform(&self) -> bool {
        self.platform_match.is_some()
    }

    /// Get the recommended engine
    pub fn recommended_engine(&self) -> Engine {
        self.best_strategy
            .as_ref()
            .map(|s| s.engine.clone())
            .unwrap_or(Engine::Http)
    }

    /// Get the recommended extraction
    pub fn recommended_extraction(&self) -> Extraction {
        self.best_strategy
            .as_ref()
            .map(|s| s.extraction.clone())
            .unwrap_or(Extraction::Auto)
    }

    /// Get KB rules as context for AI prompts
    pub fn kb_context_for_ai(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref pm) = self.platform_match {
            parts.push(format!(
                "Platform detected: {} (confidence: {:.0}%)",
                pm.platform_name,
                pm.score * 100.0
            ));

            if !pm.evidence.is_empty() {
                parts.push(format!("Evidence: {}", pm.evidence.join(", ")));
            }
        }

        if !self.kb_strategies.is_empty() {
            parts.push("KB recommends:".to_string());
            for strategy in &self.kb_strategies {
                let engine_str = match &strategy.engine {
                    Engine::Http => "HTTP",
                    Engine::Playwright => "Playwright",
                    Engine::Rss => "RSS",
                    Engine::Shell { .. } => "Shell",
                };
                parts.push(format!("  - Engine: {} ({})", engine_str, strategy.reason));
            }
        }

        if let Some(ref validation) = self.validation {
            if validation.success {
                parts.push(format!(
                    "Validation: Passed (confidence: {:.0}%)",
                    validation.confidence() * 100.0
                ));
            } else if let Some(ref err) = validation.error {
                parts.push(format!("Validation: Failed - {}", err));
            }
        }

        parts.join("\n")
    }
}

/// Analyze a URL using the platform detection system
///
/// This function:
/// 1. Fetches the page (HTTP, optionally JS)
/// 2. Extracts PageFacts
/// 3. Detects platform using KB
/// 4. Gets KB strategies for the detected platform + intent
/// 5. Validates strategies and picks the best one
pub fn analyze_url_with_platform_kb(
    url: &str,
    intent: Intent,
    http_content: Option<&PageContent>,
    js_content: Option<&PageContent>,
) -> Result<PlatformAnalysis> {
    // Load the platform KB
    let kb = PlatformKB::load_default();

    // Build PageFacts from available content
    let (facts, _html_source) = build_page_facts(url, http_content, js_content);

    // Detect platform
    let platform_matches = platform::detect_platform(&facts, &kb);
    let platform_match = platform_matches.into_iter().next();

    // Get KB strategies
    let kb_strategies = if let Some(ref pm) = platform_match {
        platform::get_strategies(&pm.platform_id, intent, &kb)
    } else {
        platform::default_strategies(intent)
    };

    // Validate strategies and find the best one
    let (best_strategy, validation) = if !kb_strategies.is_empty() {
        let (strategy, result) = validate::try_strategies_with_fallback(
            url,
            kb_strategies.clone(),
            intent,
            &facts,
        );
        (Some(strategy), Some(result))
    } else {
        (None, None)
    };

    // Build summary
    let summary = build_summary(&platform_match, &best_strategy, &validation);

    // Calculate overall confidence
    let confidence = calculate_confidence(&platform_match, &validation);

    Ok(PlatformAnalysis {
        platform_match,
        page_facts: facts,
        kb_strategies,
        best_strategy,
        validation,
        summary,
        confidence,
    })
}

/// Build PageFacts from available content
fn build_page_facts(
    url: &str,
    http_content: Option<&PageContent>,
    js_content: Option<&PageContent>,
) -> (PageFacts, &'static str) {
    let headers = HashMap::new();

    match (http_content, js_content) {
        (Some(http), Some(js)) => {
            // Both available - use combined facts
            let facts = PageFacts::with_js_content(
                url,
                &http.url,
                &http.html,
                Some(&js.html),
                js.text.as_deref(),
                headers,
            );
            (facts, "dual")
        }
        (Some(http), None) => {
            // HTTP only
            let facts = PageFacts::new(url, &http.url, &http.html);
            (facts, "http")
        }
        (None, Some(js)) => {
            // JS only
            let facts = PageFacts::new(url, &js.url, &js.html);
            (facts, "js")
        }
        (None, None) => {
            // No content - return empty facts
            (PageFacts::default(), "none")
        }
    }
}

/// Build a human-readable summary
fn build_summary(
    platform_match: &Option<PlatformMatch>,
    best_strategy: &Option<Strategy>,
    validation: &Option<ValidationResult>,
) -> String {
    let mut parts = Vec::new();

    if let Some(ref pm) = platform_match {
        parts.push(format!("Detected {} ({:.0}% confidence)", pm.platform_name, pm.score * 100.0));
    } else {
        parts.push("Unknown platform".to_string());
    }

    if let Some(ref strategy) = best_strategy {
        let engine_str = match &strategy.engine {
            Engine::Http => "HTTP",
            Engine::Playwright => "JavaScript rendering",
            Engine::Rss => "RSS feed",
            Engine::Shell { .. } => "shell command",
        };
        parts.push(format!("Recommended: {}", engine_str));
    }

    if let Some(ref v) = validation {
        if v.success {
            parts.push(format!("Validated ({:.0}%)", v.confidence() * 100.0));
        } else {
            parts.push("Validation failed".to_string());
        }
    }

    parts.join(" | ")
}

/// Calculate overall confidence score
fn calculate_confidence(
    platform_match: &Option<PlatformMatch>,
    validation: &Option<ValidationResult>,
) -> f32 {
    let platform_score = platform_match.as_ref().map(|pm| pm.score).unwrap_or(0.3);
    let validation_score = validation.as_ref().map(|v| v.confidence()).unwrap_or(0.5);

    // Weighted average: platform detection (40%) + validation (60%)
    platform_score * 0.4 + validation_score * 0.6
}

/// Quick platform check without full validation
pub fn quick_platform_check(url: &str, html: &str) -> Option<String> {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new(url, url, html);
    let matches = platform::detect_platform(&facts, &kb);

    matches.into_iter().next().map(|pm| pm.platform_name)
}

/// Get platform-specific recommendations as a formatted string
pub fn get_platform_recommendations(url: &str, html: &str, intent: Intent) -> String {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new(url, url, html);
    let matches = platform::detect_platform(&facts, &kb);

    if let Some(pm) = matches.into_iter().next() {
        let strategies = platform::get_strategies(&pm.platform_id, intent, &kb);

        let mut lines = vec![
            format!("Platform: {} ({:.0}% confidence)", pm.platform_name, pm.score * 100.0),
        ];

        if !strategies.is_empty() {
            lines.push("Recommended strategies:".to_string());
            for (i, s) in strategies.iter().enumerate() {
                let engine_str = match &s.engine {
                    Engine::Http => "HTTP",
                    Engine::Playwright => "Playwright",
                    Engine::Rss => "RSS",
                    Engine::Shell { .. } => "Shell",
                };
                lines.push(format!("  {}. {} - {}", i + 1, engine_str, s.reason));
            }
        }

        if !pm.evidence.is_empty() {
            lines.push(format!("Evidence: {}", pm.evidence.join(", ")));
        }

        lines.join("\n")
    } else {
        "No known platform detected. Using default strategies.".to_string()
    }
}

// ============================================================================
// User-Friendly Preview Functions
// ============================================================================

/// Format a user-friendly preview for a known platform (GitHub, Reddit, etc.)
/// Shows what the user will be watching, not technical config
pub fn format_known_platform_preview(
    url: &str,
    platform_name: &str,
    transform_description: &str,
    latest_item: Option<&str>,
) -> String {
    let domain = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_else(|| "site".to_string());

    let mut output = String::new();
    output.push_str(&format!("✨ {}\n\n", domain.cyan()));
    output.push_str(&format!("   Found: {}\n", transform_description));

    if let Some(item) = latest_item {
        output.push_str(&format!("   Latest: \"{}\"\n", item));
    }

    output
}

/// Format a user-friendly preview for an e-commerce/product page
pub fn format_product_preview(
    url: &str,
    product_name: Option<&str>,
    price: Option<&str>,
    stock_status: Option<&str>,
    platform: Option<&str>,
) -> String {
    let domain = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_else(|| "site".to_string());

    let mut output = String::new();

    // Header with platform if detected
    if let Some(p) = platform {
        output.push_str(&format!("✨ {} ({})\n\n", domain.cyan(), p));
    } else {
        output.push_str(&format!("✨ {}\n\n", domain.cyan()));
    }

    // Product info
    if let Some(name) = product_name {
        output.push_str(&format!("   Found: \"{}\"\n", name));
    }
    if let Some(p) = price {
        output.push_str(&format!("   Price: {}\n", p));
    }
    if let Some(status) = stock_status {
        output.push_str(&format!("   Stock: {}\n", status));
    }

    output
}

/// Format a success message after watch creation (no jargon)
pub fn format_watch_created(
    name: &str,
    intent_description: &str,
    interval_secs: u64,
    uses_ai: bool,
) -> String {
    let interval_str = utils::format_interval(interval_secs);

    let mut output = String::new();
    output.push_str(&format!("\n   Created \"{}\" - {}\n", name.bold(), intent_description));

    if uses_ai {
        output.push_str("   AI will filter and summarize changes\n");
    }

    output.push_str(&format!("   Checking every {} ✓\n", interval_str));

    output
}

/// Convert technical engine/extraction to user-friendly description
pub fn describe_watch_intent(
    engine: &Engine,
    has_agent: bool,
    agent_instructions: Option<&str>,
) -> String {
    // Derive intent from agent instructions if available
    if let Some(instructions) = agent_instructions {
        let lower = instructions.to_lowercase();
        if lower.contains("price") || lower.contains("drop") || lower.contains("$") {
            return "watching for price drops".to_string();
        }
        if lower.contains("stock") || lower.contains("available") || lower.contains("sold out") {
            return "watching for availability changes".to_string();
        }
        if lower.contains("release") || lower.contains("version") {
            return "watching for new releases".to_string();
        }
        if lower.contains("job") || lower.contains("position") || lower.contains("hiring") {
            return "watching for new job postings".to_string();
        }
    }

    // Fallback based on engine
    match engine {
        Engine::Rss => "watching for new items".to_string(),
        Engine::Playwright => {
            if has_agent {
                "watching for changes".to_string()
            } else {
                "watching for visual changes".to_string()
            }
        }
        Engine::Http => "watching for content changes".to_string(),
        Engine::Shell { .. } => "watching command output".to_string(),
    }
}

/// User-friendly error messages with recovery suggestions
pub fn friendly_error_message(error: &str, url: &str) -> String {
    let lower = error.to_lowercase();

    if lower.contains("403") || lower.contains("forbidden") {
        return format!(
            "Site blocked the request. Try: kto new \"{}\" --js\n   (Uses browser mode to bypass basic blocks)",
            url
        );
    }

    if lower.contains("404") || lower.contains("not found") {
        return format!(
            "Page not found at this URL. Please verify:\n   {}",
            url
        );
    }

    if lower.contains("connection") || lower.contains("timeout") || lower.contains("dns") {
        return format!(
            "Couldn't connect to the site. Check:\n   1. The URL is correct: {}\n   2. Your internet connection\n   3. The site is online",
            url
        );
    }

    if lower.contains("empty") || lower.contains("no content") {
        return format!(
            "Page appears empty. The site may need JavaScript.\n   Try: kto new \"{}\" --js",
            url
        );
    }

    if lower.contains("ssl") || lower.contains("certificate") {
        return format!(
            "SSL/certificate error. The site may have an expired certificate:\n   {}",
            url
        );
    }

    // Generic fallback
    format!("Error: {}", error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quick_platform_check_shopify() {
        let html = r#"
            <html>
            <head>
                <script src="https://cdn.shopify.com/s/files/1/0123/shop.js"></script>
            </head>
            <body></body>
            </html>
        "#;

        let result = quick_platform_check("https://example.myshopify.com", html);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Shopify"));
    }

    #[test]
    fn test_quick_platform_check_wordpress() {
        let html = r#"
            <html>
            <head>
                <link rel="stylesheet" href="/wp-content/themes/theme/style.css">
            </head>
            <body></body>
            </html>
        "#;

        let result = quick_platform_check("https://example.com", html);
        assert!(result.is_some());
        assert!(result.unwrap().contains("WordPress"));
    }

    #[test]
    fn test_quick_platform_check_unknown() {
        let html = r#"<html><head></head><body><p>Hello</p></body></html>"#;

        let result = quick_platform_check("https://example.com", html);
        assert!(result.is_none());
    }

    #[test]
    fn test_get_platform_recommendations() {
        let html = r#"
            <html>
            <head>
                <script src="https://cdn.shopify.com/s/files/1/0123/shop.js"></script>
            </head>
            <body></body>
            </html>
        "#;

        let recommendations = get_platform_recommendations(
            "https://example.myshopify.com/products/test",
            html,
            Intent::Stock,
        );

        assert!(recommendations.contains("Shopify"));
        assert!(recommendations.contains("Playwright") || recommendations.contains("strategy"));
    }
}
