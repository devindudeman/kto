# CLAUDE.md - kto Project Context

## Project Overview
kto is a universal web change detector CLI tool written in Rust. It monitors any web page for changes and uses AI (Claude CLI) as the intelligence layer to understand and summarize what changed.

## Design Philosophy

**Core principle:** Detect change → Let AI understand it → Send useful notification.

- **AI is the intelligence layer.** Don't parse prices with regex. Let Claude understand context.
- **Universal by default.** Works on ANY website without domain-specific configuration.
- **No platform-specific code.** Never add Shopify/Amazon/WooCommerce special cases. AI handles all sites uniformly.
- **Simple data model.** Detect change, store diff, let AI interpret.
- **Graceful degradation.** When AI fails, show structured "what changed" fallback (not garbled diffs).
- **Intent-first workflow.** User provides instructions like "alert me on price drops" - AI decides when to notify.

## Build Commands
```bash
# Standard build
cargo build

# Build with TUI dashboard
cargo build --features tui

# Run tests
cargo test

# Run with features
cargo run --features tui -- <command>
```

## Architecture

### Directory Structure
```
src/
├── main.rs              # CLI entry point (~120 lines, dispatch only)
├── commands/            # Command implementations
│   ├── mod.rs           # Re-exports
│   ├── watch.rs         # new, list, show, edit, delete, pause, resume
│   ├── check.rs         # test, run, daemon, check_watch (core logic)
│   ├── notify.rs        # notify set/show/test
│   ├── remind.rs        # remind new/list/delete/pause/resume
│   ├── profile.rs       # profile show/edit/setup/infer/preview/clear/forget
│   ├── service.rs       # systemd/launchd/cron service management
│   └── misc.rs          # doctor, export, import, memory, diff, logs, ui
├── utils.rs             # Shared utilities (duration parsing, URL extraction)
├── cli.rs               # Clap command definitions
├── db.rs                # SQLite database with refinery migrations
├── watch.rs             # Watch, Snapshot, Change, AgentMemory structs
├── interests.rs         # InterestProfile, Interest, GlobalMemory structs
├── fetch.rs             # HTTP fetching, Playwright JS rendering, shell command execution
├── extract.rs           # Content extraction (auto, selector, RSS, JSON-LD)
├── normalize.rs         # Content normalization and hashing
├── diff.rs              # Text diffing between snapshots
├── filter.rs            # User-defined change filtering rules
├── notify.rs            # Notification dispatch (ntfy, Gotify, Slack, Discord, Telegram, Pushover, Matrix)
├── agent.rs             # Claude CLI integration for AI analysis
├── config.rs            # Global configuration, NotifyTarget
├── tui/                 # Ratatui-based terminal dashboard (feature-gated)
│   ├── mod.rs           # Entry point, event loop, re-exports
│   ├── types.rs         # Enums (Mode, Pane, EditField, etc.)
│   ├── state.rs         # App struct and all *State structs
│   ├── input.rs         # Key/mouse handlers for each mode
│   ├── render.rs        # All render_* functions
│   ├── utils.rs         # Helpers (interval parsing, rect centering)
│   └── editor.rs        # External $EDITOR integration
└── error.rs             # Error types
```

### Core Data Flow (check_watch)
```
1. fetch::fetch()           → Get content (HTTP, Playwright, RSS, or Shell)
2. extract::extract()       → Pull content (auto/selector/RSS/JSON-LD)
3. normalize::normalize()   → Clean text (strip dates, IDs, whitespace)
4. normalize::hash_content() → SHA-256 hash for comparison
5. diff::diff()             → Generate diff if hash changed
6. filter::evaluate_filters() → Check user-defined filter rules
7. agent::analyze_change()  → AI analysis (if enabled)
8. notify::send_notification() → Send to configured target (respects quiet hours)
```

### Database
- SQLite at `~/.local/share/kto/kto.db` (or `$KTO_DB` env var)
- Tables: watches, snapshots, changes, agent_memory, reminders, global_memory
- Migrations in `migrations/` using refinery

### Watch Configuration Options

| Field | Description | Default |
|-------|-------------|---------|
| `name` | Human-readable identifier | Required |
| `url` | URL to monitor (or `shell://command` for shell watches) | Required |
| `engine` | `http`, `playwright`, `rss`, or `shell` | `http` |
| `extraction` | `auto`, `selector`, `full`, `meta`, `rss`, `json_ld` | `auto` |
| `normalization` | Strip whitespace, dates, random IDs | whitespace only |
| `filters` | Rules to filter which changes trigger notifications | none |
| `agent_config` | AI analysis settings (enabled, instructions) | disabled |
| `interval_secs` | Check frequency | 900 (15 min) |
| `headers` | Custom HTTP headers | none |
| `cookie_file` | Path to Netscape-format cookies | none |
| `storage_state` | Playwright session state file | none |
| `notify_target` | Per-watch notification override | global default |
| `tags` | List of tags for organization | empty |
| `use_profile` | Include user's interest profile in AI analysis | `false` |

### AI Agent Configuration

When `agent_config.enabled = true`:
- Claude CLI is invoked with old content, new content, diff, and user instructions
- Agent can use persistent memory (counters, values, notes) across checks
- Agent decides: should we notify? what title/summary?

**Agent instructions examples:**
- "Alert me when the price drops below $50"
- "Only notify on new job postings, ignore updates to existing ones"
- "Track the version number and alert on major version bumps"
- "Summarize new articles, skip if just date changes"

### User Interest Profile

A global user profile that describes what the user cares about, passed to AI agents to help filter relevant changes. This lets kto "get to know you" and alert on things that match your interests.

**Profile location:** `~/.config/kto/interests.toml`

**Profile structure:**
```toml
[profile]
description = """
I'm a software engineer interested in:
- Rust and systems programming
- AI/ML developments
"""

[[interests]]
name = "Rust"
keywords = ["rust", "cargo", "tokio"]
weight = 0.9        # 0.0-1.0, higher = more important
scope = "narrow"    # "broad" or "narrow"
sources = []        # Watches that suggested this (for inferred interests)
```

**Profile commands:**
```bash
kto profile show                    # Display profile and learned patterns
kto profile edit                    # Open in $EDITOR
kto profile setup                   # Interactive guided setup
kto profile infer                   # AI analyzes watches to suggest interests
kto profile preview "My Watch"      # See what AI receives for a watch
kto profile clear                   # Delete profile
kto profile forget --learned        # Clear learned patterns (keep static profile)
```

**Enabling profile on watches:**
```bash
# When creating a watch
kto new "https://news.ycombinator.com" --agent --use-profile

# On existing watch
kto edit "HN" --use-profile true
```

**Precedence rules:**
1. Watch-specific instructions ALWAYS take priority
2. Profile interests BROADEN what's relevant, never narrow
3. If watch says "only X", focus on X regardless of profile
4. If watch is general ("alert on interesting changes"), use profile to filter noise

**Global memory:** AI observations persist across watches in the `global_memory` table. Observations older than 30 days automatically decay. Clear with `kto profile forget`.

## Inspection Commands

kto has three commands for inspecting URLs - each with a distinct purpose:

| Command | Purpose | Requires Existing Watch? | Saves to DB? |
|---------|---------|--------------------------|--------------|
| `preview` | **See what kto extracts** - One-shot fetch to inspect content before creating a watch | No | No |
| `watch` | **Ephemeral monitoring** - Real-time loop until Ctrl+C, shows diffs | No | No |
| `test` | **Test existing watch** - Compare current fetch to last saved snapshot | Yes | No |

### preview - Inspect a URL before monitoring
```bash
# See what kto extracts from a page (auto extraction)
kto preview "https://example.com"

# Try different extraction strategies
kto preview "https://example.com" --selector ".price"
kto preview "https://example.com" --json-ld
kto preview "https://example.com" --full

# Use JavaScript rendering for SPAs
kto preview "https://example.com" --js

# Show more content (default: 2000 chars)
kto preview "https://example.com" --limit 5000
```

### watch - Real-time ephemeral monitoring
```bash
# Watch a URL in real-time without saving to database
kto watch "https://example.com" --interval 5s
kto watch "https://news.ycombinator.com" -i 30s
kto watch "https://example.com" --selector ".price" --js
```

### test - Check an existing watch for changes
```bash
# Manually trigger a check on a saved watch (read-only, no notification)
kto test "My Watch"
kto test "My Watch" --json
```

## Testing & Development

### Test Database Isolation
```bash
KTO_DB=/tmp/test.db kto list --json
```

### Non-Interactive Mode
```bash
kto new "https://example.com" --name test --yes
kto notify set --ntfy my-topic
```

### JSON Output
```bash
kto list --json
kto show <watch> --json
kto test <watch> --json
kto history <watch> --json
```

## Key Dependencies
- **clap** - CLI argument parsing
- **rusqlite** + **refinery** - Database
- **ureq** - HTTP client
- **scraper** - HTML parsing
- **similar** - Text diffing
- **serde** + **serde_json** - Serialization
- **ratatui** + **crossterm** - TUI (feature-gated)
- **inquire** - Interactive prompts
- **chrono** - Date/time handling
- **zstd** - Snapshot compression

## Service Management
The daemon can be installed as a system service:
- Linux: systemd user service (`~/.config/systemd/user/kto.service`)
- macOS: launchd agent (`~/Library/LaunchAgents/com.kto.daemon.plist`)
- Fallback: cron job

## Environment Variables
- `KTO_DB` - Override database path for testing
- `HOME` - Used for config/data directories

## Common Development Tasks

### Adding a new command
1. Add variant to `Commands` enum in `src/cli.rs`
2. Add match arm in `run()` in `src/main.rs`
3. Implement `cmd_<name>()` in appropriate `src/commands/*.rs` module

### Adding a notification target
1. Add variant to `NotifyTarget` in `src/config.rs`
2. Implement sending in `src/notify.rs`
3. Add CLI flags in `NotifyCommands::Set`

### Modifying database schema
1. Add migration file in `migrations/` directory
2. Update relevant structs in `src/watch.rs`
3. Update database methods in `src/db.rs`

### IMPORTANT: Development Workflow
**Before testing changes that affect daemon or TUI:**
```bash
# 1. Kill any running daemon (old code sends wrong notifications!)
ps -ef | grep "[k]to"
kill <daemon_pid>

# 2. Build with TUI feature if testing TUI changes
cargo build --features tui

# 3. Run with the new code
cargo run -- run  # one-shot check
cargo run -- daemon  # start new daemon

# 4. Check notification log for correct formatting
tail -50 ~/.local/share/kto/notifications.log
```

**Why this matters:**
- An old daemon keeps running with old code even after you edit files
- Notifications from old daemon will have old formatting/behavior
- Always kill existing daemons before testing notification changes

## TUI Keybindings

Run with `kto ui`:

| Key | Action |
|-----|--------|
| `j/k` or `↓/↑` | Navigate list |
| `Tab` | Cycle between Watches/Changes/Reminders panes |
| `Enter` | View change details |
| `/` | Search/filter watches |
| `p` | Pause/Resume watch or reminder |
| `t` | Test watch (read-only preview) |
| `c` | Force check (saves snapshot) |
| `n` | New watch/reminder (context-aware wizard) |
| `D` | Describe watch (view full config) |
| `e` | Edit watch or reminder |
| `d` | Delete watch or reminder |
| `L` | Activity logs (all changes across watches) |
| `N` | Notification setup |
| `M` | View agent memory (watches pane) |
| `E` | Show error details for selected watch |
| `r` | Refresh from database |
| `?` | Help |
| `q` | Quit |
| Mouse click | Select watches/changes/reminders, click wizard buttons |

### Change Diff View
| Key | Action |
|-----|--------|
| `j/k` | Scroll up/down |
| `u` | Toggle diff format (inline/unified) |
| `Esc/q` | Close |

### Status Bar
The status bar shows contextual information:
- **Watches pane**: "5 watches (3 active, 2 AI)"
- **Changes pane**: "Watch Name > Change #1" (breadcrumb)
- **Reminders pane**: "3 reminders (2 active)"

### Edit Mode
| Key | Action |
|-----|--------|
| `Tab/j/k` | Navigate fields |
| `Space` | Toggle boolean fields |
| `-/+` | Adjust interval |
| `e` | Open AI instructions in $EDITOR |
| `f` | Manage filters (on Filters field) |
| `T` | Test notification (on Notify field) |
| `Enter` | Save changes |
| `Esc` | Cancel |

### Filter List
| Key | Action |
|-----|--------|
| `n` | Add new filter |
| `e/Enter` | Edit selected filter |
| `d` | Delete filter |
| `j/k` | Navigate |
| `Esc` | Back to edit |

### Memory Inspector
| Key | Action |
|-----|--------|
| `Tab` | Switch sections (Counters/Values/Notes) |
| `j/k` | Navigate items |
| `d` | Delete selected item |
| `C` | Clear all memory |
| `r` | Refresh |
| `Esc` | Close |
