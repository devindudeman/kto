//! Watch management commands: new, list, show, edit, delete, pause, resume

use std::thread;

use chrono::Utc;
use colored::Colorize;
use inquire::{Confirm, Select, Text};
use uuid::Uuid;

use kto::agent::{self, EnhancedSetupSuggestion};
use kto::config::Config;
use kto::db::Database;
use kto::extract;
use kto::fetch::{self, check_playwright, PageContent, PlaywrightStatus};
use kto::normalize::{hash_content, normalize};
use kto::watch::{AgentConfig, Engine, Extraction, Snapshot, Watch};
use kto::error::Result;

use crate::utils::{extract_url, format_interval, get_clipboard_content, parse_interval_str, truncate_str};
use super::prompt_notification_setup;

/// Confidence threshold below which we show low-confidence UI
const CONFIDENCE_THRESHOLD: f32 = 0.7;

/// Create a new watch
pub fn cmd_new(
    description: Option<String>,
    name_override: Option<String>,
    interval_str: String,
    use_js: bool,
    use_rss: bool,
    use_shell: bool,
    use_agent: bool,
    agent_instructions: Option<String>,
    selector: Option<String>,
    clipboard: bool,
    tags: Vec<String>,
    use_profile: bool,
    yes: bool,
) -> Result<()> {
    let db = Database::open()?;

    // Parse interval (supports 30s, 5m, 2h, 1d, 1w formats)
    let interval = parse_interval_str(&interval_str)?;

    // --yes requires a description
    if yes && description.is_none() && !clipboard {
        return Err(kto::KtoError::ConfigError(
            "--yes requires a description argument or --clipboard".into()
        ));
    }

    // Determine if we're in interactive mode (--yes disables interactivity)
    let interactive = !yes && name_override.is_none() && atty::is(atty::Stream::Stdin);

    // Get the description/URL from user or clipboard
    let input = if clipboard {
        // Try to read from clipboard
        match get_clipboard_content() {
            Some(content) => {
                println!("  Read from clipboard: {}", truncate_str(&content, 60));
                content
            }
            None => {
                return Err(kto::KtoError::ConfigError(
                    "Could not read from clipboard. Make sure you have content copied.".into()
                ));
            }
        }
    } else {
        match description {
            Some(d) => d,
            None if interactive => {
                Text::new("What do you want to watch?")
                    .with_help_message("Enter a URL and optionally describe what to watch for")
                    .prompt()
                    .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?
            }
            None => {
                return Err(kto::KtoError::ConfigError(
                    "URL required. Usage: kto new <URL> --name <NAME>".into()
                ));
            }
        }
    };

    // Handle shell command case - input is the command, not a URL
    if use_shell {
        let command = input.trim().to_string();
        let name = name_override.unwrap_or_else(|| {
            // Generate name from command (first word or truncated)
            let first_word = command.split_whitespace().next().unwrap_or("shell");
            format!("shell:{}", first_word)
        });

        // Execute command to get initial content
        println!("\n  Executing: {}", command);
        let content = fetch::fetch("", Engine::Shell { command: command.clone() }, &std::collections::HashMap::new())?;
        let extracted = content.text.clone().unwrap_or_default();

        if extracted.is_empty() {
            println!("  Warning: Command produced no output.");
        } else {
            println!("  Got {} bytes of output.", extracted.len());
        }

        // Create watch with shell engine
        let mut watch = Watch::new(name.clone(), format!("shell://{}", command));
        watch.interval_secs = interval.max(10);
        watch.engine = Engine::Shell { command };
        watch.extraction = Extraction::Full;
        watch.tags = tags;

        // Configure agent if requested
        if use_agent {
            watch.agent_config = Some(AgentConfig {
                enabled: true,
                prompt_template: None,
                instructions: agent_instructions,
            });
        }

        let db = Database::open()?;
        db.insert_watch(&watch)?;

        // Create initial snapshot
        let normalized = normalize(&extracted, &watch.normalization);
        let hash = hash_content(&normalized);

        let snapshot = Snapshot {
            id: Uuid::new_v4(),
            watch_id: watch.id,
            fetched_at: Utc::now(),
            raw_html: None, // No HTML for shell commands
            extracted: normalized,
            content_hash: hash.clone(),
        };
        db.insert_snapshot(&snapshot)?;

        println!("\n  Created shell watch \"{}\"", name);
        println!("  Initial hash: {}", &hash[..8]);
        if watch.agent_config.is_some() {
            println!("  AI Agent: enabled");
        }
        if !watch.tags.is_empty() {
            println!("  Tags: {}", watch.tags.join(", "));
        }
        println!("  Checking every {}", format_interval(watch.interval_secs));
        println!("\n  Run `kto daemon` to start monitoring.");

        return Ok(());
    }

    // Try to extract URL from input
    let url = extract_url(&input).ok_or_else(|| {
        kto::KtoError::ConfigError("Could not find a valid URL in your input".into())
    })?;

    // Detect if user expressed intent (what to watch for)
    let has_intent = input.contains(" for ") || input.contains(" when ") || input.contains(" if ")
        || input.contains("watch for") || input.contains("notify me") || input.contains("alert")
        || input.contains("price") || input.contains("stock") || input.contains("available")
        || input.contains("back in") || input.contains("drop");

    // Check if Claude CLI is available for enhanced wizard
    let claude_available = agent::claude_version().is_some();

    // Use enhanced wizard flow when intent detected and Claude available
    // Works in both interactive and --yes mode (auto-accepts in --yes mode)
    let use_enhanced_wizard = has_intent && claude_available && !use_agent && !use_rss && !use_shell;

    // Enhanced wizard flow with dual fetch and smart analysis
    let (engine, content, extracted, title, enhanced_suggestion) = if use_enhanced_wizard {
        println!("\n  Analyzing {}...", url);

        // Perform dual fetch: HTTP and Playwright in parallel
        let (http_content, js_content) = dual_fetch(&url)?;

        // Extract content from both fetches
        let http_extracted = http_content.as_ref()
            .and_then(|c| extract::extract(c, &Extraction::Auto).ok());
        let js_extracted = js_content.as_ref()
            .and_then(|c| extract::extract(c, &Extraction::Auto).ok());

        // Get title from whichever fetch succeeded
        let title = js_content.as_ref()
            .and_then(|c| extract::extract_title(&c.html))
            .or_else(|| http_content.as_ref().and_then(|c| extract::extract_title(&c.html)))
            .unwrap_or_else(|| "Untitled".to_string());

        // Call enhanced AI analysis with both content versions
        println!("  Analyzing with AI (dual fetch)...");
        let suggestion = match agent::analyze_for_setup_v2(
            &input,
            http_extracted.as_deref(),
            js_extracted.as_deref(),
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  AI analysis failed: {} (using fallback)", e);
                EnhancedSetupSuggestion::fallback(&url, &input)
            }
        };

        // Determine which content/engine to use based on AI recommendation
        let (final_engine, final_content) = if suggestion.needs_js && js_content.is_some() {
            (Engine::Playwright, js_content.unwrap())
        } else if http_content.is_some() {
            (Engine::Http, http_content.unwrap())
        } else if js_content.is_some() {
            (Engine::Playwright, js_content.unwrap())
        } else {
            return Err(kto::KtoError::ConfigError("Both HTTP and JS fetches failed".into()));
        };

        let final_extracted = if suggestion.needs_js && js_extracted.is_some() {
            js_extracted.unwrap()
        } else {
            http_extracted.or(js_extracted).unwrap_or_default()
        };

        (final_engine, final_content, final_extracted, title, Some(suggestion))
    } else {
        // Traditional flow: determine engine first, then fetch

        // Determine engine to use - with smart probing in interactive mode
        let engine = if use_rss {
            // Validate RSS flag - warn if URL doesn't look like RSS
            if !fetch::detect_rss_url(&url) {
                eprintln!("  Note: URL doesn't look like an RSS feed, but --rss was specified.");
                eprintln!("  Will attempt to parse as RSS anyway.");
            }
            Engine::Rss
        } else if use_js {
            // Check if Playwright is available
            match check_playwright() {
                PlaywrightStatus::Ready => Engine::Playwright,
                status => {
                    eprintln!("  Warning: Playwright not ready. {}", status.install_instructions());
                    eprintln!("  Falling back to HTTP fetch.");
                    Engine::Http
                }
            }
        } else if interactive {
            // In interactive mode, probe the URL to suggest the best engine
            println!("\n  Analyzing {}...", url);
            match fetch::probe_url(&url) {
                Ok(probe) => {
                    // Show what we found
                    if let Some(ref msg) = probe.message {
                        println!("  {}", msg);
                    }

                    // If RSS detected in content or URL, offer to use it
                    if probe.suggested_engine == Engine::Rss {
                        println!("  Using RSS engine.");
                        Engine::Rss
                    }
                    // If RSS link found in page, offer to use it instead
                    else if let Some(ref rss_link) = probe.rss_url {
                        let use_rss = Confirm::new(&format!("RSS feed found at {}. Use that instead?", rss_link))
                            .with_default(true)
                            .prompt()
                            .unwrap_or(false);
                        if use_rss {
                            // Note: we'd need to change the URL too - for now just suggest
                            println!("  Tip: Run `kto new \"{}\" --rss` to watch the feed directly.", rss_link);
                            probe.suggested_engine
                        } else {
                            probe.suggested_engine
                        }
                    }
                    // If Playwright suggested
                    else if probe.suggested_engine == Engine::Playwright {
                        // Check if available
                        match check_playwright() {
                            PlaywrightStatus::Ready => {
                                let use_js = Confirm::new("Enable JavaScript rendering?")
                                    .with_default(true)
                                    .prompt()
                                    .unwrap_or(false);
                                if use_js { Engine::Playwright } else { Engine::Http }
                            }
                            status => {
                                println!("  JavaScript rendering recommended but not available.");
                                println!("  {}", status.install_instructions());
                                Engine::Http
                            }
                        }
                    } else {
                        probe.suggested_engine
                    }
                }
                Err(e) => {
                    // Probe failed, fall back to simple URL pattern detection
                    eprintln!("  Could not analyze page: {}", e);
                    if fetch::detect_rss_url(&url) {
                        println!("  URL looks like RSS feed, using RSS engine.");
                        Engine::Rss
                    } else {
                        Engine::Http
                    }
                }
            }
        } else if fetch::detect_rss_url(&url) {
            // Non-interactive: auto-detect RSS from URL pattern
            println!("\n  Detected RSS feed URL, using RSS engine.");
            Engine::Rss
        } else {
            Engine::Http
        };

        let engine_label = match &engine {
            Engine::Playwright => " (with JS)".to_string(),
            Engine::Rss => " (as RSS feed)".to_string(),
            Engine::Http => "".to_string(),
            Engine::Shell { .. } => " (shell command)".to_string(),
        };
        println!("  Fetching {}{}...", url, engine_label);

        // Fetch the page
        let content = fetch::fetch(&url, engine.clone(), &std::collections::HashMap::new())?;

        // Determine extraction strategy
        let extraction = match (&selector, &engine) {
            (Some(ref sel), _) => Extraction::Selector { selector: sel.clone() },
            (None, Engine::Rss) => Extraction::Rss,
            (None, _) => Extraction::Auto,
        };

        // Extract content
        let extracted = extract::extract(&content, &extraction)?;
        let title = extract::extract_title(&content.html)
            .unwrap_or_else(|| "Untitled".to_string());

        // Check if extraction got reasonable content, suggest JS if not
        if extracted.len() < 50 && !use_js {
            println!("\n  Warning: Very little content extracted ({} chars).", extracted.len());
            println!("  This page may require JavaScript rendering. Try: kto new <URL> --js");
        }

        (engine, content, extracted, title, None)
    };

    // Determine extraction strategy based on selector or engine
    let extraction = match (&selector, &engine) {
        (Some(ref sel), _) => Extraction::Selector { selector: sel.clone() },
        (None, Engine::Rss) => Extraction::Rss,
        (None, _) => Extraction::Auto,
    };

    // Apply enhanced AI suggestions or use traditional flow
    let (name, final_url, final_interval, final_agent_enabled, final_agent_instructions, final_extraction, final_engine) =
        if let Some(ref suggestion) = enhanced_suggestion {
            // Enhanced wizard flow with variant display
            let result = display_enhanced_confirmation(
                &url,
                suggestion,
                &extraction,
                engine.clone(),
                &name_override,
                interval,
                yes,
            )?;
            result
        } else {
            // Traditional flow - No enhanced AI suggestion
            if !yes {
                let preview: String = extracted.chars().take(200).collect();
                println!("\n  Title: {}", title);
                println!("  Content preview: {}...\n", preview.trim());
            }

            let name = match name_override {
                Some(n) => n,
                None if interactive => {
                    Text::new("Name for this watch?")
                        .with_default(&title)
                        .prompt()
                        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?
                }
                None => title.clone(),
            };

            // Intent-first flow: ask what changes matter BEFORE asking about AI
            let (agent_enabled, final_instructions) = if use_agent {
                // Explicit --agent flag always enables, use provided instructions
                (true, agent_instructions.clone())
            } else if interactive {
                // Interactive mode: ask about intent first
                println!();
                let intent = Text::new("What changes matter to you?")
                    .with_help_message("e.g., 'price drops', 'new articles', 'back in stock' (Enter to skip)")
                    .prompt()
                    .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

                if !intent.trim().is_empty() {
                    // User provided intent
                    if claude_available {
                        // Preview the intent and confirm
                        println!();
                        println!("  Intent captured: \"{}\"", intent.trim());

                        // Warn about potential shell escaping issues
                        if !intent.contains('$') && intent.chars().any(|c| c.is_ascii_digit()) {
                            println!("  Note: If you meant a price like $100, make sure the '$' is included.");
                        }

                        println!("  AI will filter changes based on this intent.");
                        (true, Some(intent.trim().to_string()))
                    } else {
                        // No Claude CLI - warn user
                        println!("  Warning: Claude CLI not found. Notifications will be basic.");
                        println!("  Install: curl -fsSL https://claude.ai/install.sh | bash");
                        (false, None)
                    }
                } else {
                    // User skipped intent - don't enable AI
                    (false, None)
                }
            } else {
                // Non-interactive mode: require explicit --agent flag
                (false, agent_instructions.clone())
            };

            (name, url.clone(), interval, agent_enabled, final_instructions, extraction.clone(), engine)
        };

    // Shell safety: warn if instructions contain $ which may have been mangled by bash
    if let Some(ref instructions) = final_agent_instructions {
        if instructions.contains('$') {
            println!("  Note: Instructions contain '$' - if using prices, this looks correct.");
        } else if instructions.chars().any(|c| c.is_ascii_digit()) {
            // Check if there's a number that might have lost its $ prefix
            let has_bare_number = instructions.split_whitespace().any(|word| {
                word.chars().all(|c| c.is_ascii_digit() || c == '.')
                    && word.parse::<f64>().is_ok()
            });
            if has_bare_number && !instructions.contains('$') {
                println!("  Warning: Instructions contain numbers without '$' symbol.");
                println!("  If you meant a price (e.g., $170), the '$' may have been");
                println!("  eaten by bash. Use single quotes: --agent-instructions 'price < $170'");
            }
        }
    }

    // Create watch with final options (enforce minimum interval)
    let mut watch = Watch::new(name.clone(), final_url.clone());
    watch.interval_secs = final_interval.max(10);
    watch.engine = final_engine;
    watch.extraction = final_extraction;
    watch.tags = tags;
    watch.use_profile = use_profile;

    // Configure agent
    if final_agent_enabled {
        watch.agent_config = Some(AgentConfig {
            enabled: true,
            prompt_template: None,
            instructions: final_agent_instructions,
        });
    }

    db.insert_watch(&watch)?;

    // Create initial snapshot
    let normalized = normalize(&extracted, &watch.normalization);
    let hash = hash_content(&normalized);

    let snapshot = Snapshot {
        id: Uuid::new_v4(),
        watch_id: watch.id,
        fetched_at: Utc::now(),
        raw_html: Some(zstd::encode_all(content.html.as_bytes(), 3)?),
        extracted: normalized,
        content_hash: hash.clone(),
    };
    db.insert_snapshot(&snapshot)?;

    println!("\n  Created watch \"{}\"", name);
    println!("  Initial hash: {}", &hash[..8]);
    println!("  Engine: {:?}", watch.engine);
    if watch.agent_config.is_some() {
        println!("  AI Agent: enabled");
    }
    if watch.use_profile {
        println!("  Profile: enabled");
    }
    if !watch.tags.is_empty() {
        println!("  Tags: {}", watch.tags.join(", "));
    }
    println!("  Checking every {}", format_interval(watch.interval_secs));

    // Prompt for notification setup if not configured and interactive (skip with --yes)
    let mut config = Config::load()?;
    if config.default_notify.is_none() && interactive && !yes {
        println!();
        if let Some(target) = prompt_notification_setup()? {
            config.default_notify = Some(target);
            config.save()?;
            println!("  Notification settings saved.");
        }
    }

    println!("\n  Run `kto daemon` to start monitoring.");

    Ok(())
}

/// List all watches
pub fn cmd_list(verbose: bool, tag_filter: Option<String>, json: bool) -> Result<()> {
    let db = Database::open()?;
    let mut watches = db.list_watches()?;

    // Filter by tag if specified
    if let Some(ref tag) = tag_filter {
        watches.retain(|w| w.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)));
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&watches)?);
        return Ok(());
    }

    if watches.is_empty() {
        if tag_filter.is_some() {
            println!("No watches found with tag '{}'.", tag_filter.unwrap());
        } else {
            println!("No watches configured. Run `kto new` to create one.");
        }
        return Ok(());
    }

    // Check if terminal supports colors
    let use_color = atty::is(atty::Stream::Stdout);

    println!("\nWatches:\n");

    if verbose {
        for watch in watches {
            let status = if watch.enabled {
                if use_color { "active".green().to_string() } else { "active".to_string() }
            } else {
                if use_color { "paused".yellow().to_string() } else { "paused".to_string() }
            };

            println!("  {} ({})", watch.name.bold(), &watch.id.to_string()[..8]);
            println!("    URL:      {}", watch.url);
            println!("    Status:   {}, every {}", status, format_interval(watch.interval_secs));
            println!("    Engine:   {:?}", watch.engine);
            if watch.agent_config.is_some() {
                println!("    AI Agent: enabled");
            }
            if !watch.tags.is_empty() {
                println!("    Tags:     {}", watch.tags.join(", "));
            }
            println!();
        }
    } else {
        // Calculate max widths for alignment
        let max_name_len = watches.iter().map(|w| w.name.len()).max().unwrap_or(20).min(30);

        for watch in watches {
            // Status indicator with color
            let status_indicator = if watch.enabled {
                if use_color { "●".green().to_string() } else { "[active]".to_string() }
            } else {
                if use_color { "○".yellow().to_string() } else { "[paused]".to_string() }
            };

            // Engine badge (RSS)
            let engine_badge = if watch.engine == Engine::Rss {
                if use_color { " RSS".magenta().to_string() } else { " [RSS]".to_string() }
            } else {
                "".to_string()
            };

            // AI badge
            let ai_badge = if watch.agent_config.is_some() {
                if use_color { " AI".cyan().to_string() } else { " [AI]".to_string() }
            } else {
                "".to_string()
            };

            // Truncate name if too long
            let name = truncate_str(&watch.name, max_name_len);
            let padded_name = format!("{:width$}", name, width = max_name_len);

            // Truncate URL if too long
            let url = truncate_str(&watch.url, 50);

            let interval = format_interval(watch.interval_secs);

            println!("  {} {}{}{} {} ({})",
                     status_indicator,
                     if use_color { padded_name.bold().to_string() } else { padded_name },
                     engine_badge,
                     ai_badge,
                     url.dimmed(),
                     interval);
        }
    }

    println!();
    Ok(())
}

/// Show details of a specific watch
pub fn cmd_show(id_or_name: &str, json: bool) -> Result<()> {
    let db = Database::open()?;
    let watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    // Show recent changes
    let changes = db.get_recent_changes(&watch.id, 5)?;

    if json {
        let output = serde_json::json!({
            "watch": watch,
            "recent_changes": changes
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("\nWatch: {}\n", watch.name);
    println!("  ID:        {}", watch.id);
    println!("  URL:       {}", watch.url);
    println!("  Status:    {}", if watch.enabled { "active" } else { "paused" });
    println!("  Interval:  {}", format_interval(watch.interval_secs));
    println!("  Engine:    {:?}", watch.engine);
    if let Some(ref agent_config) = watch.agent_config {
        println!("  AI Agent:  {}", if agent_config.enabled { "enabled" } else { "disabled" });
        if let Some(ref instructions) = agent_config.instructions {
            println!("  Instructions: {}", instructions);
        }
    }
    if watch.use_profile {
        println!("  Profile:   enabled");
    }
    println!("  Created:   {}", watch.created_at.format("%Y-%m-%d %H:%M"));

    if !changes.is_empty() {
        println!("\n  Recent changes:");
        for change in changes {
            let notified = if change.notified { "notified" } else { "not notified" };
            println!("    {} - {}", change.detected_at.format("%Y-%m-%d %H:%M"), notified);
        }
    }

    Ok(())
}

/// Edit a watch
pub fn cmd_edit(
    id_or_name: &str,
    new_name: Option<String>,
    new_interval: Option<String>,
    new_enabled: Option<bool>,
    new_agent: Option<bool>,
    new_agent_instructions: Option<String>,
    new_selector: Option<String>,
    new_notify: Option<String>,
    new_use_profile: Option<bool>,
) -> Result<()> {
    use inquire::Select;

    let db = Database::open()?;
    let mut watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    let has_flags = new_name.is_some() || new_interval.is_some() || new_enabled.is_some()
        || new_agent.is_some() || new_agent_instructions.is_some() || new_selector.is_some()
        || new_notify.is_some() || new_use_profile.is_some();

    if has_flags {
        // Flag-based editing (non-interactive)
        let mut changes = Vec::new();

        if let Some(name) = new_name {
            watch.name = name.clone();
            changes.push(format!("name -> {}", name));
        }

        if let Some(ref interval_str) = new_interval {
            let interval = parse_interval_str(interval_str)?;
            watch.interval_secs = interval;
            changes.push(format!("interval -> {}", format_interval(interval)));
        }

        if let Some(enabled) = new_enabled {
            watch.enabled = enabled;
            changes.push(format!("enabled -> {}", enabled));
        }

        if let Some(agent) = new_agent {
            if agent {
                if watch.agent_config.is_none() {
                    watch.agent_config = Some(AgentConfig {
                        enabled: true,
                        prompt_template: None,
                        instructions: None,
                    });
                } else if let Some(ref mut config) = watch.agent_config {
                    config.enabled = true;
                }
                changes.push("agent -> enabled".to_string());
            } else {
                if let Some(ref mut config) = watch.agent_config {
                    config.enabled = false;
                }
                changes.push("agent -> disabled".to_string());
            }
        }

        if let Some(instructions) = new_agent_instructions {
            if watch.agent_config.is_none() {
                watch.agent_config = Some(AgentConfig {
                    enabled: true,
                    prompt_template: None,
                    instructions: Some(instructions.clone()),
                });
            } else if let Some(ref mut config) = watch.agent_config {
                config.instructions = Some(instructions.clone());
            }
            changes.push(format!("agent_instructions -> {}", instructions));
        }

        if let Some(selector) = new_selector {
            watch.extraction = Extraction::Selector { selector: selector.clone() };
            changes.push(format!("selector -> {}", selector));
        }

        if let Some(notify_str) = new_notify {
            if notify_str.to_lowercase() == "none" || notify_str.to_lowercase() == "clear" {
                watch.notify_target = None;
                changes.push("notify -> cleared (will use global default)".to_string());
            } else {
                // Parse the notify string (format: "type:value" or "type:value:value2")
                let target = super::parse_notify_string(&notify_str)?;
                let description = super::describe_notify_target(&target);
                watch.notify_target = Some(target);
                changes.push(format!("notify -> {}", description));
            }
        }

        if let Some(profile) = new_use_profile {
            watch.use_profile = profile;
            changes.push(format!("use_profile -> {}", profile));
        }

        db.update_watch(&watch)?;

        println!("\nUpdated watch '{}':", watch.name);
        for change in changes {
            println!("  {}", change);
        }
    } else if atty::is(atty::Stream::Stdin) {
        // Interactive editing
        println!("\nEditing watch: {}\n", watch.name);
        println!("  Current settings:");
        println!("    Name:     {}", watch.name);
        println!("    URL:      {}", watch.url);
        println!("    Interval: {}", format_interval(watch.interval_secs));
        println!("    Status:   {}", if watch.enabled { "active" } else { "paused" });
        println!("    Engine:   {:?}", watch.engine);
        if let Some(ref config) = watch.agent_config {
            println!("    AI Agent: {}", if config.enabled { "enabled" } else { "disabled" });
            if let Some(ref inst) = config.instructions {
                println!("    Instructions: {}", inst);
            }
        } else {
            println!("    AI Agent: not configured");
        }
        println!();

        loop {
            let options = vec![
                "Change name",
                "Change interval",
                "Toggle pause/resume",
                "Toggle AI agent",
                "Set agent instructions",
                "Done",
            ];

            let choice = Select::new("What would you like to change?", options)
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            match choice {
                "Change name" => {
                    let new = Text::new("New name:")
                        .with_default(&watch.name)
                        .prompt()
                        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;
                    watch.name = new;
                    println!("  Name updated.");
                }
                "Change interval" => {
                    let current = format_interval(watch.interval_secs);
                    let new = Text::new("New interval (e.g., 5m, 1h, 30s):")
                        .with_default(&current)
                        .prompt()
                        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

                    if let Ok(secs) = parse_interval_str(&new) {
                        watch.interval_secs = secs;
                        println!("  Interval updated to {}.", format_interval(secs));
                    } else {
                        println!("  Invalid interval format. Use 30s, 5m, 1h, etc.");
                    }
                }
                "Toggle pause/resume" => {
                    watch.enabled = !watch.enabled;
                    println!("  Watch {}.", if watch.enabled { "resumed" } else { "paused" });
                }
                "Toggle AI agent" => {
                    if let Some(ref mut config) = watch.agent_config {
                        config.enabled = !config.enabled;
                        println!("  AI agent {}.", if config.enabled { "enabled" } else { "disabled" });
                    } else {
                        watch.agent_config = Some(AgentConfig {
                            enabled: true,
                            prompt_template: None,
                            instructions: None,
                        });
                        println!("  AI agent enabled.");
                    }
                }
                "Set agent instructions" => {
                    let current = watch.agent_config.as_ref()
                        .and_then(|c| c.instructions.as_deref())
                        .unwrap_or("");
                    let new = Text::new("Agent instructions:")
                        .with_default(current)
                        .with_help_message("What should the AI focus on when analyzing changes?")
                        .prompt()
                        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

                    if watch.agent_config.is_none() {
                        watch.agent_config = Some(AgentConfig {
                            enabled: true,
                            prompt_template: None,
                            instructions: if new.is_empty() { None } else { Some(new) },
                        });
                    } else if let Some(ref mut config) = watch.agent_config {
                        config.instructions = if new.is_empty() { None } else { Some(new) };
                    }
                    println!("  Instructions updated.");
                }
                "Done" => break,
                _ => {}
            }
        }

        db.update_watch(&watch)?;
        println!("\nWatch '{}' updated.", watch.name);
    } else {
        println!("No flags provided and not running interactively.");
        println!("Use flags like --interval 300 or run in a terminal for interactive mode.");
    }

    Ok(())
}

/// Pause a watch
pub fn cmd_pause(id_or_name: &str) -> Result<()> {
    let db = Database::open()?;
    let mut watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    watch.enabled = false;
    db.update_watch(&watch)?;

    println!("Paused watch: {}", watch.name);
    Ok(())
}

/// Resume a paused watch
pub fn cmd_resume(id_or_name: &str) -> Result<()> {
    let db = Database::open()?;
    let mut watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    watch.enabled = true;
    db.update_watch(&watch)?;

    println!("Resumed watch: {}", watch.name);
    Ok(())
}

/// Delete a watch
pub fn cmd_delete(id_or_name: &str, skip_confirm: bool) -> Result<()> {
    let db = Database::open()?;
    let watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    if !skip_confirm {
        let confirm = Confirm::new(&format!("Delete watch '{}'?", watch.name))
            .with_default(false)
            .prompt()
            .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

        if !confirm {
            println!("Cancelled.");
            return Ok(());
        }
    }

    db.delete_watch(&watch.id)?;
    println!("Deleted watch: {}", watch.name);
    Ok(())
}

// ============================================================================
// Enhanced Wizard Helper Functions
// ============================================================================

/// Perform parallel HTTP and Playwright fetches for dual content analysis
fn dual_fetch(url: &str) -> Result<(Option<PageContent>, Option<PageContent>)> {
    let url_owned = url.to_string();

    // Start HTTP fetch in a thread
    let url_http = url_owned.clone();
    let http_handle = thread::spawn(move || {
        fetch::fetch(&url_http, Engine::Http, &std::collections::HashMap::new())
    });

    // Start Playwright fetch if available
    let playwright_available = check_playwright().is_ready();
    let js_handle = if playwright_available {
        let url_js = url_owned.clone();
        Some(thread::spawn(move || {
            fetch::fetch(&url_js, Engine::Playwright, &std::collections::HashMap::new())
        }))
    } else {
        None
    };

    // Wait for HTTP result
    let http_result = http_handle
        .join()
        .map_err(|_| kto::KtoError::ConfigError("HTTP fetch thread panicked".into()))?;
    let http_content = http_result.ok();

    // Wait for Playwright result if started
    let js_content = if let Some(handle) = js_handle {
        handle
            .join()
            .map_err(|_| kto::KtoError::ConfigError("Playwright fetch thread panicked".into()))?
            .ok()
    } else {
        None
    };

    // Report what we got
    let http_status = if http_content.is_some() { "✓" } else { "✗" };
    let js_status = if js_content.is_some() {
        "✓"
    } else if playwright_available {
        "✗"
    } else {
        "–"
    };
    println!("  Fetched: HTTP {} | JS {}", http_status, js_status);

    Ok((http_content, js_content))
}

/// Display enhanced confirmation UI with variants and current status
fn display_enhanced_confirmation(
    url: &str,
    suggestion: &EnhancedSetupSuggestion,
    default_extraction: &Extraction,
    default_engine: Engine,
    name_override: &Option<String>,
    _default_interval: u64,
    yes: bool,
) -> Result<(String, String, u64, bool, Option<String>, Extraction, Engine)> {
    // Check if we need to show low-confidence UI
    let low_confidence = suggestion.confidence < CONFIDENCE_THRESHOLD;

    if !yes {
        // Display analysis results
        println!();
        println!("  {}", "Analysis Results".bold().underline());
        println!();

        // Current status
        if let Some(ref status) = suggestion.current_status {
            println!("  Status:  {}", status.cyan());
        }

        // Engine recommendation
        let engine_text = if suggestion.needs_js {
            let reason = suggestion.js_reason.as_ref().map(|r| format!(" ({})", r)).unwrap_or_default();
            format!("{}{}", "JavaScript required".yellow(), reason)
        } else {
            "HTTP".to_string()
        };
        println!("  Engine:  {}", engine_text);

        // Detected variants (limit to 5 for display)
        if !suggestion.variants.is_empty() {
            println!();
            let more = if suggestion.variants.len() > 5 {
                format!(" (+{} more)", suggestion.variants.len() - 5)
            } else {
                String::new()
            };
            println!("  Variants:{}", more);
            for (i, variant) in suggestion.variants.iter().take(5).enumerate() {
                let status_str = variant.status.as_deref().unwrap_or("?");
                let is_match = suggestion.intent_match.as_ref().map(|m| m.variant_index == i).unwrap_or(false);
                let marker = if is_match { " ← intent".yellow().to_string() } else { "".to_string() };
                println!("    {}. {} - {}{}", i + 1, variant.name, status_str, marker);
            }
        }

        // Recommended setup
        println!();
        println!("  Suggested:");
        println!("    Name:     {}", suggestion.name);
        println!("    Interval: {}", format_interval(suggestion.interval_secs));
        if let Some(ref instructions) = suggestion.agent_instructions {
            let display_instructions = truncate_str(instructions, 60);
            println!("    AI:       \"{}\"", display_instructions);
        }

        // Show uncertainty reasons if low confidence
        if low_confidence && !suggestion.uncertainty_reasons.is_empty() {
            println!();
            println!("  {} Low confidence ({:.0}%):", "⚠".yellow(), suggestion.confidence * 100.0);
            for reason in &suggestion.uncertainty_reasons {
                println!("    • {}", reason);
            }
        }
        println!();
    }

    // Determine final URL (with variant if matched)
    let final_url = if let Some(ref intent_match) = suggestion.intent_match {
        if let Some(variant) = suggestion.variants.get(intent_match.variant_index) {
            if let Some(ref url_hint) = variant.url_hint {
                construct_variant_url(url, url_hint)
            } else {
                url.to_string()
            }
        } else {
            url.to_string()
        }
    } else {
        url.to_string()
    };

    // Show variant URL if different
    if final_url != url && !yes {
        println!("  Using variant URL: {}", final_url.cyan());
        println!();
    }

    // User confirmation or customization
    if yes {
        // Auto-accept with --yes
        let name = name_override.clone().unwrap_or_else(|| suggestion.name.clone());
        let engine = if suggestion.needs_js { Engine::Playwright } else { default_engine };
        let extraction = suggestion.selector_hint.as_ref()
            .map(|sel| Extraction::Selector { selector: sel.clone() })
            .unwrap_or_else(|| default_extraction.clone());

        return Ok((
            name,
            final_url,
            suggestion.interval_secs,
            suggestion.agent_enabled,
            suggestion.agent_instructions.clone(),
            extraction,
            engine,
        ));
    }

    // Offer choices: Create, Customize, Cancel
    let choices = if !suggestion.variants.is_empty() && suggestion.variants.len() > 1 {
        vec!["Create Watch", "Select Different Variant", "Customize", "Cancel"]
    } else {
        vec!["Create Watch", "Customize", "Cancel"]
    };

    let choice = Select::new("What would you like to do?", choices)
        .prompt()
        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

    match choice {
        "Create Watch" => {
            let name = name_override.clone().unwrap_or_else(|| suggestion.name.clone());
            let engine = if suggestion.needs_js { Engine::Playwright } else { default_engine };
            let extraction = suggestion.selector_hint.as_ref()
                .map(|sel| Extraction::Selector { selector: sel.clone() })
                .unwrap_or_else(|| default_extraction.clone());

            Ok((
                name,
                final_url,
                suggestion.interval_secs,
                suggestion.agent_enabled,
                suggestion.agent_instructions.clone(),
                extraction,
                engine,
            ))
        }
        "Select Different Variant" => {
            // Let user select which variant to monitor
            let variant_names: Vec<String> = suggestion.variants.iter()
                .enumerate()
                .map(|(i, v)| {
                    let status = v.status.as_deref().unwrap_or("unknown");
                    format!("{}. {} - {}", i + 1, v.name, status)
                })
                .collect();

            let selected = Select::new("Which variant do you want to monitor?", variant_names)
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            // Parse the selection to get index
            let selected_idx = selected.split('.').next()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .map(|n| n - 1)
                .unwrap_or(0);

            let selected_variant = &suggestion.variants[selected_idx];

            // Construct URL with variant
            let variant_url = if let Some(ref hint) = selected_variant.url_hint {
                construct_variant_url(url, hint)
            } else {
                url.to_string()
            };

            // Update name to include variant
            let name = name_override.clone().unwrap_or_else(|| {
                format!("{} {}", suggestion.name, selected_variant.name)
            });

            // Update instructions to be variant-specific
            let instructions = Some(format!(
                "Monitor {} variant. Alert when status changes from '{}'",
                selected_variant.name,
                selected_variant.status.as_deref().unwrap_or("current")
            ));

            let engine = if suggestion.needs_js { Engine::Playwright } else { default_engine };
            let extraction = suggestion.selector_hint.as_ref()
                .map(|sel| Extraction::Selector { selector: sel.clone() })
                .unwrap_or_else(|| default_extraction.clone());

            println!("  Selected variant: {}", selected_variant.name);
            if variant_url != url {
                println!("  Using URL: {}", variant_url.cyan());
            }

            Ok((
                name,
                variant_url,
                suggestion.interval_secs,
                true,
                instructions,
                extraction,
                engine,
            ))
        }
        "Customize" => {
            // Manual customization flow
            let name = Text::new("Name for this watch?")
                .with_default(&name_override.clone().unwrap_or_else(|| suggestion.name.clone()))
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            let interval_str = Text::new("Check interval (e.g., 5m, 1h)?")
                .with_default(&format_interval(suggestion.interval_secs))
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            let custom_interval = crate::utils::parse_interval_str(&interval_str)
                .unwrap_or(suggestion.interval_secs);

            let use_ai = Confirm::new("Enable AI analysis?")
                .with_default(suggestion.agent_enabled)
                .prompt()
                .unwrap_or(suggestion.agent_enabled);

            let instructions = if use_ai {
                let inst = Text::new("What should AI watch for?")
                    .with_default(suggestion.agent_instructions.as_deref().unwrap_or(""))
                    .prompt()
                    .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;
                if inst.is_empty() { None } else { Some(inst) }
            } else {
                None
            };

            let use_js = if suggestion.needs_js {
                Confirm::new("Use JavaScript rendering (recommended)?")
                    .with_default(true)
                    .prompt()
                    .unwrap_or(true)
            } else {
                Confirm::new("Use JavaScript rendering?")
                    .with_default(false)
                    .prompt()
                    .unwrap_or(false)
            };

            let engine = if use_js { Engine::Playwright } else { Engine::Http };
            let extraction = suggestion.selector_hint.as_ref()
                .map(|sel| Extraction::Selector { selector: sel.clone() })
                .unwrap_or_else(|| default_extraction.clone());

            Ok((
                name,
                final_url,
                custom_interval,
                use_ai,
                instructions,
                extraction,
                engine,
            ))
        }
        "Cancel" | _ => {
            Err(kto::KtoError::ConfigError("Watch creation cancelled".into()))
        }
    }
}

/// Construct a URL with variant parameters
fn construct_variant_url(base_url: &str, url_hint: &str) -> String {
    // Parse the base URL
    if let Ok(mut parsed) = url::Url::parse(base_url) {
        // Check if url_hint is a full query param (contains =)
        if url_hint.contains('=') {
            // Split the hint into key=value pairs
            for param in url_hint.split('&') {
                if let Some((key, value)) = param.split_once('=') {
                    // Remove existing param with same key, add new one
                    let pairs: Vec<(String, String)> = parsed.query_pairs()
                        .filter(|(k, _)| k != key)
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect();

                    parsed.set_query(None);
                    for (k, v) in pairs {
                        parsed.query_pairs_mut().append_pair(&k, &v);
                    }
                    parsed.query_pairs_mut().append_pair(key, value);
                }
            }
        } else {
            // Just append as-is (might be a path segment or raw param)
            let query = parsed.query().map(|q| format!("{}&{}", q, url_hint))
                .unwrap_or_else(|| url_hint.to_string());
            parsed.set_query(Some(&query));
        }
        parsed.to_string()
    } else {
        // Fallback: just append
        if base_url.contains('?') {
            format!("{}&{}", base_url, url_hint)
        } else {
            format!("{}?{}", base_url, url_hint)
        }
    }
}
