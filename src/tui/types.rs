//! TUI type definitions - enums, modes, and field types

/// Which pane has focus
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Pane {
    #[default]
    Watches,
    Changes,
    Reminders,
}

/// Current application mode
#[derive(Clone, PartialEq)]
pub enum Mode {
    Normal,
    Help,
    Confirm(ConfirmAction),
    Edit,
    EditReminder,
    Search,
    ViewChange,
    Wizard,
    ReminderWizard,
    Describe,      // Full watch details view
    Logs,          // Activity logs view
    NotifySetup,   // Notification configuration
    FilterList,    // Viewing/managing filters for a watch
    FilterEdit,    // Editing a single filter
    MemoryInspector, // Agent memory inspection
    ProfileInspector, // User interest profile inspection
}

#[derive(Clone, Copy, PartialEq)]
pub enum ConfirmAction {
    Delete,
    DeleteReminder,
    Test,
    ForceCheck,
}

#[derive(Clone, PartialEq)]
pub enum EditField {
    Name,
    Interval,
    Engine,
    Extraction,
    Enabled,
    Agent,
    AgentInstructions,
    UseProfile,
    Filters,
    Notify,
    NotifyCustom,
}

#[derive(Clone, PartialEq, Debug)]
pub enum NotifyType {
    Ntfy,
    Gotify,
    Slack,
    Discord,
    Telegram,
    Pushover,
    Command,
}

impl NotifyType {
    pub fn next(&self) -> Self {
        match self {
            NotifyType::Ntfy => NotifyType::Gotify,
            NotifyType::Gotify => NotifyType::Slack,
            NotifyType::Slack => NotifyType::Discord,
            NotifyType::Discord => NotifyType::Telegram,
            NotifyType::Telegram => NotifyType::Pushover,
            NotifyType::Pushover => NotifyType::Command,
            NotifyType::Command => NotifyType::Ntfy,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            NotifyType::Ntfy => "ntfy",
            NotifyType::Gotify => "Gotify",
            NotifyType::Slack => "Slack",
            NotifyType::Discord => "Discord",
            NotifyType::Telegram => "Telegram",
            NotifyType::Pushover => "Pushover",
            NotifyType::Command => "Command",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            NotifyType::Ntfy => "topic-name",
            NotifyType::Gotify => "https://gotify.example.com|token",
            NotifyType::Slack => "https://hooks.slack.com/...",
            NotifyType::Discord => "https://discord.com/api/webhooks/...",
            NotifyType::Telegram => "chat_id|bot_token",
            NotifyType::Pushover => "user_key|api_token",
            NotifyType::Command => "command to run",
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum ReminderEditField {
    Name,
    TriggerTime,
    Recurring,
    Interval,
    Enabled,
}

// === Filter Management Types ===

#[derive(Clone, PartialEq, Debug)]
pub enum FilterCondition {
    Contains,
    NotContains,
    Matches,
    SizeGt,
}

#[derive(Clone, PartialEq)]
pub enum FilterEditField {
    Target,
    Condition,
    Value,
}

// === Memory Inspector Types ===

#[derive(Clone, PartialEq)]
pub enum MemorySection {
    Counters,
    LastValues,
    Notes,
}

// === Profile Inspector Types ===

#[derive(Clone, PartialEq)]
pub enum ProfileSection {
    Description,
    Interests,
    GlobalMemory,
}

// === Diff View Types ===

#[derive(Clone, Copy, PartialEq, Default)]
pub enum DiffViewMode {
    #[default]
    Inline,
    Unified,
}

// === Wizard Types ===

#[derive(Clone, Debug, PartialEq)]
pub enum WizardStep {
    Url,
    Engine,
    Name,
    Extraction,
    Interval,
    Agent,
    Review,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReminderWizardStep {
    Name,
    When,
    Recurring,
    Review,
}

// Re-export FilterTarget for convenience
pub use crate::watch::FilterTarget as FilterTargetType;
