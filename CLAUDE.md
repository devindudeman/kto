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
├── intent.rs            # ParsedIntent, threshold parsing, to_instructions()
├── transforms.rs        # URL transform rules for known sites (GitHub, GitLab, Reddit, etc.)
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

## URL Transform System

kto includes an intent-based URL transform system that detects when users want to monitor well-known sites (GitHub, GitLab, Reddit, etc.) and automatically suggests optimal URLs and engines.

### How It Works

1. **Intent Detection**: kto detects user intent from keywords in the command (e.g., "releases", "news", "price")
2. **Rule Matching**: Rules check if the URL matches a known pattern (e.g., `github.com/*/*`)
3. **Transform Suggestion**: If a match is found, kto suggests a better URL (e.g., `/releases.atom`)
4. **User Choice**: User can accept the suggestion or use the original URL

### Supported Sites

| Site | Intent | Transform | Engine |
|------|--------|-----------|--------|
| github.com | Release | `/releases.atom` | RSS |
| gitlab.com | Release | `/-/releases.atom` | RSS |
| codeberg.org | Release | `/releases.rss` | RSS |
| news.ycombinator.com | News | `/rss` | RSS |
| reddit.com | News | `.rss` | RSS |
| pypi.org | Release | `/rss` | RSS |
| crates.io | Release | `/versions` | HTTP |
| hub.docker.com | Release | `/tags` | Playwright |

### Examples

```bash
# GitHub releases - kto suggests releases.atom feed
$ kto new "https://github.com/astral-sh/ruff for new releases"
  Detected: GitHub releases Atom feed
  URL: https://github.com/astral-sh/ruff/releases.atom
  Engine: RSS

# Reddit subreddit - kto suggests RSS feed
$ kto new "https://reddit.com/r/rust for news"
  Detected: Reddit subreddit RSS feed
  URL: https://reddit.com/r/rust.rss
  Engine: RSS

# Unknown site - falls back to AI analysis
$ kto new "https://example.com/product for price drops"
  Analyzing with AI...
```

### Intent Keywords

| Intent | Keywords |
|--------|----------|
| Release | release, changelog, version, update |
| Price | price, deal, discount, sale, cost, $ |
| Stock | stock, available, availability, back in, restock |
| Jobs | job, career, hiring, position, opening |
| News | news, article, blog, post, feed |

### TUI Integration

In the TUI wizard (`n` key), when using templates like "Changelog/Release Watcher":
1. Select the template
2. Enter a GitHub/GitLab URL
3. kto automatically detects the feed URL
4. Press `Tab` to accept or `x` to use original URL

### Adding New Rules

Rules are defined in `src/transforms.rs` as declarative structs:

```rust
TransformRule {
    host: "github.com",
    path_pattern: Some("*/*"),       // matches /owner/repo
    intent: Intent::Release,
    transform: UrlTransform::AppendPath("/releases.atom"),
    engine: Engine::Rss,
    confidence: 0.95,
    description: "GitHub releases Atom feed",
}
```

## Natural Language URL Discovery

When a user runs `kto new` without a URL, kto uses AI to discover the best URL to monitor based on the user's natural language description. Requires Claude CLI.

### How It Works

```
User input (no URL) → Parse intent (src/intent.rs) → Claude CLI + WebSearch discovers URL
  → Show results with confidence → User confirms → Preflight fetch/extract → Create watch
```

1. `extract_url()` finds no URL in input
2. `ParsedIntent::new()` parses goal, threshold, target item, keywords
3. `agent::discover_url()` calls Claude CLI with WebSearch to research and find the best public URL
4. Results shown with host prominently displayed (security), confidence color-coded
5. User confirms or picks alternative, enters manual URL, or escalates to deep research
6. Preflight fetch validates the URL extracts useful content before creating the watch

### Examples

```bash
# AI discovers CoinGecko API for bitcoin price
kto new "let me know when bitcoin goes above 100k"

# AI finds product page for stock monitoring
kto new "alert me when RTX 5090 is back in stock"

# AI finds GitHub releases feed
kto new "notify me about new Rust releases"

# With --yes: auto-accepts if confidence >= 0.5 AND preflight extraction succeeds
kto new "bitcoin price" --yes
```

### Key Files

| File | Role |
|------|------|
| `src/intent.rs` | `ParsedIntent`, threshold parsing, `to_instructions()` |
| `src/agent.rs` | `UrlDiscoveryResult`, `discover_url()`, `URL_DISCOVERY_PROMPT` |
| `src/commands/watch.rs` | `run_url_discovery_flow()`, `display_discovery_results()`, `create_watch_from_discovery()` |

### Confidence Thresholds

- **Interactive mode**: >= 0.3 to show results (user confirms)
- **`--yes` mode**: >= 0.5 to auto-accept
- **< 0.3**: Treated as discovery failure, offers manual URL entry

### Fallbacks

1. Tries Claude with `--allowedTools WebSearch,WebFetch --max-turns 5` first
2. Falls back to Claude without web tools (`--max-turns 3`) if web search fails
3. 60-second timeout on Claude subprocess to prevent hangs
4. All non-essential fields use `#[serde(default)]` for resilient JSON parsing

## Deep Research Mode

When the wizard's simple approaches fail (URL transforms don't match, basic AI analysis has low confidence), deep research mode allows Claude to spend more tokens thoroughly analyzing a page.

### Usage

```bash
# Explicit opt-in with --research flag
kto new "https://shop.example.com/product for price drops" --research

# When basic analysis has low confidence, kto suggests using deep research
kto new "https://shop.example.com/product for price drops"
  Analyzing...
  Low confidence (45%). Tip: Use --research for thorough page analysis
```

### What Deep Research Does

1. **Dual fetch**: Fetches with both HTTP and Playwright (JavaScript) to compare content
2. **Site type detection**: Identifies platform (Shopify, WordPress, WooCommerce, etc.)
3. **Feed discovery**: Discovers RSS/Atom feeds from link tags and common paths
4. **JSON-LD extraction**: Extracts structured data for stable monitoring
5. **Web search** (if available): Searches for site-specific APIs and monitoring tips
6. **Selector stability analysis**: Recommends stable CSS selectors

### Output

```
  Deep Research Results

  Summary: Discovered hidden JSON API that's more reliable than scraping

  Web Research Findings:
    - Searched: "shopify product api", "monitor shopify availability"
    - Found: Shopify stores expose /products/[handle].json endpoint

  Discovered Feeds:
    - /collections/all.atom (products only, no stock info)

  Discovered APIs:
    - /products/widget.json - Returns full product data including variants

  Recommended Approach:
    Engine: HTTP (JSON API doesn't need JS)
    Extraction: JSON path $.product.variants[*].available
      Price in structured data is more stable than DOM selectors

  Key Insights:
    - JSON API found via web search - not in page HTML
    - API updates before DOM, catch restocks faster

  Confidence: 95%
```

### Implementation Details

- Uses Claude CLI with `--max-turns 5` and `--allowedTools WebSearch,WebFetch` for web search
- Falls back to page-only analysis if web search is unavailable
- Discovers feeds via `<link rel="alternate">` tags and common paths like `/feed`, `/rss`, etc.
- Detects site type from HTML signatures (Shopify, WordPress, WooCommerce, etc.)

## Testing & Development

### Testing Strategy

kto has three levels of testing with distinct purposes:

| Test Type | Command | Purpose | Deterministic |
|-----------|---------|---------|---------------|
| Unit Tests | `make test` | Component-level validation | Yes |
| E2E Tests | `make test-e2e` | Change detection accuracy | Yes |
| Live Exploration | Ad-hoc orchestration | Discover edge cases | No |

**E2E tests are the quality gate.** They validate that kto correctly detects changes using a local test server with controlled mutations.

### Running Tests

```bash
# Unit tests (fast, run often)
make test
cargo test

# E2E change detection tests (run before releases)
make test-e2e
python3 tests/e2e/run_suite.py

# All tests
make test-all

# E2E with verbose output
python3 tests/e2e/run_suite.py --verbose

# Run specific E2E scenario
python3 tests/e2e/run_suite.py --scenario price
```

### E2E Test Suite

Located in `tests/e2e/`, the suite uses a local HTTP server with mutation API:

```
tests/e2e/
├── README.md           # Full documentation
├── run_suite.py        # Test runner (22 scenarios)
└── harness/
    └── server.py       # Local test server (no dependencies)
```

**What it validates:**
- True positives: Price drops, stock changes, new releases detected
- True negatives: Static content, noise don't trigger false positives
- Error handling: 403, 500, timeouts handled gracefully
- Idempotence: Repeated runs don't cause spurious alerts

**Metrics tracked:**
- Precision (target ≥95%): Low false positives
- Recall (target ≥90%): Catch real changes
- Noise Rate (target <5%): Normalization working

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

## Learning Loop / Orchestration

kto includes a learning loop system that discovers what makes monitors effective and feeds that knowledge into the creation pipeline (`kto new`).

### Purpose

- **Before**: `kto new "alert me when bitcoin drops"` creates a generic monitor
- **After**: Knowledge base says "price intents work best with selector extraction, 5-min intervals" and creates an effective monitor immediately

### Architecture

```
Learning Loop (12h) → knowledge.json → kto new (creation pipeline)

Loop: Intent TOML → Create Monitors → Cycle:
  1. Observe  (run checks)
  2. Evaluate (deterministic ground truth in E2E)
  3. Experiment (A/B test variations)
  4. Learn (extract creation rules → knowledge.json)
```

**E2E mode** is the primary learning source (deterministic ground truth). **Live mode** validates E2E-learned rules against real websites but does NOT generate new rules.

### Entry Point

```bash
python scripts/orchestrate.py --intents scripts/intents/example_e2e.toml --duration 12
python scripts/orchestrate.py --intents scripts/intents/example_e2e.toml --duration 0.1  # smoke test
python scripts/orchestrate.py --resume --state-dir /tmp/kto-orchestrate/  # resume
```

### Knowledge File

`~/.local/share/kto/knowledge.json` — schema-versioned creation rules scoped by intent type + domain class.

Rules are consumed by `src/agent.rs:load_creation_knowledge()` and applied as defaults in `cmd_new` with precedence: user override > discovery result > domain rule > intent rule > global default.

### Key Files

| File | Role |
|------|------|
| `scripts/orchestrate.py` | Entry point, arg parsing, main loop |
| `scripts/orchestrate/config.py` | Config, intent weights, SLA map |
| `scripts/orchestrate/state.py` | RunState, MonitorState, atomic persistence |
| `scripts/orchestrate/cycle.py` | CycleRunner: observe → evaluate → experiment → learn |
| `scripts/orchestrate/efficacy.py` | F1-based per-intent scoring |
| `scripts/orchestrate/evaluator.py` | Deterministic E2E eval + Claude live validation |
| `scripts/orchestrate/experimenter.py` | Time-blocked A/B protocol |
| `scripts/orchestrate/knowledge.py` | Schema-versioned rules, decay, precedence |
| `scripts/orchestrate/kto_client.py` | kto CLI wrapper with timeouts |
| `scripts/orchestrate/report.py` | Learning-focused report |
| `scripts/orchestrate/server_bridge.py` | E2E test server mutation API client |
| `scripts/orchestrate/intents.py` | TOML intent loader |
| `scripts/orchestrate/log.py` | Structured JSONL + human logging |
| `scripts/intents/example_e2e.toml` | E2E intent definitions |
| `scripts/intents/example_live.toml` | Live intent definitions |
| `src/agent.rs` | `load_creation_knowledge()` for Rust-side consumption |
| `src/commands/watch.rs` | Consults knowledge base in `cmd_new` |

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
| `n` | New watch/reminder (context-aware wizard with templates) |
| `D` | Describe watch (view full config) |
| `e` | Edit watch or reminder |
| `d` | Delete watch or reminder |
| `H` | Health dashboard (daemon status, watch health) |
| `L` | Activity logs (all changes across watches) |
| `N` | Notification setup |
| `M` | View agent memory (watches pane) |
| `E` | Show error details for selected watch |
| `r` | Refresh from database |
| `?` | Help |
| `q` | Quit |
| Mouse click | Select watches/changes/reminders, click wizard buttons |

### Status Indicators
| Symbol | Meaning |
|--------|---------|
| `●` | Active, checked recently (healthy) |
| `◐` | Active but stale (no check in 2x interval) |
| `○` | Paused |
| `✗` | Error |

### Change Diff View
| Key | Action |
|-----|--------|
| `j/k` | Scroll up/down |
| `u` | Toggle diff format (inline/unified) |
| `Esc/q` | Close |

### Status Bar
The status bar shows:
- Watch count with active/AI breakdown
- Daemon status indicator (running/stale/stopped)
- Rotating pro tips when idle
- Contextual hints for current mode

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

### Watch Wizard Templates
When creating a new watch (`n`), the wizard starts with template selection:
- **Custom** - Start from scratch with full control
- **Price Drop Monitor** - Pre-configured AI instructions for tracking price drops
- **Back-in-Stock Alert** - Monitor availability changes
- **Job Posting Tracker** - Track new job listings, ignore updates
- **Changelog/Release Watcher** - Monitor software releases and updates

Templates pre-fill AI agent settings with appropriate instructions.

### Health Dashboard (`H`)
| Key | Action |
|-----|--------|
| `r` | Refresh health data |
| `L` | Open activity logs |
| `Esc` | Close |

Shows:
- Daemon status (running/stale/stopped)
- Watch health breakdown (healthy/stale/error/paused)
- Notification configuration
- Quiet hours status
