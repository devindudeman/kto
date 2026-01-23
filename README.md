# kto

A generic, flexible web change watcher with AI-powered analysis.

kto monitors web pages for changes and notifies you when something interesting happens. It can use Claude AI to intelligently filter noise and only alert you about changes that matter.

## Features

- **Simple setup** - Just give it a URL and describe what you're watching for
- **Smart URL detection** - Auto-detects optimal endpoints for GitHub, Reddit, HN, PyPI, and more
- **Deep research mode** - Thorough AI analysis to find the best monitoring approach
- **AI-powered analysis** - Uses Claude to understand changes, not just detect them
- **Multiple notification channels** - ntfy, Gotify, Slack, Discord, Telegram, Pushover, Matrix, or custom commands
- **Shell command monitoring** - Watch output of any command, not just URLs
- **JavaScript support** - Render JS-heavy pages with Playwright
- **Watch tags** - Organize watches with tags and filter by tag
- **Quiet hours** - Suppress notifications during sleep/focus time
- **Reminders** - Set one-time or recurring reminders with simple time syntax
- **TUI dashboard** - Interactive terminal interface for managing watches and reminders
- **Background service** - Runs as systemd/launchd service or cron job
- **Machine-readable output** - JSON output for scripting and automation

## Installation

```bash
# Recommended: prebuilt binary (fast, no compile)
cargo binstall kto

# From crates.io (requires Rust)
cargo install kto

# Via install script
curl -fsSL https://raw.githubusercontent.com/devindudeman/kto/main/install.sh | bash

# From source
git clone https://github.com/devindudeman/kto
cd kto && cargo install --path . --features tui
```

## Quick Start

```bash
# First-time setup (guided wizard)
kto init

# Or manually:
kto new "https://news.ycombinator.com for AI news"
kto notify set --ntfy my-alerts
kto service install
```

## Shell Completions

```bash
# Bash
kto completions bash >> ~/.bashrc

# Zsh
kto completions zsh >> ~/.zshrc

# Fish
kto completions fish > ~/.config/fish/completions/kto.fish
```

## Dependencies
- For JS rendering: Node.js + Playwright (`kto enable-js`)
- For AI analysis: [Claude CLI](https://claude.ai/cli) (optional)

## Usage

### Creating Watches

```bash
# Interactive wizard with AI suggestions
kto new "https://example.com/product for price drops"

# Quick non-interactive setup
kto new "https://example.com" --name "Example" --yes

# With JavaScript rendering
kto new "https://spa-site.com" --js

# Monitor shell command output
kto new "docker ps" --shell --name "containers"
kto new "df -h" --shell --tag system --tag disk

# With tags for organization
kto new "https://example.com" --name "Example" --tag work --tag important

# From clipboard
kto new --clipboard

# Deep research for complex sites (uses more AI tokens)
kto new "https://shop.example.com/product for price drops" --deep
```

### Smart URL Detection

kto automatically detects optimal URLs for common sites:

```bash
# GitHub releases - auto-detects Atom feed
kto new "https://github.com/astral-sh/ruff for new releases"
# → Uses https://github.com/astral-sh/ruff/releases.atom with RSS engine

# Reddit - auto-detects RSS feed
kto new "https://reddit.com/r/rust for news"
# → Uses https://reddit.com/r/rust.rss

# Hacker News, PyPI, GitLab, Codeberg also supported
```

### Inspecting URLs

```bash
# Preview what kto extracts (before creating a watch)
kto preview "https://example.com"
kto preview "https://example.com" --js        # With JavaScript
kto preview "https://example.com" --limit 5000

# Real-time ephemeral monitoring (no database)
kto watch "https://example.com" --interval 30s
```

### Managing Watches

```bash
# List all watches
kto list
kto list --json       # Machine-readable
kto list --tag work   # Filter by tag

# Show details
kto show "My Watch"
kto show "My Watch" --json

# Edit a watch
kto edit "My Watch" --interval 300
kto edit "My Watch" --agent true --agent-instructions "Alert on price drops"

# Pause/resume
kto pause "My Watch"
kto resume "My Watch"

# Delete
kto delete "My Watch"
```

### Reminders

Set simple reminders that trigger notifications without monitoring a URL:

```bash
# Reminder in 30 minutes
kto remind new "Take a break" --in 30m

# Reminder at a specific time (uses your local timezone)
kto remind new "Team standup" --at 09:00

# Recurring reminder
kto remind new "Weekly review" --at 10:00 --every 1w

# With a note/message body
kto remind new "Call mom" --in 2h --note "Ask about weekend plans"

# List reminders
kto remind list

# Pause/resume
kto remind pause "Team standup"
kto remind resume "Team standup"

# Delete
kto remind delete "Take a break"
```

Time formats for `--in`: `30s`, `5m`, `2h`, `1d`, `1w`

### Testing & History

```bash
# Test a watch (fetch now, show what would happen)
kto test "My Watch"
kto test "My Watch" --json

# View change history
kto history "My Watch"
kto history "My Watch" --limit 50 --json

# View recent activity across all watches
kto logs
kto logs -f  # Follow mode
```

### Running the Daemon

```bash
# One-time check (for cron)
kto run

# Continuous daemon (foreground)
kto daemon

# Install as system service (recommended)
kto service install              # Auto-detect systemd/launchd
kto service install --cron       # Use cron instead
kto service install --cron --cron-interval 10  # Every 10 minutes

# Manage service
kto service status
kto service logs
kto service logs -f              # Follow
kto service uninstall
```

### Notifications

```bash
# Interactive setup
kto notify set

# Direct setup (non-interactive)
kto notify set --ntfy my-topic
kto notify set --gotify-server https://gotify.example.com --gotify-token APP_TOKEN
kto notify set --slack https://hooks.slack.com/...
kto notify set --discord https://discord.com/api/webhooks/...
kto notify set --telegram-token BOT_TOKEN --telegram-chat CHAT_ID
kto notify set --pushover-user USER_KEY --pushover-token API_TOKEN
kto notify set --matrix-server https://matrix.org --matrix-room ROOM_ID --matrix-token TOKEN
kto notify set --command "notify-send 'kto' '\$SUMMARY'"

# View current settings
kto notify show

# Send test notification
kto notify test

# Quiet hours (suppress notifications during this time)
kto notify quiet --start 22:00 --end 08:00
kto notify quiet --disable

# Per-watch notification override
kto edit "My Watch" --notify ntfy:special-alerts
kto edit "Other Watch" --notify gotify:https://gotify.example.com:APP_TOKEN
kto edit "Work Watch" --notify none  # Disable for this watch
```

Notifications include a diff preview showing what changed:
```
https://example.com/page
+3 / -2 changes

[-old text][+new text] some context
```

### TUI Dashboard

```bash
kto ui
```

Navigate with `j/k` or arrow keys. Press `Tab` to cycle between Watches, Changes, and Reminders panes. Use `e` to edit watches, `p` to pause/resume, `d` to delete, `?` for help, `q` to quit.

## Configuration

Configuration is stored in `~/.config/kto/config.toml`:

```toml
default_interval_secs = 900

[default_notify]
type = "ntfy"
topic = "my-alerts"

# Per-domain rate limits (requests per second)
# Prevents IP bans when watching multiple pages on same domain
[rate_limits]
"amazon.com" = 0.5        # 2 second delay between amazon requests
"reddit.com" = 1.0        # 1 second delay
```

Database is stored at `~/.local/share/kto/kto.db`.

### Environment Variables

- `KTO_DB` - Override database path (useful for testing)

## AI-Powered Analysis

When Claude CLI is installed, kto can use AI to:

1. **Smart setup** - Analyze pages and suggest optimal watch settings
2. **Change filtering** - Only notify about meaningful changes (ignore timestamps, ads, etc.)
3. **Summaries** - Get one-line summaries of what changed

```bash
# Enable AI agent for a watch
kto edit "My Watch" --agent true

# With custom instructions
kto edit "My Watch" --agent-instructions "Alert when price drops below $50"
```

### Deep Research Mode

For complex sites where simple analysis isn't enough, use `--deep` for thorough investigation:

```bash
kto new "https://shop.example.com/product for price drops" --deep
```

Deep research mode:
- Fetches with both HTTP and JavaScript to compare content
- Discovers RSS/Atom feeds and JSON-LD structured data
- Searches the web for site-specific APIs and monitoring tips
- Recommends stable CSS selectors

Requires [Claude CLI](https://claude.ai/cli):
```bash
curl -fsSL https://claude.ai/install.sh | bash
```

## JavaScript Rendering

For pages that require JavaScript:

```bash
# One-time setup
kto enable-js

# Create watch with JS rendering
kto new "https://spa-site.com" --js
```

## Scripting & Automation

All read commands support `--json` for machine-readable output:

```bash
# Get watch count
kto list --json | jq 'length'

# Get specific watch URL
kto show "My Watch" --json | jq '.watch.url'

# Check if changes detected
kto test "My Watch" --json | jq '.changed'

# Create watch non-interactively
KTO_DB=/tmp/test.db kto new "https://example.com" --name test --yes
```

## Health Check

```bash
kto doctor
```

Shows status of:
- kto binary version
- Claude CLI (for AI features)
- Node.js (for JS rendering)
- Playwright/Chromium
- Database

## License

MIT

## Contributing

Contributions welcome! Please read CLAUDE.md for development context.
