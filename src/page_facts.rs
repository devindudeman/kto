//! PageFacts - Unified feature extraction from web pages
//!
//! This module provides a single, comprehensive struct that captures all relevant
//! features from a web page. Facts are computed once and used throughout the
//! platform detection and configuration pipeline.

use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

/// A link tag from the HTML head
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkTag {
    pub rel: String,
    pub href: Option<String>,
    pub link_type: Option<String>,
    pub title: Option<String>,
}

/// A discovered feed with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredFeed {
    pub url: String,
    pub title: Option<String>,
    pub feed_type: String,
    pub discovery_method: String,
}

/// Unified feature extraction from a web page
///
/// PageFacts captures all relevant information about a page in one pass,
/// enabling consistent platform detection and strategy selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageFacts {
    /// Original URL
    pub url: String,
    /// Final URL after redirects
    pub final_url: String,
    /// Raw HTML content
    #[serde(skip_serializing)]
    pub html: String,
    /// HTTP response headers
    pub headers: HashMap<String, String>,

    // === Extracted features ===

    /// Script src URLs found in the page
    pub script_srcs: Vec<String>,
    /// Stylesheet href URLs
    pub stylesheet_hrefs: Vec<String>,
    /// Link tags from head
    pub link_tags: Vec<LinkTag>,
    /// Meta tags (name/property -> content)
    pub meta_tags: HashMap<String, String>,
    /// JSON-LD structured data (parsed)
    pub json_ld: Option<serde_json::Value>,
    /// JSON-LD types found (e.g., "Product", "Article")
    pub json_ld_types: Vec<String>,
    /// Discovered RSS/Atom feeds
    pub discovered_feeds: Vec<DiscoveredFeed>,

    // === JavaScript-specific (from Playwright if available) ===

    /// Known JS framework globals detected (e.g., "__NEXT_DATA__", "__NUXT__")
    pub js_globals: Vec<String>,
    /// JS-rendered HTML (if different from HTTP HTML)
    #[serde(skip_serializing)]
    pub js_rendered_html: Option<String>,
    /// Plain text from JS render
    pub js_rendered_text: Option<String>,

    // === Computed hints ===

    /// Likely a Single Page Application
    pub is_spa: bool,
    /// Bot protection detected (Cloudflare, CAPTCHA, etc.)
    pub has_bot_protection: bool,
    /// Content length of extracted text
    pub content_length: usize,
    /// Page title
    pub title: Option<String>,
    /// Meta generator tag
    pub meta_generator: Option<String>,
}

impl Default for PageFacts {
    fn default() -> Self {
        Self {
            url: String::new(),
            final_url: String::new(),
            html: String::new(),
            headers: HashMap::new(),
            script_srcs: Vec::new(),
            stylesheet_hrefs: Vec::new(),
            link_tags: Vec::new(),
            meta_tags: HashMap::new(),
            json_ld: None,
            json_ld_types: Vec::new(),
            discovered_feeds: Vec::new(),
            js_globals: Vec::new(),
            js_rendered_html: None,
            js_rendered_text: None,
            is_spa: false,
            has_bot_protection: false,
            content_length: 0,
            title: None,
            meta_generator: None,
        }
    }
}

impl PageFacts {
    /// Create a new PageFacts from URL and HTML content
    pub fn new(url: &str, final_url: &str, html: &str) -> Self {
        let mut facts = Self {
            url: url.to_string(),
            final_url: final_url.to_string(),
            html: html.to_string(),
            ..Default::default()
        };
        facts.extract_features();
        facts
    }

    /// Create PageFacts with both HTTP and JS-rendered content
    pub fn with_js_content(
        url: &str,
        final_url: &str,
        http_html: &str,
        js_html: Option<&str>,
        js_text: Option<&str>,
        headers: HashMap<String, String>,
    ) -> Self {
        let mut facts = Self {
            url: url.to_string(),
            final_url: final_url.to_string(),
            html: http_html.to_string(),
            headers,
            js_rendered_html: js_html.map(String::from),
            js_rendered_text: js_text.map(String::from),
            ..Default::default()
        };
        facts.extract_features();
        facts.detect_spa_indicators();
        facts
    }

    /// Extract all features from the HTML
    fn extract_features(&mut self) {
        let document = Html::parse_document(&self.html);

        self.extract_scripts(&document);
        self.extract_stylesheets(&document);
        self.extract_link_tags(&document);
        self.extract_meta_tags(&document);
        self.extract_json_ld(&document);
        self.discover_feeds(&document);
        self.extract_title(&document);
        self.detect_bot_protection();
        self.calculate_content_length(&document);
    }

    /// Extract script src URLs
    fn extract_scripts(&mut self, document: &Html) {
        if let Ok(selector) = Selector::parse("script[src]") {
            for element in document.select(&selector) {
                if let Some(src) = element.value().attr("src") {
                    self.script_srcs.push(src.to_string());
                }
            }
        }
    }

    /// Extract stylesheet href URLs
    fn extract_stylesheets(&mut self, document: &Html) {
        if let Ok(selector) = Selector::parse("link[rel='stylesheet'][href]") {
            for element in document.select(&selector) {
                if let Some(href) = element.value().attr("href") {
                    self.stylesheet_hrefs.push(href.to_string());
                }
            }
        }
    }

    /// Extract all link tags from head
    fn extract_link_tags(&mut self, document: &Html) {
        if let Ok(selector) = Selector::parse("link") {
            for element in document.select(&selector) {
                let rel = element.value().attr("rel").unwrap_or_default().to_string();
                let href = element.value().attr("href").map(String::from);
                let link_type = element.value().attr("type").map(String::from);
                let title = element.value().attr("title").map(String::from);

                if !rel.is_empty() {
                    self.link_tags.push(LinkTag {
                        rel,
                        href,
                        link_type,
                        title,
                    });
                }
            }
        }
    }

    /// Extract meta tags
    fn extract_meta_tags(&mut self, document: &Html) {
        if let Ok(selector) = Selector::parse("meta[name], meta[property]") {
            for element in document.select(&selector) {
                let key = element
                    .value()
                    .attr("name")
                    .or_else(|| element.value().attr("property"))
                    .map(String::from);
                let content = element.value().attr("content").map(String::from);

                if let (Some(k), Some(c)) = (key, content) {
                    // Special handling for generator
                    if k == "generator" {
                        self.meta_generator = Some(c.clone());
                    }
                    self.meta_tags.insert(k, c);
                }
            }
        }
    }

    /// Extract JSON-LD structured data
    fn extract_json_ld(&mut self, document: &Html) {
        if let Ok(selector) = Selector::parse(r#"script[type="application/ld+json"]"#) {
            let mut json_ld_items: Vec<serde_json::Value> = Vec::new();

            for element in document.select(&selector) {
                let text: String = element.text().collect();
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    // Extract types
                    self.extract_json_ld_types(&json);
                    json_ld_items.push(json);
                }
            }

            if !json_ld_items.is_empty() {
                self.json_ld = if json_ld_items.len() == 1 {
                    Some(json_ld_items.remove(0))
                } else {
                    Some(serde_json::Value::Array(json_ld_items))
                };
            }
        }
    }

    /// Extract @type from JSON-LD recursively (handles nested objects)
    fn extract_json_ld_types(&mut self, json: &serde_json::Value) {
        match json {
            serde_json::Value::Object(map) => {
                // Extract @type from this object
                if let Some(t) = map.get("@type") {
                    match t {
                        serde_json::Value::String(s) => {
                            if !self.json_ld_types.contains(s) {
                                self.json_ld_types.push(s.clone());
                            }
                        }
                        serde_json::Value::Array(arr) => {
                            for item in arr {
                                if let serde_json::Value::String(s) = item {
                                    if !self.json_ld_types.contains(s) {
                                        self.json_ld_types.push(s.clone());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Recursively check all values in this object
                for (key, value) in map {
                    // Skip @context as it's metadata
                    if key != "@context" {
                        self.extract_json_ld_types(value);
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    self.extract_json_ld_types(item);
                }
            }
            _ => {}
        }
    }

    /// Discover RSS/Atom feeds from link tags
    fn discover_feeds(&mut self, document: &Html) {
        if let Ok(selector) = Selector::parse(
            r#"link[rel="alternate"][type="application/rss+xml"],
               link[rel="alternate"][type="application/atom+xml"],
               link[rel="alternate"][type="application/feed+json"]"#,
        ) {
            for element in document.select(&selector) {
                if let Some(href) = element.value().attr("href") {
                    let resolved_url = self.resolve_url(href);
                    let feed_type = element
                        .value()
                        .attr("type")
                        .map(|t| {
                            if t.contains("atom") {
                                "atom"
                            } else if t.contains("json") {
                                "json"
                            } else {
                                "rss"
                            }
                        })
                        .unwrap_or("rss");
                    let title = element.value().attr("title").map(String::from);

                    self.discovered_feeds.push(DiscoveredFeed {
                        url: resolved_url,
                        title,
                        feed_type: feed_type.to_string(),
                        discovery_method: "link-tag".to_string(),
                    });
                }
            }
        }
    }

    /// Resolve a potentially relative URL against the final URL
    fn resolve_url(&self, relative: &str) -> String {
        if relative.starts_with("http://") || relative.starts_with("https://") {
            return relative.to_string();
        }

        if let Ok(base) = Url::parse(&self.final_url) {
            if let Ok(resolved) = base.join(relative) {
                return resolved.to_string();
            }
        }

        relative.to_string()
    }

    /// Extract page title
    fn extract_title(&mut self, document: &Html) {
        if let Ok(selector) = Selector::parse("title") {
            if let Some(element) = document.select(&selector).next() {
                let title: String = element.text().collect();
                self.title = Some(title.trim().to_string());
            }
        }
    }

    /// Detect bot protection patterns
    fn detect_bot_protection(&mut self) {
        let lower_html = self.html.to_lowercase();
        self.has_bot_protection = lower_html.contains("cloudflare")
            || lower_html.contains("cf-ray")
            || lower_html.contains("captcha")
            || lower_html.contains("please enable javascript")
            || lower_html.contains("browser check")
            || lower_html.contains("checking your browser")
            || lower_html.contains("ddos-guard")
            || lower_html.contains("recaptcha");
    }

    /// Calculate content length from visible text
    fn calculate_content_length(&mut self, document: &Html) {
        if let Ok(body_selector) = Selector::parse("body") {
            if let Some(body) = document.select(&body_selector).next() {
                let text: String = body.text().collect::<Vec<_>>().join(" ");
                // Collapse whitespace
                let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                self.content_length = collapsed.len();
            }
        }
    }

    /// Detect SPA indicators from JS globals and content differences
    fn detect_spa_indicators(&mut self) {
        let lower_html = self.html.to_lowercase();

        // Check for known SPA framework indicators
        let spa_indicators = [
            "__NEXT_DATA__",
            "__NUXT__",
            "__GATSBY",
            "window.__REDUX",
            "window.__APOLLO",
            "ng-app",
            "ng-view",
            "data-reactroot",
            "data-react-helmet",
            "_app-root",
        ];

        for indicator in spa_indicators {
            if self.html.contains(indicator) || lower_html.contains(&indicator.to_lowercase()) {
                self.js_globals.push(indicator.to_string());
            }
        }

        // Also check script contents for framework indicators
        let framework_scripts = [
            "/_next/",
            "/_nuxt/",
            "/gatsby-",
            "vue.min.js",
            "react.production",
            "angular.min.js",
        ];

        for script in &self.script_srcs {
            for pattern in framework_scripts {
                if script.contains(pattern) && !self.js_globals.contains(&pattern.to_string()) {
                    self.js_globals.push(pattern.to_string());
                }
            }
        }

        // Content length comparison between HTTP and JS
        let http_len = self.content_length;
        let js_len = self
            .js_rendered_text
            .as_ref()
            .map(|t| t.len())
            .unwrap_or(0);

        // If JS content is significantly larger, likely SPA
        self.is_spa = !self.js_globals.is_empty()
            || (js_len > http_len * 2 && js_len > 500)
            || (http_len < 300 && js_len > 1000);
    }

    /// Check if HTML contains a pattern (case-insensitive)
    pub fn html_contains(&self, pattern: &str) -> bool {
        self.html.to_lowercase().contains(&pattern.to_lowercase())
    }

    /// Check if any script src matches a pattern
    pub fn has_script_from(&self, host_pattern: &str) -> bool {
        self.script_srcs
            .iter()
            .any(|src| src.contains(host_pattern))
    }

    /// Check if any stylesheet href matches a pattern
    pub fn has_stylesheet_from(&self, pattern: &str) -> bool {
        self.stylesheet_hrefs.iter().any(|href| href.contains(pattern))
    }

    /// Get the host from the final URL
    pub fn host(&self) -> Option<String> {
        Url::parse(&self.final_url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
    }

    /// Check if JSON-LD contains a specific type
    pub fn has_json_ld_type(&self, type_name: &str) -> bool {
        self.json_ld_types
            .iter()
            .any(|t| t.eq_ignore_ascii_case(type_name))
    }

    /// Get a summary of key facts for debugging
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(host) = self.host() {
            parts.push(format!("Host: {}", host));
        }

        if !self.json_ld_types.is_empty() {
            parts.push(format!("JSON-LD: {}", self.json_ld_types.join(", ")));
        }

        if !self.discovered_feeds.is_empty() {
            parts.push(format!("{} feed(s)", self.discovered_feeds.len()));
        }

        if let Some(gen) = &self.meta_generator {
            parts.push(format!("Generator: {}", gen));
        }

        if self.is_spa {
            parts.push("SPA detected".to_string());
        }

        if self.has_bot_protection {
            parts.push("Bot protection".to_string());
        }

        parts.push(format!("Content: {} chars", self.content_length));

        parts.join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_extraction() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Test Page</title>
                <meta name="generator" content="WordPress 6.0">
                <link rel="alternate" type="application/rss+xml" href="/feed.xml" title="RSS">
                <link rel="stylesheet" href="/style.css">
                <script src="https://example.com/app.js"></script>
            </head>
            <body>
                <p>Hello world</p>
            </body>
            </html>
        "#;

        let facts = PageFacts::new("https://example.com", "https://example.com", html);

        assert_eq!(facts.title, Some("Test Page".to_string()));
        assert_eq!(
            facts.meta_generator,
            Some("WordPress 6.0".to_string())
        );
        assert_eq!(facts.discovered_feeds.len(), 1);
        assert_eq!(facts.discovered_feeds[0].feed_type, "rss");
        assert_eq!(facts.script_srcs.len(), 1);
        assert!(!facts.is_spa);
    }

    #[test]
    fn test_json_ld_extraction() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <script type="application/ld+json">
                {
                    "@context": "https://schema.org",
                    "@type": "Product",
                    "name": "Test Product",
                    "offers": {
                        "@type": "Offer",
                        "price": "99.99"
                    }
                }
                </script>
            </head>
            <body></body>
            </html>
        "#;

        let facts = PageFacts::new("https://example.com", "https://example.com", html);

        assert!(facts.json_ld.is_some());
        assert!(facts.has_json_ld_type("Product"));
        assert!(facts.has_json_ld_type("Offer"));
    }

    #[test]
    fn test_spa_detection() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <script src="/_next/static/chunks/app.js"></script>
            </head>
            <body>
                <div id="__next"></div>
                <script id="__NEXT_DATA__" type="application/json">{"props":{}}</script>
            </body>
            </html>
        "#;

        let facts = PageFacts::with_js_content(
            "https://example.com",
            "https://example.com",
            html,
            None,
            None,
            HashMap::new(),
        );

        assert!(facts.is_spa);
        assert!(!facts.js_globals.is_empty());
    }

    #[test]
    fn test_bot_protection_detection() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head></head>
            <body>
                <div id="cf-wrapper">
                    Checking your browser before accessing the site.
                </div>
            </body>
            </html>
        "#;

        let facts = PageFacts::new("https://example.com", "https://example.com", html);

        assert!(facts.has_bot_protection);
    }

    #[test]
    fn test_shopify_detection() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <script src="https://cdn.shopify.com/s/files/1/0123/shop.js"></script>
            </head>
            <body>
                <script>window.Shopify = {};</script>
            </body>
            </html>
        "#;

        let facts = PageFacts::new("https://example.myshopify.com", "https://example.myshopify.com", html);

        assert!(facts.html_contains("cdn.shopify.com"));
        assert!(facts.has_script_from("cdn.shopify.com"));
    }
}
