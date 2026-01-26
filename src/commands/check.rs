//! Check and monitoring commands: test, run, daemon, watch (ephemeral), check_watch

use chrono::Utc;
use colored::Colorize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use kto::agent::{self, AgentContext};
use kto::config::Config;
use kto::db::Database;
use kto::diff;
use kto::extract;
use kto::fetch;
use kto::filter::{evaluate_filters, FilterContext};
use kto::interests::InterestProfile;
use kto::normalize::{hash_content, normalize};
use kto::notify::{send_notification, NotificationPayload};
use kto::watch::{Change, Engine, Extraction, Snapshot, Watch};
use kto::error::Result;

use crate::utils::{extract_domain, format_interval, parse_interval_str};

/// Test a watch (fetch now, show what would happen)
pub fn cmd_test(id_or_name: &str, json: bool) -> Result<()> {
    let db = Database::open()?;
    let watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    if !json {
        println!("\nTesting watch: {}\n", watch.name);
        println!("  Fetching {}...", watch.url);
    }

    // Fetch the page
    let content = fetch::fetch(&watch.url, watch.engine.clone(), &watch.headers)?;

    // Extract content
    let extracted = extract::extract(&content, &watch.extraction)?;
    let normalized = normalize(&extracted, &watch.normalization);
    let new_hash = hash_content(&normalized);

    // Compare with last snapshot
    let last = db.get_latest_snapshot(&watch.id)?;

    let (changed, old_hash, diff_size, filter_passed) = if let Some(ref last) = last {
        if new_hash != last.content_hash {
            let diff_result = diff::diff(&last.extracted, &normalized);
            let ctx = FilterContext {
                old_content: &last.extracted,
                new_content: &normalized,
                diff: &diff_result,
            };
            let fp = evaluate_filters(&watch.filters, &ctx);
            (true, Some(last.content_hash.clone()), diff_result.diff_size, fp)
        } else {
            (false, Some(last.content_hash.clone()), 0, false)
        }
    } else {
        (false, None, 0, false)
    };

    if json {
        let output = serde_json::json!({
            "watch_name": watch.name,
            "url": watch.url,
            "extracted_chars": normalized.len(),
            "new_hash": new_hash,
            "old_hash": old_hash,
            "changed": changed,
            "diff_size": diff_size,
            "filter_passed": filter_passed,
            "first_fetch": last.is_none()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("  Extracted {} characters", normalized.len());
    println!("  New hash: {}", &new_hash[..8]);

    if let Some(ref last) = last {
        println!("  Last hash: {}", &last.content_hash[..8]);

        if changed {
            println!("\n  CHANGE DETECTED!\n");

            let diff_result = diff::diff(&last.extracted, &normalized);
            println!("  Diff size: {} chars ({} additions, {} deletions)",
                     diff_result.diff_size, diff_result.additions, diff_result.deletions);

            // Show diff preview
            let preview: String = diff_result.diff_text.chars().take(300).collect();
            println!("\n  Diff preview:\n  {}", preview);

            println!("\n  Filters: {}", if filter_passed { "PASSED" } else { "BLOCKED" });
        } else {
            println!("\n  No change detected.");
        }
    } else {
        println!("\n  This is the first fetch (no previous snapshot to compare).");
    }

    Ok(())
}

/// Ephemeral watch mode - watch a URL in real-time without saving to database
pub fn cmd_watch(url: &str, interval: &str, selector: Option<String>, js: bool) -> Result<()> {
    use std::io::{self, Write};

    // Parse interval
    let interval_secs = parse_interval_str(interval)?;

    let engine = if js { Engine::Playwright } else { Engine::Http };
    let extraction = match selector {
        Some(s) => Extraction::Selector { selector: s },
        None => Extraction::Auto,
    };
    let normalization = kto::watch::Normalization::default();
    let headers = std::collections::HashMap::new();

    println!("\n{} {} every {}", "Watching".cyan().bold(), url, format_interval(interval_secs));
    println!("Press {} to stop\n", "Ctrl+C".yellow());

    let mut last_content: Option<String> = None;
    let mut last_hash: Option<String> = None;
    let mut check_count = 0;

    loop {
        check_count += 1;
        print!("[{}] Checking... ", check_count);
        let _ = io::stdout().flush();

        // Fetch and extract
        match fetch::fetch(url, engine.clone(), &headers) {
            Ok(content) => {
                match extract::extract(&content, &extraction) {
                    Ok(extracted) => {
                        let normalized = normalize(&extracted, &normalization);
                        let hash = hash_content(&normalized);

                        if let Some(ref prev_hash) = last_hash {
                            if hash != *prev_hash {
                                println!("{}", "CHANGED!".green().bold());

                                // Show diff
                                if let Some(ref prev_content) = last_content {
                                    let diff_result = diff::diff(prev_content, &normalized);
                                    println!("\n{}\n", diff_result.diff_text);
                                }
                            } else {
                                println!("{}", "no change".dimmed());
                            }
                        } else {
                            println!("{} ({} chars)", "baseline captured".blue(), normalized.len());
                        }

                        last_hash = Some(hash);
                        last_content = Some(normalized);
                    }
                    Err(e) => {
                        println!("{}: {}", "extraction error".red(), e);
                    }
                }
            }
            Err(e) => {
                println!("{}: {}", "fetch error".red(), e);
            }
        }

        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}

/// Preview what kto extracts from a URL (no database, just fetch and show)
pub fn cmd_preview(url: &str, selector: Option<String>, js: bool, full: bool, json_ld: bool, limit: usize) -> Result<()> {
    let engine = if js { Engine::Playwright } else { Engine::Http };
    let extraction = if json_ld {
        Extraction::JsonLd { types: None }
    } else if full {
        Extraction::Full
    } else if let Some(s) = selector {
        Extraction::Selector { selector: s }
    } else {
        Extraction::Auto
    };
    let headers = std::collections::HashMap::new();

    println!("\n{} {}", "Fetching".cyan().bold(), url);
    println!("  Engine: {}", if js { "playwright (JS)" } else { "http" });
    println!("  Extraction: {}\n", match &extraction {
        Extraction::Auto => "auto (main content)".to_string(),
        Extraction::Full => "full (entire body)".to_string(),
        Extraction::Selector { selector } => format!("selector ({})", selector),
        Extraction::JsonLd { .. } => "json-ld (structured data)".to_string(),
        _ => format!("{:?}", extraction),
    });

    // Fetch
    let content = fetch::fetch(url, engine, &headers)?;
    println!("  {} Fetched {} bytes of HTML", "✓".green(), content.html.len());

    // Extract
    let extracted = extract::extract(&content, &extraction)?;
    println!("  {} Extracted {} characters\n", "✓".green(), extracted.len());

    // Show content
    println!("{}", "─".repeat(60).dimmed());
    if extracted.len() > limit {
        println!("{}", &extracted[..limit]);
        println!("\n{}", format!("... truncated ({} chars total, showing {})", extracted.len(), limit).dimmed());
    } else {
        println!("{}", extracted);
    }
    println!("{}", "─".repeat(60).dimmed());

    // Show hash
    let hash = hash_content(&extracted);
    println!("\n  Hash: {}", &hash[..16]);

    Ok(())
}

/// Show change history for a watch
pub fn cmd_history(id_or_name: &str, limit: usize, json: bool) -> Result<()> {
    let db = Database::open()?;
    let watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    let changes = db.get_recent_changes(&watch.id, limit)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&changes)?);
        return Ok(());
    }

    if changes.is_empty() {
        println!("No changes recorded for '{}'.", watch.name);
        return Ok(());
    }

    println!("\nChange history for '{}':\n", watch.name);
    for change in changes {
        // Status indicators
        let status = if change.notified {
            "SENT".green().to_string()
        } else if !change.filter_passed {
            "FILTERED".yellow().to_string()
        } else {
            "SKIPPED".red().to_string()
        };

        println!("  {} | {} | {} chars",
                 change.detected_at.format("%Y-%m-%d %H:%M"),
                 status,
                 change.diff.len());

        // Show AI reasoning if available
        if let Some(ref resp) = change.agent_response {
            // Get notify decision
            let ai_notify = resp.get("notify").and_then(|v| v.as_bool()).unwrap_or(true);

            if let Some(title) = resp.get("title").and_then(|t| t.as_str()) {
                println!("    AI: {}", title.cyan());
            }

            // Show why AI filtered (if it did)
            if !ai_notify {
                if let Some(summary) = resp.get("summary").and_then(|s| s.as_str()) {
                    println!("    Reason: {}", summary.dimmed());
                }
            }
        } else if watch.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false) {
            // AI was enabled but didn't run
            println!("    AI: {}", "(no response)".dimmed());
        }
    }

    Ok(())
}

/// Check all due watches once (for cron)
pub fn cmd_run() -> Result<()> {
    let db = Database::open()?;
    let config = Config::load()?;
    let watches = db.list_watches()?;

    let enabled: Vec<_> = watches.into_iter().filter(|w| w.enabled).collect();

    if enabled.is_empty() {
        println!("No active watches.");
        return Ok(());
    }

    println!("Checking {} watches...\n", enabled.len());

    for watch in enabled {
        if let Err(e) = check_watch(&db, &config, &watch) {
            eprintln!("  [ERROR] {}: {}", watch.name, e);
        }
    }

    println!("\nDone.");
    Ok(())
}

/// Run continuously with internal scheduler
pub fn cmd_daemon() -> Result<()> {
    let db = Database::open()?;
    let config = Config::load()?;

    // Write PID file for daemon detection
    let pid_path = Config::data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("daemon.pid");
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&pid_path, std::process::id().to_string());

    // Set up Ctrl+C handler - also cleans up PID file
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let pid_path_clone = pid_path.clone();
    ctrlc::set_handler(move || {
        println!("\n\nShutting down...");
        // Clean up PID file
        let _ = std::fs::remove_file(&pid_path_clone);
        r.store(false, Ordering::SeqCst);
    }).map_err(|e| kto::KtoError::ConfigError(format!("Failed to set Ctrl+C handler: {}", e)))?;

    println!("\nkto daemon starting...\n");

    // Track when each watch is next due
    let mut next_check: HashMap<uuid::Uuid, Instant> = HashMap::new();

    // Initialize with staggered start times (jitter)
    let watches = db.list_watches()?;
    let enabled: Vec<_> = watches.into_iter().filter(|w| w.enabled).collect();

    if enabled.is_empty() {
        println!("No active watches. Add one with `kto new`.");
        return Ok(());
    }

    println!("Monitoring {} watches:\n", enabled.len());
    for (i, watch) in enabled.iter().enumerate() {
        // Stagger initial checks over first 30 seconds
        let jitter = Duration::from_millis((i as u64 * 5000) % 30000);
        next_check.insert(watch.id, Instant::now() + jitter);
        println!("  {} - every {}", watch.name, format_interval(watch.interval_secs));
    }
    println!("\nPress Ctrl+C to stop.\n");

    // Track when to reload watch configs from DB (every 15 seconds)
    let mut last_watch_reload = Instant::now();
    let watch_reload_interval = Duration::from_secs(15);
    let mut enabled = enabled;

    // Track last fetch time per domain for rate limiting
    let mut last_domain_fetch: HashMap<String, Instant> = HashMap::new();

    // Main loop
    while running.load(Ordering::SeqCst) {
        let now = Instant::now();

        // Reload watches periodically (every 15 seconds) to pick up config changes
        if now.duration_since(last_watch_reload) >= watch_reload_interval {
            let watches = db.list_watches()?;
            enabled = watches.into_iter().filter(|w| w.enabled).collect();
            last_watch_reload = now;
        }

        // Find watches that are due
        for watch in &enabled {
            let due_at = next_check.get(&watch.id).copied().unwrap_or(now);

            if now >= due_at {
                // Apply per-domain rate limiting
                if let Some(domain) = extract_domain(&watch.url) {
                    if let Some(&rate_limit) = config.rate_limits.get(&domain) {
                        // rate_limit is requests per second, so delay = 1/rate seconds
                        let min_delay = Duration::from_secs_f64(1.0 / rate_limit);
                        if let Some(&last_fetch) = last_domain_fetch.get(&domain) {
                            let elapsed = now.duration_since(last_fetch);
                            if elapsed < min_delay {
                                let wait = min_delay - elapsed;
                                std::thread::sleep(wait);
                            }
                        }
                    }
                    last_domain_fetch.insert(domain, Instant::now());
                }

                // Check this watch
                match check_watch(&db, &config, watch) {
                    Ok(()) => {}
                    Err(e) => eprintln!("  [ERROR] {}: {}", watch.name, e),
                }

                // Schedule next check with jitter (±10%)
                let jitter_range = (watch.interval_secs as f64 * 0.1) as u64;
                let jitter = if jitter_range > 0 {
                    (rand::random::<u64>() % (jitter_range * 2)) as i64 - jitter_range as i64
                } else {
                    0
                };
                let next = watch.interval_secs as i64 + jitter;
                let next_duration = Duration::from_secs(next.max(10) as u64);
                next_check.insert(watch.id, Instant::now() + next_duration);
            }
        }

        // Check due reminders
        if let Ok(due_reminders) = db.get_due_reminders() {
            for reminder in due_reminders {
                // Send notification
                let notify_target = reminder.notify_target.as_ref().or(config.default_notify.as_ref());
                if let Some(target) = notify_target {
                    let payload = NotificationPayload {
                        watch_id: reminder.id.to_string(),
                        watch_name: format!("Reminder: {}", reminder.name),
                        url: String::new(),
                        old_content: String::new(),
                        new_content: reminder.message.clone().unwrap_or_default(),
                        diff: String::new(),
                        smart_summary: None,
                        agent_title: Some("Reminder".to_string()),
                        agent_bullets: reminder.message.as_ref().map(|m| vec![m.clone()]),
                        agent_summary: Some(reminder.name.clone()),
                        agent_analysis: None,
                        agent_error: None,
                        detected_at: Utc::now(),
                    };

                    if send_notification(target, &payload).is_ok() {
                        println!("  [REMINDER] {}", reminder.name);
                    }
                }

                // Handle recurring vs one-shot
                if let Some(interval) = reminder.interval_secs {
                    // Recurring: advance from trigger_at to preserve original time-of-day
                    // Add intervals until we get a future time
                    let interval_duration = chrono::Duration::seconds(interval as i64);
                    let mut next_trigger = reminder.trigger_at + interval_duration;
                    let now = Utc::now();
                    while next_trigger <= now {
                        next_trigger = next_trigger + interval_duration;
                    }
                    let _ = db.update_reminder_trigger(&reminder.id, next_trigger);
                } else {
                    // One-shot: delete the reminder
                    let _ = db.delete_reminder(&reminder.id);
                }
            }
        }

        // Sleep for a short interval before checking again
        std::thread::sleep(Duration::from_secs(1));
    }

    println!("Daemon stopped.");
    Ok(())
}

/// Check a single watch for changes
pub fn check_watch(db: &Database, config: &Config, watch: &Watch) -> Result<()> {
    print!("  Checking {}...", watch.name);

    // Fetch the page
    let content = fetch::fetch(&watch.url, watch.engine.clone(), &watch.headers)?;

    // Extract and normalize
    let extracted = extract::extract(&content, &watch.extraction)?;

    // Runtime safety net: if extraction yields very little content and we're not
    // already using Full, try Full extraction as a fallback. This catches cases
    // where the page structure changed and the configured strategy no longer works.
    let extracted = if extracted.len() < 50 && !matches!(watch.extraction, Extraction::Full) {
        eprintln!("  [WARN] {} extraction yielded only {} chars, trying full",
                  watch.name, extracted.len());
        extract::extract(&content, &Extraction::Full).unwrap_or(extracted)
    } else {
        extracted
    };

    let normalized = normalize(&extracted, &watch.normalization);
    let new_hash = hash_content(&normalized);

    // Get last snapshot
    let last = db.get_latest_snapshot(&watch.id)?;

    // Create new snapshot
    let new_snapshot = Snapshot {
        id: Uuid::new_v4(),
        watch_id: watch.id,
        fetched_at: Utc::now(),
        raw_html: Some(zstd::encode_all(content.html.as_bytes(), 3)?),
        extracted: normalized.clone(),
        content_hash: new_hash.clone(),
    };
    db.insert_snapshot(&new_snapshot)?;

    // Cleanup old snapshots
    db.cleanup_snapshots(&watch.id, 50, 5)?;

    // Check for changes
    if let Some(old) = last {
        if new_hash != old.content_hash {
            println!(" CHANGE DETECTED");

            let diff_result = diff::diff(&old.extracted, &normalized);

            // Check filters
            let ctx = FilterContext {
                old_content: &old.extracted,
                new_content: &normalized,
                diff: &diff_result,
            };
            let filter_passed = evaluate_filters(&watch.filters, &ctx);

            // Create change record
            let mut change = Change {
                id: Uuid::new_v4(),
                watch_id: watch.id,
                detected_at: Utc::now(),
                old_snapshot_id: old.id,
                new_snapshot_id: new_snapshot.id,
                diff: diff_result.diff_text.clone(),
                filter_passed,
                agent_response: None,
                notified: false,
            };

            // Run AI agent only if explicitly enabled for this watch
            let should_run_ai = watch.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false);

            // Track AI error for notification
            let mut agent_error: Option<String> = None;

            // Run agent if enabled and filter passed
            if filter_passed && should_run_ai {
                let memory = db.get_agent_memory(&watch.id)?;
                let custom_instructions = watch.agent_config
                    .as_ref()
                    .and_then(|c| c.instructions.as_deref());

                // Load profile if use_profile is enabled for this watch
                let profile = if watch.use_profile {
                    InterestProfile::load().ok()
                } else {
                    None
                };

                // Load global memory for cross-watch learning
                let global_memory = db.get_global_memory().ok();

                let ctx = AgentContext {
                    old_content: &old.extracted,
                    new_content: &normalized,
                    diff: &diff_result.diff_text,
                    memory: &memory,
                    custom_instructions,
                    profile: profile.as_ref(),
                    global_memory: global_memory.as_ref(),
                };

                match agent::analyze_change(&ctx) {
                    Ok(resp) => {
                        change.agent_response = Some(serde_json::to_value(&resp)?);

                        // Update per-watch memory (truncate if over limit)
                        if let Some(mut new_memory) = resp.memory_update {
                            new_memory.truncate_to_limit();
                            db.update_agent_memory(&watch.id, &new_memory)?;
                        }

                        // Update global memory with observation if provided
                        if let Some(ref observation) = resp.global_observation {
                            if let Ok(mut global_mem) = db.get_global_memory() {
                                global_mem.add_observation(
                                    observation.clone(),
                                    watch.name.clone(),
                                    0.7, // Default confidence for AI observations
                                );
                                global_mem.apply_decay();
                                global_mem.truncate_to_limit();
                                let _ = db.update_global_memory(&global_mem);
                            }
                        }

                        // Agent can gate notifications
                        if !resp.notify {
                            change.filter_passed = false;
                        }
                    }
                    Err(e) => {
                        // Capture the error for the notification
                        let error_str = e.to_string();
                        eprintln!("  AI analysis failed: {}", error_str);
                        agent_error = Some(error_str);
                        // Don't gate notification - still send with fallback
                    }
                }
            }

            // Send notification if appropriate
            // Use per-watch notify target if set, otherwise fall back to global default
            if change.filter_passed {
                let notify_target = watch.notify_target.as_ref().or(config.default_notify.as_ref());
                if let Some(target) = notify_target {
                    // Extract new structured fields from agent response
                    let agent_title = change.agent_response.as_ref()
                        .and_then(|r| r.get("title"))
                        .and_then(|s| s.as_str())
                        .map(String::from);

                    let agent_bullets = change.agent_response.as_ref()
                        .and_then(|r| r.get("bullets"))
                        .and_then(|b| b.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect::<Vec<_>>());

                    let agent_summary = change.agent_response.as_ref()
                        .and_then(|r| r.get("summary"))
                        .and_then(|s| s.as_str())
                        .map(String::from);

                    let payload = NotificationPayload {
                        watch_id: watch.id.to_string(),
                        watch_name: watch.name.clone(),
                        url: watch.url.clone(),
                        old_content: old.extracted,
                        new_content: normalized,
                        diff: diff_result.diff_text.clone(),
                        smart_summary: diff_result.summary.clone(),
                        agent_title,
                        agent_bullets,
                        agent_summary,
                        agent_analysis: None,
                        agent_error: agent_error.clone(),
                        detected_at: Utc::now(),
                    };

                    if send_notification(target, &payload).is_ok() {
                        change.notified = true;
                    }
                }
            }

            db.insert_change(&change)?;
        } else {
            println!(" no change");
        }
    } else {
        println!(" first snapshot");
    }

    Ok(())
}
