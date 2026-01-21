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
fn strip_code_fencing(s: &str) -> String {
    let trimmed = s.trim();

    // Check for ```json or ``` prefix
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
    }
}
