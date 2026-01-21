//! User interest profile for AI-aware change filtering.
//!
//! This module provides a global user profile that can be passed to AI agents
//! to help them understand what changes are relevant to the user.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::{KtoError, Result};

/// The user's interest profile - describes what they care about.
///
/// This is stored in `~/.config/kto/interests.toml` and can be edited manually
/// or through `kto profile` commands.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InterestProfile {
    /// Free-form description of the user (background context)
    #[serde(default)]
    pub profile: ProfileDescription,

    /// Structured interests with keywords and weights
    #[serde(default)]
    pub interests: Vec<Interest>,
}

/// Free-form profile description
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileDescription {
    /// Free-form description of who the user is and what they care about
    #[serde(default)]
    pub description: String,
}

/// A structured interest with keywords and relevance scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interest {
    /// Human-readable name for this interest (e.g., "AI/ML", "Rust")
    pub name: String,

    /// Keywords associated with this interest
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Relevance weight (0.0 - 1.0), higher = more important
    #[serde(default = "default_weight")]
    pub weight: f64,

    /// Matching scope: "broad" includes related topics, "narrow" is exact matches only
    #[serde(default = "default_scope")]
    pub scope: InterestScope,

    /// Source watches that contributed to this interest (for inferred interests)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

fn default_weight() -> f64 {
    0.5
}

fn default_scope() -> InterestScope {
    InterestScope::Broad
}

/// Interest matching scope
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InterestScope {
    /// Match related topics (more permissive)
    #[default]
    Broad,
    /// Match exact keywords only (more restrictive)
    Narrow,
}

impl InterestProfile {
    /// Load the interest profile from the default location
    pub fn load() -> Result<Self> {
        let path = Self::profile_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    /// Save the interest profile to the default location
    pub fn save(&self) -> Result<()> {
        let path = Self::profile_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| KtoError::ConfigError(e.to_string()))?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Get the profile file path
    pub fn profile_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "kto")
            .ok_or_else(|| KtoError::ConfigError("Could not determine config directory".into()))?;
        Ok(dirs.config_dir().join("interests.toml"))
    }

    /// Check if the profile is empty (no meaningful content)
    pub fn is_empty(&self) -> bool {
        self.profile.description.trim().is_empty() && self.interests.is_empty()
    }

    /// Generate the profile section for the AI prompt
    pub fn to_prompt_section(&self) -> String {
        let mut sections = Vec::new();

        // Add free-form description if present
        if !self.profile.description.trim().is_empty() {
            sections.push(format!(
                "=== USER PROFILE (background context) ===\n{}",
                self.profile.description.trim()
            ));
        }

        // Add structured interests if present
        if !self.interests.is_empty() {
            let mut interest_lines = Vec::new();
            for interest in &self.interests {
                let keywords = interest.keywords.join(", ");
                let scope = match interest.scope {
                    InterestScope::Broad => "broad",
                    InterestScope::Narrow => "narrow",
                };
                interest_lines.push(format!(
                    "- {} (weight: {:.1}, {}): {}",
                    interest.name, interest.weight, scope, keywords
                ));
            }
            sections.push(format!(
                "=== INTEREST KEYWORDS (relevance hints) ===\n{}",
                interest_lines.join("\n")
            ));
        }

        sections.join("\n\n")
    }

    /// Create a template profile for new users
    pub fn template() -> Self {
        Self {
            profile: ProfileDescription {
                description: r#"# Describe what you're interested in here.
# This helps kto's AI understand what changes matter to you.
#
# Example:
# I'm a software engineer interested in:
# - Rust and systems programming
# - AI/ML developments, especially Claude and LLMs
# - Startup news and funding rounds
"#.to_string(),
            },
            interests: vec![
                Interest {
                    name: "Example Interest".to_string(),
                    keywords: vec!["keyword1".to_string(), "keyword2".to_string()],
                    weight: 0.7,
                    scope: InterestScope::Broad,
                    sources: vec![],
                },
            ],
        }
    }
}

/// Global memory that persists learnings across all watches.
///
/// Unlike per-watch AgentMemory, this tracks patterns and observations
/// that apply globally to the user's preferences.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalMemory {
    /// Observations learned from watching user behavior
    #[serde(default)]
    pub observations: Vec<Observation>,

    /// Inferred interest signals (topic -> confidence score)
    #[serde(default)]
    pub interest_signals: HashMap<String, f64>,

    /// Which watch last updated this memory
    #[serde(default)]
    pub last_updated_by_watch: Option<String>,

    /// Count of observations by source watch (for provenance)
    #[serde(default)]
    pub observation_count_by_watch: HashMap<String, u32>,
}

/// An observation learned from monitoring behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// The observation text (e.g., "User ignores minor UI changes")
    pub text: String,

    /// Which watch generated this observation
    pub source_watch: String,

    /// When this observation was made
    pub created_at: DateTime<Utc>,

    /// Confidence in this observation (0.0 - 1.0)
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    0.5
}

impl GlobalMemory {
    /// Maximum size in bytes (32KB - larger than per-watch memory)
    pub const MAX_SIZE: usize = 32 * 1024;

    /// Maximum age for observations before decay (30 days)
    pub const MAX_OBSERVATION_AGE_DAYS: i64 = 30;

    /// Serialize to JSON string
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| KtoError::ConfigError(e.to_string()))
    }

    /// Parse from JSON string
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| KtoError::ConfigError(e.to_string()))
    }

    /// Check if memory is empty
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty() && self.interest_signals.is_empty()
    }

    /// Add an observation with provenance tracking
    pub fn add_observation(&mut self, text: String, source_watch: String, confidence: f64) {
        self.observations.push(Observation {
            text,
            source_watch: source_watch.clone(),
            created_at: Utc::now(),
            confidence,
        });

        // Track provenance
        *self.observation_count_by_watch.entry(source_watch.clone()).or_insert(0) += 1;
        self.last_updated_by_watch = Some(source_watch);
    }

    /// Apply recency decay - reduce confidence of old observations
    pub fn apply_decay(&mut self) {
        let cutoff = Utc::now() - Duration::days(Self::MAX_OBSERVATION_AGE_DAYS);

        // Remove very old observations
        self.observations.retain(|obs| obs.created_at > cutoff);

        // Apply decay to confidence based on age
        for obs in &mut self.observations {
            let age_days = (Utc::now() - obs.created_at).num_days();
            let decay_factor = 1.0 - (age_days as f64 / Self::MAX_OBSERVATION_AGE_DAYS as f64 * 0.5);
            obs.confidence *= decay_factor.max(0.5); // Don't decay below 50%
        }
    }

    /// Clear all observations (keep interest_signals)
    pub fn clear_observations(&mut self) {
        self.observations.clear();
        self.observation_count_by_watch.clear();
        self.last_updated_by_watch = None;
    }

    /// Clear everything
    pub fn clear_all(&mut self) {
        self.observations.clear();
        self.interest_signals.clear();
        self.observation_count_by_watch.clear();
        self.last_updated_by_watch = None;
    }

    /// Generate the memory section for the AI prompt
    pub fn to_prompt_section(&self) -> String {
        if self.is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();

        // Add observations with source attribution
        if !self.observations.is_empty() {
            lines.push("=== LEARNED PATTERNS (from watching your behavior) ===".to_string());
            for obs in &self.observations {
                lines.push(format!(
                    "- {} (confidence: {:.1}, source: {})",
                    obs.text, obs.confidence, obs.source_watch
                ));
            }
        }

        // Add interest signals
        if !self.interest_signals.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("=== INFERRED INTERESTS (from monitoring patterns) ===".to_string());
            let mut signals: Vec<_> = self.interest_signals.iter().collect();
            signals.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (topic, score) in signals.iter().take(10) {
                lines.push(format!("- {}: {:.1}", topic, score));
            }
        }

        lines.join("\n")
    }

    /// Truncate to fit within size limit
    pub fn truncate_to_limit(&mut self) {
        // Remove oldest observations until under limit
        while let Ok(json) = self.to_json() {
            if json.len() <= Self::MAX_SIZE {
                break;
            }
            if self.observations.is_empty() {
                // If no observations left, truncate interest_signals
                let signals: Vec<_> = self.interest_signals.drain().collect();
                let keep_count = signals.len().saturating_sub(1);
                for (k, v) in signals.into_iter().take(keep_count) {
                    self.interest_signals.insert(k, v);
                }
                if self.interest_signals.is_empty() {
                    break;
                }
            } else {
                // Remove oldest observation
                self.observations.remove(0);
            }
        }
    }
}

/// Response from AI when inferring interests from watches
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferredInterests {
    /// Suggested interests based on watch analysis
    pub interests: Vec<Interest>,

    /// Overall confidence in the inference
    #[serde(default = "default_confidence")]
    pub confidence: f64,

    /// Reasoning for the inferred interests
    #[serde(default)]
    pub reasoning: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interest_profile_default() {
        let profile = InterestProfile::default();
        assert!(profile.is_empty());
    }

    #[test]
    fn test_interest_profile_template() {
        let profile = InterestProfile::template();
        assert!(!profile.is_empty());
        assert!(!profile.interests.is_empty());
    }

    #[test]
    fn test_interest_profile_to_prompt() {
        let profile = InterestProfile {
            profile: ProfileDescription {
                description: "I'm a software engineer".to_string(),
            },
            interests: vec![
                Interest {
                    name: "Rust".to_string(),
                    keywords: vec!["rust".to_string(), "cargo".to_string()],
                    weight: 0.8,
                    scope: InterestScope::Narrow,
                    sources: vec![],
                },
            ],
        };

        let prompt = profile.to_prompt_section();
        assert!(prompt.contains("software engineer"));
        assert!(prompt.contains("Rust"));
        assert!(prompt.contains("narrow"));
    }

    #[test]
    fn test_global_memory_add_observation() {
        let mut memory = GlobalMemory::default();
        memory.add_observation(
            "User ignores UI changes".to_string(),
            "HN".to_string(),
            0.8,
        );

        assert_eq!(memory.observations.len(), 1);
        assert_eq!(memory.observation_count_by_watch.get("HN"), Some(&1));
        assert_eq!(memory.last_updated_by_watch, Some("HN".to_string()));
    }

    #[test]
    fn test_global_memory_clear() {
        let mut memory = GlobalMemory::default();
        memory.add_observation("test".to_string(), "watch".to_string(), 0.5);
        memory.interest_signals.insert("AI".to_string(), 0.9);

        memory.clear_observations();
        assert!(memory.observations.is_empty());
        assert!(!memory.interest_signals.is_empty()); // Kept

        memory.clear_all();
        assert!(memory.interest_signals.is_empty()); // Now cleared
    }

    #[test]
    fn test_global_memory_to_prompt() {
        let mut memory = GlobalMemory::default();
        memory.add_observation(
            "User prefers price changes".to_string(),
            "Amazon".to_string(),
            0.9,
        );
        memory.interest_signals.insert("pricing".to_string(), 0.85);

        let prompt = memory.to_prompt_section();
        assert!(prompt.contains("LEARNED PATTERNS"));
        assert!(prompt.contains("price changes"));
        assert!(prompt.contains("INFERRED INTERESTS"));
        assert!(prompt.contains("pricing"));
    }
}
