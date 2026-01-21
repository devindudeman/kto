//! Miscellaneous commands: doctor, export, import, memory, diff, logs, ui

use colored::Colorize;
use std::time::Duration;

use kto::agent;
use kto::config::Config;
use kto::db::Database;
use kto::fetch::{self, check_playwright, PlaywrightStatus};
use kto::watch::Change;
use kto::error::Result;

/// Interactive TUI dashboard
#[cfg(feature = "tui")]
pub fn cmd_ui() -> Result<()> {
    kto::tui::run()
}

#[cfg(not(feature = "tui"))]
pub fn cmd_ui() -> Result<()> {
    eprintln!("TUI not available. Rebuild with: cargo build --features tui");
    Ok(())
}

/// Export watches to JSON
pub fn cmd_export(watch: Option<String>) -> Result<()> {
    let db = Database::open()?;

    let watches = if let Some(id_or_name) = watch {
        let watch = db.get_watch(&id_or_name)?
            .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.clone()))?;
        vec![watch]
    } else {
        db.list_watches()?
    };

    let json = serde_json::to_string_pretty(&watches)?;
    println!("{}", json);
    Ok(())
}

/// Import watches from JSON
pub fn cmd_import(dry_run: bool) -> Result<()> {
    use std::io::{self, Read};

    // Read JSON from stdin
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    let watches: Vec<kto::watch::Watch> = serde_json::from_str(&input)
        .map_err(|e| kto::KtoError::ConfigError(format!("Invalid JSON: {}", e)))?;

    if watches.is_empty() {
        println!("No watches to import.");
        return Ok(());
    }

    let db = Database::open()?;
    let existing_names: std::collections::HashSet<String> = db.list_watches()?
        .into_iter()
        .map(|w| w.name)
        .collect();

    println!("\n{} watch(es) to import:\n", watches.len());
    for watch in &watches {
        let status = if existing_names.contains(&watch.name) {
            "SKIP (exists)"
        } else {
            "NEW"
        };
        println!("  [{}] {} - {}", status, watch.name, watch.url);
    }

    if dry_run {
        println!("\n(dry-run mode - no changes made)");
        return Ok(());
    }

    let mut imported = 0;
    let mut skipped = 0;

    for watch in watches {
        if existing_names.contains(&watch.name) {
            skipped += 1;
            continue;
        }

        // Create a new watch with a fresh ID
        let mut new_watch = watch;
        new_watch.id = uuid::Uuid::new_v4();
        new_watch.created_at = chrono::Utc::now();

        db.insert_watch(&new_watch)?;
        imported += 1;
    }

    println!("\nImported {} watch(es), skipped {} (already exist).", imported, skipped);
    Ok(())
}

/// Show the latest diff for a watch
pub fn cmd_diff(id_or_name: &str, limit: usize) -> Result<()> {
    let db = Database::open()?;
    let watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    let changes = db.get_recent_changes(&watch.id, limit)?;

    if changes.is_empty() {
        println!("\nNo changes recorded for '{}'.", watch.name);
        println!("Run `kto test \"{}\"` to check for changes.", watch.name);
        return Ok(());
    }

    println!("\nRecent changes for '{}':\n", watch.name);

    for (i, change) in changes.iter().enumerate() {
        // Format time ago
        let ago = chrono::Utc::now().signed_duration_since(change.detected_at);
        let time_ago = if ago.num_seconds() < 60 {
            format!("{}s ago", ago.num_seconds())
        } else if ago.num_minutes() < 60 {
            format!("{}m ago", ago.num_minutes())
        } else if ago.num_hours() < 24 {
            format!("{}h ago", ago.num_hours())
        } else {
            format!("{}d ago", ago.num_days())
        };

        println!("{}. {} ({})", i + 1, change.detected_at.format("%Y-%m-%d %H:%M"), time_ago);

        if let Some(ref resp) = change.agent_response {
            if let Some(summary) = resp.get("summary").and_then(|s: &serde_json::Value| s.as_str()) {
                println!("   AI: {}", summary.cyan());
            }
        }

        // Show colored diff
        for line in change.diff.lines().take(20) {
            if line.starts_with('+') && !line.starts_with("+++") {
                println!("   {}", line.green());
            } else if line.starts_with('-') && !line.starts_with("---") {
                println!("   {}", line.red());
            } else if line.starts_with('@') {
                println!("   {}", line.cyan());
            } else {
                println!("   {}", line);
            }
        }

        if change.diff.lines().count() > 20 {
            println!("   ... ({} more lines)", change.diff.lines().count() - 20);
        }
        println!();
    }

    Ok(())
}

/// View or manage AI agent memory for a watch
pub fn cmd_memory(id_or_name: &str, json: bool, clear: bool) -> Result<()> {
    let db = Database::open()?;
    let watch = db.get_watch(id_or_name)?
        .ok_or_else(|| kto::KtoError::WatchNotFound(id_or_name.to_string()))?;

    if clear {
        // Clear memory
        db.clear_agent_memory(&watch.id)?;
        println!("Cleared AI memory for '{}'.", watch.name);
        return Ok(());
    }

    let memory = db.get_agent_memory(&watch.id)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&memory)?);
        return Ok(());
    }

    println!("\nAI Memory for '{}':\n", watch.name);

    // Counters
    if memory.counters.is_empty() {
        println!("  Counters: (none)");
    } else {
        println!("  {}:", "Counters".bold());
        for (key, value) in &memory.counters {
            println!("    {}: {}", key, value.to_string().cyan());
        }
    }
    println!();

    // Last Values
    if memory.last_values.is_empty() {
        println!("  Last Values: (none)");
    } else {
        println!("  {}:", "Last Values".bold());
        for (key, value) in &memory.last_values {
            let display = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => value.to_string(),
            };
            println!("    {}: {}", key, display.green());
        }
    }
    println!();

    // Notes
    if memory.notes.is_empty() {
        println!("  Notes: (none)");
    } else {
        println!("  {}:", "Notes".bold());
        for note in &memory.notes {
            println!("    • {}", note.yellow());
        }
    }

    // Agent config info
    if let Some(ref config) = watch.agent_config {
        println!();
        println!("  {}:", "Agent Config".bold());
        println!("    Enabled: {}", if config.enabled { "yes".green() } else { "no".red() });
        if let Some(ref inst) = config.instructions {
            println!("    Intent: \"{}\"", inst);
        }
    }

    println!();
    println!("  Tip: Use 'kto memory \"{}\" --clear' to reset memory.", watch.name);

    Ok(())
}

/// Tail activity log
pub fn cmd_logs(lines: usize, follow: bool) -> Result<()> {
    let db = Database::open()?;

    // Initial display
    let changes = db.get_all_recent_changes(lines)?;

    if changes.is_empty() {
        println!("No changes recorded yet.");
        if !follow {
            return Ok(());
        }
    } else {
        println!("\nRecent changes:\n");
        for (change, watch_name) in &changes {
            print_change_log(change, watch_name);
        }
    }

    if follow {
        println!("\nWatching for new changes... (Ctrl+C to stop)\n");

        let mut last_seen = changes.first().map(|(c, _)| c.detected_at);

        loop {
            std::thread::sleep(Duration::from_secs(2));

            let new_changes = db.get_all_recent_changes(10)?;

            for (change, watch_name) in new_changes {
                if let Some(last) = last_seen {
                    if change.detected_at <= last {
                        continue;
                    }
                }
                print_change_log(&change, &watch_name);
                last_seen = Some(change.detected_at);
            }
        }
    }

    Ok(())
}

fn print_change_log(change: &Change, watch_name: &str) {
    let time = change.detected_at.format("%Y-%m-%d %H:%M:%S");
    let status = if change.notified { "notified" } else { "silent" };
    let filter = if change.filter_passed { "pass" } else { "skip" };

    // Truncate diff for display
    let diff_preview: String = change.diff
        .chars()
        .take(60)
        .collect::<String>()
        .replace('\n', " ");

    println!("  {} | {} | {} | {} | {}...",
             time, watch_name, filter, status, diff_preview.trim());
}

/// Check all dependencies and suggest fixes
pub fn cmd_doctor() -> Result<()> {
    println!("\nkto doctor\n");

    // Check kto itself
    println!("  kto binary: v{}", env!("CARGO_PKG_VERSION"));

    // Check Claude CLI
    match agent::claude_version() {
        Some(v) => println!("  Claude CLI: {} (installed)", v),
        None => println!("  Claude CLI: NOT INSTALLED"),
    }

    // Check Node.js
    let node = std::process::Command::new("node")
        .arg("--version")
        .output();
    match node {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout);
            println!("  Node.js: {}", v.trim());
        }
        _ => println!("  Node.js: NOT INSTALLED"),
    }

    // Check Playwright
    match check_playwright() {
        PlaywrightStatus::Ready => println!("  Playwright: ready"),
        PlaywrightStatus::NodeMissing => println!("  Playwright: Node.js required"),
        PlaywrightStatus::PlaywrightMissing => println!("  Playwright: not installed"),
        PlaywrightStatus::BrowserMissing => println!("  Playwright: browser not installed"),
    }

    // Check database
    match Database::open() {
        Ok(_) => println!("  Database: OK"),
        Err(e) => println!("  Database: ERROR - {}", e),
    }

    println!();
    Ok(())
}

/// Set up Playwright/Chromium for JS rendering
pub fn cmd_enable_js() -> Result<()> {
    println!("\nSetting up JavaScript rendering...\n");

    // Check if Node.js is available
    let node_available = std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !node_available {
        println!("  Node.js is required for JavaScript rendering.");
        println!("  Install from: https://nodejs.org/");
        return Ok(());
    }

    let data_dir = Config::data_dir()?;
    std::fs::create_dir_all(&data_dir)?;

    // Check if playwright is already installed locally
    let node_modules = data_dir.join("node_modules").join("playwright");
    let needs_install = !node_modules.exists();

    if needs_install {
        // Create package.json if it doesn't exist
        let package_json = data_dir.join("package.json");
        if !package_json.exists() {
            println!("  Initializing kto JavaScript environment...");
            let output = std::process::Command::new("npm")
                .args(["init", "-y"])
                .current_dir(&data_dir)
                .output()?;

            if !output.status.success() {
                return Err(kto::KtoError::PlaywrightError("Failed to initialize npm".into()));
            }
        }

        // Install playwright locally in the data directory
        println!("  Installing Playwright...");
        let output = std::process::Command::new("npm")
            .args(["install", "playwright"])
            .current_dir(&data_dir)
            .status()?;

        if !output.success() {
            return Err(kto::KtoError::PlaywrightError("Failed to install Playwright".into()));
        }
    } else {
        println!("  Playwright package is installed.");
    }

    // Check if Chromium browser is installed
    let status = check_playwright();
    if !status.is_ready() {
        println!("  Installing Chromium browser (~280MB)...");
        let output = std::process::Command::new("npx")
            .args(["playwright", "install", "chromium"])
            .current_dir(&data_dir)
            .status()?;

        if !output.success() {
            return Err(kto::KtoError::PlaywrightError("Failed to install Chromium".into()));
        }
    } else {
        println!("  Chromium browser is installed.");
    }

    // Install system dependencies (requires sudo for system packages)
    // Check if we're in an interactive terminal
    if atty::is(atty::Stream::Stdin) {
        println!("  Installing system dependencies for Chromium...");
        println!("  (This requires sudo - you may be prompted for your password)\n");

        // Run sudo with inherited stdio so user can enter password interactively
        let deps_result = std::process::Command::new("sudo")
            .args(["npx", "playwright", "install-deps", "chromium"])
            .current_dir(&data_dir)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status();

        match deps_result {
            Ok(status) if status.success() => {
                println!("\n  System dependencies installed.");
            }
            Ok(_) => {
                return Err(kto::KtoError::PlaywrightError(
                    "Failed to install system dependencies".into()
                ));
            }
            Err(e) => {
                return Err(kto::KtoError::PlaywrightError(
                    format!("Failed to run sudo: {}", e)
                ));
            }
        }
    } else {
        // Non-interactive mode - can't prompt for sudo password
        return Err(kto::KtoError::PlaywrightError(
            "Cannot install system dependencies in non-interactive mode. Please run `kto enable-js` in a terminal.".into()
        ));
    }

    // Ensure render script is in place
    fetch::ensure_render_script()?;

    println!("\n  JavaScript rendering is now enabled.");
    println!("  Create a watch with: kto new \"https://...\" --js");
    Ok(())
}
