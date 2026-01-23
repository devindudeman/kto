//! Live tests against real websites
//! These tests require network access

use kto::fetch;
use kto::page_facts::PageFacts;
use kto::platform::{detect_platform, get_strategies, PlatformKB};
use kto::transforms::Intent;
use kto::watch::Engine;
use std::collections::HashMap;

/// Helper to fetch and analyze a URL
fn analyze_live_url(url: &str, intent: Intent) {
    println!("\n=== Testing: {} ===", url);
    println!("Intent: {:?}", intent);

    // Fetch the page
    let content = match fetch::fetch(url, Engine::Http, &HashMap::new()) {
        Ok(c) => c,
        Err(e) => {
            println!("  FETCH ERROR: {}", e);
            return;
        }
    };

    println!("  Fetched {} bytes", content.html.len());

    // Build PageFacts
    let facts = PageFacts::new(url, &content.url, &content.html);
    println!("  PageFacts: {}", facts.summary());

    // Load KB and detect platform
    let kb = PlatformKB::load_default();
    let matches = detect_platform(&facts, &kb);

    if matches.is_empty() {
        println!("  Platform: Unknown (no matches)");
    } else {
        println!("  Platform matches:");
        for m in matches.iter().take(3) {
            println!("    - {} ({:.0}%)", m.platform_name, m.score * 100.0);
            if !m.evidence.is_empty() {
                println!("      Evidence: {}", m.evidence.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
            }
        }

        // Get strategies for the top match
        let top = &matches[0];
        let strategies = get_strategies(&top.platform_id, intent, &kb);
        println!("  Recommended strategies:");
        for (i, s) in strategies.iter().take(2).enumerate() {
            let engine_str = match &s.engine {
                Engine::Http => "HTTP",
                Engine::Playwright => "Playwright",
                Engine::Rss => "RSS",
                Engine::Shell { .. } => "Shell",
            };
            println!("    {}. {} - {}", i + 1, engine_str, s.reason);
        }
    }

    // Show JSON-LD if found
    if !facts.json_ld_types.is_empty() {
        println!("  JSON-LD types: {:?}", facts.json_ld_types);
    }

    // Show discovered feeds
    if !facts.discovered_feeds.is_empty() {
        println!("  Feeds found:");
        for feed in facts.discovered_feeds.iter().take(2) {
            println!("    - {} ({})", feed.url, feed.feed_type);
        }
    }

    println!();
}

#[test]
#[ignore] // Run with: cargo test live -- --ignored --nocapture
fn test_live_github() {
    analyze_live_url("https://github.com/astral-sh/ruff", Intent::Release);
}

#[test]
#[ignore]
fn test_live_pypi() {
    analyze_live_url("https://pypi.org/project/requests/", Intent::Release);
}

#[test]
#[ignore]
fn test_live_hackernews() {
    analyze_live_url("https://news.ycombinator.com/", Intent::News);
}

#[test]
#[ignore]
fn test_live_reddit() {
    analyze_live_url("https://www.reddit.com/r/rust/", Intent::News);
}

#[test]
#[ignore]
fn test_live_crates_io() {
    analyze_live_url("https://crates.io/crates/serde", Intent::Release);
}

#[test]
#[ignore]
fn test_live_npm() {
    analyze_live_url("https://www.npmjs.com/package/react", Intent::Release);
}

#[test]
#[ignore]
fn test_live_wordpress_blog() {
    // WordPress.org blog uses WordPress
    analyze_live_url("https://wordpress.org/news/", Intent::News);
}

/// Run all live tests at once
#[test]
#[ignore]
fn test_all_live_platforms() {
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║           Live Platform Detection Tests                       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");

    let tests = vec![
        ("https://github.com/astral-sh/ruff", Intent::Release, "GitHub"),
        ("https://pypi.org/project/requests/", Intent::Release, "PyPI"),
        ("https://news.ycombinator.com/", Intent::News, "Hacker News"),
        ("https://crates.io/crates/serde", Intent::Release, "crates.io"),
        ("https://wordpress.org/news/", Intent::News, "WordPress"),
    ];

    let mut passed = 0;
    let mut failed = 0;

    for (url, intent, expected_name) in tests {
        println!("\n--- Testing {} ({}) ---", expected_name, url);

        let content = match fetch::fetch(url, Engine::Http, &HashMap::new()) {
            Ok(c) => c,
            Err(e) => {
                println!("  ✗ FETCH FAILED: {}", e);
                failed += 1;
                continue;
            }
        };

        let facts = PageFacts::new(url, &content.url, &content.html);
        let kb = PlatformKB::load_default();
        let matches = detect_platform(&facts, &kb);

        if matches.is_empty() {
            println!("  ✗ No platform detected (expected {})", expected_name);
            failed += 1;
        } else {
            let top = &matches[0];
            println!("  ✓ Detected: {} ({:.0}%)", top.platform_name, top.score * 100.0);

            let strategies = get_strategies(&top.platform_id, intent, &kb);
            if !strategies.is_empty() {
                let engine_str = match &strategies[0].engine {
                    Engine::Http => "HTTP",
                    Engine::Playwright => "Playwright",
                    Engine::Rss => "RSS",
                    Engine::Shell { .. } => "Shell",
                };
                println!("  ✓ Strategy: {} - {}", engine_str, strategies[0].reason);
            }
            passed += 1;
        }
    }

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║  Results: {} passed, {} failed                                 ║", passed, failed);
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    assert!(failed == 0, "Some live tests failed");
}

#[test]
#[ignore]
fn test_live_shopify_store() {
    // Test a real Shopify store
    analyze_live_url("https://shop.lego.com/en-US", Intent::Price);
}

#[test]
#[ignore]
fn test_live_nextjs_app() {
    // Vercel's own site uses Next.js
    analyze_live_url("https://vercel.com", Intent::News);
}

#[test]
#[ignore]
fn test_live_real_shopify() {
    // Allbirds is a well-known Shopify store
    analyze_live_url("https://www.allbirds.com/products/mens-wool-runners", Intent::Stock);
}
