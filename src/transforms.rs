//! URL Transform System - Intent-based URL transformations for common sites
//!
//! This module provides declarative URL transformation rules that detect user intent
//! (like "watch for releases") and automatically suggest optimal URLs and engines
//! for well-known sites (GitHub, GitLab, Reddit, etc.).

use crate::watch::Engine;

/// User intent detected from natural language input
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Software releases, changelogs, version updates
    Release,
    /// Price tracking, deals, discounts
    Price,
    /// Availability monitoring, back-in-stock
    Stock,
    /// Job postings, career pages
    Jobs,
    /// News, articles, blog posts
    News,
    /// No specific intent detected
    Generic,
}

impl Intent {
    /// Detect intent from user prompt using keyword matching
    pub fn detect(prompt: &str) -> Self {
        let lower = prompt.to_lowercase();

        // Release/changelog keywords
        if lower.contains("release")
            || lower.contains("changelog")
            || lower.contains("version")
            || lower.contains("update")
            || lower.contains("new version")
        {
            return Intent::Release;
        }

        // Price keywords
        if lower.contains("price")
            || lower.contains("deal")
            || lower.contains("discount")
            || lower.contains("sale")
            || lower.contains("cost")
            || lower.contains("$")
        {
            return Intent::Price;
        }

        // Stock/availability keywords
        if lower.contains("stock")
            || lower.contains("available")
            || lower.contains("availability")
            || lower.contains("back in")
            || lower.contains("restock")
            || lower.contains("inventory")
        {
            return Intent::Stock;
        }

        // Job keywords
        if lower.contains("job")
            || lower.contains("career")
            || lower.contains("hiring")
            || lower.contains("position")
            || lower.contains("opening")
        {
            return Intent::Jobs;
        }

        // News keywords
        if lower.contains("news")
            || lower.contains("article")
            || lower.contains("blog")
            || lower.contains("post")
            || lower.contains("feed")
        {
            return Intent::News;
        }

        Intent::Generic
    }
}

/// How to transform a URL
#[derive(Debug, Clone)]
pub enum UrlTransform {
    /// Append a path to the URL (e.g., "/releases.atom")
    AppendPath(&'static str),
    /// Replace the entire path with a new one
    ReplacePath(&'static str),
    /// Append a suffix to the current path (e.g., ".rss")
    AppendSuffix(&'static str),
}

impl UrlTransform {
    /// Apply the transform to a URL
    pub fn apply(&self, url: &url::Url) -> url::Url {
        let mut result = url.clone();
        match self {
            UrlTransform::AppendPath(path) => {
                let current_path = result.path().trim_end_matches('/');
                result.set_path(&format!("{}{}", current_path, path));
            }
            UrlTransform::ReplacePath(path) => {
                result.set_path(path);
            }
            UrlTransform::AppendSuffix(suffix) => {
                let current_path = result.path().to_string();
                result.set_path(&format!("{}{}", current_path, suffix));
            }
        }
        result
    }
}

/// A declarative rule for transforming URLs based on host, path pattern, and intent
#[derive(Debug, Clone)]
pub struct TransformRule {
    /// Host to match (e.g., "github.com")
    pub host: &'static str,
    /// Optional path pattern to match (glob-like: "/*/*" means exactly two path segments)
    pub path_pattern: Option<&'static str>,
    /// Intent this rule applies to
    pub intent: Intent,
    /// How to transform the URL
    pub transform: UrlTransform,
    /// Engine to use for the transformed URL
    pub engine: Engine,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Human-readable description
    pub description: &'static str,
}

impl TransformRule {
    /// Check if this rule matches the given URL and intent
    pub fn matches(&self, url: &url::Url, intent: Intent) -> bool {
        // Check host
        if url.host_str() != Some(self.host) {
            return false;
        }

        // Check intent
        if self.intent != intent {
            return false;
        }

        // Check path pattern if specified
        if let Some(pattern) = self.path_pattern {
            if !matches_path_pattern(url.path(), pattern) {
                return false;
            }
        }

        true
    }
}

/// Result of a successful transform match
#[derive(Debug, Clone)]
pub struct TransformMatch {
    /// The transformed URL
    pub url: url::Url,
    /// Engine to use
    pub engine: Engine,
    /// Confidence score
    pub confidence: f32,
    /// Human-readable description
    pub description: &'static str,
}

/// Match a URL path against a simple pattern
/// Pattern syntax:
/// - "*" matches a single path segment
/// - "/*/*" matches paths with exactly 2 segments (e.g., "/owner/repo")
/// - "/r/*" matches paths starting with "/r/" and one segment after
fn matches_path_pattern(path: &str, pattern: &str) -> bool {
    let path_segments: Vec<&str> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    let pattern_segments: Vec<&str> = pattern.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();

    if path_segments.len() != pattern_segments.len() {
        return false;
    }

    for (path_seg, pattern_seg) in path_segments.iter().zip(pattern_segments.iter()) {
        if *pattern_seg != "*" && *pattern_seg != *path_seg {
            return false;
        }
    }

    true
}

/// Static array of transform rules for well-known sites
pub static TRANSFORM_RULES: &[TransformRule] = &[
    // GitHub releases
    TransformRule {
        host: "github.com",
        path_pattern: Some("*/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/releases.atom"),
        engine: Engine::Rss,
        confidence: 0.95,
        description: "GitHub releases Atom feed",
    },
    // GitLab releases
    TransformRule {
        host: "gitlab.com",
        path_pattern: Some("*/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/-/releases.atom"),
        engine: Engine::Rss,
        confidence: 0.95,
        description: "GitLab releases Atom feed",
    },
    // Codeberg releases
    TransformRule {
        host: "codeberg.org",
        path_pattern: Some("*/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/releases.rss"),
        engine: Engine::Rss,
        confidence: 0.95,
        description: "Codeberg releases RSS feed",
    },
    // SourceHut releases
    TransformRule {
        host: "sr.ht",
        path_pattern: Some("~*/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/refs/rss.xml"),
        engine: Engine::Rss,
        confidence: 0.90,
        description: "SourceHut refs RSS feed",
    },
    // Hacker News front page
    TransformRule {
        host: "news.ycombinator.com",
        path_pattern: None,
        intent: Intent::News,
        transform: UrlTransform::ReplacePath("/rss"),
        engine: Engine::Rss,
        confidence: 0.95,
        description: "Hacker News RSS feed",
    },
    // Reddit subreddit
    TransformRule {
        host: "www.reddit.com",
        path_pattern: Some("r/*"),
        intent: Intent::News,
        transform: UrlTransform::AppendSuffix(".rss"),
        engine: Engine::Rss,
        confidence: 0.95,
        description: "Reddit subreddit RSS feed",
    },
    // Reddit (without www)
    TransformRule {
        host: "reddit.com",
        path_pattern: Some("r/*"),
        intent: Intent::News,
        transform: UrlTransform::AppendSuffix(".rss"),
        engine: Engine::Rss,
        confidence: 0.95,
        description: "Reddit subreddit RSS feed",
    },
    // Old Reddit
    TransformRule {
        host: "old.reddit.com",
        path_pattern: Some("r/*"),
        intent: Intent::News,
        transform: UrlTransform::AppendSuffix(".rss"),
        engine: Engine::Rss,
        confidence: 0.95,
        description: "Reddit subreddit RSS feed",
    },
    // PyPI releases
    TransformRule {
        host: "pypi.org",
        path_pattern: Some("project/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/rss"),
        engine: Engine::Rss,
        confidence: 0.90,
        description: "PyPI package RSS feed",
    },
    // crates.io (Rust packages) - no native RSS, suggest page monitoring
    TransformRule {
        host: "crates.io",
        path_pattern: Some("crates/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/versions"),
        engine: Engine::Http,
        confidence: 0.80,
        description: "crates.io versions page",
    },
    // npm packages - no native RSS, suggest page monitoring
    TransformRule {
        host: "www.npmjs.com",
        path_pattern: Some("package/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("?activeTab=versions"),
        engine: Engine::Playwright,
        confidence: 0.75,
        description: "npm package versions (requires JS)",
    },
    // Docker Hub tags
    TransformRule {
        host: "hub.docker.com",
        path_pattern: Some("r/*/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/tags"),
        engine: Engine::Playwright,
        confidence: 0.80,
        description: "Docker Hub tags page",
    },
    // Docker Hub (library images)
    TransformRule {
        host: "hub.docker.com",
        path_pattern: Some("_/*"),
        intent: Intent::Release,
        transform: UrlTransform::AppendPath("/tags"),
        engine: Engine::Playwright,
        confidence: 0.80,
        description: "Docker Hub official image tags",
    },
];

/// Try to find a matching transform rule for the given URL and intent
pub fn match_transform(url: &url::Url, intent: Intent) -> Option<TransformMatch> {
    // Only process http/https URLs
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }

    for rule in TRANSFORM_RULES {
        if rule.matches(url, intent) {
            let transformed_url = rule.transform.apply(url);
            return Some(TransformMatch {
                url: transformed_url,
                engine: rule.engine.clone(),
                confidence: rule.confidence,
                description: rule.description,
            });
        }
    }

    None
}

/// Combined function to detect intent and match transform in one call
pub fn detect_and_match(prompt: &str, url: &url::Url) -> Option<TransformMatch> {
    let intent = Intent::detect(prompt);
    if intent == Intent::Generic {
        return None;
    }
    match_transform(url, intent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_detection() {
        assert_eq!(Intent::detect("watch for new releases"), Intent::Release);
        assert_eq!(Intent::detect("alert on price drops"), Intent::Price);
        assert_eq!(Intent::detect("notify when back in stock"), Intent::Stock);
        assert_eq!(Intent::detect("track job postings"), Intent::Jobs);
        assert_eq!(Intent::detect("follow the news"), Intent::News);
        assert_eq!(Intent::detect("just watch this page"), Intent::Generic);
    }

    #[test]
    fn test_github_releases_transform() {
        let url = url::Url::parse("https://github.com/astral-sh/ruff").unwrap();
        let result = match_transform(&url, Intent::Release).unwrap();

        assert_eq!(result.url.as_str(), "https://github.com/astral-sh/ruff/releases.atom");
        assert_eq!(result.engine, Engine::Rss);
        assert!(result.confidence > 0.9);
    }

    #[test]
    fn test_gitlab_releases_transform() {
        let url = url::Url::parse("https://gitlab.com/inkscape/inkscape").unwrap();
        let result = match_transform(&url, Intent::Release).unwrap();

        assert_eq!(result.url.as_str(), "https://gitlab.com/inkscape/inkscape/-/releases.atom");
        assert_eq!(result.engine, Engine::Rss);
    }

    #[test]
    fn test_reddit_news_transform() {
        let url = url::Url::parse("https://www.reddit.com/r/rust").unwrap();
        let result = match_transform(&url, Intent::News).unwrap();

        assert_eq!(result.url.as_str(), "https://www.reddit.com/r/rust.rss");
        assert_eq!(result.engine, Engine::Rss);
    }

    #[test]
    fn test_hn_news_transform() {
        let url = url::Url::parse("https://news.ycombinator.com").unwrap();
        let result = match_transform(&url, Intent::News).unwrap();

        assert_eq!(result.url.as_str(), "https://news.ycombinator.com/rss");
        assert_eq!(result.engine, Engine::Rss);
    }

    #[test]
    fn test_pypi_releases_transform() {
        let url = url::Url::parse("https://pypi.org/project/requests").unwrap();
        let result = match_transform(&url, Intent::Release).unwrap();

        assert_eq!(result.url.as_str(), "https://pypi.org/project/requests/rss");
        assert_eq!(result.engine, Engine::Rss);
    }

    #[test]
    fn test_no_match_wrong_intent() {
        let url = url::Url::parse("https://github.com/astral-sh/ruff").unwrap();
        // GitHub doesn't have price tracking rules
        let result = match_transform(&url, Intent::Price);
        assert!(result.is_none());
    }

    #[test]
    fn test_no_match_unknown_host() {
        let url = url::Url::parse("https://example.com/some/path").unwrap();
        let result = match_transform(&url, Intent::Release);
        assert!(result.is_none());
    }

    #[test]
    fn test_combined_detect_and_match() {
        let url = url::Url::parse("https://github.com/tokio-rs/tokio").unwrap();
        let result = detect_and_match("watch for new releases", &url).unwrap();

        assert_eq!(result.url.as_str(), "https://github.com/tokio-rs/tokio/releases.atom");
    }

    #[test]
    fn test_path_pattern_matching() {
        assert!(matches_path_pattern("/owner/repo", "*/*"));
        assert!(matches_path_pattern("/r/rust", "r/*"));
        assert!(!matches_path_pattern("/owner/repo/extra", "*/*"));
        assert!(!matches_path_pattern("/owner", "*/*"));
    }
}
