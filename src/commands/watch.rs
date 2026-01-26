//! Watch management commands: new, list, show, edit, delete, pause, resume

use std::thread;

use chrono::Utc;
use colored::Colorize;
use inquire::{Confirm, Select, Text};
use uuid::Uuid;

use kto::agent::{self, DeepResearchResult, EnhancedSetupSuggestion, UrlDiscoveryResult};
use kto::config::Config;
use kto::db::Database;
use kto::extract;
use kto::fetch::{self, check_playwright, PageContent, PlaywrightStatus};
use kto::intent::ParsedIntent;
use kto::normalize::{hash_content, normalize};
use kto::transforms::{self, Intent, TransformMatch};
use kto::watch::{AgentConfig, Engine, Extraction, Snapshot, Watch};
use kto::error::Result;

use crate::utils::{extract_url, format_interval, get_clipboard_content, parse_interval_str, truncate_str};
use super::platform_detect;
use super::prompt_notification_setup;

/// Confidence threshold below which we show low-confidence UI
const CONFIDENCE_THRESHOLD: f32 = 0.7;

/// Check if kto daemon is currently running
pub fn is_daemon_running() -> bool {
    // Method 1: Check PID file (manual daemon start)
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return check_daemon_process(),
    };
    let pid_path = std::path::Path::new(&home).join(".local/share/kto/daemon.pid");
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            if std::path::Path::new("/proc").join(pid.to_string()).exists() {
                return true;
            }
        }
    }

    // Method 2: Check systemd user service
    if let Ok(output) = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "kto"])
        .output()
    {
        if output.status.success() {
            let status = String::from_utf8_lossy(&output.stdout);
            if status.trim() == "active" {
                return true;
            }
        }
    }

    // Method 3: Check for running kto daemon process
    check_daemon_process()
}

/// Check if there's a kto daemon process running via pgrep
fn check_daemon_process() -> bool {
    if let Ok(output) = std::process::Command::new("pgrep")
        .args(["-f", "kto daemon"])
        .output()
    {
        return output.status.success() && !output.stdout.is_empty();
    }
    false
}

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
    research: bool,
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
        if !is_daemon_running() {
            println!("\n  Run `kto daemon` to start monitoring.");
        }

        return Ok(());
    }

    // Try to extract URL from input, or discover one via AI
    let mut url = match extract_url(&input) {
        Some(u) => u,
        None => {
            // No URL found — try AI-powered URL discovery
            let claude_available = agent::claude_version().is_some();
            if !claude_available {
                return Err(kto::KtoError::ConfigError(format!(
                    "No URL found in: '{}'\n  \
                     Tip: Include a URL (e.g., https://example.com/page)\n  \
                     Or install Claude CLI for AI-powered URL discovery:\n  \
                     curl -fsSL https://claude.ai/install.sh | bash",
                    truncate_str(&input, 50)
                )));
            }

            // Enter URL discovery flow
            return run_url_discovery_flow(
                &input,
                &db,
                name_override,
                interval,
                tags,
                use_profile,
                use_agent,
                agent_instructions,
                selector,
                research,
                yes,
                interactive,
            );
        }
    };

    // Try URL transform detection first (for known sites like GitHub, GitLab, etc.)
    let detected_intent = Intent::detect(&input);
    let transform_match = if detected_intent != Intent::Generic {
        if let Ok(parsed_url) = url::Url::parse(&url) {
            transforms::match_transform(&parsed_url, detected_intent)
        } else {
            None
        }
    } else {
        None
    };

    // If we have a high-confidence transform match, create watch automatically
    // This is the "zero-prompt happy path" for known platforms
    if let Some(ref transform) = transform_match {
        if transform.confidence >= 0.8 {
            // Auto-create for high confidence matches
            return create_watch_from_transform_magical(
                &db,
                &url,
                transform,
                name_override,
                interval,
                tags,
                use_profile,
                interactive,
                yes,
            );
        } else if transform.confidence >= 0.5 && interactive && !yes {
            // Medium confidence - show preview and ask for confirmation
            let accepted = display_transform_suggestion(
                &url,
                transform,
                &name_override,
                interval,
                &tags,
                use_profile,
                yes,
                interactive,
            )?;

            if let Some((name, final_url, final_engine, final_extraction, final_interval)) = accepted {
                return create_watch_from_transform(
                    &db,
                    name,
                    final_url,
                    final_engine,
                    final_extraction,
                    final_interval,
                    tags,
                    use_profile,
                    interactive,
                    yes,
                );
            }
            // User declined - fall through to normal flow
        }
    }

    // Detect if user expressed intent (what to watch for)
    let has_intent = input.contains(" for ") || input.contains(" when ") || input.contains(" if ")
        || input.contains("watch for") || input.contains("notify me") || input.contains("alert")
        || input.contains("price") || input.contains("stock") || input.contains("available")
        || input.contains("back in") || input.contains("drop");

    // Consult knowledge base for learned defaults (from learning loop)
    let knowledge_defaults = if has_intent {
        let detected = transforms::Intent::detect(&input);
        let intent_str = match detected {
            transforms::Intent::Price => "price",
            transforms::Intent::Stock => "stock",
            transforms::Intent::Release => "release",
            transforms::Intent::Jobs => "jobs",
            transforms::Intent::News => "news",
            transforms::Intent::Generic => "generic",
        };
        match agent::load_creation_knowledge(intent_str, None) {
            Some(k) => {
                eprintln!("  {} Using learned defaults for {} intents", "i".cyan(), intent_str);
                Some(k)
            }
            None => None,
        }
    } else {
        None
    };

    // Check if Claude CLI is available for enhanced wizard
    let claude_available = agent::claude_version().is_some();

    // Use enhanced wizard flow when intent detected and Claude available
    // Works in both interactive and --yes mode (auto-accepts in --yes mode)
    let use_enhanced_wizard = has_intent && claude_available && !use_agent && !use_rss && !use_shell;

    // Check if we should use deep research mode
    let should_research = research;

    // Deep research flow - more thorough analysis with web search
    if should_research && claude_available {
        return run_deep_research_flow(
            &input,
            &url,
            name_override,
            interval,
            tags,
            use_profile,
            yes,
            interactive,
        );
    }

    // Enhanced wizard flow with dual fetch and smart analysis
    let (engine, content, extracted, title, enhanced_suggestion) = if use_enhanced_wizard {
        println!("\n  Analyzing {}...", url);

        // Perform dual fetch: HTTP and Playwright in parallel
        let (http_content, js_content) = dual_fetch(&url)?;

        // Run platform detection with KB
        let platform_analysis = platform_detect::analyze_url_with_platform_kb(
            &url,
            detected_intent,
            http_content.as_ref(),
            js_content.as_ref(),
        ).ok();

        // Show platform detection results (if detected)
        if let Some(ref analysis) = platform_analysis {
            if analysis.has_platform() {
                if let Some(ref pm) = analysis.platform_match {
                    println!("  Platform: {} ({:.0}% confidence)", pm.platform_name.cyan(), pm.score * 100.0);
                }
            }
        }

        // Extract content from both fetches using smart strategy selection
        let http_extracted = http_content.as_ref()
            .map(|c| extract::pick_best_extraction(c, detected_intent).1);
        let js_extracted = js_content.as_ref()
            .map(|c| extract::pick_best_extraction(c, detected_intent).1);

        // Get title from whichever fetch succeeded
        let title = js_content.as_ref()
            .and_then(|c| extract::extract_title(&c.html))
            .or_else(|| http_content.as_ref().and_then(|c| extract::extract_title(&c.html)))
            .unwrap_or_else(|| "Untitled".to_string());

        // Call enhanced AI analysis with both content versions
        // Include KB context if platform was detected
        println!("  Analyzing with AI (dual fetch)...");
        let suggestion = match agent::analyze_for_setup_v2(
            &input,
            http_extracted.as_deref(),
            js_extracted.as_deref(),
        ) {
            Ok(mut s) => {
                // Apply KB recommendations if AI confidence is lower than KB
                if let Some(ref analysis) = platform_analysis {
                    if let Some(ref best) = analysis.best_strategy {
                        // If platform detection suggests JS and AI didn't, prefer KB
                        if matches!(best.engine, Engine::Playwright) && !s.needs_js {
                            if analysis.confidence > s.confidence {
                                s.needs_js = true;
                                s.js_reason = Some(format!(
                                    "KB recommends for {}: {}",
                                    analysis.platform_match.as_ref().map(|p| p.platform_name.as_str()).unwrap_or("platform"),
                                    best.reason
                                ));
                            }
                        }
                    }
                }
                s
            }
            Err(e) => {
                eprintln!("  AI analysis failed: {} (using fallback)", e);
                // Use KB recommendations as fallback
                if let Some(ref analysis) = platform_analysis {
                    if let Some(ref best) = analysis.best_strategy {
                        let mut fallback = EnhancedSetupSuggestion::fallback(&url, &input);
                        fallback.needs_js = matches!(best.engine, Engine::Playwright);
                        fallback.js_reason = Some(format!("KB: {}", best.reason));
                        fallback.confidence = analysis.confidence;
                        fallback
                    } else {
                        EnhancedSetupSuggestion::fallback(&url, &input)
                    }
                } else {
                    EnhancedSetupSuggestion::fallback(&url, &input)
                }
            }
        };

        // Determine which content/engine to use based on AI recommendation (augmented by KB)
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
                            println!("  Switching to RSS feed.");
                            url = rss_link.clone();
                            Engine::Rss
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

        // Fetch the page with friendly error handling
        let content = match fetch::fetch(&url, engine.clone(), &std::collections::HashMap::new()) {
            Ok(c) => c,
            Err(e) => {
                let msg = platform_detect::friendly_error_message(&e.to_string(), &url);
                return Err(kto::KtoError::ConfigError(msg));
            }
        };

        // Determine extraction strategy (smart: compare multiple strategies)
        let (extraction, mut extracted) = match (&selector, &engine) {
            (Some(ref sel), _) => {
                let ext = Extraction::Selector { selector: sel.clone() };
                let extr = extract::extract(&content, &ext)?;
                (ext, extr)
            }
            (None, Engine::Rss) => {
                let ext = Extraction::Rss;
                let extr = extract::extract(&content, &ext)?;
                (ext, extr)
            }
            (None, _) => extract::pick_best_extraction(&content, detected_intent),
        };
        let title = extract::extract_title(&content.html)
            .unwrap_or_else(|| "Untitled".to_string());

        // Smart fallback: if HTTP content is thin, auto-retry with Playwright
        let (final_engine, final_content) = if extracted.len() < 200 && !use_js && engine == Engine::Http {
            // Check if Playwright is available
            if check_playwright().is_ready() {
                println!("  Site needs visual rendering. Switching to browser mode...");
                match fetch::fetch(&url, Engine::Playwright, &std::collections::HashMap::new()) {
                    Ok(js_content) => {
                        let js_extracted = extract::extract(&js_content, &extraction)
                            .unwrap_or_else(|_| extracted.clone());
                        if js_extracted.len() > extracted.len() {
                            extracted = js_extracted;
                            (Engine::Playwright, js_content)
                        } else {
                            (engine, content)
                        }
                    }
                    Err(_) => (engine, content),
                }
            } else {
                println!("\n  Note: Page may need JavaScript. Run `kto init` to enable browser mode.");
                (engine, content)
            }
        } else {
            (engine, content)
        };

        (final_engine, final_content, extracted, title, None)
    };

    // Determine extraction strategy based on selector or engine
    // Uses multi-strategy comparison when no selector/RSS is specified
    let extraction = match (&selector, &engine) {
        (Some(ref sel), _) => Extraction::Selector { selector: sel.clone() },
        (None, Engine::Rss) => Extraction::Rss,
        (None, _) => {
            let (best_ext, _) = extract::pick_best_extraction(&content, detected_intent);
            best_ext
        }
    };

    // Apply enhanced AI suggestions or use traditional flow
    let (name, final_url, final_interval, final_agent_enabled, final_agent_instructions, final_extraction, final_engine) =
        if let Some(ref suggestion) = enhanced_suggestion {
            // Enhanced wizard flow with variant display
            match display_enhanced_confirmation(
                &url,
                suggestion,
                &extraction,
                engine.clone(),
                &name_override,
                interval,
                yes,
            ) {
                Ok(result) => result,
                Err(kto::KtoError::RetryWithDeepResearch) => {
                    // User requested deep research - run that flow instead
                    return run_deep_research_flow(
                        &input,
                        &url,
                        name_override,
                        interval,
                        tags,
                        use_profile,
                        yes,
                        interactive,
                    );
                }
                Err(e) => return Err(e),
            }
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
                // Interactive mode: use selection for common intents
                println!();
                let intent_options = vec![
                    "Price changes (sales, drops, increases)",
                    "Back in stock / availability",
                    "New content or updates",
                    "Any changes (notify on all)",
                    "Custom (I'll describe it)",
                ];

                let choice = Select::new("What do you want to watch for?", intent_options)
                    .prompt()
                    .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

                let (agent_needed, instructions) = match choice {
                    "Price changes (sales, drops, increases)" => {
                        (true, Some("Alert when price changes. Include old and new price.".to_string()))
                    }
                    "Back in stock / availability" => {
                        (true, Some("Alert when item becomes available or goes out of stock.".to_string()))
                    }
                    "New content or updates" => {
                        (true, Some("Alert when new content is added. Summarize what's new.".to_string()))
                    }
                    "Any changes (notify on all)" => {
                        (false, None)
                    }
                    "Custom (I'll describe it)" => {
                        let custom_intent = Text::new("Describe what changes matter:")
                            .with_help_message("e.g., 'price drops below $50', 'new job postings'")
                            .prompt()
                            .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

                        if custom_intent.trim().is_empty() {
                            (false, None)
                        } else {
                            (true, Some(custom_intent.trim().to_string()))
                        }
                    }
                    _ => (false, None),
                };

                if agent_needed && !claude_available {
                    println!("  Note: Smart filtering requires Claude CLI.");
                    println!("  Install: curl -fsSL https://claude.ai/install.sh | bash");
                    println!("  Will notify on all changes for now.");
                    (false, None)
                } else {
                    (agent_needed, instructions)
                }
            } else {
                // Non-interactive mode: require explicit --agent flag
                (false, agent_instructions.clone())
            };

            (name, url.clone(), interval, agent_enabled, final_instructions, extraction.clone(), engine)
        };

    // Apply knowledge base defaults where user/AI didn't specify
    // Precedence: user override > discovery/AI result > knowledge default > global default
    let (final_interval, final_engine, final_extraction) = if let Some(ref kb) = knowledge_defaults {
        let ki = if interval != 900 { final_interval } else {
            kb.interval_secs.map(|i| i as u64).unwrap_or(final_interval)
        };
        let ke = if use_js || use_rss {
            final_engine.clone()
        } else if enhanced_suggestion.is_some() {
            final_engine.clone() // AI suggestion takes precedence
        } else {
            match kb.engine.as_deref() {
                Some("rss") => Engine::Rss,
                Some("playwright") => Engine::Playwright,
                _ => final_engine.clone(),
            }
        };
        let kx = if selector.is_some() {
            final_extraction.clone()
        } else if enhanced_suggestion.is_some() {
            final_extraction.clone()
        } else {
            match kb.extraction.as_deref() {
                Some("selector") => {
                    if let Some(ref sel) = kb.selector {
                        Extraction::Selector { selector: sel.clone() }
                    } else {
                        final_extraction.clone()
                    }
                }
                Some("rss") => Extraction::Rss,
                Some("json_ld") => Extraction::JsonLd { types: None },
                Some("full") => Extraction::Full,
                _ => final_extraction.clone(),
            }
        };
        (ki, ke, kx)
    } else {
        (final_interval, final_engine, final_extraction)
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

    // User-friendly success output (no jargon)
    let has_agent = watch.agent_config.is_some();
    let agent_instructions = watch.agent_config.as_ref().and_then(|c| c.instructions.as_deref());
    let intent_description = platform_detect::describe_watch_intent(
        &watch.engine,
        has_agent,
        agent_instructions,
    );

    let success_msg = platform_detect::format_watch_created(
        &name,
        &intent_description,
        watch.interval_secs,
        has_agent,
    );
    println!("{}", success_msg);

    // Show tags if present
    if !watch.tags.is_empty() {
        println!("   Tags: {}", watch.tags.join(", "));
    }

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

    if !is_daemon_running() {
        println!("\n  Run `kto daemon` to start monitoring.");
    }

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
    new_engine: Option<String>,
    new_extraction: Option<String>,
    new_notify: Option<String>,
    new_use_profile: Option<bool>,
) -> Result<()> {
    use inquire::Select;

    let db = Database::open()?;
    let mut watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    let has_flags = new_name.is_some() || new_interval.is_some() || new_enabled.is_some()
        || new_agent.is_some() || new_agent_instructions.is_some() || new_selector.is_some()
        || new_engine.is_some() || new_extraction.is_some()
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

        if let Some(ref engine_str) = new_engine {
            match engine_str.to_lowercase().as_str() {
                "http" => {
                    watch.engine = Engine::Http;
                    changes.push("engine -> http".to_string());
                }
                "playwright" | "js" => {
                    watch.engine = Engine::Playwright;
                    changes.push("engine -> playwright".to_string());
                }
                "rss" => {
                    watch.engine = Engine::Rss;
                    changes.push("engine -> rss".to_string());
                }
                other => {
                    return Err(kto::KtoError::ConfigError(format!(
                        "Unknown engine '{}'. Valid options: http, playwright, rss", other
                    )));
                }
            }
        }

        if let Some(ref extraction_str) = new_extraction {
            match extraction_str.to_lowercase().as_str() {
                "auto" => {
                    watch.extraction = Extraction::Auto;
                    changes.push("extraction -> auto".to_string());
                }
                "full" => {
                    watch.extraction = Extraction::Full;
                    changes.push("extraction -> full".to_string());
                }
                "rss" => {
                    watch.extraction = Extraction::Rss;
                    changes.push("extraction -> rss".to_string());
                }
                "json-ld" | "json_ld" | "jsonld" => {
                    watch.extraction = Extraction::JsonLd { types: None };
                    changes.push("extraction -> json-ld".to_string());
                }
                other => {
                    return Err(kto::KtoError::ConfigError(format!(
                        "Unknown extraction '{}'. Valid options: auto, full, rss, json-ld", other
                    )));
                }
            }
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
/// If `skip_http` is true, only perform Playwright fetch (for known JS-heavy sites)
fn dual_fetch(url: &str) -> Result<(Option<PageContent>, Option<PageContent>)> {
    dual_fetch_with_hint(url, false)
}

/// Perform HTTP and/or Playwright fetches based on hints
/// If `skip_http` is true, only perform Playwright fetch (for known JS-heavy sites like npm)
fn dual_fetch_with_hint(url: &str, skip_http: bool) -> Result<(Option<PageContent>, Option<PageContent>)> {
    let url_owned = url.to_string();

    // Start HTTP fetch in a thread (unless skipped for known JS-heavy sites)
    let http_handle = if !skip_http {
        let url_http = url_owned.clone();
        Some(thread::spawn(move || {
            fetch::fetch(&url_http, Engine::Http, &std::collections::HashMap::new())
        }))
    } else {
        None
    };

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
    let http_content = if let Some(handle) = http_handle {
        handle
            .join()
            .map_err(|_| kto::KtoError::ConfigError("HTTP fetch thread panicked".into()))?
            .ok()
    } else {
        None
    };

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
    let http_status = if skip_http {
        "–" // Skipped
    } else if http_content.is_some() {
        "✓"
    } else {
        "✗"
    };
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

    // Check if Claude is available for deep research option
    let claude_available = agent::claude_version().is_some();

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

    // Offer choices: Create, Customize, Cancel (+ Deep Research if low confidence)
    let mut choices = vec!["Create Watch"];

    // Add Deep Research option at the top if low confidence and Claude available
    if low_confidence && claude_available {
        choices.insert(0, "Run Deep Research");
    }

    if !suggestion.variants.is_empty() && suggestion.variants.len() > 1 {
        choices.push("Select Different Variant");
    }
    choices.push("Customize");
    choices.push("Cancel");

    let choice = Select::new("What would you like to do?", choices)
        .prompt()
        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

    match choice {
        "Run Deep Research" => {
            // Signal to caller to retry with deep research mode
            return Err(kto::KtoError::RetryWithDeepResearch);
        }
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

// ============================================================================
// URL Transform Helper Functions
// ============================================================================

/// Display a transform suggestion and let user accept/decline
/// Returns Some((name, url, engine, extraction, interval)) if accepted, None if declined
fn display_transform_suggestion(
    original_url: &str,
    transform: &TransformMatch,
    name_override: &Option<String>,
    default_interval: u64,
    _tags: &[String],
    _use_profile: bool,
    yes: bool,
    interactive: bool,
) -> Result<Option<(String, String, Engine, Extraction, u64)>> {
    let transformed_url = transform.url.as_str();

    // Generate a default name from the URL
    let default_name = generate_name_from_url(&transform.url);

    if yes {
        // Auto-accept with --yes flag
        let name = name_override.clone().unwrap_or(default_name);
        let extraction = if transform.engine == Engine::Rss {
            Extraction::Rss
        } else {
            Extraction::Auto
        };

        // Show user-friendly output (no jargon)
        println!("\n  Found: {}", transform.description);

        return Ok(Some((
            name,
            transformed_url.to_string(),
            transform.engine.clone(),
            extraction,
            default_interval,
        )));
    }

    if !interactive {
        // Non-interactive without --yes, just show info
        return Ok(None);
    }

    // Interactive mode - show user-friendly suggestion
    println!();
    let platform_name = detect_platform_name(transform.url.host_str());
    println!("  ✨ {} detected", platform_name.cyan());
    println!();
    println!("  Found: {}", transform.description.green());
    println!();

    // User-friendly description instead of technical jargon
    if transform.engine == Engine::Rss {
        println!("  Will notify you when new items are published.");
    } else {
        println!("  Will monitor the page for changes.");
    }
    println!();

    let choices = vec!["Accept (recommended)", "Use original URL instead", "Cancel"];
    let choice = Select::new("How would you like to proceed?", choices)
        .prompt()
        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

    match choice {
        "Accept (recommended)" => {
            let name = match name_override {
                Some(n) => n.clone(),
                None => {
                    Text::new("Name for this watch?")
                        .with_default(&default_name)
                        .prompt()
                        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?
                }
            };

            let extraction = if transform.engine == Engine::Rss {
                Extraction::Rss
            } else {
                Extraction::Auto
            };

            Ok(Some((
                name,
                transformed_url.to_string(),
                transform.engine.clone(),
                extraction,
                default_interval,
            )))
        }
        "Use original URL instead" => {
            // User declined - return None to fall through to normal flow
            println!("  Using original URL: {}", original_url);
            Ok(None)
        }
        "Cancel" | _ => {
            Err(kto::KtoError::ConfigError("Watch creation cancelled".into()))
        }
    }
}

/// Create a watch directly from transform match (bypassing AI analysis)
fn create_watch_from_transform(
    db: &Database,
    name: String,
    url: String,
    engine: Engine,
    extraction: Extraction,
    interval: u64,
    tags: Vec<String>,
    use_profile: bool,
    interactive: bool,
    yes: bool,
) -> Result<()> {
    println!("\n  Fetching {}...", url);

    // Fetch the page to create initial snapshot
    let content = fetch::fetch(&url, engine.clone(), &std::collections::HashMap::new())?;

    // Extract content
    let extracted = extract::extract(&content, &extraction)?;

    // Create watch
    let mut watch = Watch::new(name.clone(), url);
    watch.interval_secs = interval.max(10);
    watch.engine = engine;
    watch.extraction = extraction;
    watch.tags = tags;
    watch.use_profile = use_profile;

    // No agent config for transform-based watches by default
    // (RSS feeds don't need AI analysis in most cases)

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

    // User-friendly success output
    let intent_description = platform_detect::describe_watch_intent(&watch.engine, false, None);
    let success_msg = platform_detect::format_watch_created(
        &name,
        &intent_description,
        watch.interval_secs,
        false,
    );
    println!("{}", success_msg);

    if !watch.tags.is_empty() {
        println!("   Tags: {}", watch.tags.join(", "));
    }

    // Prompt for notification setup if not configured and interactive
    let mut config = Config::load()?;
    if config.default_notify.is_none() && interactive && !yes {
        println!();
        if let Some(target) = super::prompt_notification_setup()? {
            config.default_notify = Some(target);
            config.save()?;
            println!("  Notification settings saved.");
        }
    }

    if !is_daemon_running() {
        println!("\n  Run `kto daemon` to start monitoring.");
    }

    Ok(())
}

/// Create a watch from a high-confidence transform match - zero prompts!
/// This is the "magical" happy path for known platforms like GitHub, Reddit, etc.
fn create_watch_from_transform_magical(
    db: &Database,
    original_url: &str,
    transform: &TransformMatch,
    name_override: Option<String>,
    default_interval: u64,
    tags: Vec<String>,
    use_profile: bool,
    interactive: bool,
    yes: bool,
) -> Result<()> {
    let transformed_url = transform.url.as_str();
    let engine = transform.engine.clone();

    // Step 1: Show we're analyzing
    println!("\n  Analyzing {}...", original_url.split('/').take(3).collect::<Vec<_>>().join("/"));

    // Step 2: Fetch to get content and validate
    let content = match fetch::fetch(transformed_url, engine.clone(), &std::collections::HashMap::new()) {
        Ok(c) => c,
        Err(e) => {
            // User-friendly error
            let msg = platform_detect::friendly_error_message(&e.to_string(), original_url);
            return Err(kto::KtoError::ConfigError(msg));
        }
    };

    // Step 3: Extract content
    let extraction = if engine == Engine::Rss {
        Extraction::Rss
    } else {
        Extraction::Auto
    };
    let extracted = extract::extract(&content, &extraction)?;

    // Step 4: Get name (from page title, URL, or override)
    let default_name = generate_name_from_url(&transform.url);
    let name = name_override.unwrap_or(default_name);

    // Step 5: Get a preview of what we're watching
    let latest_item = if engine == Engine::Rss {
        // For RSS, get the first item title
        extract_first_rss_item(&extracted)
    } else {
        None
    };

    // Step 6: Show the user-friendly preview
    println!();
    let preview = platform_detect::format_known_platform_preview(
        original_url,
        &detect_platform_name(transform.url.host_str()),
        transform.description,
        latest_item.as_deref(),
    );
    println!("{}", preview);

    // Step 7: Create watch
    let mut watch = Watch::new(name.clone(), transformed_url.to_string());
    watch.interval_secs = default_interval.max(10);
    watch.engine = engine.clone();
    watch.extraction = extraction;
    watch.tags = tags;
    watch.use_profile = use_profile;

    db.insert_watch(&watch)?;

    // Step 8: Create initial snapshot
    let normalized = normalize(&extracted, &watch.normalization);
    let hash = hash_content(&normalized);

    let snapshot = Snapshot {
        id: Uuid::new_v4(),
        watch_id: watch.id,
        fetched_at: Utc::now(),
        raw_html: Some(zstd::encode_all(content.html.as_bytes(), 3)?),
        extracted: normalized,
        content_hash: hash,
    };
    db.insert_snapshot(&snapshot)?;

    // Step 9: Show success message (no jargon!)
    let intent_description = platform_detect::describe_watch_intent(&watch.engine, false, None);
    let success_msg = platform_detect::format_watch_created(
        &name,
        &intent_description,
        watch.interval_secs,
        false,
    );
    println!("{}", success_msg);

    // Step 10: Prompt for notification setup if needed
    let mut config = Config::load()?;
    if config.default_notify.is_none() && interactive && !yes {
        println!();
        if let Some(target) = super::prompt_notification_setup()? {
            config.default_notify = Some(target);
            config.save()?;
            println!("  Notification settings saved.");
        }
    }

    if !is_daemon_running() {
        println!("\n  Run `kto daemon` to start monitoring.");
    }

    Ok(())
}

/// Extract the first item title from RSS feed content
fn extract_first_rss_item(rss_content: &str) -> Option<String> {
    // RSS content is already formatted by fetch, look for first item
    for line in rss_content.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('-') && !trimmed.starts_with('[') {
            // Skip header lines, return first actual content
            if trimmed.len() > 5 && trimmed.len() < 100 {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Get a human-readable platform name from host
fn detect_platform_name(host: Option<&str>) -> String {
    match host {
        Some("github.com") => "GitHub Repository".to_string(),
        Some("gitlab.com") => "GitLab Project".to_string(),
        Some("codeberg.org") => "Codeberg Repository".to_string(),
        Some("news.ycombinator.com") => "Hacker News".to_string(),
        Some(h) if h.contains("reddit.com") => "Reddit".to_string(),
        Some("pypi.org") => "PyPI Package".to_string(),
        Some("crates.io") => "Crates.io Package".to_string(),
        Some("hub.docker.com") => "Docker Hub".to_string(),
        Some("www.npmjs.com") => "npm Package".to_string(),
        Some(h) => h.to_string(),
        None => "Site".to_string(),
    }
}

/// Generate a human-readable name from a URL
fn generate_name_from_url(url: &url::Url) -> String {
    // Try to extract meaningful name from path
    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // For GitHub/GitLab repos: "owner/repo" -> "owner/repo releases"
    if let Some(host) = url.host_str() {
        if (host == "github.com" || host == "gitlab.com" || host == "codeberg.org")
            && segments.len() >= 2
        {
            let owner = segments[0];
            let repo = segments[1];
            return format!("{}/{}", owner, repo);
        }

        // For Reddit: "r/subreddit" -> "r/subreddit"
        if host.contains("reddit.com") && segments.len() >= 2 && segments[0] == "r" {
            return format!("r/{}", segments[1]);
        }

        // For HN
        if host == "news.ycombinator.com" {
            return "Hacker News".to_string();
        }

        // For PyPI
        if host == "pypi.org" && segments.len() >= 2 && segments[0] == "project" {
            return format!("PyPI: {}", segments[1]);
        }
    }

    // Fallback: use host
    url.host_str()
        .unwrap_or("Watch")
        .to_string()
}

// ============================================================================
// Deep Research Mode
// ============================================================================

/// Run the deep research flow for watch creation
fn run_deep_research_flow(
    input: &str,
    url: &str,
    name_override: Option<String>,
    default_interval: u64,
    tags: Vec<String>,
    use_profile: bool,
    yes: bool,
    interactive: bool,
) -> Result<()> {
    let db = Database::open()?;

    println!("\n  {} Deep Research Mode", "🔬".bold());
    println!("  Analyzing {}...", url);

    // Check if there's a transform rule that specifies Playwright
    // If so, skip HTTP fetch to avoid timeout on JS-heavy sites (e.g., npm)
    let parsed_url = url::Url::parse(url).ok();
    let detected_intent = Intent::detect(input);
    let transform_match = parsed_url
        .as_ref()
        .and_then(|u| transforms::match_transform(u, detected_intent));

    let skip_http = transform_match
        .as_ref()
        .map(|m| m.engine == Engine::Playwright)
        .unwrap_or(false);

    if skip_http {
        println!("  Note: Transform rule specifies Playwright - skipping HTTP fetch");
    }

    // Step 1: Dual fetch (HTTP and Playwright)
    let (http_content, js_content) = dual_fetch_with_hint(url, skip_http)?;

    // Step 2: Extract content from both using smart strategy selection
    let http_html = http_content.as_ref().map(|c| c.html.as_str());
    let http_extracted = http_content.as_ref()
        .map(|c| extract::pick_best_extraction(c, detected_intent).1);
    let js_extracted = js_content.as_ref()
        .map(|c| extract::pick_best_extraction(c, detected_intent).1);

    // Step 3: Detect site type
    let site_type = http_html.and_then(|html| fetch::detect_site_type(url, html));
    if let Some(ref st) = site_type {
        println!("  Detected: {}", st.cyan());
    }

    // Step 4: Discover feeds from HTML
    let discovered_feeds = if let Some(html) = http_html {
        println!("  Discovering feeds...");
        let feeds = fetch::discover_feeds(url, html);
        if !feeds.is_empty() {
            println!("  Found {} feed(s)", feeds.len());
        }
        feeds
    } else {
        vec![]
    };

    // Step 5: Extract JSON-LD
    let jsonld_data = http_html.and_then(|html| extract::extract_raw_jsonld(html));
    if jsonld_data.is_some() {
        println!("  Found JSON-LD structured data");
    }

    // Step 6: Run deep research analysis
    println!("  Analyzing with AI (this may take a moment)...");
    let research_result = match agent::deep_research_analysis(
        url,
        input,
        http_extracted.as_deref(),
        js_extracted.as_deref(),
        &discovered_feeds,
        jsonld_data.as_deref(),
        site_type.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  Research failed: {} (using fallback)", e);
            DeepResearchResult::fallback(url, input)
        }
    };

    // Step 7: Display results
    display_research_results(&research_result);

    // Step 7.5: Apply URL modifications (variant params, etc.)
    let modified_url = apply_url_modifications(url, &research_result);
    if modified_url != url {
        println!("  URL modified: {}", modified_url.cyan());
    }

    // Step 8: User confirmation or auto-accept
    let (name, final_url, final_engine, final_extraction, final_interval, agent_enabled, agent_instructions) =
        if yes {
            // Auto-accept
            let name = name_override.unwrap_or_else(|| {
                if let Ok(parsed) = url::Url::parse(&modified_url) {
                    generate_name_from_url(&parsed)
                } else {
                    "Watch".to_string()
                }
            });
            let engine = research_result.engine.to_engine();
            let extraction = match research_result.extraction.strategy.as_str() {
                "selector" => {
                    if let Some(ref sel) = research_result.extraction.selector {
                        Extraction::Selector { selector: sel.clone() }
                    } else {
                        Extraction::Auto
                    }
                }
                "rss" => Extraction::Rss,
                "json_ld" => Extraction::JsonLd { types: None },
                _ => Extraction::Auto,
            };

            (
                name,
                modified_url.clone(),
                engine,
                extraction,
                research_result.interval_secs,
                research_result.agent_instructions.is_some(),
                research_result.agent_instructions.clone(),
            )
        } else if interactive {
            // Interactive confirmation
            confirm_research_results(
                &modified_url,
                &research_result,
                &name_override,
                default_interval,
            )?
        } else {
            return Err(kto::KtoError::ConfigError(
                "Deep research requires interactive mode or --yes flag".into()
            ));
        };

    // Step 9: Fetch with final engine
    println!("\n  Fetching with {:?} engine...", final_engine);
    let content = fetch::fetch(&final_url, final_engine.clone(), &std::collections::HashMap::new())?;
    let extracted = extract::extract(&content, &final_extraction)?;

    // Step 10: Create watch
    let mut watch = Watch::new(name.clone(), final_url);
    watch.interval_secs = final_interval.max(10);
    watch.engine = final_engine;
    watch.extraction = final_extraction;
    watch.tags = tags;
    watch.use_profile = use_profile;

    if agent_enabled {
        watch.agent_config = Some(AgentConfig {
            enabled: true,
            prompt_template: None,
            instructions: agent_instructions,
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

    // Prompt for notification setup if not configured
    let mut config = Config::load()?;
    if config.default_notify.is_none() && interactive && !yes {
        println!();
        if let Some(target) = super::prompt_notification_setup()? {
            config.default_notify = Some(target);
            config.save()?;
            println!("  Notification settings saved.");
        }
    }

    if !is_daemon_running() {
        println!("\n  Run `kto daemon` to start monitoring.");
    }

    Ok(())
}

/// Apply URL modifications from research results (variant params, etc.)
fn apply_url_modifications(url: &str, result: &DeepResearchResult) -> String {
    if let Some(ref mods) = result.url_modifications {
        if let Some(ref variant) = mods.variant_param {
            if !variant.is_empty() {
                if url.contains('?') {
                    return format!("{}&variant={}", url, variant);
                } else {
                    return format!("{}?variant={}", url, variant);
                }
            }
        }
    }
    url.to_string()
}

/// Display deep research results
fn display_research_results(result: &DeepResearchResult) {
    println!();
    println!("  {}", "Deep Research Results".bold().underline());
    println!();
    println!("  {}", result.summary);

    // Web research findings (if available)
    if let Some(ref web) = result.web_research {
        println!();
        println!("  {}:", "Web Research".bold());
        if !web.queries_made.is_empty() {
            println!("    Searched: {}", web.queries_made.join(", "));
        }
        for finding in &web.relevant_findings {
            println!("    • {}", finding);
        }
        if !web.api_endpoints.is_empty() {
            println!();
            println!("  Discovered APIs:");
            for api in &web.api_endpoints {
                let auth = if api.requires_auth { " (auth required)" } else { "" };
                println!("    {} - {}{}", api.url_pattern, api.description, auth);
            }
        }
        if !web.community_tips.is_empty() {
            println!();
            println!("  Community Tips:");
            for tip in &web.community_tips {
                println!("    • {}", tip);
            }
        }
    }

    // Discovered feeds
    if !result.discovered_feeds.is_empty() {
        println!();
        println!("  {}:", "Discovered Feeds".bold());
        for feed in &result.discovered_feeds {
            let intent_marker = if feed.matches_intent {
                " ← matches intent".green().to_string()
            } else {
                "".to_string()
            };
            println!("    {} ({}, via {}){}", feed.url, feed.feed_type, feed.discovery_method, intent_marker);
        }
    }

    // Recommended approach
    println!();
    println!("  {}:", "Recommended Approach".bold());
    println!("    Engine: {}", result.engine.engine_type.cyan());
    println!("      {}", result.engine.reason.dimmed());
    println!("    Extraction: {}", result.extraction.strategy.cyan());
    if let Some(ref sel) = result.extraction.selector {
        println!("      Selector: {}", sel);
    }
    println!("      {}", result.extraction.reason.dimmed());

    // URL modifications
    if let Some(ref mods) = result.url_modifications {
        if mods.variant_param.is_some() {
            println!("    URL Mod: variant={}", mods.variant_param.as_ref().unwrap().cyan());
            println!("      {}", mods.reason.dimmed());
        }
    }

    // Recommended selectors
    if !result.selectors.is_empty() {
        println!();
        println!("  Stable Selectors:");
        for sel in &result.selectors {
            let stability = format!("{:.0}%", sel.stability_score * 100.0);
            println!("    {} ({})", sel.selector, stability.dimmed());
            println!("      {}", sel.description.dimmed());
        }
    }

    // Key insights
    if !result.insights.is_empty() {
        println!();
        println!("  {}:", "Key Insights".bold());
        for insight in &result.insights {
            println!("    • {}", insight);
        }
    }

    // Agent instructions
    if let Some(ref instructions) = result.agent_instructions {
        println!();
        println!("  AI Instructions: \"{}\"", truncate_str(instructions, 60));
    }

    // Confidence
    println!();
    let confidence_color = if result.confidence >= 0.8 {
        format!("{:.0}%", result.confidence * 100.0).green()
    } else if result.confidence >= 0.5 {
        format!("{:.0}%", result.confidence * 100.0).yellow()
    } else {
        format!("{:.0}%", result.confidence * 100.0).red()
    };
    println!("  Confidence: {}", confidence_color);
    println!();
}

/// Interactive confirmation of research results
fn confirm_research_results(
    url: &str,
    result: &DeepResearchResult,
    name_override: &Option<String>,
    _default_interval: u64,
) -> Result<(String, String, Engine, Extraction, u64, bool, Option<String>)> {
    // Check if there's a feed that matches intent
    let matching_feed = result.discovered_feeds.iter().find(|f| f.matches_intent);

    // Build choices
    let mut choices = vec!["Accept recommendations", "Customize"];

    // Add option to use matching feed if available
    if matching_feed.is_some() {
        choices.insert(1, "Use discovered feed");
    }

    choices.push("Cancel");

    let choice = Select::new("What would you like to do?", choices)
        .prompt()
        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

    match choice {
        "Accept recommendations" => {
            let name = match name_override {
                Some(n) => n.clone(),
                None => {
                    let default_name = if let Ok(parsed) = url::Url::parse(url) {
                        generate_name_from_url(&parsed)
                    } else {
                        "Watch".to_string()
                    };
                    Text::new("Name for this watch?")
                        .with_default(&default_name)
                        .prompt()
                        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?
                }
            };

            let engine = result.engine.to_engine();
            let extraction = match result.extraction.strategy.as_str() {
                "selector" => {
                    if let Some(ref sel) = result.extraction.selector {
                        Extraction::Selector { selector: sel.clone() }
                    } else {
                        Extraction::Auto
                    }
                }
                "rss" => Extraction::Rss,
                "json_ld" => Extraction::JsonLd { types: None },
                _ => Extraction::Auto,
            };

            Ok((
                name,
                url.to_string(),
                engine,
                extraction,
                result.interval_secs,
                result.agent_instructions.is_some(),
                result.agent_instructions.clone(),
            ))
        }
        "Use discovered feed" => {
            let feed = matching_feed.expect("Feed should exist");
            let name = match name_override {
                Some(n) => n.clone(),
                None => {
                    let default_name = feed.title.clone().unwrap_or_else(|| {
                        if let Ok(parsed) = url::Url::parse(&feed.url) {
                            generate_name_from_url(&parsed)
                        } else {
                            "Watch".to_string()
                        }
                    });
                    Text::new("Name for this watch?")
                        .with_default(&default_name)
                        .prompt()
                        .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?
                }
            };

            println!("  Using feed: {}", feed.url.cyan());

            Ok((
                name,
                feed.url.clone(),
                Engine::Rss,
                Extraction::Rss,
                result.interval_secs,
                false, // RSS feeds usually don't need AI
                None,
            ))
        }
        "Customize" => {
            // Full customization
            let name = Text::new("Name for this watch?")
                .with_default(&name_override.clone().unwrap_or_else(|| {
                    if let Ok(parsed) = url::Url::parse(url) {
                        generate_name_from_url(&parsed)
                    } else {
                        "Watch".to_string()
                    }
                }))
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            // Engine selection
            let engine_choices = vec!["HTTP", "JavaScript (Playwright)", "RSS"];
            let default_engine_idx = match result.engine.engine_type.as_str() {
                "playwright" | "js" => 1,
                "rss" => 2,
                _ => 0,
            };
            let engine_choice = Select::new("Engine:", engine_choices)
                .with_starting_cursor(default_engine_idx)
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            let engine = match engine_choice {
                "JavaScript (Playwright)" => Engine::Playwright,
                "RSS" => Engine::Rss,
                _ => Engine::Http,
            };

            // Extraction strategy
            let extraction = if engine == Engine::Rss {
                Extraction::Rss
            } else {
                let extraction_choices = vec!["Auto", "CSS Selector", "JSON-LD"];
                let extraction_choice = Select::new("Extraction:", extraction_choices)
                    .prompt()
                    .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

                match extraction_choice {
                    "CSS Selector" => {
                        let default_sel = result.extraction.selector.as_deref()
                            .or_else(|| result.selectors.first().map(|s| s.selector.as_str()))
                            .unwrap_or("");
                        let sel = Text::new("CSS Selector:")
                            .with_default(default_sel)
                            .prompt()
                            .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;
                        Extraction::Selector { selector: sel }
                    }
                    "JSON-LD" => Extraction::JsonLd { types: None },
                    _ => Extraction::Auto,
                }
            };

            // Interval
            let interval_str = Text::new("Check interval (e.g., 5m, 1h)?")
                .with_default(&format_interval(result.interval_secs))
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            let interval = crate::utils::parse_interval_str(&interval_str)
                .unwrap_or(result.interval_secs);

            // AI agent
            let use_ai = Confirm::new("Enable AI analysis?")
                .with_default(result.agent_instructions.is_some())
                .prompt()
                .unwrap_or(false);

            let instructions = if use_ai {
                let inst = Text::new("AI instructions:")
                    .with_default(result.agent_instructions.as_deref().unwrap_or(""))
                    .prompt()
                    .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;
                if inst.is_empty() { None } else { Some(inst) }
            } else {
                None
            };

            Ok((name, url.to_string(), engine, extraction, interval, use_ai, instructions))
        }
        "Cancel" | _ => {
            Err(kto::KtoError::ConfigError("Watch creation cancelled".into()))
        }
    }
}

// ============================================================================
// URL Discovery Mode (Natural Language → URL)
// ============================================================================

/// Minimum confidence for interactive discovery (user confirms)
const DISCOVERY_CONFIDENCE_INTERACTIVE: f32 = 0.3;
/// Minimum confidence for auto-accept in --yes mode
const DISCOVERY_CONFIDENCE_AUTO: f32 = 0.5;

/// Run the AI-powered URL discovery flow when no URL was provided
fn run_url_discovery_flow(
    input: &str,
    db: &Database,
    name_override: Option<String>,
    interval: u64,
    tags: Vec<String>,
    use_profile: bool,
    _use_agent: bool,
    _agent_instructions: Option<String>,
    _selector: Option<String>,
    _research: bool,
    yes: bool,
    interactive: bool,
) -> Result<()> {
    // Parse intent from input
    let parsed_intent = ParsedIntent::new(input);

    println!("\n  Discovering URL for: \"{}\"", truncate_str(input, 60));
    if !parsed_intent.keywords_found.is_empty() {
        println!("  Detected: {} ({})", parsed_intent.brief_description(),
                 parsed_intent.keywords_found.join(", ").dimmed());
    }
    println!("  Searching...");

    // Call AI discovery
    let discovery = match agent::discover_url(input, &parsed_intent) {
        Ok(d) => d,
        Err(e) => {
            return Err(kto::KtoError::ConfigError(format!(
                "URL discovery failed: {}\n  \
                 Tip: Try providing a URL directly:\n  \
                 kto new \"https://example.com for price drops\"",
                e
            )));
        }
    };

    // Validate confidence threshold
    if discovery.confidence < DISCOVERY_CONFIDENCE_INTERACTIVE {
        return Err(kto::KtoError::ConfigError(format!(
            "Could not find a reliable URL for: '{}'\n  \
             AI confidence: {:.0}%{}\n  \
             Tip: Try providing a URL directly:\n  \
             kto new \"https://example.com for price drops\"",
            truncate_str(input, 50),
            discovery.confidence * 100.0,
            if !discovery.reasoning.is_empty() {
                format!(" ({})", truncate_str(&discovery.reasoning, 80))
            } else {
                String::new()
            }
        )));
    }

    // In --yes mode, require higher confidence
    if yes && discovery.confidence < DISCOVERY_CONFIDENCE_AUTO {
        return Err(kto::KtoError::ConfigError(format!(
            "URL discovery confidence too low for --yes mode ({:.0}%, need {:.0}%)\n  \
             Run interactively to review and confirm the discovered URL.",
            discovery.confidence * 100.0,
            DISCOVERY_CONFIDENCE_AUTO * 100.0
        )));
    }

    // Display results and get confirmation
    let action = display_discovery_results(&discovery, yes, interactive)?;

    match action {
        DiscoveryAction::Accept => {
            create_watch_from_discovery(
                db, &discovery, name_override, interval, tags, use_profile, yes, interactive,
            )
        }
        DiscoveryAction::UseAlternative(idx) => {
            let alt = &discovery.alternatives[idx];
            // Create a modified discovery with the alternative URL
            let mut modified = discovery.clone();
            modified.url = alt.url.clone();
            if !alt.engine.is_empty() {
                modified.engine = alt.engine.clone();
            }
            create_watch_from_discovery(
                db, &modified, name_override, interval, tags, use_profile, yes, interactive,
            )
        }
        DiscoveryAction::DeepResearch => {
            // Redirect to deep research with the discovered URL
            run_deep_research_flow(
                input,
                &discovery.url,
                name_override,
                interval,
                tags,
                use_profile,
                yes,
                interactive,
            )
        }
        DiscoveryAction::ManualUrl => {
            // Let user type a URL manually
            let manual_url = Text::new("Enter URL:")
                .with_help_message("Paste the full URL (e.g., https://example.com/page)")
                .prompt()
                .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

            if manual_url.trim().is_empty() {
                return Err(kto::KtoError::ConfigError("No URL provided".into()));
            }

            // Validate URL
            let url = if manual_url.starts_with("http://") || manual_url.starts_with("https://") {
                manual_url.trim().to_string()
            } else {
                format!("https://{}", manual_url.trim())
            };

            if url::Url::parse(&url).is_err() {
                return Err(kto::KtoError::ConfigError(format!("Invalid URL: {}", url)));
            }

            println!("  Using URL: {}", url);
            create_watch_from_discovery(
                db,
                &UrlDiscoveryResult {
                    url,
                    alternatives: vec![],
                    engine: "http".into(),
                    extraction_strategy: "auto".into(),
                    selector: None,
                    suggested_name: discovery.suggested_name.clone(),
                    agent_instructions: discovery.agent_instructions.clone(),
                    interval_secs: interval,
                    reasoning: String::new(),
                    confidence: 1.0,
                    queries_made: vec![],
                },
                name_override,
                interval,
                tags,
                use_profile,
                yes,
                interactive,
            )
        }
        DiscoveryAction::Cancel => {
            Err(kto::KtoError::ConfigError("Watch creation cancelled".into()))
        }
    }
}

/// Actions the user can take after seeing discovery results
enum DiscoveryAction {
    Accept,
    UseAlternative(usize),
    DeepResearch,
    ManualUrl,
    Cancel,
}

/// Display discovery results and get user confirmation
fn display_discovery_results(
    discovery: &UrlDiscoveryResult,
    yes: bool,
    interactive: bool,
) -> Result<DiscoveryAction> {
    // Extract host for security transparency
    let host = url::Url::parse(&discovery.url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    // Confidence indicator
    let confidence_str = if discovery.confidence >= 0.8 {
        format!("{:.0}%", discovery.confidence * 100.0).green().to_string()
    } else if discovery.confidence >= 0.5 {
        format!("{:.0}%", discovery.confidence * 100.0).yellow().to_string()
    } else {
        format!("{:.0}%", discovery.confidence * 100.0).red().to_string()
    };

    println!();
    println!("  {} Found URL on {}", "->".green(), host.cyan().bold());
    println!("     {}", discovery.url);
    println!();

    if !discovery.suggested_name.is_empty() {
        println!("  Name:       {}", discovery.suggested_name);
    }
    if !discovery.agent_instructions.is_empty() {
        println!("  AI Watch:   \"{}\"", truncate_str(&discovery.agent_instructions, 60));
    }
    println!("  Confidence: {}", confidence_str);
    println!();

    // Auto-accept in --yes mode
    if yes {
        println!("  Auto-accepting (--yes mode, confidence {:.0}%)", discovery.confidence * 100.0);
        return Ok(DiscoveryAction::Accept);
    }

    if !interactive {
        return Ok(DiscoveryAction::Accept);
    }

    // Build choice list
    let mut choices = vec!["Create Watch"];

    let has_alternatives = !discovery.alternatives.is_empty();
    if has_alternatives {
        choices.push("Use Alternative URL");
    }

    choices.push("Show Details");
    choices.push("Deep Research");
    choices.push("Enter URL Manually");
    choices.push("Cancel");

    loop {
        let choice = Select::new("What would you like to do?", choices.clone())
            .prompt()
            .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

        match choice {
            "Create Watch" => return Ok(DiscoveryAction::Accept),
            "Use Alternative URL" => {
                let alt_labels: Vec<String> = discovery.alternatives.iter()
                    .enumerate()
                    .map(|(i, alt)| {
                        let desc = if alt.description.is_empty() {
                            String::new()
                        } else {
                            format!(" - {}", alt.description)
                        };
                        format!("{}. {}{}", i + 1, alt.url, desc)
                    })
                    .collect();

                let selected = Select::new("Select alternative:", alt_labels)
                    .prompt()
                    .map_err(|e| kto::KtoError::ConfigError(e.to_string()))?;

                let idx = selected.split('.').next()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .map(|n| n - 1)
                    .unwrap_or(0);

                return Ok(DiscoveryAction::UseAlternative(idx));
            }
            "Show Details" => {
                println!();
                println!("  {}", "Details".bold().underline());
                println!("  Engine:     {}", discovery.engine);
                println!("  Extraction: {}", discovery.extraction_strategy);
                if let Some(ref sel) = discovery.selector {
                    println!("  Selector:   {}", sel);
                }
                println!("  Interval:   {}", format_interval(discovery.interval_secs));
                if !discovery.reasoning.is_empty() {
                    println!("  Reasoning:  {}", discovery.reasoning);
                }
                if !discovery.queries_made.is_empty() {
                    println!("  Searches:   {}", discovery.queries_made.join(", "));
                }
                if !discovery.alternatives.is_empty() {
                    println!("  Alternatives:");
                    for alt in &discovery.alternatives {
                        let eng = if alt.engine.is_empty() { "" } else { &alt.engine };
                        println!("    {} ({}) {}", alt.url, eng, alt.description.dimmed());
                    }
                }
                println!();
                // Continue loop to show choices again
            }
            "Deep Research" => return Ok(DiscoveryAction::DeepResearch),
            "Enter URL Manually" => return Ok(DiscoveryAction::ManualUrl),
            "Cancel" | _ => return Ok(DiscoveryAction::Cancel),
        }
    }
}

/// Create a watch from a URL discovery result with preflight validation
fn create_watch_from_discovery(
    db: &Database,
    discovery: &UrlDiscoveryResult,
    name_override: Option<String>,
    default_interval: u64,
    tags: Vec<String>,
    use_profile: bool,
    yes: bool,
    interactive: bool,
) -> Result<()> {
    let engine = discovery.to_engine();
    let extraction = discovery.to_extraction();
    let url = &discovery.url;
    let interval = if discovery.interval_secs >= 10 {
        discovery.interval_secs
    } else {
        default_interval
    };

    // Preflight: fetch and validate the discovered URL
    println!("\n  Fetching {}...", url);
    let content = match fetch::fetch(url, engine.clone(), &std::collections::HashMap::new()) {
        Ok(c) => c,
        Err(e) => {
            // Try alternatives
            let mut last_err = e;
            let mut found = None;
            for alt in &discovery.alternatives {
                let alt_engine = match alt.engine.to_lowercase().as_str() {
                    "rss" | "atom" => Engine::Rss,
                    "playwright" | "js" => Engine::Playwright,
                    _ => Engine::Http,
                };
                println!("  Primary URL failed, trying alternative: {}", alt.url);
                match fetch::fetch(&alt.url, alt_engine, &std::collections::HashMap::new()) {
                    Ok(c) => {
                        found = Some((alt.url.clone(), c));
                        break;
                    }
                    Err(e2) => {
                        last_err = e2;
                    }
                }
            }

            if let Some((_alt_url, c)) = found {
                c
            } else {
                let msg = platform_detect::friendly_error_message(&last_err.to_string(), url);
                return Err(kto::KtoError::ConfigError(format!(
                    "Could not fetch discovered URL: {}\n  \
                     Try providing a URL directly or use --research for thorough analysis.",
                    msg
                )));
            }
        }
    };

    // Validate extraction produces meaningful content
    let extracted = match extract::extract(&content, &extraction) {
        Ok(e) => e,
        Err(_) => {
            // Fall back to auto extraction
            match extract::extract(&content, &Extraction::Auto) {
                Ok(e) => e,
                Err(e) => {
                    return Err(kto::KtoError::ConfigError(format!(
                        "Could not extract content from {}: {}",
                        url, e
                    )));
                }
            }
        }
    };

    if extracted.len() < 50 {
        if !yes {
            println!("  {} Extracted content is short ({} chars).", "Warning:".yellow(), extracted.len());
            if interactive {
                let proceed = Confirm::new("Content looks thin. Proceed anyway?")
                    .with_default(true)
                    .prompt()
                    .unwrap_or(true);
                if !proceed {
                    return Err(kto::KtoError::ConfigError("Watch creation cancelled".into()));
                }
            }
        }
    }

    // Determine name
    let name = name_override.unwrap_or_else(|| {
        if !discovery.suggested_name.is_empty() {
            discovery.suggested_name.clone()
        } else if let Ok(parsed) = url::Url::parse(url) {
            generate_name_from_url(&parsed)
        } else {
            "Watch".to_string()
        }
    });

    // Create watch
    let mut watch = Watch::new(name.clone(), url.to_string());
    watch.interval_secs = interval.max(10);
    watch.engine = engine;
    watch.extraction = extraction;
    watch.tags = tags;
    watch.use_profile = use_profile;

    // Always enable agent for discovery-based watches
    if !discovery.agent_instructions.is_empty() {
        watch.agent_config = Some(AgentConfig {
            enabled: true,
            prompt_template: None,
            instructions: Some(discovery.agent_instructions.clone()),
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

    // User-friendly success output
    let has_agent = watch.agent_config.is_some();
    let agent_instructions = watch.agent_config.as_ref().and_then(|c| c.instructions.as_deref());
    let intent_description = platform_detect::describe_watch_intent(
        &watch.engine,
        has_agent,
        agent_instructions,
    );

    let success_msg = platform_detect::format_watch_created(
        &name,
        &intent_description,
        watch.interval_secs,
        has_agent,
    );
    println!("{}", success_msg);

    if !watch.tags.is_empty() {
        println!("   Tags: {}", watch.tags.join(", "));
    }

    // Prompt for notification setup if not configured and interactive
    let mut config = Config::load()?;
    if config.default_notify.is_none() && interactive && !yes {
        println!();
        if let Some(target) = prompt_notification_setup()? {
            config.default_notify = Some(target);
            config.save()?;
            println!("  Notification settings saved.");
        }
    }

    if !is_daemon_running() {
        println!("\n  Run `kto daemon` to start monitoring.");
    }

    Ok(())
}
