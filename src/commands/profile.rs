//! Profile commands: show, edit, setup, clear, forget, infer, preview

use std::process::Command;

use colored::Colorize;

use kto::db::Database;
use kto::error::{KtoError, Result};
use kto::interests::{InferredInterests, Interest, InterestProfile, InterestScope, ProfileDescription};

/// Show the current interest profile
pub fn cmd_profile_show(json: bool) -> Result<()> {
    let profile = InterestProfile::load()?;
    let db = Database::open()?;
    let global_memory = db.get_global_memory()?;

    if json {
        let output = serde_json::json!({
            "profile": profile,
            "global_memory": global_memory,
            "path": InterestProfile::profile_path()?.to_string_lossy(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let path = InterestProfile::profile_path()?;
    println!("\n{}\n", "Interest Profile".cyan().bold());
    println!("  Path: {}\n", path.display());

    if profile.is_empty() {
        println!("  {}", "(No profile configured)".dimmed());
        println!("\n  Run {} to create one.", "kto profile edit".yellow());
        println!("  Or run {} to infer from your watches.\n", "kto profile infer".yellow());
    } else {
        // Show description
        if !profile.profile.description.trim().is_empty() {
            println!("  {}", "Description:".bold());
            for line in profile.profile.description.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    println!("    {}", trimmed);
                }
            }
            println!();
        }

        // Show interests
        if !profile.interests.is_empty() {
            println!("  {}", "Interests:".bold());
            for interest in &profile.interests {
                let scope = match interest.scope {
                    InterestScope::Broad => "broad",
                    InterestScope::Narrow => "narrow",
                };
                println!("    {} (weight: {:.1}, {})",
                         interest.name.green(),
                         interest.weight,
                         scope);
                if !interest.keywords.is_empty() {
                    println!("      Keywords: {}", interest.keywords.join(", ").dimmed());
                }
                if !interest.sources.is_empty() {
                    println!("      Sources: {}", interest.sources.join(", ").dimmed());
                }
            }
            println!();
        }
    }

    // Show global memory
    if !global_memory.is_empty() {
        println!("  {}", "Learned Patterns:".bold());
        for obs in global_memory.observations.iter().take(5) {
            println!("    - {} (from {}, confidence: {:.1})",
                     obs.text, obs.source_watch, obs.confidence);
        }
        if global_memory.observations.len() > 5 {
            println!("    ... and {} more", global_memory.observations.len() - 5);
        }
        println!();

        if !global_memory.interest_signals.is_empty() {
            println!("  {}", "Inferred Interest Signals:".bold());
            let mut signals: Vec<_> = global_memory.interest_signals.iter().collect();
            signals.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (topic, score) in signals.iter().take(5) {
                println!("    - {}: {:.2}", topic, score);
            }
            println!();
        }
    }

    Ok(())
}

/// Open the profile in $EDITOR
pub fn cmd_profile_edit() -> Result<()> {
    let path = InterestProfile::profile_path()?;

    // Create parent directory if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Create template if file doesn't exist
    if !path.exists() {
        let template = InterestProfile::template();
        template.save()?;
        println!("Created new profile at: {}", path.display());
    }

    // Get editor from environment
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "nano".to_string());

    println!("Opening {} with {}...", path.display(), editor);

    let status = Command::new(&editor)
        .arg(&path)
        .status()?;

    if status.success() {
        // Validate the edited file
        match InterestProfile::load() {
            Ok(profile) => {
                println!("\n{} Profile saved.", "✓".green());
                if !profile.interests.is_empty() {
                    println!("  {} interests configured.", profile.interests.len());
                }
            }
            Err(e) => {
                eprintln!("\n{} Profile has errors: {}", "✗".red(), e);
                eprintln!("  Run {} to fix.", "kto profile edit".yellow());
            }
        }
    }

    Ok(())
}

/// Interactive guided setup
pub fn cmd_profile_setup() -> Result<()> {
    use inquire::{MultiSelect, Text};

    println!("\n{}\n", "Interest Profile Setup".cyan().bold());
    println!("This helps kto's AI understand what changes matter to you.\n");

    // Get description
    let description = Text::new("Describe yourself and what you're interested in (optional):")
        .with_help_message("E.g., 'I'm a developer interested in Rust, AI, and startups'")
        .prompt_skippable()
        .map_err(|e| KtoError::ConfigError(e.to_string()))?
        .unwrap_or_default();

    // Suggest some common interests
    let common_interests = vec![
        "Technology & Programming",
        "AI/Machine Learning",
        "Rust",
        "JavaScript/TypeScript",
        "Python",
        "Startups & Business",
        "Finance & Markets",
        "Gaming",
        "Science",
        "Security",
    ];

    let selected = MultiSelect::new("Select interests that apply to you:", common_interests)
        .with_help_message("Use space to select, enter to confirm")
        .prompt()
        .map_err(|e| KtoError::ConfigError(e.to_string()))?;

    // Build the profile
    let mut profile = InterestProfile {
        profile: ProfileDescription { description },
        interests: Vec::new(),
    };

    // Add selected interests with default keywords
    for interest_name in selected {
        let keywords = match interest_name {
            "Technology & Programming" => vec!["software", "tech", "programming", "code"],
            "AI/Machine Learning" => vec!["ai", "ml", "llm", "gpt", "claude", "machine learning"],
            "Rust" => vec!["rust", "cargo", "tokio", "async rust"],
            "JavaScript/TypeScript" => vec!["javascript", "typescript", "node", "react", "vue"],
            "Python" => vec!["python", "pip", "django", "flask"],
            "Startups & Business" => vec!["startup", "funding", "vc", "business"],
            "Finance & Markets" => vec!["finance", "stocks", "crypto", "market"],
            "Gaming" => vec!["game", "gaming", "steam", "release"],
            "Science" => vec!["science", "research", "study", "discovery"],
            "Security" => vec!["security", "vulnerability", "cve", "exploit"],
            _ => vec![],
        };

        profile.interests.push(Interest {
            name: interest_name.to_string(),
            keywords: keywords.into_iter().map(String::from).collect(),
            weight: 0.7,
            scope: InterestScope::Broad,
            sources: vec![],
        });
    }

    // Save
    profile.save()?;
    let path = InterestProfile::profile_path()?;

    println!("\n{} Profile saved to: {}", "✓".green(), path.display());
    println!("\n  {} interests configured.", profile.interests.len());
    println!("\n  Tip: Enable profile on watches with: {}", "kto edit <watch> --use-profile".yellow());

    Ok(())
}

/// Clear the profile (delete interests.toml)
pub fn cmd_profile_clear(yes: bool) -> Result<()> {
    use inquire::Confirm;

    let path = InterestProfile::profile_path()?;

    if !path.exists() {
        println!("No profile to clear.");
        return Ok(());
    }

    if !yes {
        let confirmed = Confirm::new("Clear your interest profile?")
            .with_default(false)
            .prompt()
            .map_err(|e| KtoError::ConfigError(e.to_string()))?;

        if !confirmed {
            println!("Cancelled.");
            return Ok(());
        }
    }

    std::fs::remove_file(&path)?;
    println!("{} Profile cleared.", "✓".green());

    Ok(())
}

/// Forget learned patterns (clear global memory)
pub fn cmd_profile_forget(learned_only: bool, yes: bool) -> Result<()> {
    use inquire::Confirm;

    let db = Database::open()?;

    if !yes {
        let msg = if learned_only {
            "Clear all learned patterns? (keeps static profile)"
        } else {
            "Clear all learned patterns?"
        };

        let confirmed = Confirm::new(msg)
            .with_default(false)
            .prompt()
            .map_err(|e| KtoError::ConfigError(e.to_string()))?;

        if !confirmed {
            println!("Cancelled.");
            return Ok(());
        }
    }

    db.clear_global_memory()?;
    println!("{} Learned patterns cleared.", "✓".green());

    Ok(())
}

/// Infer interests from existing watches
pub fn cmd_profile_infer(yes: bool) -> Result<()> {
    use inquire::Confirm;

    println!("\n{}\n", "Inferring Interests from Watches".cyan().bold());

    let db = Database::open()?;
    let watches = db.list_watches()?;

    if watches.is_empty() {
        println!("No watches found. Create some watches first with {}.", "kto new".yellow());
        return Ok(());
    }

    println!("Analyzing {} watches...\n", watches.len());

    // Build context for AI
    let mut watch_summaries = Vec::new();
    for watch in &watches {
        // Get latest snapshot content
        let content_preview = if let Ok(Some(snapshot)) = db.get_latest_snapshot(&watch.id) {
            let preview: String = snapshot.extracted.chars().take(500).collect();
            preview
        } else {
            "(no content yet)".to_string()
        };

        watch_summaries.push(format!(
            "\"{}\" - {}\nContent sample: {}",
            watch.name, watch.url, content_preview
        ));
    }

    // Call AI to infer interests
    let inference_prompt = format!(
        r#"Analyze these watch targets to infer what topics the user is interested in.

IMPORTANT: Do NOT fetch any URLs. All the information you need is provided below.

WATCH TARGETS AND CONTENT:
{}

Based on the watch names, URLs, and content samples above, what topics/interests does this user care about?

Respond ONLY with JSON (no markdown, no code fences):
{{
  "interests": [
    {{
      "name": "Interest name",
      "keywords": ["keyword1", "keyword2"],
      "weight": 0.8,
      "scope": "broad",
      "sources": ["Watch Name 1", "Watch Name 2"]
    }}
  ],
  "confidence": 0.85,
  "reasoning": "Brief explanation"
}}

Guidelines:
- Infer 3-7 interests based on patterns in the URLs and content
- weight: 0.0-1.0 based on how strongly this interest is indicated
- scope: "broad" for general topics, "narrow" for specific technologies
- sources: which watches suggest this interest
- Be specific: "Rust programming" not just "programming"
"#,
        watch_summaries.join("\n\n---\n\n")
    );

    let system_prompt = "You are a user interest analyzer. Respond only with valid JSON. Do not use any tools - just analyze the provided content.";

    // Ensure workspace directory exists
    std::fs::create_dir_all("/tmp/kto-workspace")?;

    let output = Command::new("claude")
        .current_dir("/tmp/kto-workspace")
        .args([
            "-p",
            "--output-format", "json",
            "--max-turns", "1",
            "--system-prompt", system_prompt,
            &inference_prompt,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KtoError::ClaudeFailed(stderr.to_string()));
    }

    // Parse response
    let stdout = String::from_utf8_lossy(&output.stdout);
    let claude_response: serde_json::Value = serde_json::from_str(&stdout)?;

    // Check for errors (max_turns, etc.)
    if claude_response["subtype"].as_str() == Some("error_max_turns") {
        return Err(KtoError::ClaudeFailed(
            "Claude hit max turns limit. Try running with fewer watches or more content in snapshots.".into()
        ));
    }

    let result_text = claude_response["result"]
        .as_str()
        .ok_or_else(|| {
            // Provide more context about what went wrong
            let subtype = claude_response["subtype"].as_str().unwrap_or("unknown");
            KtoError::ClaudeFailed(format!("No result in response (subtype: {})", subtype))
        })?;

    // Strip code fencing if present
    let json_text = result_text.trim()
        .strip_prefix("```json").unwrap_or(result_text.trim())
        .strip_prefix("```").unwrap_or(result_text.trim())
        .strip_suffix("```").unwrap_or(result_text.trim())
        .trim();

    let inferred: InferredInterests = serde_json::from_str(json_text)
        .map_err(|e| KtoError::ClaudeFailed(format!("Failed to parse inference: {}", e)))?;

    // Display results
    println!("Based on what you're keeping tabs on, you seem interested in:\n");

    for (i, interest) in inferred.interests.iter().enumerate() {
        let scope = match interest.scope {
            InterestScope::Broad => "broad",
            InterestScope::Narrow => "narrow",
        };
        println!("{}. {} (weight: {:.1}, {})",
                 i + 1,
                 interest.name.green().bold(),
                 interest.weight,
                 scope);
        if !interest.sources.is_empty() {
            println!("   Sources: {}", interest.sources.join(", ").dimmed());
        }
        if !interest.keywords.is_empty() {
            println!("   Keywords: {}", interest.keywords.join(", "));
        }
        println!();
    }

    if let Some(ref reasoning) = inferred.reasoning {
        println!("{}: {}\n", "Analysis".bold(), reasoning);
    }

    println!("Overall confidence: {:.0}%\n", inferred.confidence * 100.0);

    // Confirm save
    let should_save = if yes {
        true
    } else {
        Confirm::new("Save these to your interest profile?")
            .with_default(true)
            .prompt()
            .map_err(|e| KtoError::ConfigError(e.to_string()))?
    };

    if should_save {
        // Load existing profile or create new
        let mut profile = InterestProfile::load().unwrap_or_default();

        // Add inferred interests
        for interest in inferred.interests {
            // Check if interest with same name already exists
            if !profile.interests.iter().any(|i| i.name == interest.name) {
                profile.interests.push(interest);
            }
        }

        profile.save()?;
        let path = InterestProfile::profile_path()?;
        println!("\n{} Profile saved to: {}", "✓".green(), path.display());
        println!("\n  Tip: Enable profile on watches with: {}", "kto edit <watch> --use-profile".yellow());
    } else {
        println!("Not saved. Run {} to edit manually.", "kto profile edit".yellow());
    }

    Ok(())
}

/// Preview what would be sent to AI for a specific watch
pub fn cmd_profile_preview(watch_name: &str) -> Result<()> {
    let db = Database::open()?;
    let watch = db.get_watch(watch_name)?
        .ok_or_else(|| KtoError::WatchNotFound(watch_name.to_string()))?;

    println!("\n{} {}\n", "Profile Preview for:".cyan().bold(), watch.name);

    // Check if profile is enabled
    if !watch.use_profile {
        println!("  {}", "Profile is NOT enabled for this watch.".yellow());
        println!("  Enable with: {}\n", format!("kto edit \"{}\" --use-profile", watch.name).dimmed());
    }

    // Load profile
    let profile = InterestProfile::load()?;
    let global_memory = db.get_global_memory()?;

    if profile.is_empty() && global_memory.is_empty() {
        println!("  No profile or learned patterns configured.");
        println!("  Run {} to create a profile.\n", "kto profile edit".yellow());
        return Ok(());
    }

    // Show what would be included in prompt
    println!("{}", "─".repeat(60).dimmed());

    if !profile.is_empty() {
        println!("{}", profile.to_prompt_section());
        println!();
    }

    if !global_memory.is_empty() {
        println!("{}", global_memory.to_prompt_section());
        println!();
    }

    if watch.use_profile {
        println!("=== PRECEDENCE RULES ===");
        println!("1. Watch-specific instructions ALWAYS take priority");
        println!("2. Profile interests BROADEN what's relevant, never narrow");
        println!("3. If watch says \"only X\", focus on X regardless of profile");
        println!("4. If watch is general, use profile to filter noise");
    }

    println!("{}", "─".repeat(60).dimmed());

    // Show watch instructions
    if let Some(ref agent_config) = watch.agent_config {
        if let Some(ref instructions) = agent_config.instructions {
            println!("\n{}", "Watch-specific instructions:".bold());
            println!("  {}", instructions);
        }
    }

    println!();
    Ok(())
}
