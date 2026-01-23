//! Platform Detection - Score-based platform identification using KB rules
//!
//! This module provides platform detection using a knowledge base of rules.
//! Each platform has detection signals with weights, and the highest-scoring
//! match above threshold is returned.

use crate::page_facts::PageFacts;
use crate::transforms::Intent;
use crate::watch::{Engine, Extraction};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// A platform match with confidence and evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformMatch {
    /// Platform identifier (e.g., "shopify", "wordpress")
    pub platform_id: String,
    /// Human-readable platform name
    pub platform_name: String,
    /// Match score (0.0 - 1.0)
    pub score: f32,
    /// Which signals matched
    pub evidence: Vec<String>,
    /// Specific rules that matched
    pub matched_rules: Vec<String>,
}

/// A monitoring strategy for a platform + intent combination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    /// Engine to use
    pub engine: Engine,
    /// Extraction method
    pub extraction: Extraction,
    /// Why this strategy is recommended
    pub reason: String,
    /// Confidence in this strategy (0.0 - 1.0)
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    /// Additional notes or caveats
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_confidence() -> f32 {
    0.8
}

/// A detection signal with weight
#[derive(Debug, Clone, Deserialize)]
pub struct DetectionSignal {
    /// Type of signal: "html_contains", "script_src", "meta_generator", "domain", "json_ld_type"
    #[serde(rename = "type")]
    pub signal_type: String,
    /// Pattern to match
    pub pattern: String,
    /// Weight for scoring (0.0 - 1.0)
    pub weight: f32,
}

/// Detection configuration for a platform
#[derive(Debug, Clone, Deserialize)]
pub struct DetectionConfig {
    /// Domains that indicate this platform (exact match)
    #[serde(default)]
    pub domains: Vec<String>,
    /// HTML patterns to search for
    #[serde(default)]
    pub html_signatures: Vec<String>,
    /// Script host patterns
    #[serde(default)]
    pub script_hosts: Vec<String>,
    /// Meta generator patterns
    #[serde(default)]
    pub meta_generator: Vec<String>,
    /// Minimum score threshold
    #[serde(default = "default_threshold")]
    pub weight_threshold: f32,
    /// Individual signals with weights
    #[serde(default)]
    pub signals: Vec<DetectionSignal>,
}

fn default_threshold() -> f32 {
    0.5
}

/// Intent-specific strategies
#[derive(Debug, Clone, Deserialize)]
pub struct IntentStrategies {
    /// Ranked list of strategies to try
    pub strategies: Vec<StrategyConfig>,
    /// Whether variants must be considered for this intent
    #[serde(default)]
    pub must_have_variant: bool,
    /// URL pattern for variants
    #[serde(default)]
    pub variant_pattern: Option<String>,
}

/// Strategy configuration from KB
#[derive(Debug, Clone, Deserialize)]
pub struct StrategyConfig {
    /// Engine type: "http", "playwright", "rss"
    pub engine: String,
    /// Extraction: "auto", "json_ld", "selector:...", "rss"
    pub extraction: String,
    /// Reason for this strategy
    pub reason: String,
}

impl StrategyConfig {
    /// Convert to Strategy struct
    pub fn to_strategy(&self) -> Strategy {
        let engine = match self.engine.to_lowercase().as_str() {
            "playwright" | "js" => Engine::Playwright,
            "rss" | "atom" | "feed" => Engine::Rss,
            _ => Engine::Http,
        };

        let extraction = if self.extraction.starts_with("selector:") {
            let selector = self.extraction.strip_prefix("selector:").unwrap_or("");
            Extraction::Selector {
                selector: selector.to_string(),
            }
        } else {
            match self.extraction.to_lowercase().as_str() {
                "json_ld" => Extraction::JsonLd { types: None },
                "rss" => Extraction::Rss,
                "full" => Extraction::Full,
                _ => Extraction::Auto,
            }
        };

        Strategy {
            engine,
            extraction,
            reason: self.reason.clone(),
            confidence: 0.8,
            notes: None,
        }
    }
}

/// Known API endpoint for a platform
#[derive(Debug, Clone, Deserialize)]
pub struct ApiEndpoint {
    /// Path pattern (may include {handle}, {id} placeholders)
    pub path: String,
    /// Response type: "json", "xml", "html"
    #[serde(rename = "type")]
    pub response_type: String,
    /// What data this endpoint provides
    pub provides: Vec<String>,
}

/// Variant detection configuration
#[derive(Debug, Clone, Deserialize)]
pub struct VariantConfig {
    /// URL parameter name for variant
    #[serde(default)]
    pub url_param: Option<String>,
    /// CSS selector to find variant elements
    #[serde(default)]
    pub selector: Option<String>,
    /// JSON path to find variants
    #[serde(default)]
    pub json_path: Option<String>,
}

/// Anti-pattern (known failure mode)
#[derive(Debug, Clone, Deserialize)]
pub struct AntiPattern {
    /// Pattern to detect
    pub pattern: String,
    /// Result type: "auth_required", "geo_blocked", "rate_limited"
    pub result: String,
    /// Human-readable message
    pub message: String,
}

/// Evidence configuration
#[derive(Debug, Clone, Deserialize)]
pub struct EvidenceConfig {
    /// CSS selectors to look for
    #[serde(default)]
    pub selectors: Vec<String>,
    /// JSON-LD types to look for
    #[serde(default)]
    pub json_ld_types: Vec<String>,
}

/// A platform definition from the knowledge base
#[derive(Debug, Clone, Deserialize)]
pub struct PlatformDef {
    /// Platform identifier
    pub id: String,
    /// Display name
    #[serde(default)]
    pub name: Option<String>,
    /// Alternative names/aliases
    #[serde(default)]
    pub aliases: Vec<String>,
    /// KB version
    #[serde(default)]
    pub version: Option<String>,

    /// Detection configuration
    pub detection: DetectionConfig,

    /// Intent-specific strategies (keyed by intent name)
    #[serde(default)]
    pub intents: HashMap<String, IntentStrategies>,

    /// Known API endpoints
    #[serde(default)]
    pub endpoints: Vec<ApiEndpoint>,

    /// Variant detection config
    #[serde(default)]
    pub variants: Option<VariantConfig>,

    /// Anti-patterns (known failure modes)
    #[serde(default)]
    pub anti_patterns: Vec<AntiPattern>,

    /// Evidence to capture
    #[serde(default)]
    pub evidence: Option<EvidenceConfig>,
}

impl PlatformDef {
    /// Get display name
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// The platform knowledge base
#[derive(Debug, Clone, Default)]
pub struct PlatformKB {
    /// Platform definitions keyed by ID
    pub platforms: HashMap<String, PlatformDef>,
}

impl PlatformKB {
    /// Load KB from TOML file
    pub fn load_from_file(path: &PathBuf) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read KB file: {}", e))?;

        Self::parse_toml(&content)
    }

    /// Parse KB from TOML string
    pub fn parse_toml(content: &str) -> Result<Self, String> {
        // Parse as generic TOML value first
        let value: toml::Value = toml::from_str(content)
            .map_err(|e| format!("Failed to parse TOML: {}", e))?;

        let mut platforms = HashMap::new();

        if let toml::Value::Table(table) = value {
            for (key, val) in table {
                // Try to parse each top-level key as a platform
                if let Ok(mut platform) = val.clone().try_into::<PlatformDef>() {
                    // Use the key as the ID if not specified
                    if platform.id.is_empty() {
                        platform.id = key.clone();
                    }
                    platforms.insert(key, platform);
                }
            }
        }

        Ok(Self { platforms })
    }

    /// Load KB from default location or embedded defaults
    pub fn load_default() -> Self {
        // Try user config first
        if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "kto") {
            let user_kb = proj_dirs.config_dir().join("platforms.toml");
            if user_kb.exists() {
                if let Ok(kb) = Self::load_from_file(&user_kb.to_path_buf()) {
                    return kb;
                }
            }
        }

        // Fall back to embedded defaults
        Self::embedded_defaults()
    }

    /// Get embedded default KB
    pub fn embedded_defaults() -> Self {
        let defaults = include_str!("../assets/platforms.toml");
        Self::parse_toml(defaults).unwrap_or_default()
    }

    /// Ensure user KB file exists (copy defaults if not)
    pub fn ensure_user_kb() -> Result<PathBuf, String> {
        let proj_dirs = directories::ProjectDirs::from("", "", "kto")
            .ok_or_else(|| "Could not find config directory".to_string())?;
        let config_dir = proj_dirs.config_dir();
        let kb_path = config_dir.join("platforms.toml");

        if !kb_path.exists() {
            fs::create_dir_all(config_dir)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;

            let defaults = include_str!("../assets/platforms.toml");
            fs::write(&kb_path, defaults)
                .map_err(|e| format!("Failed to write KB file: {}", e))?;
        }

        Ok(kb_path)
    }

    /// Get platform by ID
    pub fn get(&self, id: &str) -> Option<&PlatformDef> {
        self.platforms.get(id)
    }

    /// Get all platform IDs
    pub fn platform_ids(&self) -> Vec<&String> {
        self.platforms.keys().collect()
    }
}

/// Detect platform from page facts using KB rules
pub fn detect_platform(facts: &PageFacts, kb: &PlatformKB) -> Vec<PlatformMatch> {
    let mut matches: Vec<PlatformMatch> = Vec::new();

    for (id, platform) in &kb.platforms {
        let (score, evidence, matched_rules) = score_platform(facts, platform);

        if score >= platform.detection.weight_threshold {
            matches.push(PlatformMatch {
                platform_id: id.clone(),
                platform_name: platform.display_name().to_string(),
                score,
                evidence,
                matched_rules,
            });
        }
    }

    // Sort by score descending
    matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    matches
}

/// Score a platform based on page facts
fn score_platform(facts: &PageFacts, platform: &PlatformDef) -> (f32, Vec<String>, Vec<String>) {
    let mut total_score = 0.0;
    let mut evidence = Vec::new();
    let mut matched_rules = Vec::new();

    let detection = &platform.detection;

    // Check exact domain matches (high confidence)
    if let Some(host) = facts.host() {
        for domain in &detection.domains {
            if host.ends_with(domain) || host == *domain {
                total_score += 0.9;
                evidence.push(format!("Domain match: {}", domain));
                matched_rules.push(format!("domain:{}", domain));
            }
        }
    }

    // Check HTML signatures
    for sig in &detection.html_signatures {
        if facts.html_contains(sig) {
            total_score += 0.3;
            evidence.push(format!("HTML contains: {}", sig));
            matched_rules.push(format!("html:{}", sig));
        }
    }

    // Check script hosts
    for host in &detection.script_hosts {
        if facts.has_script_from(host) {
            total_score += 0.4;
            evidence.push(format!("Script from: {}", host));
            matched_rules.push(format!("script:{}", host));
        }
    }

    // Check meta generator
    if let Some(gen) = &facts.meta_generator {
        for pattern in &detection.meta_generator {
            if gen.to_lowercase().contains(&pattern.to_lowercase()) {
                total_score += 0.6;
                evidence.push(format!("Meta generator: {}", gen));
                matched_rules.push(format!("generator:{}", pattern));
            }
        }
    }

    // Check individual signals with weights
    for signal in &detection.signals {
        let matched = match signal.signal_type.as_str() {
            "html_contains" => facts.html_contains(&signal.pattern),
            "script_src" => facts.has_script_from(&signal.pattern),
            "meta_generator" => {
                facts.meta_generator
                    .as_ref()
                    .map(|g| g.to_lowercase().contains(&signal.pattern.to_lowercase()))
                    .unwrap_or(false)
            }
            "domain" => {
                facts.host()
                    .map(|h| h.ends_with(&signal.pattern) || h == signal.pattern)
                    .unwrap_or(false)
            }
            "json_ld_type" => facts.has_json_ld_type(&signal.pattern),
            "stylesheet" => facts.has_stylesheet_from(&signal.pattern),
            _ => false,
        };

        if matched {
            total_score += signal.weight;
            evidence.push(format!("{}: {}", signal.signal_type, signal.pattern));
            matched_rules.push(format!("{}:{}", signal.signal_type, signal.pattern));
        }
    }

    // Normalize score to 0-1 range (cap at 1.0)
    let normalized_score = total_score.min(1.0);

    (normalized_score, evidence, matched_rules)
}

/// Get strategies for a platform + intent combination
pub fn get_strategies(platform_id: &str, intent: Intent, kb: &PlatformKB) -> Vec<Strategy> {
    let intent_key = match intent {
        Intent::Release => "release",
        Intent::Price => "price",
        Intent::Stock => "stock",
        Intent::Jobs => "jobs",
        Intent::News => "news",
        Intent::Generic => "generic",
    };

    if let Some(platform) = kb.get(platform_id) {
        if let Some(intent_config) = platform.intents.get(intent_key) {
            return intent_config
                .strategies
                .iter()
                .map(|s| s.to_strategy())
                .collect();
        }
    }

    // Return default strategies if not found in KB
    default_strategies(intent)
}

/// Get default strategies for an intent (when platform is unknown)
pub fn default_strategies(intent: Intent) -> Vec<Strategy> {
    match intent {
        Intent::Release => vec![
            Strategy {
                engine: Engine::Rss,
                extraction: Extraction::Rss,
                reason: "RSS feeds are ideal for release tracking".to_string(),
                confidence: 0.7,
                notes: Some("Try to discover RSS feed first".to_string()),
            },
            Strategy {
                engine: Engine::Http,
                extraction: Extraction::Auto,
                reason: "Fallback to HTML if no RSS".to_string(),
                confidence: 0.5,
                notes: None,
            },
        ],
        Intent::Price => vec![
            Strategy {
                engine: Engine::Http,
                extraction: Extraction::JsonLd { types: Some(vec!["Product".to_string(), "Offer".to_string()]) },
                reason: "JSON-LD provides stable price data".to_string(),
                confidence: 0.8,
                notes: None,
            },
            Strategy {
                engine: Engine::Playwright,
                extraction: Extraction::Auto,
                reason: "Fallback if no JSON-LD".to_string(),
                confidence: 0.6,
                notes: Some("Some sites render prices with JS".to_string()),
            },
        ],
        Intent::Stock => vec![
            Strategy {
                engine: Engine::Playwright,
                extraction: Extraction::Auto,
                reason: "Stock status often requires JS for button states".to_string(),
                confidence: 0.8,
                notes: None,
            },
            Strategy {
                engine: Engine::Http,
                extraction: Extraction::JsonLd { types: Some(vec!["Product".to_string(), "Offer".to_string()]) },
                reason: "Fallback to JSON-LD availability".to_string(),
                confidence: 0.5,
                notes: Some("May not reflect real-time stock".to_string()),
            },
        ],
        Intent::Jobs => vec![
            Strategy {
                engine: Engine::Http,
                extraction: Extraction::Auto,
                reason: "Job pages are usually static HTML".to_string(),
                confidence: 0.7,
                notes: None,
            },
            Strategy {
                engine: Engine::Playwright,
                extraction: Extraction::Auto,
                reason: "Some job boards use JS".to_string(),
                confidence: 0.5,
                notes: None,
            },
        ],
        Intent::News => vec![
            Strategy {
                engine: Engine::Rss,
                extraction: Extraction::Rss,
                reason: "News sites typically have RSS".to_string(),
                confidence: 0.8,
                notes: None,
            },
            Strategy {
                engine: Engine::Http,
                extraction: Extraction::Auto,
                reason: "Fallback to HTML scraping".to_string(),
                confidence: 0.5,
                notes: None,
            },
        ],
        Intent::Generic => vec![
            Strategy {
                engine: Engine::Http,
                extraction: Extraction::Auto,
                reason: "Default strategy".to_string(),
                confidence: 0.5,
                notes: None,
            },
        ],
    }
}

/// Check for anti-patterns in page facts
pub fn check_anti_patterns(facts: &PageFacts, platform: &PlatformDef) -> Option<AntiPattern> {
    for anti in &platform.anti_patterns {
        if facts.html_contains(&anti.pattern) {
            return Some(anti.clone());
        }
    }
    None
}

/// Get variant configuration for a platform
pub fn get_variant_config(platform_id: &str, kb: &PlatformKB) -> Option<VariantConfig> {
    kb.get(platform_id).and_then(|p| p.variants.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_kb() -> PlatformKB {
        let toml = r#"
[shopify]
id = "shopify"
name = "Shopify Store"

[shopify.detection]
domains = ["myshopify.com"]
html_signatures = ["cdn.shopify.com", "Shopify.theme"]
script_hosts = ["cdn.shopify.com"]
weight_threshold = 0.5

[[shopify.detection.signals]]
type = "html_contains"
pattern = "Shopify"
weight = 0.3

[shopify.intents.stock]
strategies = [
    { engine = "playwright", extraction = "auto", reason = "Button states need JS" }
]
must_have_variant = true
variant_pattern = "?variant={id}"

[shopify.intents.price]
strategies = [
    { engine = "http", extraction = "json_ld", reason = "JSON-LD has stable prices" }
]

[shopify.variants]
url_param = "variant"
selector = "[data-variant-id]"
"#;

        PlatformKB::parse_toml(toml).unwrap()
    }

    #[test]
    fn test_kb_parsing() {
        let kb = sample_kb();
        assert!(kb.platforms.contains_key("shopify"));

        let shopify = kb.get("shopify").unwrap();
        assert_eq!(shopify.display_name(), "Shopify Store");
        assert!(!shopify.detection.domains.is_empty());
    }

    #[test]
    fn test_platform_detection() {
        let kb = sample_kb();

        let html = r#"
            <html>
            <head>
                <script src="https://cdn.shopify.com/s/files/1/shop.js"></script>
            </head>
            <body>Shopify.theme</body>
            </html>
        "#;

        let facts = PageFacts::new("https://test.myshopify.com", "https://test.myshopify.com", html);
        let matches = detect_platform(&facts, &kb);

        assert!(!matches.is_empty());
        assert_eq!(matches[0].platform_id, "shopify");
        assert!(matches[0].score >= 0.5);
    }

    #[test]
    fn test_get_strategies() {
        let kb = sample_kb();

        let strategies = get_strategies("shopify", Intent::Stock, &kb);
        assert!(!strategies.is_empty());
        assert!(matches!(strategies[0].engine, Engine::Playwright));

        let price_strategies = get_strategies("shopify", Intent::Price, &kb);
        assert!(!price_strategies.is_empty());
        assert!(matches!(price_strategies[0].engine, Engine::Http));
    }

    #[test]
    fn test_default_strategies() {
        let strategies = default_strategies(Intent::News);
        assert!(!strategies.is_empty());
        assert!(matches!(strategies[0].engine, Engine::Rss));
    }
}
