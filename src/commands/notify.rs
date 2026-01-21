//! Notification commands: set, show, test, quiet

use chrono::Utc;
use colored::Colorize;
use inquire::{Select, Text};
use std::io::{self, Write};

use kto::config::{Config, NotifyTarget, QuietHours};
use kto::notify::{send_notification, NotificationPayload};
use kto::error::Result;

/// Set up notification target
pub fn cmd_notify_set(
    ntfy: Option<String>,
    slack: Option<String>,
    discord: Option<String>,
    gotify_server: Option<String>,
    gotify_token: Option<String>,
    command: Option<String>,
    telegram_token: Option<String>,
    telegram_chat: Option<String>,
    pushover_user: Option<String>,
    pushover_token: Option<String>,
    matrix_server: Option<String>,
    matrix_room: Option<String>,
    matrix_token: Option<String>,
) -> Result<()> {
    let mut config = Config::load()?;

    // Check if any direct flags were provided
    let direct_target = if let Some(topic) = ntfy {
        Some(NotifyTarget::Ntfy { topic, server: None })
    } else if let Some(webhook_url) = slack {
        Some(NotifyTarget::Slack { webhook_url })
    } else if let Some(webhook_url) = discord {
        Some(NotifyTarget::Discord { webhook_url })
    } else if gotify_server.is_some() || gotify_token.is_some() {
        // Gotify requires both server and token
        match (gotify_server, gotify_token) {
            (Some(server), Some(token)) => Some(NotifyTarget::Gotify { server, token }),
            (Some(_), None) => {
                return Err(kto::KtoError::ConfigError(
                    "--gotify-server requires --gotify-token".into()
                ));
            }
            (None, Some(_)) => {
                return Err(kto::KtoError::ConfigError(
                    "--gotify-token requires --gotify-server".into()
                ));
            }
            (None, None) => None,
        }
    } else if telegram_token.is_some() || telegram_chat.is_some() {
        // Telegram requires both token and chat_id
        match (telegram_token, telegram_chat) {
            (Some(bot_token), Some(chat_id)) => Some(NotifyTarget::Telegram { bot_token, chat_id }),
            (Some(_), None) => {
                return Err(kto::KtoError::ConfigError(
                    "--telegram-token requires --telegram-chat".into()
                ));
            }
            (None, Some(_)) => {
                return Err(kto::KtoError::ConfigError(
                    "--telegram-chat requires --telegram-token".into()
                ));
            }
            (None, None) => None,
        }
    } else if pushover_user.is_some() || pushover_token.is_some() {
        // Pushover requires both user and token
        match (pushover_user, pushover_token) {
            (Some(user_key), Some(api_token)) => Some(NotifyTarget::Pushover { user_key, api_token }),
            (Some(_), None) => {
                return Err(kto::KtoError::ConfigError(
                    "--pushover-user requires --pushover-token".into()
                ));
            }
            (None, Some(_)) => {
                return Err(kto::KtoError::ConfigError(
                    "--pushover-token requires --pushover-user".into()
                ));
            }
            (None, None) => None,
        }
    } else if matrix_server.is_some() || matrix_room.is_some() || matrix_token.is_some() {
        // Matrix requires server, room, and token
        match (matrix_server, matrix_room, matrix_token) {
            (Some(homeserver), Some(room_id), Some(access_token)) => {
                Some(NotifyTarget::Matrix { homeserver, room_id, access_token })
            }
            _ => {
                return Err(kto::KtoError::ConfigError(
                    "Matrix requires --matrix-server, --matrix-room, and --matrix-token".into()
                ));
            }
        }
    } else if let Some(cmd) = command {
        Some(NotifyTarget::Command { command: cmd })
    } else {
        None
    };

    if let Some(target) = direct_target {
        // Direct flag provided - no interactive prompt
        config.default_notify = Some(target.clone());
        config.save()?;
        println!("Notification settings saved.");
        prompt_test_notification(&target);
    } else {
        // No flags - run interactive prompt
        println!("\nNotification Setup\n");

        if let Some(target) = prompt_notification_setup()? {
            config.default_notify = Some(target.clone());
            config.save()?;
            println!("\n  Notification settings saved!");
            prompt_test_notification(&target);
        } else {
            println!("\n  Notification setup cancelled.");
        }
    }

    Ok(())
}

/// Show current notification settings
pub fn cmd_notify_show() -> Result<()> {
    let config = Config::load()?;

    println!("\nNotification Settings\n");

    match &config.default_notify {
        Some(target) => {
            println!("  Target: {}", describe_notify_target(target));
        }
        None => {
            println!("  No notification target configured.");
            println!("  Run `kto notify set` to configure notifications.");
        }
    }

    // Show quiet hours
    match &config.quiet_hours {
        Some(quiet) => {
            let status = if quiet.is_quiet_now() {
                "ACTIVE NOW".yellow()
            } else {
                "scheduled".normal()
            };
            println!("\n  Quiet hours: {} - {} ({})", quiet.start, quiet.end, status);
        }
        None => {
            println!("\n  Quiet hours: not configured");
        }
    }

    if let Ok(path) = Config::config_path() {
        println!("\n  Config file: {}", path.display());
    }

    Ok(())
}

/// Configure quiet hours
pub fn cmd_notify_quiet(start: Option<String>, end: Option<String>, disable: bool) -> Result<()> {
    let mut config = Config::load()?;

    if disable {
        config.quiet_hours = None;
        config.save()?;
        println!("Quiet hours disabled.");
        return Ok(());
    }

    match (start, end) {
        (Some(s), Some(e)) => {
            // Validate time format
            if chrono::NaiveTime::parse_from_str(&s, "%H:%M").is_err() {
                return Err(kto::KtoError::ConfigError(
                    format!("Invalid start time '{}'. Use HH:MM format (e.g., 22:00)", s)
                ));
            }
            if chrono::NaiveTime::parse_from_str(&e, "%H:%M").is_err() {
                return Err(kto::KtoError::ConfigError(
                    format!("Invalid end time '{}'. Use HH:MM format (e.g., 08:00)", e)
                ));
            }

            config.quiet_hours = Some(QuietHours {
                start: s.clone(),
                end: e.clone(),
                timezone: None,
            });
            config.save()?;
            println!("Quiet hours set: {} to {}", s, e);
            println!("Notifications will be suppressed during this time.");
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(kto::KtoError::ConfigError(
                "Both --start and --end are required".into()
            ));
        }
        (None, None) => {
            // Interactive mode
            let start = Text::new("Quiet hours start time (HH:MM):")
                .with_default("22:00")
                .with_help_message("When to stop sending notifications")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            let end = Text::new("Quiet hours end time (HH:MM):")
                .with_default("08:00")
                .with_help_message("When to resume notifications")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            config.quiet_hours = Some(QuietHours {
                start: start.clone(),
                end: end.clone(),
                timezone: None,
            });
            config.save()?;
            println!("\nQuiet hours set: {} to {}", start, end);
            println!("Notifications will be suppressed during this time.");
        }
    }

    Ok(())
}

/// Send a test notification
pub fn cmd_notify_test() -> Result<()> {
    let config = Config::load()?;

    match &config.default_notify {
        Some(target) => {
            println!("\nSending test notification...");

            let payload = NotificationPayload {
                watch_id: "test".to_string(),
                watch_name: "Test Notification".to_string(),
                url: "https://example.com/test-page".to_string(),
                old_content: "Old content".to_string(),
                new_content: "New content with changes".to_string(),
                diff: "[-Old content][+New content] with changes".to_string(),
                smart_summary: Some("Content changed".to_string()),
                agent_title: Some("Test: Content Updated".to_string()),
                agent_bullets: Some(vec![
                    "This is a test notification".to_string(),
                    "Your kto setup is working".to_string(),
                ]),
                agent_summary: None,
                agent_analysis: None,
                agent_error: None,
                detected_at: Utc::now(),
            };

            match send_notification(target, &payload) {
                Ok(()) => println!("  Test notification sent successfully!"),
                Err(e) => println!("  Failed to send notification: {}", e),
            }
        }
        None => {
            println!("\nNo notification target configured.");
            println!("Run `kto notify set` to configure notifications.");
        }
    }

    Ok(())
}

/// Interactive notification setup prompt
pub fn prompt_notification_setup() -> Result<Option<NotifyTarget>> {
    let options = vec![
        "ntfy.sh (easy push notifications)",
        "Gotify (self-hosted)",
        "Slack webhook",
        "Discord webhook",
        "Custom command",
        "Skip for now",
    ];

    let choice = Select::new("Where should I notify you when changes are detected?", options)
        .prompt()
        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

    match choice {
        "ntfy.sh (easy push notifications)" => {
            let topic = Text::new("ntfy topic name:")
                .with_default("kto-alerts")
                .with_help_message("Get notifications at ntfy.sh/<topic> or via the ntfy app")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            println!("\n  Install the ntfy app and subscribe to: {}", topic);
            println!("  Or visit: https://ntfy.sh/{}", topic);

            Ok(Some(NotifyTarget::Ntfy { topic, server: None }))
        }
        "Gotify (self-hosted)" => {
            let server = Text::new("Gotify server URL:")
                .with_help_message("e.g., https://gotify.example.com")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            let token = Text::new("Gotify application token:")
                .with_help_message("Create an app in Gotify and copy its token")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            println!("\n  Notifications will be sent to: {}", server);

            Ok(Some(NotifyTarget::Gotify { server, token }))
        }
        "Slack webhook" => {
            let url = Text::new("Slack webhook URL:")
                .with_help_message("Create at: https://api.slack.com/messaging/webhooks")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            Ok(Some(NotifyTarget::Slack { webhook_url: url }))
        }
        "Discord webhook" => {
            let url = Text::new("Discord webhook URL:")
                .with_help_message("Server Settings > Integrations > Webhooks")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            Ok(Some(NotifyTarget::Discord { webhook_url: url }))
        }
        "Custom command" => {
            let cmd = Text::new("Command to run:")
                .with_help_message("e.g., notify-send 'kto' '$SUMMARY' or osascript -e 'display notification'")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            Ok(Some(NotifyTarget::Command { command: cmd }))
        }
        _ => Ok(None),
    }
}

/// Prompt the user to test their notification settings
fn prompt_test_notification(target: &NotifyTarget) {
    use inquire::Confirm;

    // Ask to test
    let test = Confirm::new("Send a test notification now?")
        .with_default(true)
        .prompt()
        .unwrap_or(false);

    if test {
        print!("  Sending test... ");
        let _ = io::stdout().flush();

        let payload = NotificationPayload {
            watch_id: "test".to_string(),
            watch_name: "Test".to_string(),
            url: "https://example.com".to_string(),
            old_content: String::new(),
            new_content: "Test notification".to_string(),
            diff: "This is a test notification from kto".to_string(),
            smart_summary: None,
            agent_title: Some("Test Notification".to_string()),
            agent_bullets: Some(vec!["kto is working correctly".to_string()]),
            agent_summary: None,
            agent_analysis: None,
            agent_error: None,
            detected_at: Utc::now(),
        };

        match send_notification(target, &payload) {
            Ok(_) => println!("{}", "Success!".green()),
            Err(e) => {
                println!("{}", "Failed".red());
                println!("  Error: {}", e);
                println!("\n  Check your settings and try again with `kto notify set`");
            }
        }
    }
}

/// Parse a notify string like "ntfy:topic" or "gotify:server:token" into a NotifyTarget
pub fn parse_notify_string(s: &str) -> Result<NotifyTarget> {
    let parts: Vec<&str> = s.splitn(4, ':').collect();
    let notify_type = parts[0].to_lowercase();

    match notify_type.as_str() {
        "ntfy" => {
            let topic = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("ntfy requires topic: --notify ntfy:mytopic".into())
            })?;
            Ok(NotifyTarget::Ntfy { topic: topic.to_string(), server: None })
        }
        "slack" => {
            let webhook = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("slack requires webhook URL: --notify slack:https://...".into())
            })?;
            Ok(NotifyTarget::Slack { webhook_url: webhook.to_string() })
        }
        "discord" => {
            let webhook = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("discord requires webhook URL: --notify discord:https://...".into())
            })?;
            Ok(NotifyTarget::Discord { webhook_url: webhook.to_string() })
        }
        "gotify" => {
            let server = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("gotify requires server and token: --notify gotify:https://server:token".into())
            })?;
            let token = parts.get(2).ok_or_else(|| {
                kto::KtoError::ConfigError("gotify requires token: --notify gotify:https://server:token".into())
            })?;
            Ok(NotifyTarget::Gotify { server: server.to_string(), token: token.to_string() })
        }
        "telegram" => {
            let bot_token = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("telegram requires token and chat: --notify telegram:BOT_TOKEN:CHAT_ID".into())
            })?;
            let chat_id = parts.get(2).ok_or_else(|| {
                kto::KtoError::ConfigError("telegram requires chat_id: --notify telegram:BOT_TOKEN:CHAT_ID".into())
            })?;
            Ok(NotifyTarget::Telegram { bot_token: bot_token.to_string(), chat_id: chat_id.to_string() })
        }
        "pushover" => {
            let user_key = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("pushover requires user and token: --notify pushover:USER_KEY:API_TOKEN".into())
            })?;
            let api_token = parts.get(2).ok_or_else(|| {
                kto::KtoError::ConfigError("pushover requires api_token: --notify pushover:USER_KEY:API_TOKEN".into())
            })?;
            Ok(NotifyTarget::Pushover { user_key: user_key.to_string(), api_token: api_token.to_string() })
        }
        "matrix" => {
            let homeserver = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("matrix requires server, room, token: --notify matrix:SERVER:ROOM:TOKEN".into())
            })?;
            let room_id = parts.get(2).ok_or_else(|| {
                kto::KtoError::ConfigError("matrix requires room_id: --notify matrix:SERVER:ROOM:TOKEN".into())
            })?;
            let access_token = parts.get(3).ok_or_else(|| {
                kto::KtoError::ConfigError("matrix requires access_token: --notify matrix:SERVER:ROOM:TOKEN".into())
            })?;
            Ok(NotifyTarget::Matrix {
                homeserver: homeserver.to_string(),
                room_id: room_id.to_string(),
                access_token: access_token.to_string(),
            })
        }
        "command" | "cmd" => {
            let cmd = parts.get(1).ok_or_else(|| {
                kto::KtoError::ConfigError("command requires a command: --notify command:my-script".into())
            })?;
            Ok(NotifyTarget::Command { command: cmd.to_string() })
        }
        _ => Err(kto::KtoError::ConfigError(format!(
            "Unknown notification type '{}'. Use: ntfy, slack, discord, gotify, telegram, pushover, matrix, command", notify_type
        )))
    }
}

/// Get a human-readable description of a notification target
pub fn describe_notify_target(target: &NotifyTarget) -> String {
    match target {
        NotifyTarget::Ntfy { topic, server } => {
            let host = server.as_deref().unwrap_or("ntfy.sh");
            format!("ntfy ({}/{})", host, topic)
        }
        NotifyTarget::Slack { webhook_url } => {
            format!("Slack ({}...)", &webhook_url[..50.min(webhook_url.len())])
        }
        NotifyTarget::Discord { webhook_url } => {
            format!("Discord ({}...)", &webhook_url[..50.min(webhook_url.len())])
        }
        NotifyTarget::Gotify { server, token: _ } => {
            format!("Gotify ({})", server)
        }
        NotifyTarget::Command { command } => {
            format!("Command: {}", command)
        }
        NotifyTarget::Telegram { chat_id, .. } => {
            format!("Telegram (chat: {})", chat_id)
        }
        NotifyTarget::Pushover { user_key, .. } => {
            format!("Pushover ({}...)", &user_key[..8.min(user_key.len())])
        }
        NotifyTarget::Email { to, .. } => {
            format!("Email ({})", to)
        }
        NotifyTarget::Matrix { room_id, .. } => {
            format!("Matrix ({})", room_id)
        }
    }
}
