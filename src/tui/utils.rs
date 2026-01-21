//! TUI utility functions - interval parsing, formatting, and helpers

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Preset intervals for easy selection
/// 30s, 1m, 2m, 5m, 10m, 15m, 30m, 1h, 2h, 3h, 6h, 12h, 1d, 2d, 3d, 4d, 5d, 6d, 1w
pub const INTERVAL_PRESETS: &[u64] = &[
    30,           // 30 seconds
    60,           // 1 minute
    120,          // 2 minutes
    300,          // 5 minutes
    600,          // 10 minutes
    900,          // 15 minutes
    1800,         // 30 minutes
    3600,         // 1 hour
    7200,         // 2 hours
    10800,        // 3 hours
    21600,        // 6 hours
    43200,        // 12 hours
    86400,        // 1 day
    172800,       // 2 days
    259200,       // 3 days
    345600,       // 4 days
    432000,       // 5 days
    518400,       // 6 days
    604800,       // 1 week
];

pub fn next_interval_preset(current: u64) -> u64 {
    for &preset in INTERVAL_PRESETS {
        if preset > current {
            return preset;
        }
    }
    // Already at max, stay there
    *INTERVAL_PRESETS.last().unwrap_or(&current)
}

pub fn prev_interval_preset(current: u64) -> u64 {
    let mut prev = INTERVAL_PRESETS[0];
    for &preset in INTERVAL_PRESETS {
        if preset >= current {
            return prev;
        }
        prev = preset;
    }
    // Already at or past max, return last
    prev
}

/// Format seconds as human-readable interval (e.g., "5m", "2h", "1d")
pub fn format_interval(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else if secs >= 604800 && secs % 604800 == 0 {
        format!("{}w", secs / 604800)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Parse HH:MM time string to next occurrence as UTC DateTime
pub fn parse_time_to_datetime(time_str: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{Local, NaiveTime, Utc};

    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let hour: u32 = parts[0].parse().ok()?;
    let minute: u32 = parts[1].parse().ok()?;

    let target_time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    let now = Local::now();
    let today = now.date_naive();

    // Try today first, then tomorrow
    let target_datetime = if let Some(dt) = today.and_time(target_time).and_local_timezone(Local).single() {
        if dt > now {
            dt
        } else {
            // Tomorrow
            let tomorrow = today.succ_opt()?;
            tomorrow.and_time(target_time).and_local_timezone(Local).single()?
        }
    } else {
        return None;
    };

    Some(target_datetime.with_timezone(&Utc))
}

/// Unified duration parser - supports "30s", "5m", "2h", "1d", "1w" or plain seconds
pub fn parse_duration_str(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }

    // Try to parse as pure number (seconds)
    if let Ok(secs) = s.parse::<u64>() {
        return Some(secs);
    }

    // Parse with suffix
    let (num_str, unit) = if s.ends_with('s') {
        (&s[..s.len()-1], 1u64)
    } else if s.ends_with('m') {
        (&s[..s.len()-1], 60u64)
    } else if s.ends_with('h') {
        (&s[..s.len()-1], 3600u64)
    } else if s.ends_with('d') {
        (&s[..s.len()-1], 86400u64)
    } else if s.ends_with('w') {
        (&s[..s.len()-1], 604800u64)
    } else {
        return None;
    };

    num_str.parse::<u64>().ok().map(|n| n * unit)
}

/// Parse extraction string back to Extraction enum
pub fn parse_extraction_string(s: &str) -> crate::watch::Extraction {
    use crate::watch::Extraction;

    let s = s.trim().to_lowercase();
    if s == "auto" || s.is_empty() {
        Extraction::Auto
    } else if s == "full" {
        Extraction::Full
    } else if s == "rss" {
        Extraction::Rss
    } else if s == "jsonld" || s == "json_ld" || s == "json-ld" {
        Extraction::JsonLd { types: None }
    } else if let Some(selector) = s.strip_prefix("css:") {
        Extraction::Selector { selector: selector.to_string() }
    } else if let Some(tags) = s.strip_prefix("meta:") {
        Extraction::Meta { tags: tags.split(',').map(|t| t.trim().to_string()).collect() }
    } else if let Some(types) = s.strip_prefix("jsonld:") {
        Extraction::JsonLd { types: Some(types.split(',').map(|t| t.trim().to_string()).collect()) }
    } else if !s.is_empty() {
        // Assume it's a CSS selector if it doesn't match known types
        Extraction::Selector { selector: s.to_string() }
    } else {
        Extraction::Auto
    }
}

/// Build NotifyTarget from structured notify type and value
pub fn build_notify_target(notify_type: &super::types::NotifyType, value: &str) -> Option<crate::config::NotifyTarget> {
    use crate::config::NotifyTarget;
    use super::types::NotifyType;

    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    match notify_type {
        NotifyType::Ntfy => Some(NotifyTarget::Ntfy {
            topic: value.to_string(),
            server: None,
        }),
        NotifyType::Gotify => {
            // Format: server|token
            let parts: Vec<&str> = value.splitn(2, '|').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                Some(NotifyTarget::Gotify {
                    server: parts[0].to_string(),
                    token: parts[1].to_string(),
                })
            } else {
                None
            }
        }
        NotifyType::Slack => Some(NotifyTarget::Slack {
            webhook_url: value.to_string(),
        }),
        NotifyType::Discord => Some(NotifyTarget::Discord {
            webhook_url: value.to_string(),
        }),
        NotifyType::Telegram => {
            // Format: chat_id|bot_token
            let parts: Vec<&str> = value.splitn(2, '|').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                Some(NotifyTarget::Telegram {
                    chat_id: parts[0].to_string(),
                    bot_token: parts[1].to_string(),
                })
            } else {
                None
            }
        }
        NotifyType::Pushover => {
            // Format: user_key|api_token
            let parts: Vec<&str> = value.splitn(2, '|').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                Some(NotifyTarget::Pushover {
                    user_key: parts[0].to_string(),
                    api_token: parts[1].to_string(),
                })
            } else {
                None
            }
        }
        NotifyType::Command => Some(NotifyTarget::Command {
            command: value.to_string(),
        }),
    }
}

/// Create a centered rectangle
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
