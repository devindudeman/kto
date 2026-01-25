use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{KtoError, Result};
use crate::interests::{GlobalMemory, InterestProfile};
use crate::watch::AgentMemory;

/// Response from the agent after analyzing a change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// Whether to notify the user
    pub notify: bool,
    /// Short title for notification header (e.g., "Price Drop", "3 New Articles")
    #[serde(default)]
    pub title: Option<String>,
    /// Key changes as bullet points (2-4 items)
    #[serde(default)]
    pub bullets: Option<Vec<String>>,
    /// One-line summary for notification (backwards compat, used if title/bullets missing)
    #[serde(default)]
    pub summary: Option<String>,
    /// Longer analysis if useful
    #[serde(default)]
    pub analysis: Option<String>,
    /// Updated memory state
    #[serde(default)]
    pub memory_update: Option<AgentMemory>,
    /// Optional global observation about user preferences (for cross-watch learning)
    #[serde(default)]
    pub global_observation: Option<String>,
}

impl AgentResponse {
    /// Get formatted notification text combining title and bullets
    pub fn formatted_notification(&self) -> String {
        // If we have title + bullets, use the new format
        if let (Some(title), Some(bullets)) = (&self.title, &self.bullets) {
            if !bullets.is_empty() {
                let bullet_text = bullets
                    .iter()
                    .map(|b| format!("• {}", b))
                    .collect::<Vec<_>>()
                    .join("\n");
                return format!("{}\n\n{}", title, bullet_text);
            }
            return title.clone();
        }

        // Fall back to summary
        self.summary.clone().unwrap_or_else(|| "Content changed".to_string())
    }
}

/// Context provided to the agent for change analysis
pub struct AgentContext<'a> {
    pub old_content: &'a str,
    pub new_content: &'a str,
    pub diff: &'a str,
    pub memory: &'a AgentMemory,
    pub custom_instructions: Option<&'a str>,
    /// User's interest profile (if use_profile is enabled for this watch)
    pub profile: Option<&'a InterestProfile>,
    /// Global memory (cross-watch learning)
    pub global_memory: Option<&'a GlobalMemory>,
}

const DEFAULT_AGENT_PROMPT: &str = r#"You are monitoring a web page for changes.

{{profile_section}}

{{global_memory_section}}

## Current change
Old: {{old_content}}
New: {{new_content}}
Diff: {{diff}}

## Your memory (observations from past changes for THIS watch)
{{memory}}

## Instructions
{{custom_instructions}}

{{precedence_rules}}

Analyze this change. Focus on WHAT changed, not the raw diff syntax.
Respond ONLY with JSON, no other text:
{
  "notify": true/false,
  "title": "Short punchy title (e.g., 'Price Drop', '3 New Articles', 'Version 2.0 Released')",
  "bullets": ["Key change 1", "Key change 2", "Key change 3"],
  "summary": "One-line fallback summary",
  "analysis": "Longer analysis if useful (optional)",
  "memory_update": { "counters": {...}, "last_values": {...}, "notes": [...] },
  "global_observation": "Optional learning about user preferences (or null)"
}

Guidelines:
- title: 2-5 words, describes the change type (not the page)
- bullets: 2-4 key points, human-readable, no diff syntax like [-old][+new]
- For news/lists: mention new items by name
- For products: show price/stock changes clearly (e.g., "$99 → $79")
- For releases: list key additions/fixes
- If user profile is present, use it to judge relevance (but watch instructions take precedence)
- global_observation: If you notice a pattern about user preferences, note it here

CRITICAL CONSISTENCY RULES:
- If your analysis says "this is not the target change" or "not what user wants", then notify MUST be false
- If analysis says "still sold out", "still unavailable", or status unchanged, then notify MUST be false
- title MUST match the actual change (e.g., if still sold out, never use "Back In Stock")
- Page recovery from errors (error page → working page) is NOT a stock change - only notify if stock status ACTUALLY changed
- When in doubt about stock/availability, check if the purchase button text changed (e.g., "Add to Cart" appeared)

Keep memory under 16KB. Only store information useful for future change analysis."#;

const PRECEDENCE_RULES: &str = r#"=== PRECEDENCE RULES ===
1. Watch-specific instructions ALWAYS take priority
2. Profile interests BROADEN what's relevant, never narrow
3. If watch says "only X", focus on X regardless of profile
4. If watch is general ("alert me on interesting changes"), use profile to filter noise"#;

/// Truncate content to avoid command-line argument limits
fn truncate_content(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        content.to_string()
    } else {
        format!("{}...[truncated, {} chars total]", &content[..max_chars], content.len())
    }
}

/// Build the prompt for the agent
fn build_prompt(ctx: &AgentContext) -> String {
    let memory_json = serde_json::to_string_pretty(&ctx.memory).unwrap_or_default();
    let instructions = ctx.custom_instructions.unwrap_or("Analyze this change and determine if it's worth notifying the user about.");

    // Truncate content to avoid "argument list too long" errors
    // Keep total prompt under ~100KB to be safe with command-line limits
    let old_content = truncate_content(ctx.old_content, 30000);
    let new_content = truncate_content(ctx.new_content, 30000);
    let diff = truncate_content(ctx.diff, 20000);

    // Build profile section if present
    let profile_section = ctx.profile
        .filter(|p| !p.is_empty())
        .map(|p| p.to_prompt_section())
        .unwrap_or_default();

    // Build global memory section if present
    let global_memory_section = ctx.global_memory
        .filter(|m| !m.is_empty())
        .map(|m| m.to_prompt_section())
        .unwrap_or_default();

    // Only include precedence rules if profile is being used
    let precedence_rules = if ctx.profile.filter(|p| !p.is_empty()).is_some() {
        PRECEDENCE_RULES
    } else {
        ""
    };

    DEFAULT_AGENT_PROMPT
        .replace("{{old_content}}", &old_content)
        .replace("{{new_content}}", &new_content)
        .replace("{{diff}}", &diff)
        .replace("{{memory}}", &memory_json)
        .replace("{{custom_instructions}}", instructions)
        .replace("{{profile_section}}", &profile_section)
        .replace("{{global_memory_section}}", &global_memory_section)
        .replace("{{precedence_rules}}", precedence_rules)
}

/// Call Claude CLI to analyze a change
pub fn analyze_change(ctx: &AgentContext) -> Result<AgentResponse> {
    // Ensure workspace directory exists
    std::fs::create_dir_all("/tmp/kto-workspace")?;

    let prompt = build_prompt(ctx);

    let system_prompt = "You are a web monitoring assistant. Respond only with valid JSON matching the schema provided. Do not include any text before or after the JSON.";

    let output = Command::new("claude")
        .current_dir("/tmp/kto-workspace")
        .args([
            "-p",
            "--output-format", "json",
            "--max-turns", "1",
            "--system-prompt", system_prompt,
            &prompt,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::ClaudeFailed(stderr.to_string()));
    }

    // Parse Claude's response
    let stdout = String::from_utf8_lossy(&output.stdout);
    let claude_response: serde_json::Value = serde_json::from_str(&stdout)?;

    // Extract the result field (Claude Code's JSON output wraps the actual response)
    let result_text = claude_response["result"]
        .as_str()
        .ok_or_else(|| KtoError::ClaudeFailed("No result in response".into()))?;

    // Strip markdown code fencing if present (Claude often wraps JSON in ```json ... ```)
    let json_text = strip_code_fencing(result_text);

    // Parse the agent's JSON response
    let agent_response: AgentResponse = serde_json::from_str(&json_text)
        .map_err(|e| KtoError::ClaudeFailed(format!("Failed to parse agent response: {}", e)))?;

    Ok(agent_response)
}

/// Strip markdown code fencing from a string (e.g., ```json ... ```)
/// Also handles cases where there's text before the code block
fn strip_code_fencing(s: &str) -> String {
    let trimmed = s.trim();

    // First, try to find a JSON code block in the response
    // This handles cases like "Here's the analysis:\n```json\n{...}\n```"
    if let Some(json_start) = trimmed.find("```json") {
        let after_fence = &trimmed[json_start + 7..]; // Skip "```json"
        if let Some(end_fence) = after_fence.find("```") {
            return after_fence[..end_fence].trim().to_string();
        }
        // No closing fence, take the rest
        return after_fence.trim().to_string();
    }

    // Try generic code block
    if let Some(code_start) = trimmed.find("```\n") {
        let after_fence = &trimmed[code_start + 4..]; // Skip "```\n"
        if let Some(end_fence) = after_fence.find("```") {
            return after_fence[..end_fence].trim().to_string();
        }
        return after_fence.trim().to_string();
    }

    // Check for ```json or ``` prefix (original behavior for clean responses)
    let without_prefix = if trimmed.starts_with("```json") {
        trimmed.strip_prefix("```json").unwrap_or(trimmed)
    } else if trimmed.starts_with("```") {
        trimmed.strip_prefix("```").unwrap_or(trimmed)
    } else {
        trimmed
    };

    // Check for ``` suffix
    let without_suffix = if without_prefix.trim().ends_with("```") {
        without_prefix.trim().strip_suffix("```").unwrap_or(without_prefix)
    } else {
        without_prefix
    };

    without_suffix.trim().to_string()
}

/// Response from setup wizard analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupSuggestion {
    /// Suggested name for the watch
    pub name: String,
    /// Suggested check interval in seconds
    pub interval_secs: u64,
    /// Whether to enable the AI agent
    pub agent_enabled: bool,
    /// Custom instructions for the agent if enabled
    #[serde(default)]
    pub agent_instructions: Option<String>,
    /// One-line summary of what was found on the page
    pub summary: String,
    /// CSS selector hint if a specific element should be watched
    #[serde(default)]
    pub selector_hint: Option<String>,
}

/// Enhanced setup suggestion with dual-fetch analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedSetupSuggestion {
    /// Suggested name for the watch
    #[serde(alias = "suggested_name")]
    pub name: String,
    /// Suggested check interval in seconds
    #[serde(alias = "suggested_interval")]
    pub interval_secs: u64,
    /// Whether to enable the AI agent (always true for intent-based watches)
    #[serde(default)]
    pub agent_enabled: bool,
    /// Custom instructions for the agent if enabled
    #[serde(default)]
    pub agent_instructions: Option<String>,
    /// CSS selector hint if a specific element should be watched
    #[serde(default)]
    pub selector_hint: Option<String>,

    // New fields for enhanced wizard

    /// Whether JavaScript rendering is required
    #[serde(default)]
    pub needs_js: bool,
    /// Reason JS is needed (if needs_js is true)
    #[serde(default)]
    pub js_reason: Option<String>,
    /// Current status relevant to user's intent (e.g., "19 inch - SOLD OUT")
    #[serde(default)]
    pub current_status: Option<String>,
    /// Detected product variants/options
    #[serde(default)]
    pub variants: Vec<Variant>,
    /// Which variant matches the user's intent
    #[serde(default)]
    pub intent_match: Option<IntentMatch>,
    /// Overall analysis confidence (0.0 - 1.0)
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    /// Reasons for uncertainty (missing data, conflicting signals, etc.)
    #[serde(default)]
    pub uncertainty_reasons: Vec<String>,
}

fn default_confidence() -> f32 {
    0.5
}

impl EnhancedSetupSuggestion {
    /// Convert to basic SetupSuggestion for backwards compatibility
    pub fn to_basic(&self) -> SetupSuggestion {
        SetupSuggestion {
            name: self.name.clone(),
            interval_secs: self.interval_secs,
            agent_enabled: self.agent_enabled,
            agent_instructions: self.agent_instructions.clone(),
            summary: self.current_status.clone().unwrap_or_else(|| "Page analyzed".to_string()),
            selector_hint: self.selector_hint.clone(),
        }
    }

    /// Create a fallback suggestion when AI analysis fails
    pub fn fallback(url: &str, intent: &str) -> Self {
        // Extract domain for name
        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| "Watch".to_string());

        Self {
            name: domain,
            interval_secs: 900,
            agent_enabled: !intent.is_empty(),
            agent_instructions: if intent.is_empty() {
                None
            } else {
                Some(format!("Monitor for: {}", intent))
            },
            selector_hint: None,
            needs_js: false,
            js_reason: None,
            current_status: None,
            variants: vec![],
            intent_match: None,
            confidence: 0.0,
            uncertainty_reasons: vec!["AI analysis failed - using defaults".to_string()],
        }
    }
}

/// A product variant or selectable option (size, color, pack, edition, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    /// Human-readable name (e.g., "19 inch", "Blue", "Pack of 3")
    pub name: String,
    /// Internal identifier if found (e.g., variant ID)
    #[serde(default)]
    pub identifier: Option<String>,
    /// Current status (e.g., "in stock", "sold out", "$99.00")
    #[serde(default)]
    pub status: Option<String>,
    /// URL parameter hint (e.g., "variant=46282113351909")
    #[serde(default)]
    pub url_hint: Option<String>,
    /// Evidence: exact snippet or selector where this was found
    #[serde(default)]
    pub evidence: Option<String>,
}

/// Match between user's intent and a detected variant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentMatch {
    /// Index into the variants array
    pub variant_index: usize,
    /// Confidence in this match (0.0 - 1.0)
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

const SETUP_WIZARD_PROMPT: &str = r#"You are helping set up a web page monitor.

## User's request
{{user_intent}}

## Page content (extracted text)
{{page_content}}

## Page title
{{page_title}}

Based on the page content and what the user wants to monitor, suggest optimal settings.

Respond ONLY with JSON, no other text:
{
  "name": "short descriptive name for this watch",
  "interval_secs": 900,
  "agent_enabled": true,
  "agent_instructions": "specific instructions for analyzing changes, or null",
  "summary": "one-line summary of what you found (e.g., 'Found product X priced at $Y')",
  "selector_hint": "CSS selector if a specific element should be watched, or null"
}

Guidelines:
- name: Keep it short (2-4 words), describe what's being watched
- interval_secs: 60-300 for fast-changing (stock, news), 900 for products, 3600+ for slow sites
- agent_enabled: true if user wants semantic analysis (price drops, specific events), false for any-change alerts
- agent_instructions: Be specific about what changes matter (e.g., "Alert when price drops below $X")
- selector_hint: Only if the page has a clear element to focus on"#;

/// Analyze a page with Claude to suggest smart watch configuration
pub fn analyze_for_setup(user_intent: &str, page_content: &str, page_title: &str) -> Result<SetupSuggestion> {
    // Ensure workspace directory exists
    std::fs::create_dir_all("/tmp/kto-workspace")?;

    // Truncate page content to avoid token limits
    let content_preview: String = page_content.chars().take(3000).collect();

    let prompt = SETUP_WIZARD_PROMPT
        .replace("{{user_intent}}", user_intent)
        .replace("{{page_content}}", &content_preview)
        .replace("{{page_title}}", page_title);

    let system_prompt = "You are a web monitoring setup assistant. Respond only with valid JSON matching the schema provided. Be concise and practical.";

    let output = Command::new("claude")
        .current_dir("/tmp/kto-workspace")
        .args([
            "-p",
            "--output-format", "json",
            "--max-turns", "1",
            "--system-prompt", system_prompt,
            &prompt,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::ClaudeFailed(stderr.to_string()));
    }

    // Parse Claude's response
    let stdout = String::from_utf8_lossy(&output.stdout);
    let claude_response: serde_json::Value = serde_json::from_str(&stdout)?;

    // Extract the result field
    let result_text = claude_response["result"]
        .as_str()
        .ok_or_else(|| KtoError::ClaudeFailed("No result in response".into()))?;

    // Strip markdown code fencing if present
    let json_text = strip_code_fencing(result_text);

    // Parse the suggestion
    let suggestion: SetupSuggestion = serde_json::from_str(&json_text)
        .map_err(|e| KtoError::ClaudeFailed(format!("Failed to parse setup suggestion: {}", e)))?;

    Ok(suggestion)
}

const ENHANCED_SETUP_PROMPT: &str = r#"You are analyzing a web page to help set up a change monitor.

=== HTTP-fetched content (SOURCE: HTTP) ===
{{http_content}}
=== END HTTP ===

=== JavaScript-rendered content (SOURCE: JS) ===
{{js_content}}
=== END JS ===

User's intent: "{{user_intent}}"

Analyze both content sources and return **strict JSON** with these fields:

{
  "needs_js": boolean,
  "js_reason": string | null,
  "current_status": string,
  "variants": [
    {
      "name": string,
      "identifier": string | null,
      "status": string | null,
      "url_hint": string | null,
      "evidence": string
    }
  ],
  "intent_match": {
    "variant_index": number,
    "confidence": number
  } | null,
  "confidence": number,
  "uncertainty_reasons": [],
  "suggested_name": string,
  "suggested_interval": number,
  "agent_enabled": boolean,
  "agent_instructions": string,
  "selector_hint": string | null
}

Field guidelines:
- needs_js: true if important content (prices, stock, variants) only appears in JS version
- js_reason: explain WHY JS is needed (e.g., "stock status only in JS version")
- current_status: one-line status relevant to user's intent (e.g., "19 inch - SOLD OUT")
- variants: any selectable options (sizes, colors, bundles, editions, etc.)
  - name: human-readable (e.g., "19 inch", "Blue", "Pack of 3")
  - identifier: internal ID if found in page source
  - status: current availability/price (e.g., "in stock", "sold out", "$99.00")
  - url_hint: URL parameter like "variant=123" or "size=xl" if discoverable
  - evidence: exact snippet or CSS selector where found (for debugging)
- intent_match: which variant matches user's intent (if any)
  - variant_index: 0-based index into variants array
  - confidence: 0.0-1.0 how confident the match is
- confidence: overall 0.0-1.0 confidence in the entire analysis
- uncertainty_reasons: list any issues (missing data, auth walls, A/B tests, incomplete load)
- suggested_name: short name (2-4 words) for the watch
- suggested_interval: check interval in seconds (60-300 for fast-changing, 900 for products)
- agent_enabled: true if user wants semantic analysis (usually true for intent-based)
- agent_instructions: specific instructions for analyzing changes (be precise about what matters)
- selector_hint: CSS selector if a specific element should be watched

IMPORTANT:
- Prefer structured data (JSON-LD in <script> tags, embedded JS configs) over DOM text
- Look for schema.org Product/Offer data for prices and availability
- If HTTP and JS content differ significantly, note in uncertainty_reasons
- For variants, check data attributes, URL patterns, and select/option elements
- Define "variant" broadly: any selectable option that affects what the user receives
- If you can't find variants, return empty array - don't make them up
- Include evidence snippets for debugging (but keep them short)
- For url_hint: look for URL parameters in links, option values, or data attributes that identify specific variants"#;

/// Analyze a page with both HTTP and JS content to suggest smart watch configuration
pub fn analyze_for_setup_v2(
    user_intent: &str,
    http_content: Option<&str>,
    js_content: Option<&str>,
) -> Result<EnhancedSetupSuggestion> {
    // Ensure workspace directory exists
    std::fs::create_dir_all("/tmp/kto-workspace")?;

    // Prepare content strings (truncate to avoid token limits)
    let http_preview: String = http_content
        .map(|c| c.chars().take(6000).collect())
        .unwrap_or_else(|| "(HTTP fetch not available)".to_string());

    let js_preview: String = js_content
        .map(|c| c.chars().take(6000).collect())
        .unwrap_or_else(|| "(JS fetch not available or same as HTTP)".to_string());

    let prompt = ENHANCED_SETUP_PROMPT
        .replace("{{http_content}}", &http_preview)
        .replace("{{js_content}}", &js_preview)
        .replace("{{user_intent}}", user_intent);

    let system_prompt = "You are a web monitoring setup assistant. Respond only with valid JSON matching the schema provided exactly. Be concise and practical. Do not include any text before or after the JSON.";

    let output = Command::new("claude")
        .current_dir("/tmp/kto-workspace")
        .args([
            "-p",
            "--output-format", "json",
            "--max-turns", "1",
            "--system-prompt", system_prompt,
            &prompt,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::ClaudeFailed(stderr.to_string()));
    }

    // Parse Claude's response
    let stdout = String::from_utf8_lossy(&output.stdout);
    let claude_response: serde_json::Value = serde_json::from_str(&stdout)?;

    // Extract the result field
    let result_text = claude_response["result"]
        .as_str()
        .ok_or_else(|| KtoError::ClaudeFailed("No result in response".into()))?;

    // Strip markdown code fencing if present
    let json_text = strip_code_fencing(result_text);

    // Parse the enhanced suggestion
    let suggestion: EnhancedSetupSuggestion = serde_json::from_str(&json_text)
        .map_err(|e| KtoError::ClaudeFailed(format!("Failed to parse enhanced setup suggestion: {}", e)))?;

    Ok(suggestion)
}

/// Check if Claude CLI is available
pub fn check_claude_cli() -> Result<()> {
    let output = Command::new("claude")
        .arg("--version")
        .output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(KtoError::ClaudeNotInstalled(
            "Install Claude CLI: curl -fsSL https://claude.ai/install.sh | bash".into()
        ))
    }
}

/// Get Claude CLI version
pub fn claude_version() -> Option<String> {
    Command::new("claude")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

// ============================================================================
// Deep Research Mode
// ============================================================================

use crate::fetch::DiscoveredFeed;
use crate::watch::Engine;

/// URL modifications to apply based on research findings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlModifications {
    /// Variant parameter to add (e.g., "46282113351909")
    #[serde(default)]
    pub variant_param: Option<String>,
    /// Why this modification is recommended
    pub reason: String,
}

/// Result of deep research analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepResearchResult {
    /// High-level summary of findings
    pub summary: String,
    /// Engine recommendation
    pub engine: EngineRecommendation,
    /// Extraction strategy recommendation
    pub extraction: ExtractionRecommendation,
    /// Discovered feeds (may include new ones found via web search)
    #[serde(default)]
    pub discovered_feeds: Vec<FeedInfo>,
    /// Recommended CSS selectors
    #[serde(default)]
    pub selectors: Vec<SelectorRecommendation>,
    /// Suggested agent instructions (if AI should be enabled)
    #[serde(default)]
    pub agent_instructions: Option<String>,
    /// Suggested check interval in seconds
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    /// Overall confidence in the analysis (0.0 - 1.0)
    #[serde(default = "default_research_confidence")]
    pub confidence: f32,
    /// Key insights from research
    #[serde(default)]
    pub insights: Vec<String>,
    /// Findings from web search (if available)
    #[serde(default)]
    pub web_research: Option<WebResearchFindings>,
    /// URL modifications to apply (variant params, etc.)
    #[serde(default)]
    pub url_modifications: Option<UrlModifications>,
}

fn default_interval() -> u64 {
    900
}

fn default_research_confidence() -> f32 {
    0.5
}

/// Engine recommendation with reasoning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineRecommendation {
    /// Recommended engine type
    #[serde(rename = "type")]
    pub engine_type: String,
    /// Why this engine is recommended
    pub reason: String,
}

impl EngineRecommendation {
    /// Convert engine type string to Engine enum
    pub fn to_engine(&self) -> Engine {
        match self.engine_type.to_lowercase().as_str() {
            "rss" | "atom" | "feed" => Engine::Rss,
            "playwright" | "js" | "javascript" => Engine::Playwright,
            _ => Engine::Http,
        }
    }
}

/// Extraction strategy recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionRecommendation {
    /// Strategy type: "auto", "selector", "rss", "json_ld"
    pub strategy: String,
    /// CSS selector if strategy is "selector"
    #[serde(default)]
    pub selector: Option<String>,
    /// Why this strategy is recommended
    pub reason: String,
}

/// Feed info for research results (can include ones from web search)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedInfo {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    pub feed_type: String,
    pub discovery_method: String,
    /// Whether this feed matches the user's intent
    #[serde(default)]
    pub matches_intent: bool,
}

impl From<DiscoveredFeed> for FeedInfo {
    fn from(feed: DiscoveredFeed) -> Self {
        FeedInfo {
            url: feed.url,
            title: feed.title,
            feed_type: feed.feed_type,
            discovery_method: feed.discovery_method,
            matches_intent: false,
        }
    }
}

/// Recommended CSS selector with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectorRecommendation {
    /// The CSS selector
    pub selector: String,
    /// What this selector captures
    pub description: String,
    /// Stability score (0.0 - 1.0) - how likely to break with page changes
    #[serde(default)]
    pub stability_score: f32,
}

/// Findings from web search during research
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebResearchFindings {
    /// Search queries that were made
    #[serde(default)]
    pub queries_made: Vec<String>,
    /// Relevant findings from search results
    #[serde(default)]
    pub relevant_findings: Vec<String>,
    /// Discovered API endpoints
    #[serde(default)]
    pub api_endpoints: Vec<ApiEndpoint>,
    /// Tips from community/documentation
    #[serde(default)]
    pub community_tips: Vec<String>,
}

/// Discovered API endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoint {
    /// URL pattern (may include wildcards like /products/*.json)
    pub url_pattern: String,
    /// What this endpoint provides
    pub description: String,
    /// Whether authentication is required
    #[serde(default)]
    pub requires_auth: bool,
}

const DEEP_RESEARCH_PROMPT: &str = r#"You are analyzing a web page to determine the OPTIMAL monitoring strategy.
This is DEEP RESEARCH mode - be thorough in your analysis.

## Page Information
URL: {{url}}
User Intent: "{{user_intent}}"
Site Type: {{site_type}}

## Pre-fetched Content (from HTTP fetch)
{{http_content}}

## Pre-fetched Content (from JS rendering, if different)
{{js_content}}

## Discovered Feeds (found via HTML parsing)
{{discovered_feeds}}

## JSON-LD Structured Data
{{jsonld_data}}

## Analysis Steps

1. **Site Type Detection**: Identify the platform (Shopify, WordPress, etc.) from HTML signatures

2. **Engine Decision**: Does content require JavaScript rendering?
   - Compare HTTP vs JS content - is key data (buttons, prices, stock status) missing from HTTP?
   - For e-commerce stock monitoring, Playwright is usually required for button state changes
   - If the "add to cart" button state is what we're monitoring, use Playwright

3. **Variant Detection**: If user mentions a specific size/color/variant:
   - Look for variant IDs in the page (data attributes, URL params, JSON data)
   - Include the variant param in url_modifications so we monitor the correct variant

4. **Feed Analysis**: Only include feeds that match user intent
   - RSS feeds typically don't include stock/price info
   - Mark feeds as matches_intent: false if they won't help with the user's goal

5. **Extraction Strategy**: What's the most stable way to extract data?
   - For stock monitoring: watch for button text changes ("Add to Cart" vs "Sold Out")
   - For price monitoring: JSON-LD Product schema is reliable
   - For news/updates: RSS feeds are ideal

## SELF-CHECK BEFORE OUTPUT
Before finalizing, verify:
1. If monitoring stock/availability on e-commerce, did you recommend Playwright?
2. If user mentioned a specific variant, did you include url_modifications with variant_param?
3. Do agent_instructions match what's actually being monitored (HTML page, not JSON API)?

## Output Format (respond with JSON only, no other text)
{
  "summary": "Brief summary of findings",
  "engine": { "type": "http|playwright|rss", "reason": "why this engine is needed" },
  "extraction": { "strategy": "auto|selector|rss|json_ld", "selector": "css selector or null", "reason": "why" },
  "url_modifications": { "variant_param": "variant ID if user wants specific variant, or null", "reason": "why this modification is needed" },
  "discovered_feeds": [
    { "url": "...", "title": "...", "feed_type": "rss|atom|json", "discovery_method": "...", "matches_intent": true/false }
  ],
  "selectors": [
    { "selector": "...", "description": "...", "stability_score": 0.8 }
  ],
  "agent_instructions": "Instructions that describe what to look for in the actual fetched content",
  "interval_secs": 300,
  "confidence": 0.85,
  "insights": ["Key finding 1", "Key finding 2"],
  "web_research": null
}

IMPORTANT:
- For stock/availability monitoring on e-commerce sites (Shopify, etc.), recommend Playwright
- Button state changes ("Add to Cart" -> "Sold Out") require JavaScript rendering
- agent_instructions should describe monitoring the actual page content, not a different endpoint
- Only mark feeds as matches_intent: true if they actually contain the data user cares about"#;

const DEEP_RESEARCH_WITH_WEB_PROMPT: &str = r#"You are analyzing a web page to determine the OPTIMAL monitoring strategy.
Use WebSearch and WebFetch to discover best practices. Do NOT rely on prior knowledge.

## Pre-fetched Content
URL: {{url}}
User Intent: "{{user_intent}}"

HTTP Content:
{{http_content}}

JS Content:
{{js_content}}

Discovered Feeds:
{{discovered_feeds}}

JSON-LD Data:
{{jsonld_data}}

## REQUIRED PHASES (follow in order)

### Phase 1: Detect Site Type
Analyze HTML signatures (scripts, meta tags, CSS classes).
Output:
- site_type: e.g., "Shopify store", "WordPress blog", "Static site"
- evidence: specific selectors/signatures found
- confidence: 0.0-1.0

### Phase 2: Research Best Practices
WebSearch: "how to monitor {site_type} for {intent}"
WebFetch 2-4 top results.
Output:
- findings: list of {rule, source_url}
- Example: {"rule": "Shopify button states require JS rendering", "source": "..."}

### Phase 3: Derive Monitoring Rules
Convert findings into concrete if/then rules:
- Example: "IF monitoring Shopify stock THEN use Playwright engine"
- Example: "IF user wants specific variant THEN include variant param in URL"

### Phase 4: Apply Rules to Config
Build final configuration. EACH choice must cite a rule from Phase 3.

## SELF-CHECK BEFORE OUTPUT
Before finalizing, verify:
1. Does engine choice cite a discovered rule?
2. Does url_modifications cite a discovered rule?
3. Do agent_instructions describe monitoring the ACTUAL URL being watched?
   - If URL ends in .json -> instructions should mention JSON parsing
   - If URL is HTML page -> instructions should mention DOM/text changes
   - NEVER write instructions for a different URL than what's configured

If any check fails, revise the config.

## Output Format (JSON only)
{
  "phase1_detection": { "site_type": "...", "evidence": "...", "confidence": 0.9 },
  "phase2_findings": [{ "rule": "...", "source": "..." }],
  "phase3_derived_rules": ["IF ... THEN ..."],
  "summary": "...",
  "engine": { "type": "http|playwright|rss", "reason": "Based on rule: ..." },
  "extraction": { "strategy": "auto|selector|rss|json_ld", "selector": "css selector or null", "reason": "Based on rule: ..." },
  "url_modifications": { "variant_param": "variant ID if user wants specific variant", "reason": "Based on rule: ..." },
  "discovered_feeds": [
    { "url": "...", "title": "...", "feed_type": "rss|atom|json", "discovery_method": "...", "matches_intent": true/false }
  ],
  "selectors": [
    { "selector": "...", "description": "...", "stability_score": 0.8 }
  ],
  "agent_instructions": "Instructions that match the actual extraction strategy - if watching HTML, describe HTML changes; if watching JSON, describe JSON parsing",
  "interval_secs": 300,
  "confidence": 0.92,
  "insights": ["..."],
  "web_research": {
    "queries_made": ["how to monitor shopify stock", "..."],
    "relevant_findings": ["Shopify button states need JS rendering"],
    "api_endpoints": [{ "url_pattern": "/products/*.json", "description": "...", "requires_auth": false }],
    "community_tips": ["..."]
  },
  "rule_to_config_mapping": {
    "engine": "Rule: Shopify buttons need JS -> Playwright",
    "url_modifications": "Rule: Specific variant needs param -> ?variant=123"
  }
}

## CONSTRAINT
Final config MUST reference at least 2 rules from phase2_findings.
If a config choice cannot cite a rule, re-run research with refined query.

IMPORTANT:
- Only include feeds in discovered_feeds if they actually match the user's intent
- For stock/availability monitoring on e-commerce sites, recommend Playwright (button states often need JS)
- If user mentions a specific variant/size/color, find and include the variant parameter in url_modifications"#;

/// Perform deep research analysis on a URL
pub fn deep_research_analysis(
    url: &str,
    user_intent: &str,
    http_content: Option<&str>,
    js_content: Option<&str>,
    discovered_feeds: &[DiscoveredFeed],
    jsonld_data: Option<&str>,
    site_type: Option<&str>,
) -> Result<DeepResearchResult> {
    // Ensure workspace directory exists
    std::fs::create_dir_all("/tmp/kto-workspace")?;

    // Format discovered feeds
    let feeds_json = if discovered_feeds.is_empty() {
        "(none found)".to_string()
    } else {
        discovered_feeds.iter()
            .map(|f| format!("- {} ({}, via {})", f.url, f.feed_type, f.discovery_method))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Truncate content to avoid token limits
    let http_preview: String = http_content
        .map(|c| c.chars().take(8000).collect())
        .unwrap_or_else(|| "(not available)".to_string());

    let js_preview: String = js_content
        .map(|c| c.chars().take(8000).collect())
        .unwrap_or_else(|| "(not available or same as HTTP)".to_string());

    let jsonld_preview: String = jsonld_data
        .map(|c| c.chars().take(4000).collect())
        .unwrap_or_else(|| "(none found)".to_string());

    let site_type_str = site_type.unwrap_or("Unknown");

    // First, try with web search enabled
    let result = try_research_with_web_search(
        url, user_intent, &http_preview, &js_preview,
        &feeds_json, &jsonld_preview, site_type_str,
    );

    match result {
        Ok(r) => Ok(r),
        Err(_) => {
            // Fall back to analysis without web search
            try_research_without_web_search(
                url, user_intent, &http_preview, &js_preview,
                &feeds_json, &jsonld_preview, site_type_str,
            )
        }
    }
}

/// Try research with web search tools enabled
fn try_research_with_web_search(
    url: &str,
    user_intent: &str,
    http_content: &str,
    js_content: &str,
    discovered_feeds: &str,
    jsonld_data: &str,
    site_type: &str,
) -> Result<DeepResearchResult> {
    let prompt = DEEP_RESEARCH_WITH_WEB_PROMPT
        .replace("{{url}}", url)
        .replace("{{user_intent}}", user_intent)
        .replace("{{site_type}}", site_type)
        .replace("{{http_content}}", http_content)
        .replace("{{js_content}}", js_content)
        .replace("{{discovered_feeds}}", discovered_feeds)
        .replace("{{jsonld_data}}", jsonld_data);

    let system_prompt = "You are a web monitoring expert. Analyze the page thoroughly and use web search to find the best monitoring approach. Respond only with valid JSON matching the schema provided.";

    // Try with --allowedTools for web search (must come before -p)
    let output = Command::new("claude")
        .current_dir("/tmp/kto-workspace")
        .args([
            "--allowedTools", "WebSearch,WebFetch",
            "-p",
            "--output-format", "json",
            "--max-turns", "5",
            "--system-prompt", system_prompt,
            &prompt,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(KtoError::ClaudeFailed(format!("Web research failed. stderr: {}. stdout: {}", stderr, &stdout.chars().take(200).collect::<String>())));
    }

    // Check if stdout is empty (command succeeded but no output)
    if output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::ClaudeFailed(format!("Claude returned no output. stderr: {}", stderr)));
    }

    parse_research_response(&output.stdout)
}

/// Try research without web search (fallback)
fn try_research_without_web_search(
    url: &str,
    user_intent: &str,
    http_content: &str,
    js_content: &str,
    discovered_feeds: &str,
    jsonld_data: &str,
    site_type: &str,
) -> Result<DeepResearchResult> {
    let prompt = DEEP_RESEARCH_PROMPT
        .replace("{{url}}", url)
        .replace("{{user_intent}}", user_intent)
        .replace("{{site_type}}", site_type)
        .replace("{{http_content}}", http_content)
        .replace("{{js_content}}", js_content)
        .replace("{{discovered_feeds}}", discovered_feeds)
        .replace("{{jsonld_data}}", jsonld_data);

    let system_prompt = "You are a web monitoring expert. Analyze the page thoroughly to find the best monitoring approach. Respond only with valid JSON matching the schema provided.";

    let output = Command::new("claude")
        .current_dir("/tmp/kto-workspace")
        .args([
            "-p",
            "--output-format", "json",
            "--max-turns", "3",
            "--system-prompt", system_prompt,
            &prompt,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::ClaudeFailed(format!("Research analysis failed: {}", stderr)));
    }

    parse_research_response(&output.stdout)
}

/// Parse the research response from Claude
fn parse_research_response(stdout: &[u8]) -> Result<DeepResearchResult> {
    let stdout_str = String::from_utf8_lossy(stdout);

    // Debug: check if stdout is empty
    if stdout_str.trim().is_empty() {
        return Err(KtoError::ClaudeFailed("Claude returned empty response".into()));
    }

    let claude_response: serde_json::Value = serde_json::from_str(&stdout_str)
        .map_err(|e| KtoError::ClaudeFailed(format!("Failed to parse Claude output as JSON: {}. First 500 chars: {}", e, &stdout_str.chars().take(500).collect::<String>())))?;

    // Extract the result field
    let result_text = claude_response["result"]
        .as_str()
        .ok_or_else(|| KtoError::ClaudeFailed(format!("No result in response. Keys: {:?}", claude_response.as_object().map(|o| o.keys().collect::<Vec<_>>()))))?;

    // Strip markdown code fencing if present
    let json_text = strip_code_fencing(result_text);

    // Parse the research result
    let result: DeepResearchResult = serde_json::from_str(&json_text)
        .map_err(|e| KtoError::ClaudeFailed(format!("Failed to parse research response: {}. First 500 chars of result: {}", e, &json_text.chars().take(500).collect::<String>())))?;

    Ok(result)
}

impl DeepResearchResult {
    /// Create a fallback result when research fails
    pub fn fallback(_url: &str, intent: &str) -> Self {
        DeepResearchResult {
            summary: "Research could not complete - using defaults".to_string(),
            engine: EngineRecommendation {
                engine_type: "http".to_string(),
                reason: "Default engine".to_string(),
            },
            extraction: ExtractionRecommendation {
                strategy: "auto".to_string(),
                selector: None,
                reason: "Default extraction".to_string(),
            },
            discovered_feeds: vec![],
            selectors: vec![],
            agent_instructions: if intent.is_empty() {
                None
            } else {
                Some(format!("Monitor for: {}", intent))
            },
            interval_secs: 900,
            confidence: 0.0,
            insights: vec!["Research failed - using default settings".to_string()],
            web_research: None,
            url_modifications: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt() {
        let memory = AgentMemory::default();
        let ctx = AgentContext {
            old_content: "Price: $100",
            new_content: "Price: $80",
            diff: "[-$100][+$80]",
            memory: &memory,
            custom_instructions: Some("Watch for price changes"),
            profile: None,
            global_memory: None,
        };

        let prompt = build_prompt(&ctx);
        assert!(prompt.contains("Price: $100"));
        assert!(prompt.contains("Price: $80"));
        assert!(prompt.contains("Watch for price changes"));
    }

    #[test]
    fn test_build_prompt_with_profile() {
        use crate::interests::{Interest, InterestProfile, InterestScope, ProfileDescription};

        let memory = AgentMemory::default();
        let profile = InterestProfile {
            profile: ProfileDescription {
                description: "I'm a developer".to_string(),
            },
            interests: vec![Interest {
                name: "Rust".to_string(),
                keywords: vec!["rust".to_string(), "cargo".to_string()],
                weight: 0.8,
                scope: InterestScope::Narrow,
                sources: vec![],
            }],
        };
        let ctx = AgentContext {
            old_content: "Version 1.0",
            new_content: "Version 2.0",
            diff: "[-1.0][+2.0]",
            memory: &memory,
            custom_instructions: Some("Alert on version changes"),
            profile: Some(&profile),
            global_memory: None,
        };

        let prompt = build_prompt(&ctx);
        assert!(prompt.contains("I'm a developer"));
        assert!(prompt.contains("Rust"));
        assert!(prompt.contains("PRECEDENCE RULES"));
    }

    #[test]
    fn test_parse_agent_response() {
        // Test new format with title + bullets
        let json = r#"{
            "notify": true,
            "title": "Price Drop",
            "bullets": ["$100 → $80 (-20%)", "Still in stock"],
            "summary": "Price dropped by 20%",
            "analysis": "The price went from $100 to $80",
            "memory_update": {
                "counters": {"price_drops": 1},
                "last_values": {"price": "$80"},
                "notes": []
            }
        }"#;

        let response: AgentResponse = serde_json::from_str(json).unwrap();
        assert!(response.notify);
        assert_eq!(response.title, Some("Price Drop".to_string()));
        assert_eq!(response.bullets.as_ref().unwrap().len(), 2);

        // Test formatted output
        let formatted = response.formatted_notification();
        assert!(formatted.contains("Price Drop"));
        assert!(formatted.contains("• $100 → $80"));
    }

    #[test]
    fn test_parse_legacy_response() {
        // Test backwards compat with old format (summary only)
        let json = r#"{
            "notify": true,
            "summary": "Price dropped by 20%"
        }"#;

        let response: AgentResponse = serde_json::from_str(json).unwrap();
        assert!(response.notify);
        assert_eq!(response.formatted_notification(), "Price dropped by 20%");
    }

    #[test]
    fn test_strip_code_fencing() {
        // With ```json prefix
        let input = "```json\n{\"foo\": \"bar\"}\n```";
        assert_eq!(strip_code_fencing(input), "{\"foo\": \"bar\"}");

        // With ``` prefix only
        let input = "```\n{\"foo\": \"bar\"}\n```";
        assert_eq!(strip_code_fencing(input), "{\"foo\": \"bar\"}");

        // Already clean JSON
        let input = "{\"foo\": \"bar\"}";
        assert_eq!(strip_code_fencing(input), "{\"foo\": \"bar\"}");

        // With extra whitespace
        let input = "  ```json\n  {\"foo\": \"bar\"}  \n```  ";
        assert_eq!(strip_code_fencing(input), "{\"foo\": \"bar\"}");

        // With preamble text before code block
        let input = "Here's the analysis:\n\n```json\n{\"foo\": \"bar\"}\n```";
        assert_eq!(strip_code_fencing(input), "{\"foo\": \"bar\"}");

        // With preamble text and content after code block
        let input = "Based on my analysis:\n```json\n{\"foo\": \"bar\"}\n```\nHope this helps!";
        assert_eq!(strip_code_fencing(input), "{\"foo\": \"bar\"}");
    }
}
