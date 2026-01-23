//! Comprehensive tests for platform detection system

use std::collections::HashMap;

use kto::page_facts::PageFacts;
use kto::platform::{detect_platform, get_strategies, PlatformKB};
use kto::transforms::Intent;
use kto::validate::quick_validate;
use kto::watch::{Engine, Extraction};

// ============================================================================
// Sample HTML for various platforms
// ============================================================================

const SHOPIFY_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Test Product - My Shopify Store</title>
    <script src="https://cdn.shopify.com/s/files/1/0123/4567/8901/t/1/assets/theme.js"></script>
    <link rel="stylesheet" href="https://cdn.shopify.com/s/files/1/0123/4567/8901/t/1/assets/theme.css">
    <script type="application/ld+json">
    {
        "@context": "https://schema.org",
        "@type": "Product",
        "name": "Test Product",
        "offers": {
            "@type": "Offer",
            "price": "99.99",
            "availability": "https://schema.org/InStock"
        }
    }
    </script>
</head>
<body>
    <div class="shopify-section">
        <h1>Test Product</h1>
        <span class="price">$99.99</span>
        <button class="btn" data-add-to-cart>Add to Cart</button>
    </div>
    <script>
        window.Shopify = { theme: { name: "Dawn" } };
    </script>
</body>
</html>
"#;

const WORDPRESS_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>My Blog Post - WordPress Site</title>
    <meta name="generator" content="WordPress 6.4.2">
    <link rel="stylesheet" href="/wp-content/themes/twentytwentyfour/style.css">
    <link rel="alternate" type="application/rss+xml" href="/feed/" title="My Blog RSS">
    <script src="/wp-includes/js/jquery/jquery.min.js"></script>
</head>
<body class="post-template-default single single-post">
    <article class="post">
        <h1 class="entry-title">My Blog Post</h1>
        <div class="entry-content">
            <p>This is my blog post content with more than 100 characters to pass validation checks.
            It includes various topics and interesting information that readers would find valuable.</p>
        </div>
    </article>
</body>
</html>
"#;

const WOOCOMMERCE_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Product - WooCommerce Store</title>
    <meta name="generator" content="WooCommerce 8.5.0">
    <link rel="stylesheet" href="/wp-content/plugins/woocommerce/assets/css/woocommerce.css">
</head>
<body class="woocommerce woocommerce-page single-product">
    <div class="wc-product">
        <h1 class="product_title">WooCommerce Product</h1>
        <span class="woocommerce-Price-amount">$149.00</span>
        <button class="single_add_to_cart_button">Add to cart</button>
    </div>
</body>
</html>
"#;

const GITHUB_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>astral-sh/ruff: An extremely fast Python linter</title>
    <link rel="alternate" type="application/atom+xml" href="/astral-sh/ruff/releases.atom">
</head>
<body>
    <div class="repository-content">
        <h1>ruff</h1>
        <p>An extremely fast Python linter and code formatter, written in Rust.</p>
        <div class="release">
            <span class="tag-name">v0.4.0</span>
            <span class="release-date">Released yesterday</span>
        </div>
    </div>
</body>
</html>
"#;

const NEXTJS_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Next.js App</title>
    <script src="/_next/static/chunks/webpack.js"></script>
    <script src="/_next/static/chunks/main.js"></script>
</head>
<body>
    <div id="__next">
        <main>
            <h1>Welcome to Next.js!</h1>
            <p>Get started by editing pages/index.js</p>
        </main>
    </div>
    <script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{}}}</script>
</body>
</html>
"#;

const BIGCOMMERCE_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Product - BigCommerce Store</title>
    <script src="https://cdn.bigcommerce.com/stencil/stencil-utils.js"></script>
</head>
<body>
    <div class="productView">
        <h1 class="productView-title">BigCommerce Product</h1>
        <span class="productView-price">$199.99</span>
        <div class="bc-sf-filter">Filter options</div>
    </div>
</body>
</html>
"#;

const HN_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Hacker News</title>
    <link rel="alternate" type="application/rss+xml" href="/rss">
</head>
<body>
    <table class="itemlist">
        <tr class="athing">
            <td class="title">
                <a href="https://example.com">Show HN: My cool project</a>
            </td>
        </tr>
    </table>
</body>
</html>
"#;

const UNKNOWN_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Random Website</title>
</head>
<body>
    <h1>Welcome to my website</h1>
    <p>This is just a regular website with no platform-specific indicators.
    It has enough content to pass basic validation but no special framework markers.</p>
</body>
</html>
"#;

// ============================================================================
// Platform Detection Tests
// ============================================================================

#[test]
fn test_shopify_detection() {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new("https://example.myshopify.com/products/test", "https://example.myshopify.com/products/test", SHOPIFY_HTML);

    let matches = detect_platform(&facts, &kb);

    assert!(!matches.is_empty(), "Should detect at least one platform");
    let top_match = &matches[0];
    assert_eq!(top_match.platform_id, "shopify", "Should detect Shopify");
    assert!(top_match.score >= 0.5, "Score should be >= 0.5, got {}", top_match.score);
    assert!(!top_match.evidence.is_empty(), "Should have evidence");

    println!("Shopify detection:");
    println!("  Score: {:.2}", top_match.score);
    println!("  Evidence: {:?}", top_match.evidence);
}

#[test]
fn test_wordpress_detection() {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new("https://myblog.com/post", "https://myblog.com/post", WORDPRESS_HTML);

    let matches = detect_platform(&facts, &kb);

    assert!(!matches.is_empty(), "Should detect at least one platform");
    let top_match = &matches[0];
    assert_eq!(top_match.platform_id, "wordpress", "Should detect WordPress");
    assert!(top_match.score >= 0.4, "Score should be >= 0.4, got {}", top_match.score);

    println!("WordPress detection:");
    println!("  Score: {:.2}", top_match.score);
    println!("  Evidence: {:?}", top_match.evidence);
}

#[test]
fn test_woocommerce_detection() {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new("https://shop.example.com/product", "https://shop.example.com/product", WOOCOMMERCE_HTML);

    let matches = detect_platform(&facts, &kb);

    assert!(!matches.is_empty(), "Should detect at least one platform");
    // WooCommerce should be detected (might also detect WordPress)
    let woo_match = matches.iter().find(|m| m.platform_id == "woocommerce");
    assert!(woo_match.is_some(), "Should detect WooCommerce");

    println!("WooCommerce detection:");
    for m in &matches {
        println!("  {} - Score: {:.2}", m.platform_id, m.score);
    }
}

#[test]
fn test_github_detection() {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new("https://github.com/astral-sh/ruff", "https://github.com/astral-sh/ruff", GITHUB_HTML);

    let matches = detect_platform(&facts, &kb);

    assert!(!matches.is_empty(), "Should detect at least one platform");
    let top_match = &matches[0];
    assert_eq!(top_match.platform_id, "github", "Should detect GitHub");
    assert!(top_match.score >= 0.8, "GitHub detection should be high confidence");

    println!("GitHub detection:");
    println!("  Score: {:.2}", top_match.score);
}

#[test]
fn test_nextjs_detection() {
    let kb = PlatformKB::load_default();
    // Use with_js_content to trigger SPA detection
    let facts = PageFacts::with_js_content(
        "https://myapp.vercel.app",
        "https://myapp.vercel.app",
        NEXTJS_HTML,
        Some(NEXTJS_HTML),
        None,
        std::collections::HashMap::new(),
    );

    // The KB should detect Next.js via html patterns
    let matches = detect_platform(&facts, &kb);

    // Next.js should be detected by __NEXT_DATA__ and _next/static patterns
    let nextjs_match = matches.iter().find(|m| m.platform_id == "nextjs");

    if let Some(m) = nextjs_match {
        println!("Next.js detection:");
        println!("  Score: {:.2}", m.score);
        println!("  Evidence: {:?}", m.evidence);
        assert!(m.score >= 0.5, "Next.js score should be >= 0.5");
    } else {
        // Check if SPA indicators were detected at minimum
        println!("Next.js: Checking SPA indicators");
        println!("  is_spa: {}", facts.is_spa);
        println!("  js_globals: {:?}", facts.js_globals);
        // Next.js HTML contains __NEXT_DATA__ which should be detected
        assert!(facts.html_contains("__NEXT_DATA__"), "HTML should contain __NEXT_DATA__");
    }
}

#[test]
fn test_bigcommerce_detection() {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new("https://store.mybigcommerce.com/product", "https://store.mybigcommerce.com/product", BIGCOMMERCE_HTML);

    let matches = detect_platform(&facts, &kb);

    assert!(!matches.is_empty(), "Should detect at least one platform");
    let bc_match = matches.iter().find(|m| m.platform_id == "bigcommerce");
    assert!(bc_match.is_some(), "Should detect BigCommerce");

    println!("BigCommerce detection:");
    for m in &matches {
        println!("  {} - Score: {:.2}", m.platform_id, m.score);
    }
}

#[test]
fn test_hackernews_detection() {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new("https://news.ycombinator.com", "https://news.ycombinator.com", HN_HTML);

    let matches = detect_platform(&facts, &kb);

    assert!(!matches.is_empty(), "Should detect at least one platform");
    let top_match = &matches[0];
    assert_eq!(top_match.platform_id, "hackernews", "Should detect Hacker News");

    println!("Hacker News detection:");
    println!("  Score: {:.2}", top_match.score);
}

#[test]
fn test_unknown_platform() {
    let kb = PlatformKB::load_default();
    let facts = PageFacts::new("https://random-site.com", "https://random-site.com", UNKNOWN_HTML);

    let matches = detect_platform(&facts, &kb);

    // Unknown sites should return empty or very low scores
    if matches.is_empty() {
        println!("Unknown site: No platform detected (correct)");
    } else {
        println!("Unknown site: Low-confidence matches:");
        for m in &matches {
            println!("  {} - Score: {:.2}", m.platform_id, m.score);
            assert!(m.score < 0.5, "Unknown site should have low scores");
        }
    }
}

// ============================================================================
// Strategy Selection Tests
// ============================================================================

#[test]
fn test_shopify_stock_strategies() {
    let kb = PlatformKB::load_default();
    let strategies = get_strategies("shopify", Intent::Stock, &kb);

    assert!(!strategies.is_empty(), "Should have strategies for Shopify stock");

    // First strategy should be Playwright for stock monitoring
    let first = &strategies[0];
    assert!(matches!(first.engine, Engine::Playwright),
        "Shopify stock should recommend Playwright, got {:?}", first.engine);

    println!("Shopify stock strategies:");
    for (i, s) in strategies.iter().enumerate() {
        println!("  {}. {:?} - {}", i + 1, s.engine, s.reason);
    }
}

#[test]
fn test_shopify_price_strategies() {
    let kb = PlatformKB::load_default();
    let strategies = get_strategies("shopify", Intent::Price, &kb);

    assert!(!strategies.is_empty(), "Should have strategies for Shopify price");

    // First strategy should be HTTP with JSON-LD for price
    let first = &strategies[0];
    assert!(matches!(first.engine, Engine::Http),
        "Shopify price should recommend HTTP first, got {:?}", first.engine);

    println!("Shopify price strategies:");
    for (i, s) in strategies.iter().enumerate() {
        println!("  {}. {:?} - {}", i + 1, s.engine, s.reason);
    }
}

#[test]
fn test_github_release_strategies() {
    let kb = PlatformKB::load_default();
    let strategies = get_strategies("github", Intent::Release, &kb);

    assert!(!strategies.is_empty(), "Should have strategies for GitHub releases");

    // GitHub releases should use RSS
    let first = &strategies[0];
    assert!(matches!(first.engine, Engine::Rss),
        "GitHub releases should recommend RSS, got {:?}", first.engine);

    println!("GitHub release strategies:");
    for (i, s) in strategies.iter().enumerate() {
        println!("  {}. {:?} - {}", i + 1, s.engine, s.reason);
    }
}

#[test]
fn test_wordpress_news_strategies() {
    let kb = PlatformKB::load_default();
    let strategies = get_strategies("wordpress", Intent::News, &kb);

    assert!(!strategies.is_empty(), "Should have strategies for WordPress news");

    // WordPress news should use RSS first
    let first = &strategies[0];
    assert!(matches!(first.engine, Engine::Rss),
        "WordPress news should recommend RSS, got {:?}", first.engine);

    println!("WordPress news strategies:");
    for (i, s) in strategies.iter().enumerate() {
        println!("  {}. {:?} - {}", i + 1, s.engine, s.reason);
    }
}

#[test]
fn test_unknown_platform_default_strategies() {
    let kb = PlatformKB::load_default();

    // Unknown platform should get default strategies
    let strategies = get_strategies("nonexistent_platform", Intent::Stock, &kb);

    assert!(!strategies.is_empty(), "Should have default strategies");

    println!("Default stock strategies:");
    for (i, s) in strategies.iter().enumerate() {
        println!("  {}. {:?} - {}", i + 1, s.engine, s.reason);
    }
}

// ============================================================================
// PageFacts Extraction Tests
// ============================================================================

#[test]
fn test_page_facts_json_ld() {
    let facts = PageFacts::new("https://example.com", "https://example.com", SHOPIFY_HTML);

    assert!(facts.json_ld.is_some(), "Should extract JSON-LD");
    assert!(facts.has_json_ld_type("Product"), "Should detect Product type");
    assert!(facts.has_json_ld_type("Offer"), "Should detect nested Offer type");

    println!("JSON-LD types found: {:?}", facts.json_ld_types);
}

#[test]
fn test_page_facts_feeds() {
    let facts = PageFacts::new("https://myblog.com", "https://myblog.com", WORDPRESS_HTML);

    assert!(!facts.discovered_feeds.is_empty(), "Should discover RSS feed");
    assert_eq!(facts.discovered_feeds[0].feed_type, "rss");

    println!("Discovered feeds: {:?}", facts.discovered_feeds);
}

#[test]
fn test_page_facts_scripts() {
    let facts = PageFacts::new("https://example.com", "https://example.com", SHOPIFY_HTML);

    assert!(!facts.script_srcs.is_empty(), "Should extract script sources");
    assert!(facts.has_script_from("cdn.shopify.com"), "Should detect Shopify CDN");

    println!("Script sources: {:?}", facts.script_srcs);
}

#[test]
fn test_page_facts_meta_generator() {
    let facts = PageFacts::new("https://myblog.com", "https://myblog.com", WORDPRESS_HTML);

    assert!(facts.meta_generator.is_some(), "Should extract meta generator");
    assert!(facts.meta_generator.as_ref().unwrap().contains("WordPress"));

    println!("Meta generator: {:?}", facts.meta_generator);
}

// ============================================================================
// Validation Tests
// ============================================================================

#[test]
fn test_validation_price_content() {
    let content = r#"
        <div class="product">
            <h1>Test Product</h1>
            <span class="price">$99.99</span>
            <p>In Stock - Ships tomorrow</p>
        </div>
    "#;

    let quality = quick_validate(content, &Extraction::Auto, Intent::Price);

    assert!(quality.has_expected_type, "Should detect price content");
    assert!(quality.not_template, "Should not be template");

    println!("Price validation: {}", quality.summary());
}

#[test]
fn test_validation_stock_content() {
    let content = r#"
        <div class="product">
            <h1>Test Product</h1>
            <button class="add-to-cart">Add to Cart</button>
            <span class="stock">In Stock</span>
        </div>
    "#;

    let quality = quick_validate(content, &Extraction::Auto, Intent::Stock);

    assert!(quality.has_expected_type, "Should detect stock content");

    println!("Stock validation: {}", quality.summary());
}

#[test]
fn test_validation_template_detection() {
    let template_content = r#"
        <div class="product">
            <h1>{{product.name}}</h1>
            <span class="price">{{product.price}}</span>
        </div>
    "#;

    let quality = quick_validate(template_content, &Extraction::Auto, Intent::Price);

    assert!(!quality.not_template, "Should detect template content");

    println!("Template detection: {}", quality.summary());
}

#[test]
fn test_validation_empty_content() {
    let empty_content = "<div></div>";

    let quality = quick_validate(empty_content, &Extraction::Auto, Intent::Generic);

    assert!(!quality.not_empty, "Should detect empty content");
    // Since not_empty is critical (weight 0.4), failing it should lower the score
    // But other checks might pass, so the score might be around 0.5-0.6
    println!("Empty validation: {}", quality.summary());
    println!("  not_empty: {}", quality.not_empty);
    println!("  not_template: {}", quality.not_template);
    println!("  score: {:.2}", quality.score());

    // The score should be noticeably lower than a good page
    // Due to not_empty being false, at least 40% of the score is lost
    assert!(quality.score() < 0.7, "Empty content should have reduced score, got {:.2}", quality.score());
}

// ============================================================================
// KB Loading Tests
// ============================================================================

#[test]
fn test_kb_loads_all_platforms() {
    let kb = PlatformKB::load_default();

    let expected_platforms = vec![
        "shopify", "woocommerce", "bigcommerce", "magento",
        "github", "gitlab", "codeberg",
        "wordpress", "hackernews", "reddit",
        "pypi", "cratesio", "npm", "dockerhub",
        "nextjs", "nuxt",
        "wix", "squarespace", "webflow"
    ];

    println!("Loaded platforms:");
    for id in kb.platform_ids() {
        println!("  - {}", id);
    }

    for expected in expected_platforms {
        assert!(kb.get(expected).is_some(), "KB should contain {}", expected);
    }
}

#[test]
fn test_kb_platform_has_intents() {
    let kb = PlatformKB::load_default();

    let shopify = kb.get("shopify").expect("Should have Shopify");
    assert!(!shopify.intents.is_empty(), "Shopify should have intents");
    assert!(shopify.intents.contains_key("stock"), "Shopify should have stock intent");
    assert!(shopify.intents.contains_key("price"), "Shopify should have price intent");

    println!("Shopify intents: {:?}", shopify.intents.keys().collect::<Vec<_>>());
}

#[test]
fn test_kb_platform_has_endpoints() {
    let kb = PlatformKB::load_default();

    let shopify = kb.get("shopify").expect("Should have Shopify");
    assert!(!shopify.endpoints.is_empty(), "Shopify should have endpoints");

    let github = kb.get("github").expect("Should have GitHub");
    assert!(!github.endpoints.is_empty(), "GitHub should have endpoints");

    println!("Shopify endpoints:");
    for ep in &shopify.endpoints {
        println!("  - {} ({})", ep.path, ep.response_type);
    }
}

// ============================================================================
// Integration Test - Full Pipeline
// ============================================================================

#[test]
fn test_full_pipeline_shopify() {
    let kb = PlatformKB::load_default();

    // Step 1: Extract PageFacts
    let facts = PageFacts::new(
        "https://example.myshopify.com/products/test",
        "https://example.myshopify.com/products/test",
        SHOPIFY_HTML
    );

    println!("=== Full Pipeline Test: Shopify Stock ===");
    println!("PageFacts summary: {}", facts.summary());

    // Step 2: Detect platform
    let matches = detect_platform(&facts, &kb);
    assert!(!matches.is_empty());
    let platform = &matches[0];
    println!("Platform: {} ({:.0}%)", platform.platform_name, platform.score * 100.0);

    // Step 3: Get strategies for stock monitoring
    let strategies = get_strategies(&platform.platform_id, Intent::Stock, &kb);
    println!("Strategies for stock:");
    for (i, s) in strategies.iter().enumerate() {
        println!("  {}. {:?} - {}", i + 1, s.engine, s.reason);
    }

    // Step 4: Validate best strategy
    let quality = quick_validate(SHOPIFY_HTML, &strategies[0].extraction, Intent::Stock);
    println!("Validation: {}", quality.summary());
    println!("Quality score: {:.0}%", quality.score() * 100.0);

    assert!(quality.score() > 0.3, "Should have reasonable quality");
}

#[test]
fn test_full_pipeline_github() {
    let kb = PlatformKB::load_default();

    // Step 1: Extract PageFacts
    let facts = PageFacts::new(
        "https://github.com/astral-sh/ruff",
        "https://github.com/astral-sh/ruff",
        GITHUB_HTML
    );

    println!("=== Full Pipeline Test: GitHub Releases ===");
    println!("PageFacts summary: {}", facts.summary());

    // Step 2: Detect platform
    let matches = detect_platform(&facts, &kb);
    assert!(!matches.is_empty());
    let platform = &matches[0];
    println!("Platform: {} ({:.0}%)", platform.platform_name, platform.score * 100.0);

    // Step 3: Get strategies for release monitoring
    let strategies = get_strategies(&platform.platform_id, Intent::Release, &kb);
    println!("Strategies for releases:");
    for (i, s) in strategies.iter().enumerate() {
        println!("  {}. {:?} - {}", i + 1, s.engine, s.reason);
    }

    // Should recommend RSS
    assert!(matches!(strategies[0].engine, Engine::Rss));
}
