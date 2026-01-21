use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Command, Stdio};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::{Config, NotifyTarget};
use crate::error::{KtoError, Result};

/// Extract individual changes as "old → new" pairs from diff
fn extract_change_pairs(diff: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut chars = diff.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '[' && chars.peek() == Some(&'-') {
            chars.next(); // consume '-'
            let mut old = String::new();
            let mut new = String::new();

            // Capture old content
            while let Some(&ch) = chars.peek() {
                if ch == ']' {
                    chars.next();
                    break;
                }
                old.push(chars.next().unwrap());
            }

            // Look for immediately following [+new]
            if chars.peek() == Some(&'[') {
                chars.next();
                if chars.peek() == Some(&'+') {
                    chars.next();
                    while let Some(&ch) = chars.peek() {
                        if ch == ']' {
                            chars.next();
                            break;
                        }
                        new.push(chars.next().unwrap());
                    }
                }
            }

            if !old.is_empty() || !new.is_empty() {
                pairs.push((old.trim().to_string(), new.trim().to_string()));
            }
        }
    }
    pairs
}

/// Get a clean text preview without raw diff markers
fn get_clean_preview(diff: &str, max_chars: usize) -> String {
    // Remove [-...] and [+...] markers, keep the content
    let mut clean = String::new();
    let mut chars = diff.chars().peekable();
    let mut in_marker = false;
    let mut marker_type = ' ';

    while let Some(c) = chars.next() {
        if c == '[' {
            if let Some(&next) = chars.peek() {
                if next == '-' || next == '+' {
                    in_marker = true;
                    marker_type = next;
                    chars.next(); // consume the - or +
                    continue;
                }
            }
        }
        if in_marker && c == ']' {
            in_marker = false;
            // Add space after removed content, nothing after added
            if marker_type == '-' {
                // Skip removed content entirely for cleaner preview
            }
            continue;
        }
        if in_marker {
            // Only include added content in preview
            if marker_type == '+' && clean.len() < max_chars {
                clean.push(c);
            }
            continue;
        }
        if clean.len() < max_chars {
            clean.push(c);
        }
    }

    // Clean up whitespace
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.len() > max_chars {
        format!("{}...", &clean[..max_chars])
    } else if clean.is_empty() {
        "Content updated".to_string()
    } else {
        clean
    }
}

/// Format AI response with title + bullets for notification
fn format_ai_message(payload: &NotificationPayload) -> Option<String> {
    // If we have title + bullets, use the new format
    if let Some(ref title) = payload.agent_title {
        let mut message = title.clone();

        if let Some(ref bullets) = payload.agent_bullets {
            if !bullets.is_empty() {
                let bullet_text = bullets
                    .iter()
                    .take(4) // Max 4 bullets
                    .map(|b| format!("• {}", b))
                    .collect::<Vec<_>>()
                    .join("\n");
                message = format!("{}\n\n{}", title, bullet_text);
            }
        }

        message.push_str(&format!("\n\n{}", payload.url));
        return Some(message);
    }

    // Fall back to legacy summary
    payload.agent_summary.as_ref().map(|s| {
        format!("{}\n\n{}", s, payload.url)
    })
}

/// Create a clean message without AI
fn format_fallback_message(payload: &NotificationPayload) -> String {
    // Count changes
    let additions = payload.diff.matches("[+").count();
    let removals = payload.diff.matches("[-").count();

    // Priority 1: Use smart_summary if available (e.g., "Price: $100 → $80")
    if let Some(ref smart) = payload.smart_summary {
        return format!("{}\n\n{}", smart, payload.url);
    }

    // Priority 2: For short diffs, show "old → new" format
    let change_pairs = extract_change_pairs(&payload.diff);
    if !change_pairs.is_empty() && payload.diff.len() < 300 {
        let mut bullets = Vec::new();
        for (old, new) in change_pairs.iter().take(4) {
            if old.is_empty() {
                bullets.push(format!("• Added: {}", truncate_str(new, 60)));
            } else if new.is_empty() {
                bullets.push(format!("• Removed: {}", truncate_str(old, 60)));
            } else {
                bullets.push(format!("• \"{}\" → \"{}\"", truncate_str(old, 30), truncate_str(new, 30)));
            }
        }
        if change_pairs.len() > 4 {
            bullets.push(format!("• ...and {} more changes", change_pairs.len() - 4));
        }
        return format!("Changes detected:\n{}\n\n{}", bullets.join("\n"), payload.url);
    }

    // Priority 3: For long diffs, show summary counts
    let summary = if additions > 0 && removals > 0 {
        format!("{} additions, {} removals", additions, removals)
    } else if additions > 0 {
        format!("{} additions", additions)
    } else if removals > 0 {
        format!("{} removals", removals)
    } else {
        "Content updated".to_string()
    };

    // Get clean preview of added content
    let preview = get_clean_preview(&payload.diff, 150);

    format!(
        "Changes detected:\n• {}\n\nPreview: {}\n\n{}",
        summary,
        preview,
        payload.url
    )
}

/// Truncate a string to max length with ellipsis
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Format the notification title with AI error handling
fn format_notification_title(payload: &NotificationPayload) -> String {
    if payload.agent_error.is_some() {
        // AI failed - show error indicator
        format!("kto: {} (AI failed)", payload.watch_name)
    } else if let Some(ref agent_title) = payload.agent_title {
        // AI succeeded with title
        format!("kto: {}: {}", payload.watch_name, agent_title)
    } else {
        // No AI
        format!("kto: {}", payload.watch_name)
    }
}

/// Get the best formatted message for notification
fn format_notification_message(payload: &NotificationPayload) -> String {
    // If AI failed, show error info first
    if let Some(ref error) = payload.agent_error {
        let mut message = format!("AI analysis failed: {}\n\n", error);
        message.push_str(&format_fallback_message(payload));
        return message;
    }

    // Try AI format first
    if let Some(msg) = format_ai_message(payload) {
        return msg;
    }

    // Fall back to clean non-AI format
    format_fallback_message(payload)
}

/// Legacy function for backwards compatibility (used by tests)
#[allow(dead_code)]
fn get_best_summary(payload: &NotificationPayload) -> String {
    // AI title takes highest priority
    if let Some(ref title) = payload.agent_title {
        return title.clone();
    }

    // Legacy AI summary
    if let Some(ref summary) = payload.agent_summary {
        return summary.clone();
    }

    // Smart pattern-detected summary is next
    if let Some(ref summary) = payload.smart_summary {
        return summary.clone();
    }

    // Fall back to counting changes
    let additions = payload.diff.matches("[+").count();
    let removals = payload.diff.matches("[-").count();

    if additions > 0 && removals > 0 {
        format!("+{} / -{} changes", additions, removals)
    } else if additions > 0 {
        format!("+{} additions", additions)
    } else if removals > 0 {
        format!("-{} removals", removals)
    } else {
        "Content changed".to_string()
    }
}

/// Notification payload sent to all targets
#[derive(Debug, Clone, Serialize)]
pub struct NotificationPayload {
    pub watch_id: String,
    pub watch_name: String,
    pub url: String,
    pub old_content: String,
    pub new_content: String,
    pub diff: String,
    /// Smart summary generated from diff pattern detection (e.g., "Price: $99 → $79")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smart_summary: Option<String>,
    /// AI-generated title (e.g., "Price Drop", "3 New Articles")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    /// AI-generated bullet points for key changes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_bullets: Option<Vec<String>>,
    /// AI agent summary (one-line, backwards compat)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_analysis: Option<String>,
    /// AI error message if analysis failed (e.g., "timeout", "rate_limit")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_error: Option<String>,
    pub detected_at: DateTime<Utc>,
}

/// Log notification to file for debugging/review
fn log_notification(payload: &NotificationPayload, target_type: &str) {
    if let Ok(data_dir) = Config::data_dir() {
        let log_path = data_dir.join("notifications.log");
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let title = payload.agent_title.as_deref().unwrap_or("(no title)");
            let message = format_notification_message(payload);
            let log_entry = format!(
                "\n{}\n{}\nWatch: {} | Target: {}\nTitle: {}\n{}\nURL: {}\n{}\n",
                "=".repeat(60),
                payload.detected_at.format("%Y-%m-%d %H:%M:%S UTC"),
                payload.watch_name,
                target_type,
                title,
                "-".repeat(40),
                payload.url,
                message,
            );
            let _ = file.write_all(log_entry.as_bytes());
        }
    }
}

/// Check if we're currently in quiet hours
pub fn is_quiet_hours() -> bool {
    if let Ok(config) = Config::load() {
        if let Some(ref quiet) = config.quiet_hours {
            return quiet.is_quiet_now();
        }
    }
    false
}

/// Send a notification to the specified target
pub fn send_notification(target: &NotifyTarget, payload: &NotificationPayload) -> Result<()> {
    // Check quiet hours
    if is_quiet_hours() {
        // Log but don't send
        log_notification(payload, "QUIET_HOURS_SUPPRESSED");
        return Ok(());
    }

    // Log notification for debugging
    let target_type = match target {
        NotifyTarget::Command { .. } => "command",
        NotifyTarget::Ntfy { .. } => "ntfy",
        NotifyTarget::Slack { .. } => "slack",
        NotifyTarget::Discord { .. } => "discord",
        NotifyTarget::Gotify { .. } => "gotify",
        NotifyTarget::Telegram { .. } => "telegram",
        NotifyTarget::Pushover { .. } => "pushover",
        NotifyTarget::Email { .. } => "email",
        NotifyTarget::Matrix { .. } => "matrix",
    };
    log_notification(payload, target_type);

    match target {
        NotifyTarget::Command { command } => send_command(command, payload),
        NotifyTarget::Ntfy { topic, server } => send_ntfy(topic, server.as_deref(), payload),
        NotifyTarget::Slack { webhook_url } => send_slack(webhook_url, payload),
        NotifyTarget::Discord { webhook_url } => send_discord(webhook_url, payload),
        NotifyTarget::Gotify { server, token } => send_gotify(server, token, payload),
        NotifyTarget::Telegram { bot_token, chat_id } => send_telegram(bot_token, chat_id, payload),
        NotifyTarget::Pushover { user_key, api_token } => send_pushover(user_key, api_token, payload),
        NotifyTarget::Email { smtp_server, smtp_port, username, password, from, to } => {
            send_email(smtp_server, *smtp_port, username, password, from, to, payload)
        }
        NotifyTarget::Matrix { homeserver, room_id, access_token } => {
            send_matrix(homeserver, room_id, access_token, payload)
        }
    }
}

/// Send notification via custom command (JSON on stdin)
fn send_command(command: &str, payload: &NotificationPayload) -> Result<()> {
    let json = serde_json::to_string(payload)?;

    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(ref mut stdin) = child.stdin {
        stdin.write_all(json.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::NotificationError(format!(
            "Command failed: {}",
            stderr
        )));
    }

    Ok(())
}

/// Send notification via ntfy
fn send_ntfy(topic: &str, server: Option<&str>, payload: &NotificationPayload) -> Result<()> {
    let server = server.unwrap_or("https://ntfy.sh");
    let url = format!("{}/{}", server, topic);

    let title = format_notification_title(payload);
    let message = format_notification_message(payload);

    ureq::post(&url)
        .header("Title", &title)
        .header("Priority", "default")
        .header("Tags", "eyes")
        .send(&message)?;

    Ok(())
}

/// Send notification via Slack webhook
fn send_slack(webhook_url: &str, payload: &NotificationPayload) -> Result<()> {
    let title = format_notification_title(payload);
    let body = format_notification_message(payload);

    let slack_payload = serde_json::json!({
        "text": format!("*{}*\n{}", title, body),
        "blocks": [
            {
                "type": "header",
                "text": {
                    "type": "plain_text",
                    "text": title
                }
            },
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": body
                }
            }
        ]
    });

    ureq::post(webhook_url)
        .header("Content-Type", "application/json")
        .send_json(&slack_payload)?;

    Ok(())
}

/// Send notification via Discord webhook
fn send_discord(webhook_url: &str, payload: &NotificationPayload) -> Result<()> {
    let title = format_notification_title(payload);
    let body = format_notification_message(payload);

    let discord_payload = serde_json::json!({
        "embeds": [
            {
                "title": title,
                "description": body,
                "url": payload.url,
                "color": 5814783, // Blue color
                "timestamp": payload.detected_at.to_rfc3339(),
                "footer": {
                    "text": "kto web monitor"
                }
            }
        ]
    });

    ureq::post(webhook_url)
        .header("Content-Type", "application/json")
        .send_json(&discord_payload)?;

    Ok(())
}

/// Send notification via Gotify
fn send_gotify(server: &str, token: &str, payload: &NotificationPayload) -> Result<()> {
    let url = format!("{}/message?token={}", server.trim_end_matches('/'), token);

    let title = format_notification_title(payload);
    let message = format_notification_message(payload);

    let gotify_payload = serde_json::json!({
        "title": title,
        "message": message,
        "priority": 5,
        "extras": {
            "client::display": {
                "contentType": "text/plain"
            },
            "client::notification": {
                "click": {
                    "url": payload.url
                }
            }
        }
    });

    ureq::post(&url)
        .header("Content-Type", "application/json")
        .send_json(&gotify_payload)?;

    Ok(())
}

/// Send notification via Telegram Bot API
fn send_telegram(bot_token: &str, chat_id: &str, payload: &NotificationPayload) -> Result<()> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);

    let title = format_notification_title(payload);
    let message = format_notification_message(payload);

    // Telegram supports basic HTML formatting
    let text = format!("<b>{}</b>\n\n{}", title, message);

    let telegram_payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "HTML",
        "disable_web_page_preview": false
    });

    ureq::post(&url)
        .header("Content-Type", "application/json")
        .send_json(&telegram_payload)?;

    Ok(())
}

/// Send notification via Pushover
fn send_pushover(user_key: &str, api_token: &str, payload: &NotificationPayload) -> Result<()> {
    let url = "https://api.pushover.net/1/messages.json";

    let title = format_notification_title(payload);
    let message = format_notification_message(payload);

    let pushover_payload = serde_json::json!({
        "token": api_token,
        "user": user_key,
        "title": title,
        "message": message,
        "url": payload.url,
        "url_title": "View Page"
    });

    ureq::post(url)
        .header("Content-Type", "application/json")
        .send_json(&pushover_payload)?;

    Ok(())
}

/// Send notification via Email (SMTP)
fn send_email(
    smtp_server: &str,
    smtp_port: Option<u16>,
    username: &str,
    password: &str,
    from: &str,
    to: &str,
    payload: &NotificationPayload,
) -> Result<()> {
    use std::io::Write;
    use std::net::TcpStream;

    let title = format_notification_title(payload);
    let message = format_notification_message(payload);

    let port = smtp_port.unwrap_or(587);
    let addr = format!("{}:{}", smtp_server, port);

    // Simple SMTP implementation (for basic use)
    // For production, consider using lettre crate
    let mut stream = TcpStream::connect(&addr)
        .map_err(|e| KtoError::NotificationError(format!("SMTP connection failed: {}", e)))?;

    // Read greeting
    let mut buf = [0u8; 1024];
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // EHLO
    write!(stream, "EHLO kto\r\n")?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // AUTH LOGIN (Base64 encoded)
    write!(stream, "AUTH LOGIN\r\n")?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    use base64::Engine as Base64Engine;
    let b64 = base64::engine::general_purpose::STANDARD;

    write!(stream, "{}\r\n", b64.encode(username))?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    write!(stream, "{}\r\n", b64.encode(password))?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // MAIL FROM
    write!(stream, "MAIL FROM:<{}>\r\n", from)?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // RCPT TO
    write!(stream, "RCPT TO:<{}>\r\n", to)?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // DATA
    write!(stream, "DATA\r\n")?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // Message
    write!(
        stream,
        "From: {}\r\nTo: {}\r\nSubject: {}\r\n\r\n{}\r\n.\r\n",
        from, to, title, message
    )?;
    let _ = std::io::Read::read(&mut stream, &mut buf);

    // QUIT
    write!(stream, "QUIT\r\n")?;

    Ok(())
}

/// Send notification via Matrix
fn send_matrix(
    homeserver: &str,
    room_id: &str,
    access_token: &str,
    payload: &NotificationPayload,
) -> Result<()> {
    // URL-encode the room ID (it contains special chars like !)
    let encoded_room = urlencoding::encode(room_id);
    let txn_id = uuid::Uuid::new_v4().to_string();

    let url = format!(
        "{}/_matrix/client/r0/rooms/{}/send/m.room.message/{}",
        homeserver.trim_end_matches('/'),
        encoded_room,
        txn_id
    );

    let title = format_notification_title(payload);
    let message = format_notification_message(payload);

    let formatted_body = format!("<b>{}</b><br/><br/>{}", title, message.replace('\n', "<br/>"));

    let matrix_payload = serde_json::json!({
        "msgtype": "m.text",
        "body": format!("{}\n\n{}", title, message),
        "format": "org.matrix.custom.html",
        "formatted_body": formatted_body
    });

    ureq::put(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", &format!("Bearer {}", access_token))
        .send_json(&matrix_payload)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_payload() -> NotificationPayload {
        NotificationPayload {
            watch_id: "test-id".to_string(),
            watch_name: "Test Watch".to_string(),
            url: "https://example.com".to_string(),
            old_content: "old".to_string(),
            new_content: "new".to_string(),
            diff: "[+new][-old]".to_string(),
            smart_summary: None,
            agent_title: None,
            agent_bullets: None,
            agent_summary: None,
            agent_analysis: None,
            agent_error: None,
            detected_at: Utc::now(),
        }
    }

    #[test]
    fn test_payload_serialization() {
        let mut payload = make_test_payload();
        payload.smart_summary = Some("Price: $99 → $79".to_string());
        payload.agent_title = Some("Price Drop".to_string());
        payload.agent_bullets = Some(vec!["$99 → $79".to_string()]);

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("Test Watch"));
        assert!(json.contains("example.com"));
        assert!(json.contains("Price Drop"));
    }

    #[test]
    fn test_get_best_summary() {
        // AI title wins
        let mut payload = make_test_payload();
        payload.agent_title = Some("Price Drop".to_string());
        payload.agent_summary = Some("Legacy".to_string());
        payload.smart_summary = Some("Smart".to_string());
        assert_eq!(get_best_summary(&payload), "Price Drop");

        // Legacy AI summary if no title
        payload.agent_title = None;
        assert_eq!(get_best_summary(&payload), "Legacy");

        // Smart summary if no AI
        payload.agent_summary = None;
        assert_eq!(get_best_summary(&payload), "Smart");

        // Fallback to generic
        payload.smart_summary = None;
        assert_eq!(get_best_summary(&payload), "+1 / -1 changes");
    }

    #[test]
    fn test_format_notification_with_ai() {
        let mut payload = make_test_payload();
        payload.agent_title = Some("3 New Stories".to_string());
        payload.agent_bullets = Some(vec![
            "Apple announces new chip".to_string(),
            "Tesla stock rises".to_string(),
        ]);

        let msg = format_notification_message(&payload);
        assert!(msg.contains("3 New Stories"));
        assert!(msg.contains("• Apple announces new chip"));
        assert!(msg.contains("• Tesla stock rises"));
        assert!(msg.contains("https://example.com"));
    }

    #[test]
    fn test_format_notification_fallback() {
        let payload = make_test_payload();
        let msg = format_notification_message(&payload);

        // Should have structured format, not raw diff
        assert!(msg.contains("Changes detected"));
        assert!(!msg.contains("[-old]")); // No raw diff markers
        assert!(msg.contains("https://example.com"));
    }

    #[test]
    fn test_clean_preview() {
        let diff = "Hello [-old][+new] world [-removed][+added] test";
        let preview = get_clean_preview(diff, 100);
        // Should show unchanged + added content, not removed
        assert!(preview.contains("Hello"));
        assert!(preview.contains("new"));
        assert!(preview.contains("added"));
        assert!(!preview.contains("old"));
        assert!(!preview.contains("removed"));
    }
}
