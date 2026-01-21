use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{KtoError, Result};

/// Global kto configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Default notification target
    #[serde(default)]
    pub default_notify: Option<NotifyTarget>,

    /// Per-domain rate limits (requests per second)
    #[serde(default)]
    pub rate_limits: HashMap<String, f64>,

    /// Default check interval in seconds
    #[serde(default = "default_interval")]
    pub default_interval_secs: u64,

    /// Quiet hours - don't send notifications during this time
    #[serde(default)]
    pub quiet_hours: Option<QuietHours>,
}

/// Quiet hours configuration - suppress notifications during specified time range
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuietHours {
    /// Start time in HH:MM format (e.g., "22:00")
    pub start: String,
    /// End time in HH:MM format (e.g., "08:00")
    pub end: String,
    /// Timezone (e.g., "America/New_York", "UTC"). Defaults to local time.
    #[serde(default)]
    pub timezone: Option<String>,
}

impl QuietHours {
    /// Check if the current time is within quiet hours
    pub fn is_quiet_now(&self) -> bool {
        use chrono::{Local, NaiveTime, Timelike};

        let now = Local::now();
        let current_time = NaiveTime::from_hms_opt(now.hour(), now.minute(), 0)
            .unwrap_or_else(|| NaiveTime::from_hms_opt(0, 0, 0).unwrap());

        let start = NaiveTime::parse_from_str(&self.start, "%H:%M")
            .unwrap_or_else(|_| NaiveTime::from_hms_opt(22, 0, 0).unwrap());
        let end = NaiveTime::parse_from_str(&self.end, "%H:%M")
            .unwrap_or_else(|_| NaiveTime::from_hms_opt(8, 0, 0).unwrap());

        // Handle overnight ranges (e.g., 22:00 to 08:00)
        if start > end {
            // Quiet if after start OR before end
            current_time >= start || current_time < end
        } else {
            // Normal range (e.g., 13:00 to 14:00)
            current_time >= start && current_time < end
        }
    }
}

fn default_interval() -> u64 {
    900 // 15 minutes
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_notify: None,
            rate_limits: HashMap::new(),
            default_interval_secs: default_interval(),
            quiet_hours: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotifyTarget {
    Command { command: String },
    Ntfy { topic: String, server: Option<String> },
    Slack { webhook_url: String },
    Discord { webhook_url: String },
    Gotify { server: String, token: String },
    /// Telegram Bot API
    Telegram { bot_token: String, chat_id: String },
    /// Pushover notifications
    Pushover { user_key: String, api_token: String },
    /// Email via SMTP
    Email {
        smtp_server: String,
        smtp_port: Option<u16>,
        username: String,
        password: String,
        from: String,
        to: String,
    },
    /// Matrix messaging
    Matrix {
        homeserver: String,
        room_id: String,
        access_token: String,
    },
}

impl Config {
    /// Load configuration from the default location
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to the default location
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| KtoError::ConfigError(e.to_string()))?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Get the config file path
    pub fn config_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "kto")
            .ok_or_else(|| KtoError::ConfigError("Could not determine config directory".into()))?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Get the data directory path
    pub fn data_dir() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "kto")
            .ok_or_else(|| KtoError::ConfigError("Could not determine data directory".into()))?;
        Ok(dirs.data_dir().to_path_buf())
    }

    /// Get the database path
    ///
    /// Supports KTO_DB environment variable for test isolation
    pub fn db_path() -> Result<PathBuf> {
        // Check for environment variable override first
        if let Ok(path) = std::env::var("KTO_DB") {
            return Ok(PathBuf::from(path));
        }
        Ok(Self::data_dir()?.join("kto.db"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.default_interval_secs, 900);
        assert!(config.rate_limits.is_empty());
    }
}
