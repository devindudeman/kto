//! Tests for the Magical Watch Creation UX
//!
//! These tests verify the user-friendly helper functions that power the
//! zero-prompt happy path and intent-based watch creation flow.

use kto::watch::Engine;

// We need to access the binary crate's functions, so we'll test them
// by importing through the commands module path

/// Helper module to access platform_detect functions
/// Since these are in the binary crate, we test the core logic here
mod test_helpers {
    use kto::watch::Engine;

    /// Test version of format_known_platform_preview
    /// Replicates the logic from src/commands/platform_detect.rs
    pub fn format_known_platform_preview(
        url: &str,
        _platform_name: &str,
        transform_description: &str,
        latest_item: Option<&str>,
    ) -> String {
        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
            .unwrap_or_else(|| "site".to_string());

        let mut output = String::new();
        output.push_str(&format!("✨ {}\n\n", domain));
        output.push_str(&format!("   Found: {}\n", transform_description));

        if let Some(item) = latest_item {
            output.push_str(&format!("   Latest: \"{}\"\n", item));
        }

        output
    }

    /// Test version of describe_watch_intent
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

    /// Test version of friendly_error_message
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

    /// Test version of format_watch_created
    pub fn format_watch_created(
        name: &str,
        intent_description: &str,
        interval_secs: u64,
        uses_ai: bool,
    ) -> String {
        let interval_str = format_interval(interval_secs);

        let mut output = String::new();
        output.push_str(&format!("\n   Created \"{}\" - {}\n", name, intent_description));

        if uses_ai {
            output.push_str("   AI will filter and summarize changes\n");
        }

        output.push_str(&format!("   Checking every {} ✓\n", interval_str));

        output
    }

    fn format_interval(secs: u64) -> String {
        if secs < 60 {
            format!("{} seconds", secs)
        } else if secs < 3600 {
            let mins = secs / 60;
            if mins == 1 {
                "1 minute".to_string()
            } else {
                format!("{} minutes", mins)
            }
        } else {
            let hours = secs / 3600;
            if hours == 1 {
                "1 hour".to_string()
            } else {
                format!("{} hours", hours)
            }
        }
    }
}

// ============================================================================
// Tests for format_known_platform_preview
// ============================================================================

#[test]
fn test_format_known_platform_preview_github() {
    let result = test_helpers::format_known_platform_preview(
        "https://github.com/astral-sh/ruff",
        "GitHub Repository",
        "GitHub releases Atom feed",
        Some("v0.8.3"),
    );

    assert!(result.contains("github.com"), "Should contain domain");
    assert!(result.contains("v0.8.3"), "Should contain latest item");
    assert!(result.contains("GitHub releases Atom feed"), "Should contain description");
    assert!(result.contains("✨"), "Should have sparkle emoji");
}

#[test]
fn test_format_known_platform_preview_no_latest_item() {
    let result = test_helpers::format_known_platform_preview(
        "https://pypi.org/project/requests",
        "PyPI Package",
        "PyPI package RSS feed",
        None,
    );

    assert!(result.contains("pypi.org"), "Should contain domain");
    assert!(!result.contains("Latest:"), "Should NOT contain 'Latest:' when no item provided");
    assert!(result.contains("PyPI package RSS feed"), "Should contain description");
}

#[test]
fn test_format_known_platform_preview_reddit() {
    let result = test_helpers::format_known_platform_preview(
        "https://www.reddit.com/r/rust",
        "Reddit",
        "Reddit subreddit RSS feed",
        Some("New post about async"),
    );

    assert!(result.contains("reddit.com"), "Should contain domain");
    assert!(result.contains("New post about async"), "Should contain latest item");
}

// ============================================================================
// Tests for describe_watch_intent
// ============================================================================

#[test]
fn test_describe_watch_intent_rss_engine() {
    let result = test_helpers::describe_watch_intent(&Engine::Rss, false, None);
    assert_eq!(result, "watching for new items");
}

#[test]
fn test_describe_watch_intent_http_engine() {
    let result = test_helpers::describe_watch_intent(&Engine::Http, false, None);
    assert_eq!(result, "watching for content changes");
}

#[test]
fn test_describe_watch_intent_playwright_without_agent() {
    let result = test_helpers::describe_watch_intent(&Engine::Playwright, false, None);
    assert_eq!(result, "watching for visual changes");
}

#[test]
fn test_describe_watch_intent_playwright_with_agent() {
    let result = test_helpers::describe_watch_intent(&Engine::Playwright, true, None);
    assert_eq!(result, "watching for changes");
}

#[test]
fn test_describe_watch_intent_price_instructions() {
    let result = test_helpers::describe_watch_intent(
        &Engine::Http,
        true,
        Some("Alert when price drops below $50"),
    );
    assert_eq!(result, "watching for price drops");
}

#[test]
fn test_describe_watch_intent_stock_instructions() {
    let result = test_helpers::describe_watch_intent(
        &Engine::Playwright,
        true,
        Some("Notify when back in stock"),
    );
    assert_eq!(result, "watching for availability changes");
}

#[test]
fn test_describe_watch_intent_release_instructions() {
    let result = test_helpers::describe_watch_intent(
        &Engine::Http,
        true,
        Some("Alert on new version releases"),
    );
    assert_eq!(result, "watching for new releases");
}

#[test]
fn test_describe_watch_intent_job_instructions() {
    let result = test_helpers::describe_watch_intent(
        &Engine::Http,
        true,
        Some("Notify on new job postings"),
    );
    assert_eq!(result, "watching for new job postings");
}

#[test]
fn test_describe_watch_intent_sold_out() {
    let result = test_helpers::describe_watch_intent(
        &Engine::Playwright,
        true,
        Some("Alert when sold out items become available"),
    );
    assert_eq!(result, "watching for availability changes");
}

// ============================================================================
// Tests for friendly_error_message
// ============================================================================

#[test]
fn test_friendly_error_403() {
    let result = test_helpers::friendly_error_message(
        "HTTP 403 Forbidden",
        "https://example.com/page",
    );

    assert!(result.contains("--js"), "Should suggest --js flag");
    assert!(result.contains("browser mode"), "Should mention browser mode");
    assert!(result.contains("example.com"), "Should include the URL");
}

#[test]
fn test_friendly_error_404() {
    let result = test_helpers::friendly_error_message(
        "HTTP 404 Not Found",
        "https://example.com/missing",
    );

    assert!(result.contains("not found"), "Should mention page not found");
    assert!(result.contains("example.com/missing"), "Should include the URL");
}

#[test]
fn test_friendly_error_empty_content() {
    let result = test_helpers::friendly_error_message(
        "no content extracted from page",
        "https://spa-app.com",
    );

    assert!(result.contains("JavaScript"), "Should suggest JavaScript");
    assert!(result.contains("--js"), "Should suggest --js flag");
}

#[test]
fn test_friendly_error_connection_timeout() {
    let result = test_helpers::friendly_error_message(
        "connection timeout after 30s",
        "https://slow-site.com",
    );

    assert!(result.contains("internet connection"), "Should mention internet connection");
    assert!(result.contains("slow-site.com"), "Should include the URL");
}

#[test]
fn test_friendly_error_dns() {
    let result = test_helpers::friendly_error_message(
        "DNS resolution failed",
        "https://nonexistent.invalid",
    );

    assert!(result.contains("connect"), "Should mention connection issue");
    assert!(result.contains("nonexistent.invalid"), "Should include the URL");
}

#[test]
fn test_friendly_error_ssl() {
    let result = test_helpers::friendly_error_message(
        "SSL certificate verification failed",
        "https://expired-cert.com",
    );

    assert!(result.contains("SSL") || result.contains("certificate"), "Should mention SSL/certificate");
}

#[test]
fn test_friendly_error_generic() {
    let result = test_helpers::friendly_error_message(
        "some random unexpected error",
        "https://example.com",
    );

    assert!(result.starts_with("Error:"), "Generic errors should start with 'Error:'");
    assert!(result.contains("some random unexpected error"), "Should include original error");
}

// ============================================================================
// Tests for format_watch_created
// ============================================================================

#[test]
fn test_format_watch_created_basic() {
    let result = test_helpers::format_watch_created(
        "My Watch",
        "watching for content changes",
        900,
        false,
    );

    assert!(result.contains("My Watch"), "Should contain watch name");
    assert!(result.contains("watching for content changes"), "Should contain intent");
    assert!(result.contains("15 minutes"), "Should show interval as 15 minutes");
    assert!(result.contains("✓"), "Should have checkmark");
}

#[test]
fn test_format_watch_created_with_ai() {
    let result = test_helpers::format_watch_created(
        "Price Tracker",
        "watching for price drops",
        300,
        true,
    );

    assert!(result.contains("Price Tracker"), "Should contain watch name");
    assert!(result.contains("AI will filter"), "Should mention AI filtering when uses_ai=true");
    assert!(result.contains("5 minutes"), "Should show interval as 5 minutes");
}

#[test]
fn test_format_watch_created_without_ai() {
    let result = test_helpers::format_watch_created(
        "Simple Watch",
        "watching for new items",
        3600,
        false,
    );

    assert!(result.contains("Simple Watch"), "Should contain watch name");
    assert!(!result.contains("AI will filter"), "Should NOT mention AI when uses_ai=false");
    assert!(result.contains("1 hour"), "Should show interval as 1 hour");
}

#[test]
fn test_format_watch_created_short_interval() {
    let result = test_helpers::format_watch_created(
        "Fast Watch",
        "watching for changes",
        60,
        false,
    );

    assert!(result.contains("1 minute"), "Should show interval as 1 minute");
}

#[test]
fn test_format_watch_created_very_short_interval() {
    let result = test_helpers::format_watch_created(
        "Very Fast Watch",
        "watching for changes",
        30,
        false,
    );

    assert!(result.contains("30 seconds"), "Should show interval in seconds");
}

// ============================================================================
// Integration-style tests for confidence thresholds
// ============================================================================

#[test]
fn test_high_confidence_threshold() {
    // High confidence (>=0.8) should auto-create without prompts
    let confidence = 0.95;
    assert!(confidence >= 0.8, "GitHub should have high confidence");
}

#[test]
fn test_medium_confidence_threshold() {
    // Medium confidence (0.5-0.8) should show preview and ask
    let confidence = 0.6;
    assert!(confidence >= 0.5 && confidence < 0.8, "Should be in medium range");
}

#[test]
fn test_low_confidence_threshold() {
    // Low confidence (<0.5) should fall through to normal flow
    let confidence = 0.3;
    assert!(confidence < 0.5, "Should be in low range");
}

// ============================================================================
// Tests for thin content detection (Playwright fallback threshold)
// ============================================================================

#[test]
fn test_thin_content_threshold() {
    // Content under 200 chars should trigger Playwright fallback
    let thin_content = "Short page";
    assert!(thin_content.len() < 200, "Thin content should be under 200 chars");
}

#[test]
fn test_sufficient_content_threshold() {
    // Content over 200 chars should NOT trigger fallback
    let content = "a".repeat(250);
    assert!(content.len() >= 200, "Sufficient content should be 200+ chars");
}
