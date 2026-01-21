use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Extraction strategy for getting content from a page
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum Extraction {
    /// Auto-detect main content using readability heuristics
    Auto,
    /// Use a specific CSS selector
    Selector { selector: String },
    /// Use entire page body
    Full,
    /// Extract specific meta tags
    Meta { tags: Vec<String> },
    /// RSS/Atom feed - use pre-formatted item text
    Rss,
    /// Extract JSON-LD structured data (schema.org)
    JsonLd {
        /// Optional filter for specific @type(s) like "Product", "Article"
        #[serde(default)]
        types: Option<Vec<String>>,
    },
}

impl Default for Extraction {
    fn default() -> Self {
        Self::Auto
    }
}

/// Normalization options applied before hashing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Normalization {
    /// Strip whitespace variations (default: true)
    #[serde(default = "default_true")]
    pub strip_whitespace: bool,
    /// Strip timestamps and dates
    #[serde(default)]
    pub strip_dates: bool,
    /// Strip random IDs or cache-busters
    #[serde(default)]
    pub strip_random_ids: bool,
    /// Custom regex patterns to ignore
    #[serde(default)]
    pub ignore_patterns: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Filter target - what the filter operates on
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterTarget {
    /// Match against new content
    New,
    /// Match against the diff text
    Diff,
    /// Match against old content
    Old,
}

/// A filter rule applied after detecting a change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    /// What to operate on
    pub on: FilterTarget,
    /// Contains this text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    /// Does not contain this text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_contains: Option<String>,
    /// Matches this regex pattern
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<String>,
    /// Diff size greater than N chars
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_gt: Option<usize>,
}

/// Fetch engine to use
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Http,
    Playwright,
    /// RSS/Atom feed parsing
    Rss,
    /// Shell command output
    Shell {
        command: String,
    },
}

/// Agent configuration for AI-powered change analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Whether the agent is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Custom prompt template (uses default if None)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_template: Option<String>,
    /// Custom instructions for this specific watch
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// A watch definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    /// Unique identifier
    pub id: Uuid,
    /// User-friendly name
    pub name: String,
    /// URL to monitor
    pub url: String,
    /// Fetch engine to use
    #[serde(default)]
    pub engine: Engine,
    /// Extraction strategy
    #[serde(default)]
    pub extraction: Extraction,
    /// Normalization options
    #[serde(default)]
    pub normalization: Normalization,
    /// Filter rules (empty = any change triggers notification)
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Agent configuration (None = no agent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_config: Option<AgentConfig>,
    /// Check interval in seconds
    pub interval_secs: u64,
    /// Whether the watch is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When the watch was created
    pub created_at: DateTime<Utc>,
    /// Custom headers for authenticated requests
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub headers: std::collections::HashMap<String, String>,
    /// Path to cookie file (Netscape format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookie_file: Option<String>,
    /// Playwright storage state file for authenticated sessions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_state: Option<String>,
    /// Per-watch notification target (overrides global default)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_target: Option<crate::config::NotifyTarget>,
    /// Tags for organization and filtering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Whether to include the user's interest profile in AI analysis
    #[serde(default)]
    pub use_profile: bool,
}

impl Watch {
    /// Create a new watch with defaults
    pub fn new(name: String, url: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            url,
            engine: Engine::default(),
            extraction: Extraction::default(),
            normalization: Normalization::default(),
            filters: Vec::new(),
            agent_config: None,
            interval_secs: 900, // 15 minutes
            enabled: true,
            created_at: Utc::now(),
            headers: std::collections::HashMap::new(),
            cookie_file: None,
            storage_state: None,
            notify_target: None,
            tags: Vec::new(),
            use_profile: false, // Opt-in per watch
        }
    }
}

/// A snapshot of page content at a point in time
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: Uuid,
    pub watch_id: Uuid,
    pub fetched_at: DateTime<Utc>,
    /// Raw HTML (zstd compressed), only kept for last 5
    pub raw_html: Option<Vec<u8>>,
    /// Extracted and normalized content
    pub extracted: String,
    /// SHA-256 hash of extracted content
    pub content_hash: String,
}

/// A detected change between snapshots
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub id: Uuid,
    pub watch_id: Uuid,
    pub detected_at: DateTime<Utc>,
    pub old_snapshot_id: Uuid,
    pub new_snapshot_id: Uuid,
    /// The diff text
    pub diff: String,
    /// Whether filters passed
    pub filter_passed: bool,
    /// Agent response if agent was invoked
    pub agent_response: Option<serde_json::Value>,
    /// Whether notification was sent
    pub notified: bool,
}

/// Agent memory stored per-watch
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentMemory {
    #[serde(default)]
    pub counters: std::collections::HashMap<String, i64>,
    /// Flexible value storage - accepts any JSON type (string, number, bool, etc.)
    #[serde(default)]
    pub last_values: std::collections::HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl AgentMemory {
    /// Maximum size in bytes (16KB)
    pub const MAX_SIZE: usize = 16 * 1024;

    /// Serialize to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Parse from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Check if memory exceeds size limit
    pub fn is_over_limit(&self) -> bool {
        self.to_json().map(|s| s.len() > Self::MAX_SIZE).unwrap_or(true)
    }

    /// Truncate oldest notes until under limit, also expire notes older than 7 days
    pub fn truncate_to_limit(&mut self) {
        use chrono::{Duration, Utc};

        // First, expire notes older than 7 days
        let cutoff = Utc::now() - Duration::days(7);
        self.notes.retain(|note| {
            // Try to extract timestamp from note (format: "2026-01-15T03:43:33" or "CRITICAL: 2026-01-15T03:43:33")
            let timestamp_str = if note.starts_with("CRITICAL:") || note.starts_with("WARNING:") {
                // Extract after the prefix
                note.split_whitespace().nth(1)
            } else {
                // First word is the timestamp
                note.split(':').next().and_then(|s| s.split_whitespace().next())
            };

            if let Some(ts) = timestamp_str {
                // Try to parse as ISO 8601 timestamp
                if let Ok(note_time) = chrono::DateTime::parse_from_rfc3339(ts) {
                    return note_time > cutoff;
                }
                // Also try parsing just the date portion with time
                if let Ok(note_time) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S") {
                    let note_utc = chrono::DateTime::<Utc>::from_naive_utc_and_offset(note_time, Utc);
                    return note_utc > cutoff;
                }
            }
            // If we can't parse the timestamp, keep the note
            true
        });

        // Then truncate if still over limit
        while self.is_over_limit() && !self.notes.is_empty() {
            self.notes.remove(0);
        }
    }
}

/// A simple reminder notification (not a web watcher)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// When to trigger (Unix timestamp)
    pub trigger_at: DateTime<Utc>,
    /// Interval for recurring reminders (None = one-shot)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
    pub enabled: bool,
    /// Per-reminder notification target override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_target: Option<crate::config::NotifyTarget>,
    pub created_at: DateTime<Utc>,
}
