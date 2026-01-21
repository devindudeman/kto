use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use once_cell::sync::Lazy;
use ureq::ResponseExt;

use crate::config::Config;
use crate::error::{KtoError, Result};
use crate::watch::Engine;

/// Default HTTP request timeout in seconds
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Shared HTTP agent for connection pooling
static HTTP_AGENT: Lazy<ureq::Agent> = Lazy::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(DEFAULT_TIMEOUT_SECS)))
        .build()
        .into()
});

/// Precompiled regex for stripping HTML tags
static HTML_TAG_RE: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"<[^>]+>").expect("Invalid HTML tag regex")
});

/// Precompiled regex for collapsing whitespace
static WHITESPACE_RE: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"\s+").expect("Invalid whitespace regex")
});

/// Content fetched from a page
#[derive(Debug, Clone)]
pub struct PageContent {
    /// Final URL after redirects
    pub url: String,
    /// Page title
    pub title: Option<String>,
    /// Raw HTML content
    pub html: String,
    /// Plain text content (body.innerText)
    pub text: Option<String>,
}

/// Result of probing a URL to determine best engine/extraction
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// Suggested engine based on analysis
    pub suggested_engine: Engine,
    /// RSS/Atom feed URL if found in HTML
    pub rss_url: Option<String>,
    /// Whether page contains JSON-LD structured data
    pub has_jsonld: bool,
    /// JSON-LD type if detected (e.g., "Product", "Article")
    pub jsonld_type: Option<String>,
    /// Content length of extracted text
    pub content_length: usize,
    /// Whether Cloudflare or similar protection detected
    pub has_bot_protection: bool,
    /// Warning/suggestion message for the user
    pub message: Option<String>,
}

/// Fetch a page using the specified engine
pub fn fetch(url: &str, engine: Engine, headers: &HashMap<String, String>) -> Result<PageContent> {
    match engine {
        Engine::Http => fetch_http(url, headers),
        Engine::Playwright => fetch_playwright(url),
        Engine::Rss => fetch_rss(url, headers),
        Engine::Shell { ref command } => fetch_shell(command),
    }
}

/// Fetch content from a shell command
fn fetch_shell(command: &str) -> Result<PageContent> {
    let output = Command::new("sh")
        .args(["-c", command])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::ConfigError(format!(
            "Shell command failed: {}",
            stderr.trim()
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();

    Ok(PageContent {
        url: format!("shell://{}", command),
        title: Some(format!("Shell: {}", truncate_command(command, 50))),
        html: text.clone(),
        text: Some(text),
    })
}

/// Truncate command for display
fn truncate_command(cmd: &str, max_len: usize) -> String {
    if cmd.len() <= max_len {
        cmd.to_string()
    } else {
        format!("{}...", &cmd[..max_len.saturating_sub(3)])
    }
}

/// Fetch using HTTP (ureq)
fn fetch_http(url: &str, headers: &HashMap<String, String>) -> Result<PageContent> {
    let mut request = HTTP_AGENT.get(url);

    // Add custom headers
    for (key, value) in headers {
        request = request.header(key, value);
    }

    // Add default User-Agent
    request = request.header(
        "User-Agent",
        "Mozilla/5.0 (compatible; kto/0.1; +https://github.com/devinbernosky/kto)",
    );

    let response = request.call()?;
    let final_url = response.get_uri().to_string();
    let html = response.into_body().read_to_string()?;

    Ok(PageContent {
        url: final_url,
        title: None, // Will be extracted later
        html,
        text: None, // Will be extracted later
    })
}

/// Fetch using Playwright (Node subprocess)
fn fetch_playwright(url: &str) -> Result<PageContent> {
    let data_dir = Config::data_dir()?;
    let script_path = get_render_script_path()?;

    // Ensure script exists
    if !script_path.exists() {
        ensure_render_script()?;
    }

    // Run from data directory so Node.js can find the local node_modules
    let output = Command::new("node")
        .arg(&script_path)
        .arg(url)
        .arg("30000") // 30 second timeout
        .current_dir(&data_dir)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Try to parse as JSON error
        if let Ok(err) = serde_json::from_str::<serde_json::Value>(&stderr) {
            let msg = err["error"].as_str().unwrap_or("unknown error");
            return Err(KtoError::PlaywrightError(msg.to_string()));
        }
        return Err(KtoError::PlaywrightError(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(&stdout)?;

    Ok(PageContent {
        url: result["url"].as_str().unwrap_or(url).to_string(),
        title: result["title"].as_str().map(String::from),
        html: result["html"].as_str().unwrap_or("").to_string(),
        text: result["text"].as_str().map(String::from),
    })
}

/// Get the path to the Playwright render script
fn get_render_script_path() -> Result<std::path::PathBuf> {
    let data_dir = Config::data_dir()?;
    Ok(data_dir.join("render.mjs"))
}

/// Ensure the render script exists in the data directory
pub fn ensure_render_script() -> Result<()> {
    let script_path = get_render_script_path()?;
    if let Some(parent) = script_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let script_content = include_str!("../assets/render.mjs");
    std::fs::write(&script_path, script_content)?;
    Ok(())
}

/// Check if Playwright is available
pub fn check_playwright() -> PlaywrightStatus {
    // Check if Node is available
    let node_available = Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !node_available {
        return PlaywrightStatus::NodeMissing;
    }

    // Check if Playwright is installed
    let playwright_available = Command::new("npx")
        .args(["playwright", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !playwright_available {
        return PlaywrightStatus::PlaywrightMissing;
    }

    // Check if Chromium browser is installed
    let browser_paths = get_browser_paths();
    for path in browser_paths {
        if std::path::Path::new(&path).exists() {
            return PlaywrightStatus::Ready;
        }
    }

    PlaywrightStatus::BrowserMissing
}

/// Get possible Playwright browser cache paths
fn get_browser_paths() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();

    vec![
        // Linux
        format!("{}/.cache/ms-playwright", home),
        // macOS
        format!("{}/Library/Caches/ms-playwright", home),
    ]
}

/// Status of Playwright installation
#[derive(Debug, Clone, PartialEq)]
pub enum PlaywrightStatus {
    Ready,
    NodeMissing,
    PlaywrightMissing,
    BrowserMissing,
}

impl PlaywrightStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, PlaywrightStatus::Ready)
    }

    pub fn install_instructions(&self) -> &'static str {
        match self {
            PlaywrightStatus::Ready => "Playwright is ready",
            PlaywrightStatus::NodeMissing => "Install Node.js: https://nodejs.org/",
            PlaywrightStatus::PlaywrightMissing => "Run: npm install -g playwright",
            PlaywrightStatus::BrowserMissing => "Run: npx playwright install chromium",
        }
    }
}

/// Determine which engine to use for a URL based on content
pub fn decide_engine(content: &PageContent, extraction_empty: bool) -> Engine {
    // If extraction yielded very little content, probably needs JS
    if extraction_empty || content.html.len() < 100 {
        return Engine::Playwright;
    }

    // Check for common bot-wall patterns
    let lower_html = content.html.to_lowercase();
    if lower_html.contains("cloudflare")
        || lower_html.contains("captcha")
        || lower_html.contains("please enable javascript")
        || lower_html.contains("browser check")
    {
        return Engine::Playwright;
    }

    Engine::Http
}

/// Fetch and parse an RSS/Atom feed
fn fetch_rss(url: &str, headers: &HashMap<String, String>) -> Result<PageContent> {
    use feed_rs::parser;
    use std::collections::BTreeMap;

    let mut request = HTTP_AGENT.get(url);

    // Add custom headers
    for (key, value) in headers {
        request = request.header(key, value);
    }

    // Add default User-Agent
    request = request.header(
        "User-Agent",
        "Mozilla/5.0 (compatible; kto/0.1; +https://github.com/devinbernosky/kto)",
    );

    let response = request.call()?;
    let final_url = response.get_uri().to_string();
    let xml = response.into_body().read_to_string()?;

    // Parse the feed
    let feed = parser::parse(xml.as_bytes())
        .map_err(|e| KtoError::FeedParseError(format!("Failed to parse feed: {}", e)))?;

    // Format items as stable, diffable text (sorted by GUID for consistency)
    let mut items_by_key: BTreeMap<String, String> = BTreeMap::new();

    for entry in &feed.entries {
        // Use entry ID as the stable key
        let guid = entry.id.clone();

        let title = entry
            .title
            .as_ref()
            .map(|t| t.content.clone())
            .unwrap_or_else(|| "(no title)".to_string());

        let link = entry.links.first().map(|l| l.href.clone());

        let published = entry
            .published
            .or(entry.updated)
            .map(|dt| dt.to_rfc3339());

        let summary = entry
            .summary
            .as_ref()
            .map(|s| truncate_text(&strip_html_tags(&s.content), 500))
            .or_else(|| {
                entry
                    .content
                    .as_ref()
                    .and_then(|c| c.body.as_ref())
                    .map(|b| truncate_text(&strip_html_tags(b), 500))
            });

        // Format as structured text block
        let mut item_text = format!("[ITEM guid=\"{}\"]\nTitle: {}\n", guid, title);

        if let Some(p) = &published {
            item_text.push_str(&format!("Published: {}\n", p));
        }
        if let Some(l) = &link {
            item_text.push_str(&format!("Link: {}\n", l));
        }
        if let Some(s) = &summary {
            item_text.push_str(&format!("Summary: {}\n", s));
        }
        item_text.push_str("[/ITEM]");

        items_by_key.insert(guid, item_text);
    }

    // Join sorted items (BTreeMap maintains key order)
    let formatted_text = items_by_key
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n");

    let title = feed.title.map(|t| t.content);

    Ok(PageContent {
        url: final_url,
        title,
        html: xml, // Keep raw XML for storage
        text: Some(formatted_text), // Pre-formatted for diffing
    })
}

/// Strip HTML tags from text
fn strip_html_tags(html: &str) -> String {
    let text = HTML_TAG_RE.replace_all(html, " ");
    // Decode common HTML entities
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    // Collapse whitespace
    WHITESPACE_RE.replace_all(&text, " ").trim().to_string()
}

/// Truncate text to max chars, breaking at word boundary
fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    // Find last space to break at word boundary
    if let Some(pos) = truncated.rfind(' ') {
        format!("{}...", &truncated[..pos])
    } else {
        format!("{}...", truncated)
    }
}

/// Detect if a URL is likely an RSS/Atom feed based on URL pattern
pub fn detect_rss_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("/feed")
        || lower.contains("/rss")
        || lower.contains("/atom")
        || lower.ends_with(".xml")
        || lower.ends_with(".rss")
        || lower.ends_with(".atom")
        || lower.contains("rss.") // e.g., rss.nytimes.com
}

/// Detect RSS from content (check for RSS/Atom XML markers)
pub fn detect_rss_content(body: &str) -> bool {
    let prefix: String = body.chars().take(500).collect();
    let prefix_lower = prefix.to_lowercase();
    prefix_lower.contains("<rss")
        || prefix_lower.contains("<feed")
        || prefix_lower.contains("xmlns=\"http://www.w3.org/2005/atom\"")
}

/// Probe a URL to determine the best engine and extraction method
/// This fetches the page once with HTTP and analyzes the response
pub fn probe_url(url: &str) -> Result<ProbeResult> {
    use scraper::Html;

    let headers = HashMap::new();

    // First, check if URL itself looks like RSS
    if detect_rss_url(url) {
        return Ok(ProbeResult {
            suggested_engine: Engine::Rss,
            rss_url: Some(url.to_string()),
            has_jsonld: false,
            jsonld_type: None,
            content_length: 0,
            has_bot_protection: false,
            message: Some("URL appears to be an RSS/Atom feed".to_string()),
        });
    }

    // Fetch with HTTP
    let content = fetch_http(url, &headers)?;
    let html_lower = content.html.to_lowercase();

    // Check if response is actually RSS/Atom
    if detect_rss_content(&content.html) {
        return Ok(ProbeResult {
            suggested_engine: Engine::Rss,
            rss_url: Some(url.to_string()),
            has_jsonld: false,
            jsonld_type: None,
            content_length: content.html.len(),
            has_bot_protection: false,
            message: Some("Content is RSS/Atom feed".to_string()),
        });
    }

    let document = Html::parse_document(&content.html);

    // Look for RSS/Atom link in HTML (using final URL for relative URL resolution)
    let rss_url = find_rss_link(&document, &content.url);

    // Check for JSON-LD
    let (has_jsonld, jsonld_type) = detect_jsonld(&document);

    // Check for bot protection
    let has_bot_protection = html_lower.contains("cloudflare")
        || html_lower.contains("cf-ray")
        || html_lower.contains("captcha")
        || html_lower.contains("please enable javascript")
        || html_lower.contains("browser check")
        || html_lower.contains("checking your browser");

    // Try to extract content to measure length
    let content_length = if let Ok(readability) = readability_js::Readability::new() {
        if let Ok(article) = readability.parse(&content.html) {
            article.text_content.trim().len()
        } else {
            // Fallback: count visible text in body
            extract_visible_text(&document)
        }
    } else {
        extract_visible_text(&document)
    };

    // Determine suggested engine
    let (suggested_engine, message) = determine_engine(
        content_length,
        has_bot_protection,
        &rss_url,
        has_jsonld,
        &jsonld_type,
    );

    Ok(ProbeResult {
        suggested_engine,
        rss_url,
        has_jsonld,
        jsonld_type,
        content_length,
        has_bot_protection,
        message,
    })
}

/// Find RSS/Atom link in HTML head and resolve relative URLs
fn find_rss_link(document: &scraper::Html, base_url: &str) -> Option<String> {
    use scraper::Selector;

    // Look for <link rel="alternate" type="application/rss+xml" href="...">
    let rss_selector = Selector::parse(
        r#"link[rel="alternate"][type="application/rss+xml"], link[rel="alternate"][type="application/atom+xml"]"#
    ).ok()?;

    let href = document
        .select(&rss_selector)
        .next()
        .and_then(|el| el.value().attr("href"))?;

    // Resolve relative URLs against the base URL
    resolve_url(base_url, href)
}

/// Resolve a potentially relative URL against a base URL
fn resolve_url(base: &str, relative: &str) -> Option<String> {
    // If already absolute, return as-is
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return Some(relative.to_string());
    }

    // Parse base URL to get components
    let base_url = url::Url::parse(base).ok()?;

    // Resolve relative URL
    base_url.join(relative).ok().map(|u| u.to_string())
}

/// Detect JSON-LD structured data and its type
fn detect_jsonld(document: &scraper::Html) -> (bool, Option<String>) {
    use scraper::Selector;

    let jsonld_selector = Selector::parse(r#"script[type="application/ld+json"]"#).ok();

    if let Some(selector) = jsonld_selector {
        for script in document.select(&selector) {
            let text: String = script.text().collect();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                // Handle both single object and array of objects
                let types = extract_jsonld_types(&json);
                if !types.is_empty() {
                    return (true, Some(types.join(", ")));
                }
                return (true, None);
            }
        }
    }

    (false, None)
}

/// Extract @type from JSON-LD (handles arrays and nested @graph)
fn extract_jsonld_types(json: &serde_json::Value) -> Vec<String> {
    let mut types = Vec::new();

    match json {
        serde_json::Value::Object(map) => {
            // Check for @type
            if let Some(t) = map.get("@type") {
                match t {
                    serde_json::Value::String(s) => types.push(s.clone()),
                    serde_json::Value::Array(arr) => {
                        for item in arr {
                            if let serde_json::Value::String(s) = item {
                                types.push(s.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Check @graph for multiple items
            if let Some(serde_json::Value::Array(graph)) = map.get("@graph") {
                for item in graph {
                    types.extend(extract_jsonld_types(item));
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                types.extend(extract_jsonld_types(item));
            }
        }
        _ => {}
    }

    types
}

/// Extract visible text length from document
fn extract_visible_text(document: &scraper::Html) -> usize {
    use scraper::Selector;

    if let Ok(body_selector) = Selector::parse("body") {
        if let Some(body) = document.select(&body_selector).next() {
            let text: String = body.text().collect::<Vec<_>>().join(" ");
            // Collapse whitespace and count
            return WHITESPACE_RE.replace_all(&text, " ").trim().len();
        }
    }
    0
}

/// Determine the best engine based on probe results
fn determine_engine(
    content_length: usize,
    has_bot_protection: bool,
    rss_url: &Option<String>,
    has_jsonld: bool,
    jsonld_type: &Option<String>,
) -> (Engine, Option<String>) {
    // Very sparse content likely needs JS rendering
    if content_length < 300 {
        return (
            Engine::Playwright,
            Some("Page has minimal content - likely requires JavaScript".to_string()),
        );
    }

    // Bot protection detected
    if has_bot_protection {
        return (
            Engine::Playwright,
            Some("Bot protection detected - JavaScript rendering recommended".to_string()),
        );
    }

    // Build informational message
    let mut info_parts = Vec::new();

    if rss_url.is_some() {
        info_parts.push("RSS feed available".to_string());
    }

    if has_jsonld {
        if let Some(t) = jsonld_type {
            info_parts.push(format!("JSON-LD: {}", t));
        } else {
            info_parts.push("JSON-LD structured data found".to_string());
        }
    }

    let message = if info_parts.is_empty() {
        None
    } else {
        Some(info_parts.join(", "))
    };

    (Engine::Http, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playwright_status() {
        let status = PlaywrightStatus::BrowserMissing;
        assert!(!status.is_ready());
        assert!(status.install_instructions().contains("npx playwright"));
    }

    #[test]
    fn test_resolve_url_absolute() {
        let result = resolve_url("https://example.com", "https://other.com/feed");
        assert_eq!(result, Some("https://other.com/feed".to_string()));
    }

    #[test]
    fn test_resolve_url_relative_path() {
        let result = resolve_url("https://example.com/blog/post", "/feed.xml");
        assert_eq!(result, Some("https://example.com/feed.xml".to_string()));
    }

    #[test]
    fn test_resolve_url_relative_no_slash() {
        let result = resolve_url("https://example.com/blog/", "feed.xml");
        assert_eq!(result, Some("https://example.com/blog/feed.xml".to_string()));
    }

    #[test]
    fn test_resolve_url_dot_relative() {
        let result = resolve_url("https://example.com/blog/post", "../feed.xml");
        assert_eq!(result, Some("https://example.com/feed.xml".to_string()));
    }

    #[test]
    fn test_detect_rss_url() {
        assert!(detect_rss_url("https://example.com/feed.xml"));
        assert!(detect_rss_url("https://example.com/feed"));
        assert!(detect_rss_url("https://example.com/rss"));
        assert!(detect_rss_url("https://rss.example.com/news"));
        assert!(!detect_rss_url("https://example.com/page"));
    }

    #[test]
    fn test_detect_rss_content() {
        assert!(detect_rss_content("<?xml version=\"1.0\"?><rss version=\"2.0\">"));
        assert!(detect_rss_content("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(!detect_rss_content("<html><head></head><body></body></html>"));
    }
}
