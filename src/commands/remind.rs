//! Reminder commands: new, list, delete, pause, resume

use chrono::Utc;
use uuid::Uuid;

use kto::db::Database;
use kto::watch::Reminder;
use kto::error::Result;

use crate::utils::{format_interval, parse_duration, parse_time_to_next};

/// Create a new reminder
pub fn cmd_remind_new(
    message: String,
    in_duration: Option<String>,
    at_time: Option<String>,
    every: Option<String>,
    note: Option<String>,
) -> Result<()> {
    let db = Database::open()?;

    // Calculate trigger time
    let trigger_at = if let Some(ref duration_str) = in_duration {
        let secs = parse_duration(duration_str)
            .ok_or_else(|| kto::KtoError::ConfigError(format!("Invalid duration: {}", duration_str)))?;
        Utc::now() + chrono::Duration::seconds(secs as i64)
    } else if let Some(ref time_str) = at_time {
        parse_time_to_next(time_str)
            .ok_or_else(|| kto::KtoError::ConfigError(format!("Invalid time format: {} (use HH:MM)", time_str)))?
    } else {
        // Default to 5 minutes from now
        Utc::now() + chrono::Duration::minutes(5)
    };

    // Parse interval for recurring reminders
    let interval_secs = every.as_ref().and_then(|s| parse_duration(s));

    let reminder = Reminder {
        id: Uuid::new_v4(),
        name: message.clone(),
        message: note,
        trigger_at,
        interval_secs,
        enabled: true,
        notify_target: None,
        created_at: Utc::now(),
    };

    db.insert_reminder(&reminder)?;

    println!("\nReminder created!");
    println!("  Name: {}", reminder.name);
    let local_time: chrono::DateTime<chrono::Local> = reminder.trigger_at.into();
    println!("  Triggers: {}", local_time.format("%Y-%m-%d %H:%M:%S"));
    if let Some(secs) = interval_secs {
        println!("  Repeats: every {}", format_interval(secs));
    } else {
        println!("  Repeats: one-time");
    }

    Ok(())
}

/// List all reminders
pub fn cmd_remind_list(json: bool) -> Result<()> {
    let db = Database::open()?;
    let reminders = db.list_reminders()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&reminders)?);
        return Ok(());
    }

    if reminders.is_empty() {
        println!("\nNo reminders set.");
        println!("Create one with: kto remind new \"message\" --in 1h");
        return Ok(());
    }

    println!("\nReminders ({}):\n", reminders.len());
    for r in reminders {
        let status = if r.enabled { "active" } else { "paused" };
        let repeat = r.interval_secs
            .map(|s| format!("every {}", format_interval(s)))
            .unwrap_or_else(|| "one-time".to_string());
        let local_time: chrono::DateTime<chrono::Local> = r.trigger_at.into();
        let time_str = local_time.format("%Y-%m-%d %H:%M");

        println!("  {} [{}] {} | {}", r.name, status, time_str, repeat);
    }
    println!();

    Ok(())
}

/// Delete a reminder
pub fn cmd_remind_delete(id_or_name: String) -> Result<()> {
    let db = Database::open()?;

    let reminder = db.get_reminder(&id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.clone()))?;

    db.delete_reminder(&reminder.id)?;
    println!("\nDeleted reminder: {}", reminder.name);

    Ok(())
}

/// Pause a reminder
pub fn cmd_remind_pause(id_or_name: String) -> Result<()> {
    let db = Database::open()?;

    let reminder = db.get_reminder(&id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.clone()))?;

    db.set_reminder_enabled(&reminder.id, false)?;
    println!("\nPaused reminder: {}", reminder.name);

    Ok(())
}

/// Resume a paused reminder
pub fn cmd_remind_resume(id_or_name: String) -> Result<()> {
    let db = Database::open()?;

    let reminder = db.get_reminder(&id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.clone()))?;

    // If the trigger time is in the past and it's a recurring reminder,
    // advance to the next occurrence to avoid immediate firing
    let now = Utc::now();
    if reminder.trigger_at < now {
        if let Some(interval) = reminder.interval_secs {
            let interval_duration = chrono::Duration::seconds(interval as i64);
            let mut next_trigger = reminder.trigger_at;
            while next_trigger <= now {
                next_trigger = next_trigger + interval_duration;
            }
            db.update_reminder_trigger(&reminder.id, next_trigger)?;
            let local_time: chrono::DateTime<chrono::Local> = next_trigger.into();
            println!("\nResumed reminder: {}", reminder.name);
            println!("  Next trigger: {}", local_time.format("%Y-%m-%d %H:%M"));
        } else {
            // One-shot reminder that's past due - just enable it
            db.set_reminder_enabled(&reminder.id, true)?;
            println!("\nResumed reminder: {}", reminder.name);
            println!("  Warning: One-time reminder is past due and will fire immediately");
        }
    } else {
        db.set_reminder_enabled(&reminder.id, true)?;
        println!("\nResumed reminder: {}", reminder.name);
    }

    // Always ensure it's enabled
    db.set_reminder_enabled(&reminder.id, true)?;

    Ok(())
}
