use clap::{Parser, Subcommand, ValueEnum};

/// Shell types for completion generation
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

#[derive(Parser)]
#[command(name = "kto")]
#[command(author, version, about = "A generic, flexible web change watcher", long_about = None)]
#[command(after_help = r#"Examples:
  kto new "https://example.com for price drops"            Create a watch with URL
  kto new "let me know when bitcoin goes above 100k"       AI discovers URL
  kto new "alert me when RTX 5090 is back in stock"        AI finds product page
  kto list                                                 List all watches
  kto test "My Watch"                                      Test a watch manually
  kto service install                                      Run kto in background

Quick Start:
  1. kto new "https://example.com" --name "Example"
  2. kto notify set --ntfy my-alerts
  3. kto service install
"#)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new watch (interactive wizard)
    #[command(after_help = r#"Examples:
  kto new "https://amazon.com/dp/... for price drops"
  kto new "let me know when bitcoin goes above 100k"    # AI discovers URL
  kto new "alert me when RTX 5090 is back in stock"     # AI finds product page
  kto new "notify me about new Rust releases"            # AI finds RSS feed
  kto new "https://news.ycombinator.com" --name "HN" --interval 5m
  kto new "https://spa-site.com" --js          # Enable JavaScript rendering
  kto new "https://rss.nytimes.com/..." --rss  # Monitor RSS/Atom feed
  kto new "docker ps" --shell --name "containers"  # Monitor command output
  kto new --clipboard                          # Read URL from clipboard
  kto new "https://example.com" --name test --yes  # Non-interactive mode
"#)]
    New {
        /// Natural language description (URL optional - AI can discover it)
        #[arg(value_name = "DESCRIPTION")]
        description: Option<String>,

        /// Watch name (for non-interactive mode)
        #[arg(long)]
        name: Option<String>,

        /// Check interval (e.g., 30s, 5m, 2h, 1d) - default 15m
        #[arg(long, default_value = "15m")]
        interval: String,

        /// Force JavaScript rendering with Playwright
        #[arg(long)]
        js: bool,

        /// Parse as RSS/Atom feed (auto-detected from URL)
        #[arg(long)]
        rss: bool,

        /// Monitor shell command output instead of URL
        #[arg(long, conflicts_with = "js", conflicts_with = "rss")]
        shell: bool,

        /// Enable AI agent for change analysis
        #[arg(long)]
        agent: bool,

        /// Custom instructions for the AI agent
        #[arg(long)]
        agent_instructions: Option<String>,

        /// CSS selector for content extraction
        #[arg(long)]
        selector: Option<String>,

        /// Read URL from clipboard
        #[arg(long)]
        clipboard: bool,

        /// Tags for organization (can be specified multiple times)
        #[arg(long, short = 't')]
        tag: Vec<String>,

        /// Include user's interest profile in AI analysis
        #[arg(long)]
        use_profile: bool,

        /// Enable deep research mode for thorough page analysis (uses more tokens)
        #[arg(long, alias = "deep")]
        research: bool,

        /// Skip all interactive prompts
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// List all watches
    #[command(after_help = r#"Examples:
  kto list                  Show all watches (compact)
  kto list -v               Show detailed view
  kto list --tag shopping   Filter by tag
  kto list --json           Output as JSON for scripting
  kto list --json | jq '.[].name'  Get watch names
"#)]
    List {
        /// Show all details
        #[arg(short, long)]
        verbose: bool,

        /// Filter by tag
        #[arg(long, short = 't')]
        tag: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show details of a specific watch
    Show {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Edit a watch (interactive or via flags)
    #[command(after_help = r#"Examples:
  kto edit "My Watch"                        Interactive editing
  kto edit "My Watch" --interval 5m          Change check interval to 5 min
  kto edit "My Watch" --interval 2h          Change check interval to 2 hours
  kto edit "My Watch" --agent true           Enable AI agent
  kto edit "My Watch" --agent-instructions "Alert on price drops below $50"
  kto edit "My Watch" --selector ".price"    Change CSS selector
  kto edit "My Watch" --engine playwright    Switch to JavaScript rendering
  kto edit "My Watch" --extraction full      Use full page content extraction
  kto edit "My Watch" --notify ntfy:alerts   Set per-watch notification
  kto edit "My Watch" --notify none          Remove per-watch notification
  kto edit "My Watch" --use-profile true     Enable interest profile for AI
"#)]
    Edit {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,

        /// New name for the watch
        #[arg(long)]
        name: Option<String>,

        /// Check interval (e.g., 30s, 5m, 2h, 1d)
        #[arg(long)]
        interval: Option<String>,

        /// Enable or disable the watch
        #[arg(long)]
        enabled: Option<bool>,

        /// Enable or disable AI agent
        #[arg(long)]
        agent: Option<bool>,

        /// Custom instructions for the AI agent
        #[arg(long)]
        agent_instructions: Option<String>,

        /// Change CSS selector for extraction
        #[arg(long)]
        selector: Option<String>,

        /// Fetch engine (http, playwright, rss)
        #[arg(long)]
        engine: Option<String>,

        /// Extraction strategy (auto, full, rss, json-ld)
        #[arg(long)]
        extraction: Option<String>,

        /// Per-watch notification target (e.g., "ntfy:topic", "slack:webhook", "discord:webhook", "gotify:server:token", "command:cmd", "none" to clear)
        #[arg(long)]
        notify: Option<String>,

        /// Enable or disable user's interest profile for AI analysis
        #[arg(long)]
        use_profile: Option<bool>,
    },

    /// Pause a watch
    Pause {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,
    },

    /// Resume a paused watch
    Resume {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,
    },

    /// Delete a watch
    Delete {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,

        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Test a watch (fetch now, show what would happen)
    #[command(after_help = r#"Examples:
  kto test "My Watch"                  Manually check for changes
  kto test "My Watch" --json           Get machine-readable output
  kto test "My Watch" --json | jq '.changed'  Check if content changed
"#)]
    Test {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Watch a URL in real-time (ephemeral, no database)
    #[command(after_help = r#"Examples:
  kto watch "https://example.com" --interval 5s     Watch every 5 seconds
  kto watch "https://news.ycombinator.com" -i 30s   Check every 30 seconds
  kto watch "https://example.com" --selector ".price"  Watch specific element
"#)]
    Watch {
        /// URL to watch
        #[arg(value_name = "URL")]
        url: String,

        /// Check interval (e.g., 5s, 30s, 1m, 5m)
        #[arg(short, long, default_value = "30s")]
        interval: String,

        /// CSS selector for content extraction
        #[arg(long)]
        selector: Option<String>,

        /// Enable JavaScript rendering with Playwright
        #[arg(long)]
        js: bool,
    },

    /// Preview what kto extracts from a URL (no database, just fetch and show)
    #[command(after_help = r#"Examples:
  kto preview "https://example.com"                 See extracted content
  kto preview "https://example.com" --selector ".price"  Extract specific element
  kto preview "https://example.com" --js            Use JavaScript rendering
  kto preview "https://example.com" --full          Show full page content
  kto preview "https://example.com" --json-ld       Extract JSON-LD structured data
"#)]
    Preview {
        /// URL to preview
        #[arg(value_name = "URL")]
        url: String,

        /// CSS selector for content extraction
        #[arg(long)]
        selector: Option<String>,

        /// Enable JavaScript rendering with Playwright
        #[arg(long)]
        js: bool,

        /// Extract full page content (not just main content)
        #[arg(long)]
        full: bool,

        /// Extract JSON-LD structured data
        #[arg(long)]
        json_ld: bool,

        /// Limit output to first N characters
        #[arg(long, default_value = "2000")]
        limit: usize,
    },

    /// Show change history for a watch
    History {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,

        /// Number of changes to show
        #[arg(short, long, default_value = "10")]
        limit: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Check all due watches once (for cron)
    Run,

    /// Run continuously with internal scheduler
    Daemon,

    /// Tail activity log
    Logs {
        /// Number of lines to show
        #[arg(short, long, default_value = "50")]
        lines: usize,

        /// Follow log output
        #[arg(short, long)]
        follow: bool,

        /// Output as JSON (machine-readable)
        #[arg(long)]
        json: bool,
    },

    /// Check all dependencies and suggest fixes
    Doctor,

    /// Set up Playwright/Chromium for JS rendering
    EnableJs,

    /// Interactive TUI dashboard
    Ui,

    /// Export watches to JSON (for backup or sharing)
    #[command(after_help = r#"Examples:
  kto export                             Export all watches to stdout
  kto export > backup.json               Save to file
  kto export "My Watch"                  Export single watch
"#)]
    Export {
        /// Watch ID or name (optional, exports all if not specified)
        #[arg(value_name = "ID_OR_NAME")]
        watch: Option<String>,
    },

    /// Import watches from JSON
    #[command(after_help = r#"Examples:
  kto import < backup.json               Import from file
  cat backup.json | kto import           Import via pipe
  kto import --dry-run < backup.json     Preview without importing
"#)]
    Import {
        /// Preview what would be imported without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Show the latest diff for a watch
    #[command(after_help = r#"Examples:
  kto diff "My Watch"                    Show latest change
  kto diff "My Watch" --limit 3          Show last 3 changes
"#)]
    Diff {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,

        /// Number of recent diffs to show
        #[arg(short, long, default_value = "1")]
        limit: usize,
    },

    /// View or manage AI agent memory for a watch
    #[command(after_help = r#"Examples:
  kto memory "My Watch"                  Show AI memory (counters, values, notes)
  kto memory "My Watch" --json           Output as JSON
  kto memory "My Watch" --clear          Clear all memory for this watch
"#)]
    Memory {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Clear all memory for this watch
        #[arg(long)]
        clear: bool,
    },

    /// Manage notification settings
    #[command(subcommand, after_help = r#"Examples:
  kto notify set                        Interactive setup
  kto notify set --ntfy my-alerts       Use ntfy.sh for notifications
  kto notify set --slack https://...    Use Slack webhook
  kto notify set --gotify-server https://gotify.example.com --gotify-token APP_TOKEN
  kto notify set --telegram-token BOT_TOKEN --telegram-chat CHAT_ID
  kto notify set --pushover-user USER_KEY --pushover-token API_TOKEN
  kto notify set --matrix-server https://matrix.org --matrix-room !room:server --matrix-token TOKEN
  kto notify show                       Show current settings
  kto notify test                       Send test notification
  kto notify quiet --start 22:00 --end 08:00   Set quiet hours
  kto notify quiet --disable            Disable quiet hours
"#)]
    Notify(NotifyCommands),

    /// Manage background service (systemd/launchd/cron)
    #[command(subcommand, after_help = r#"Examples:
  kto service install                   Auto-detect and install service
  kto service install --cron            Use cron instead of systemd/launchd
  kto service status                    Check if service is running
  kto service logs -f                   Follow service logs
  kto service uninstall                 Remove background service
"#)]
    Service(ServiceCommands),

    /// Create and manage reminders (simple scheduled notifications)
    #[command(subcommand, after_help = r#"Examples:
  kto remind new "Buy milk" --in 2h              One-shot reminder in 2 hours
  kto remind new "Stand up" --every 1h           Recurring hourly reminder
  kto remind new "Check stocks" --at 09:00       Daily at 9 AM
  kto remind list                                List all reminders
  kto remind delete "Buy milk"                   Delete a reminder
  kto remind pause "Stand up"                    Pause a reminder
  kto remind resume "Stand up"                   Resume a reminder
"#)]
    Remind(RemindCommands),

    /// Manage your interest profile (helps AI understand what matters to you)
    #[command(subcommand, after_help = r#"Examples:
  kto profile show                    Show current profile and learned patterns
  kto profile edit                    Open profile in $EDITOR
  kto profile setup                   Interactive guided setup
  kto profile infer                   Infer interests from your watches
  kto profile preview "My Watch"      Preview what AI receives for a watch
  kto profile clear                   Delete your profile
  kto profile forget --learned        Clear learned patterns (keep static profile)
"#)]
    Profile(ProfileCommands),

    /// Generate shell completions
    #[command(after_help = r#"Examples:
  kto completions bash >> ~/.bashrc           Add bash completions
  kto completions zsh >> ~/.zshrc             Add zsh completions
  kto completions fish > ~/.config/fish/completions/kto.fish
  kto completions powershell >> $PROFILE      Add PowerShell completions
"#)]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: CompletionShell,
    },

    /// Interactive first-time setup wizard
    #[command(after_help = r#"Examples:
  kto init                    Run the guided setup wizard
"#)]
    Init,
}

#[derive(Subcommand)]
pub enum ServiceCommands {
    /// Install kto as a background service
    Install {
        /// Use cron instead of systemd/launchd
        #[arg(long)]
        cron: bool,

        /// Cron interval in minutes (default: 5)
        #[arg(long, default_value = "5")]
        cron_interval: u32,
    },

    /// Uninstall the background service
    Uninstall,

    /// Show service status
    Status,

    /// Show service logs
    Logs {
        /// Number of lines to show
        #[arg(short, long, default_value = "50")]
        lines: usize,

        /// Follow log output
        #[arg(short, long)]
        follow: bool,
    },
}

#[derive(Subcommand)]
pub enum NotifyCommands {
    /// Set up notification target (interactive or via flags)
    Set {
        /// ntfy topic (e.g., my-topic or https://ntfy.sh/my-topic)
        #[arg(long)]
        ntfy: Option<String>,

        /// Slack webhook URL
        #[arg(long)]
        slack: Option<String>,

        /// Discord webhook URL
        #[arg(long)]
        discord: Option<String>,

        /// Gotify server URL (e.g., https://gotify.example.com)
        #[arg(long)]
        gotify_server: Option<String>,

        /// Gotify application token
        #[arg(long)]
        gotify_token: Option<String>,

        /// Custom command to execute
        #[arg(long)]
        command: Option<String>,

        /// Telegram bot token
        #[arg(long)]
        telegram_token: Option<String>,

        /// Telegram chat ID
        #[arg(long)]
        telegram_chat: Option<String>,

        /// Pushover user key
        #[arg(long)]
        pushover_user: Option<String>,

        /// Pushover API token
        #[arg(long)]
        pushover_token: Option<String>,

        /// Matrix homeserver URL
        #[arg(long)]
        matrix_server: Option<String>,

        /// Matrix room ID
        #[arg(long)]
        matrix_room: Option<String>,

        /// Matrix access token
        #[arg(long)]
        matrix_token: Option<String>,
    },

    /// Show current notification settings
    Show,

    /// Send a test notification
    Test,

    /// Configure quiet hours (suppress notifications during this time)
    Quiet {
        /// Start time in HH:MM format (e.g., "22:00")
        #[arg(long)]
        start: Option<String>,

        /// End time in HH:MM format (e.g., "08:00")
        #[arg(long)]
        end: Option<String>,

        /// Disable quiet hours
        #[arg(long)]
        disable: bool,
    },
}

#[derive(Subcommand)]
pub enum RemindCommands {
    /// Create a new reminder
    New {
        /// Reminder name/message
        #[arg(value_name = "MESSAGE")]
        message: String,

        /// Trigger in duration (e.g., "30m", "2h", "1d")
        #[arg(long, value_name = "DURATION", conflicts_with = "at")]
        r#in: Option<String>,

        /// Trigger at specific time (e.g., "09:00", "14:30")
        #[arg(long, value_name = "TIME", conflicts_with = "in")]
        at: Option<String>,

        /// Repeat every duration (e.g., "1h", "1d", "1w")
        #[arg(long, value_name = "DURATION")]
        every: Option<String>,

        /// Additional message text
        #[arg(long)]
        note: Option<String>,
    },

    /// List all reminders
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Delete a reminder
    Delete {
        /// Reminder ID or name
        #[arg(value_name = "ID_OR_NAME")]
        reminder: String,
    },

    /// Pause a reminder
    Pause {
        /// Reminder ID or name
        #[arg(value_name = "ID_OR_NAME")]
        reminder: String,
    },

    /// Resume a paused reminder
    Resume {
        /// Reminder ID or name
        #[arg(value_name = "ID_OR_NAME")]
        reminder: String,
    },
}

#[derive(Subcommand)]
pub enum ProfileCommands {
    /// Show your interest profile and learned patterns
    Show {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Edit your interest profile in $EDITOR
    Edit,

    /// Interactive guided profile setup
    Setup,

    /// Infer interests from your existing watches
    Infer {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Preview what AI receives for a specific watch
    Preview {
        /// Watch ID or name
        #[arg(value_name = "ID_OR_NAME")]
        watch: String,
    },

    /// Clear your interest profile
    Clear {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Forget learned patterns (clear global memory)
    Forget {
        /// Only clear learned patterns, keep static profile
        #[arg(long)]
        learned: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
}
