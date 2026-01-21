//! Shared utility functions

/// Minimum interval to prevent tight loops (10 seconds)
pub const MIN_INTERVAL_SECS: u64 = 10;

/// Unified duration parser - supports "30s", "5m", "2h", "1d", "1w" or plain seconds
/// This is the canonical duration parser - use this everywhere
pub fn parse_duration(s: &str) -> Option<u64> {
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

/// Parse interval string like "5s", "30s", "1m", "5m", "2h", "1d", "1w" into seconds
/// Enforces a minimum of 10 seconds to prevent accidental tight loops
pub fn parse_interval_str(s: &str) -> kto::Result<u64> {
    let secs = parse_duration(s).ok_or_else(|| {
        kto::KtoError::ConfigError(format!(
            "Invalid interval '{}'. Use format like 30s, 5m, 2h, 1d, 1w", s
        ))
    })?;

    if secs < MIN_INTERVAL_SECS {
        return Err(kto::KtoError::ConfigError(format!(
            "Interval {}s is too short. Minimum is {}s to prevent rate limiting.",
            secs, MIN_INTERVAL_SECS
        )));
    }

    Ok(secs)
}

/// Parse duration with minimum enforcement (10 seconds)
#[allow(dead_code)]
pub fn parse_duration_min(s: &str, min_secs: u64) -> Option<u64> {
    parse_duration(s).map(|secs| secs.max(min_secs))
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

/// Truncate a string to max_len characters (not bytes), adding "..." if truncated.
/// Safe for non-ASCII content (emoji, CJK, etc).
pub fn truncate_str(s: &str, max_len: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        chars[..max_len].iter().collect()
    } else {
        format!("{}...", chars[..max_len - 3].iter().collect::<String>())
    }
}

/// Extract a URL from user input (handles both full URLs and bare domains)
pub fn extract_url(input: &str) -> Option<String> {
    // Look for http:// or https:// URLs first
    for word in input.split_whitespace() {
        if word.starts_with("http://") || word.starts_with("https://") {
            if url::Url::parse(word).is_ok() {
                return Some(word.to_string());
            }
        }
    }

    // Try to find bare domains and auto-add https://
    for word in input.split_whitespace() {
        // Skip words that look like paths or flags
        if word.starts_with('/') || word.starts_with('-') {
            continue;
        }
        // Check if it looks like a domain (contains a dot, no spaces)
        if word.contains('.') && !word.contains(' ') {
            let with_scheme = format!("https://{}", word);
            if url::Url::parse(&with_scheme).is_ok() {
                return Some(with_scheme);
            }
        }
    }
    None
}

/// Extract domain from URL for rate limiting
pub fn extract_domain(url: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_string()))
}

/// Get content from system clipboard
pub fn get_clipboard_content() -> Option<String> {
    // Try platform-specific clipboard commands
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("pbpaste").output().ok()?;
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try xclip first (X11)
        if let Ok(output) = std::process::Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
        {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }

        // Try xsel as fallback
        if let Ok(output) = std::process::Command::new("xsel")
            .args(["--clipboard", "--output"])
            .output()
        {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }

        // Try wl-paste for Wayland
        if let Ok(output) = std::process::Command::new("wl-paste").output() {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, we'd use powershell
        if let Ok(output) = std::process::Command::new("powershell")
            .args(["-command", "Get-Clipboard"])
            .output()
        {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }
    }

    None
}

/// Parse time string (HH:MM) to next occurrence as DateTime<Utc>
pub fn parse_time_to_next(time_str: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{Local, NaiveTime, Utc};

    // Parse HH:MM format
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("30s"), Some(30));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("2h"), Some(7200));
        assert_eq!(parse_duration("1d"), Some(86400));
        assert_eq!(parse_duration("1w"), Some(604800));
        assert_eq!(parse_duration("300"), Some(300));
        assert_eq!(parse_duration("invalid"), None);
    }

    #[test]
    fn test_format_interval() {
        assert_eq!(format_interval(30), "30s");
        assert_eq!(format_interval(300), "5m");
        assert_eq!(format_interval(7200), "2h");
        assert_eq!(format_interval(86400), "1d");
        assert_eq!(format_interval(604800), "1w");
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("ab", 3), "ab");
    }

    #[test]
    fn test_extract_url() {
        assert_eq!(extract_url("https://example.com"), Some("https://example.com".to_string()));
        assert_eq!(extract_url("check example.com for updates"), Some("https://example.com".to_string()));
        assert_eq!(extract_url("no url here"), None);
    }
}
