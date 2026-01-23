//! TUI state structures - App state and all related state types

use ratatui::layout::Rect;
use uuid::Uuid;
use chrono::Utc;
use std::time::Instant;

use crate::db::Database;
use crate::error::Result;
use crate::interests::{GlobalMemory, InterestProfile};
use crate::transforms::Intent;
use crate::watch::{AgentConfig, AgentMemory, Change, Engine, Filter, FilterTarget, Reminder, Snapshot, Watch};
use crate::fetch::fetch;
use crate::extract;
use crate::normalize::{normalize, hash_content};
use crate::diff;

use super::types::*;
use super::utils::{parse_extraction_string, build_notify_target, parse_time_to_datetime, parse_duration_str, format_interval};

/// Layout areas for click detection
#[derive(Default, Clone, Copy)]
pub struct LayoutAreas {
    pub watches: Rect,
    pub reminders: Rect,
    pub changes: Rect,
    pub details: Rect,
    pub logs_modal: Rect,
    pub wizard_modal: Rect,
    pub reminder_wizard_modal: Rect,
}

/// Main application state
pub struct App {
    pub watches: Vec<Watch>,
    pub selected_watch: usize,
    pub changes: Vec<Change>,
    pub reminders: Vec<Reminder>,
    pub selected_reminder: usize,
    pub mode: Mode,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub db: Database,
    pub edit_state: Option<EditState>,
    pub reminder_edit_state: Option<ReminderEditState>,
    pub wizard_state: Option<WizardState>,
    pub reminder_wizard_state: Option<ReminderWizardState>,
    // k9s-style additions
    pub focus: Pane,
    pub selected_change: usize,
    pub filter_text: String,
    pub scroll_offset: usize,
    // Layout areas for click detection
    pub layout_areas: LayoutAreas,
    // Logs view
    pub all_changes: Vec<(Change, String)>, // (change, watch_name)
    pub selected_log: usize,
    // Notify setup
    pub notify_setup_state: Option<NotifySetupState>,
    // Track previous mode for proper back navigation
    pub previous_mode: Option<Mode>,
    // Filter management
    pub filter_edit_state: Option<FilterEditState>,
    pub selected_filter: usize,
    // Memory inspector
    pub memory_inspector_state: Option<MemoryInspectorState>,
    // Profile inspector
    pub profile_inspector_state: Option<ProfileInspectorState>,
    // Diff view mode
    pub diff_view_mode: DiffViewMode,
    // Error tracking for watches (watch_id -> last error message)
    pub watch_errors: std::collections::HashMap<uuid::Uuid, String>,
    // Flag to signal terminal needs full redraw (after returning from external editor)
    pub needs_full_redraw: bool,
    // Scroll offsets for list views (auto-calculated based on selection)
    pub watches_scroll: usize,
    pub reminders_scroll: usize,
    pub changes_scroll: usize,
    pub logs_scroll: usize,
    // Rotating tips state
    pub tip_index: usize,
    pub last_tip_change: Instant,
    // Health dashboard state
    pub health_dashboard_state: Option<HealthDashboardState>,
}

impl App {
    pub fn new() -> Result<Self> {
        let db = Database::open()?;
        let watches = db.list_watches()?;
        let changes = if let Some(first) = watches.first() {
            db.get_recent_changes(&first.id, 20)?
        } else {
            Vec::new()
        };
        let reminders = db.list_reminders()?;
        let all_changes = db.get_all_recent_changes(50)?;

        Ok(Self {
            watches,
            selected_watch: 0,
            changes,
            reminders,
            selected_reminder: 0,
            mode: Mode::Normal,
            should_quit: false,
            status_message: None,
            db,
            edit_state: None,
            reminder_edit_state: None,
            wizard_state: None,
            reminder_wizard_state: None,
            focus: Pane::Watches,
            selected_change: 0,
            filter_text: String::new(),
            scroll_offset: 0,
            layout_areas: LayoutAreas::default(),
            all_changes,
            selected_log: 0,
            notify_setup_state: None,
            previous_mode: None,
            filter_edit_state: None,
            selected_filter: 0,
            memory_inspector_state: None,
            profile_inspector_state: None,
            diff_view_mode: DiffViewMode::default(),
            watch_errors: std::collections::HashMap::new(),
            needs_full_redraw: false,
            watches_scroll: 0,
            reminders_scroll: 0,
            changes_scroll: 0,
            logs_scroll: 0,
            tip_index: 0,
            last_tip_change: Instant::now(),
            health_dashboard_state: None,
        })
    }

    pub fn start_edit(&mut self) {
        if let Some(watch) = self.selected_watch() {
            self.edit_state = Some(EditState::from_watch(watch));
            self.mode = Mode::Edit;
        }
    }

    pub fn save_edit(&mut self) -> Result<()> {
        if let Some(edit_state) = &self.edit_state {
            if let Some(watch) = self.watches.iter_mut().find(|w| w.id == edit_state.watch_id) {
                watch.name = edit_state.name.clone();
                watch.interval_secs = edit_state.interval_secs.max(10); // Enforce minimum 10s
                watch.enabled = edit_state.enabled;
                watch.engine = edit_state.engine.clone();

                // Parse extraction string back to Extraction enum
                watch.extraction = parse_extraction_string(&edit_state.extraction);

                // Handle notify target
                watch.notify_target = if edit_state.notify_use_global {
                    None // Use global config
                } else {
                    build_notify_target(&edit_state.notify_type, &edit_state.notify_value)
                };

                // Handle agent config
                let instructions = if edit_state.agent_instructions.is_empty() {
                    None
                } else {
                    Some(edit_state.agent_instructions.clone())
                };

                if edit_state.agent_enabled {
                    if watch.agent_config.is_none() {
                        watch.agent_config = Some(AgentConfig {
                            enabled: true,
                            prompt_template: None,
                            instructions,
                        });
                    } else if let Some(ref mut config) = watch.agent_config {
                        config.enabled = true;
                        config.instructions = instructions;
                    }
                } else if let Some(ref mut config) = watch.agent_config {
                    config.enabled = false;
                    config.instructions = instructions;
                }

                // Handle use_profile
                watch.use_profile = edit_state.use_profile;

                self.db.update_watch(watch)?;
                self.status_message = Some(format!("Saved changes to '{}'", watch.name));
            }
        }
        self.edit_state = None;
        self.mode = Mode::Normal;
        Ok(())
    }

    pub fn cancel_edit(&mut self) {
        self.edit_state = None;
        self.mode = Mode::Normal;
        self.status_message = Some("Edit cancelled".to_string());
    }

    pub fn start_reminder_edit(&mut self) {
        if let Some(reminder) = self.selected_reminder() {
            self.reminder_edit_state = Some(ReminderEditState::from_reminder(reminder));
            self.mode = Mode::EditReminder;
        }
    }

    pub fn save_reminder_edit(&mut self) -> Result<()> {
        if let Some(edit_state) = &self.reminder_edit_state {
            if let Some(reminder) = self.reminders.iter_mut().find(|r| r.id == edit_state.reminder_id) {
                reminder.name = edit_state.name.clone();
                reminder.enabled = edit_state.enabled;

                // Parse trigger time (HH:MM) to next occurrence
                if let Some(new_trigger) = parse_time_to_datetime(&edit_state.trigger_time) {
                    reminder.trigger_at = new_trigger;
                }

                // Handle recurring
                if edit_state.recurring {
                    reminder.interval_secs = parse_duration_str(&edit_state.interval_input);
                } else {
                    reminder.interval_secs = None;
                }

                self.db.update_reminder(reminder)?;
                self.status_message = Some(format!("Saved changes to '{}'", reminder.name));
            }
        }
        self.reminder_edit_state = None;
        self.mode = Mode::Normal;
        Ok(())
    }

    pub fn cancel_reminder_edit(&mut self) {
        self.reminder_edit_state = None;
        self.mode = Mode::Normal;
        self.status_message = Some("Edit cancelled".to_string());
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.watches = self.db.list_watches()?;
        self.reminders = self.db.list_reminders()?;
        self.apply_filter();
        if let Some(watch) = self.filtered_watches().get(self.selected_watch) {
            self.changes = self.db.get_recent_changes(&watch.id, 20)?;
        } else {
            self.changes = Vec::new();
        }
        Ok(())
    }

    pub fn selected_watch(&self) -> Option<&Watch> {
        self.filtered_watches().get(self.selected_watch).copied()
    }

    /// Get watches filtered by search text
    pub fn filtered_watches(&self) -> Vec<&Watch> {
        if self.filter_text.is_empty() {
            self.watches.iter().collect()
        } else {
            let filter_lower = self.filter_text.to_lowercase();
            self.watches
                .iter()
                .filter(|w| w.name.to_lowercase().contains(&filter_lower)
                    || w.url.to_lowercase().contains(&filter_lower))
                .collect()
        }
    }

    fn apply_filter(&mut self) {
        let filtered = self.filtered_watches();
        if self.selected_watch >= filtered.len() {
            self.selected_watch = filtered.len().saturating_sub(1);
        }
    }

    pub fn next_watch(&mut self) {
        let filtered = self.filtered_watches();
        if !filtered.is_empty() {
            self.selected_watch = (self.selected_watch + 1) % filtered.len();
            self.update_changes();
        }
    }

    pub fn previous_watch(&mut self) {
        let filtered = self.filtered_watches();
        if !filtered.is_empty() {
            self.selected_watch = if self.selected_watch == 0 {
                filtered.len() - 1
            } else {
                self.selected_watch - 1
            };
            self.update_changes();
        }
    }

    pub fn first_watch(&mut self) {
        if !self.filtered_watches().is_empty() {
            self.selected_watch = 0;
            self.update_changes();
        }
    }

    pub fn last_watch(&mut self) {
        let filtered = self.filtered_watches();
        if !filtered.is_empty() {
            self.selected_watch = filtered.len() - 1;
            self.update_changes();
        }
    }

    pub fn next_change(&mut self) {
        if !self.changes.is_empty() {
            self.selected_change = (self.selected_change + 1) % self.changes.len();
        }
    }

    pub fn previous_change(&mut self) {
        if !self.changes.is_empty() {
            self.selected_change = if self.selected_change == 0 {
                self.changes.len() - 1
            } else {
                self.selected_change - 1
            };
        }
    }

    pub fn first_change(&mut self) {
        if !self.changes.is_empty() {
            self.selected_change = 0;
        }
    }

    pub fn last_change(&mut self) {
        if !self.changes.is_empty() {
            self.selected_change = self.changes.len() - 1;
        }
    }

    pub fn next_reminder(&mut self) {
        if !self.reminders.is_empty() {
            self.selected_reminder = (self.selected_reminder + 1) % self.reminders.len();
        }
    }

    pub fn previous_reminder(&mut self) {
        if !self.reminders.is_empty() {
            self.selected_reminder = if self.selected_reminder == 0 {
                self.reminders.len() - 1
            } else {
                self.selected_reminder - 1
            };
        }
    }

    pub fn selected_reminder(&self) -> Option<&Reminder> {
        self.reminders.get(self.selected_reminder)
    }

    pub fn toggle_reminder_enabled(&mut self) -> Result<()> {
        if let Some(reminder) = self.reminders.get(self.selected_reminder) {
            let new_enabled = !reminder.enabled;
            self.db.set_reminder_enabled(&reminder.id, new_enabled)?;
            self.reminders[self.selected_reminder].enabled = new_enabled;
            let status = if new_enabled { "resumed" } else { "paused" };
            self.status_message = Some(format!("Reminder {}", status));
        }
        Ok(())
    }

    pub fn delete_selected_reminder(&mut self) -> Result<()> {
        if let Some(reminder) = self.reminders.get(self.selected_reminder) {
            self.db.delete_reminder(&reminder.id)?;
            let name = reminder.name.clone();
            self.reminders.remove(self.selected_reminder);
            if self.selected_reminder >= self.reminders.len() && self.selected_reminder > 0 {
                self.selected_reminder -= 1;
            }
            self.status_message = Some(format!("Deleted reminder: {}", name));
        }
        Ok(())
    }

    pub fn update_changes(&mut self) {
        if let Some(watch) = self.selected_watch() {
            self.changes = self.db.get_recent_changes(&watch.id, 20).unwrap_or_default();
            self.selected_change = 0;
        }
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        if let Some(watch_ref) = self.selected_watch() {
            let watch_id = watch_ref.id;
            if let Some(watch) = self.watches.iter_mut().find(|w| w.id == watch_id) {
                watch.enabled = !watch.enabled;
                self.db.update_watch(watch)?;
                let action = if watch.enabled { "Resumed" } else { "Paused" };
                self.status_message = Some(format!("{} {}", action, watch.name));
            }
        }
        Ok(())
    }

    pub fn delete_selected(&mut self) -> Result<()> {
        if let Some(watch) = self.selected_watch() {
            let name = watch.name.clone();
            let id = watch.id;
            self.db.delete_watch(&id)?;
            self.status_message = Some(format!("Deleted {}", name));
            self.refresh()?;
            let filtered_len = self.filtered_watches().len();
            if self.selected_watch >= filtered_len && self.selected_watch > 0 {
                self.selected_watch -= 1;
            }
        }
        Ok(())
    }

    pub fn test_selected(&mut self) -> Result<()> {
        if let Some(watch) = self.selected_watch() {
            let name = watch.name.clone();
            let watch_id = watch.id;
            let watch = watch.clone();

            self.status_message = Some(format!("Testing {}...", name));

            // Fetch content
            let content = match fetch(&watch.url, watch.engine, &watch.headers) {
                Ok(c) => c,
                Err(e) => {
                    let err_msg = format!("Fetch failed: {}", e);
                    self.watch_errors.insert(watch_id, err_msg.clone());
                    self.status_message = Some(format!("Test failed: {}", e));
                    return Ok(());
                }
            };

            // Extract and normalize
            let extracted = match extract::extract(&content, &watch.extraction) {
                Ok(e) => e,
                Err(e) => {
                    let err_msg = format!("Extract failed: {}", e);
                    self.watch_errors.insert(watch_id, err_msg.clone());
                    self.status_message = Some(format!("Extract failed: {}", e));
                    return Ok(());
                }
            };
            let normalized = normalize(&extracted, &watch.normalization);
            let new_hash = hash_content(&normalized);

            // Clear error on success
            self.watch_errors.remove(&watch_id);

            // Get last snapshot hash
            let last_hash = self.db.get_latest_snapshot(&watch.id)
                .ok()
                .flatten()
                .map(|s| s.content_hash);

            // Compare
            let result = match last_hash {
                Some(ref h) if h == &new_hash => format!("{}: No change", name),
                Some(_) => format!("{}: Change detected!", name),
                None => format!("{}: First snapshot ({})", name, &new_hash[..8]),
            };

            self.status_message = Some(result);
        }
        Ok(())
    }

    /// Force a full check - fetches, saves snapshot, detects changes
    pub fn check_selected(&mut self) -> Result<()> {
        if let Some(watch) = self.selected_watch() {
            let name = watch.name.clone();
            let watch_id = watch.id;
            let watch = watch.clone();

            // Fetch content
            let content = match fetch(&watch.url, watch.engine, &watch.headers) {
                Ok(c) => c,
                Err(e) => {
                    let err_msg = format!("Fetch failed: {}", e);
                    self.watch_errors.insert(watch_id, err_msg.clone());
                    self.status_message = Some(format!("Check failed: {}", e));
                    return Ok(());
                }
            };

            // Extract and normalize
            let extracted = match extract::extract(&content, &watch.extraction) {
                Ok(e) => e,
                Err(e) => {
                    let err_msg = format!("Extract failed: {}", e);
                    self.watch_errors.insert(watch_id, err_msg.clone());
                    self.status_message = Some(format!("Extract failed: {}", e));
                    return Ok(());
                }
            };
            let normalized = normalize(&extracted, &watch.normalization);
            let new_hash = hash_content(&normalized);

            // Clear error on success
            self.watch_errors.remove(&watch_id);

            // Get last snapshot
            let last = self.db.get_latest_snapshot(&watch.id).ok().flatten();

            // Create and save new snapshot
            let new_snapshot = Snapshot {
                id: Uuid::new_v4(),
                watch_id: watch.id,
                fetched_at: Utc::now(),
                raw_html: zstd::encode_all(content.html.as_bytes(), 3).ok(),
                extracted: normalized.clone(),
                content_hash: new_hash.clone(),
            };

            if let Err(e) = self.db.insert_snapshot(&new_snapshot) {
                self.status_message = Some(format!("Save failed: {}", e));
                return Ok(());
            }

            // Cleanup old snapshots
            let _ = self.db.cleanup_snapshots(&watch.id, 50, 5);

            // Check for changes
            let result = if let Some(old) = last {
                if new_hash != old.content_hash {
                    let diff_result = diff::diff(&old.extracted, &normalized);

                    // Record the change
                    let change = Change {
                        id: Uuid::new_v4(),
                        watch_id: watch.id,
                        detected_at: Utc::now(),
                        old_snapshot_id: old.id,
                        new_snapshot_id: new_snapshot.id,
                        diff: diff_result.diff_text.clone(),
                        filter_passed: true,
                        agent_response: None,
                        notified: false,
                    };
                    let _ = self.db.insert_change(&change);

                    format!("{}: Changed! (+{} -{} chars)", name, diff_result.additions, diff_result.deletions)
                } else {
                    format!("{}: No change (checked)", name)
                }
            } else {
                format!("{}: First snapshot saved", name)
            };

            self.status_message = Some(result);
            self.update_changes();
        }
        Ok(())
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: usize, max_lines: usize) {
        self.scroll_offset = (self.scroll_offset + lines).min(max_lines.saturating_sub(1));
    }

    /// Update watches scroll to keep selected item visible
    pub fn update_watches_scroll(&mut self, visible_height: usize) {
        let selected = self.selected_watch;
        self.watches_scroll = calculate_scroll_offset(selected, self.watches_scroll, visible_height);
    }

    /// Update reminders scroll to keep selected item visible
    pub fn update_reminders_scroll(&mut self, visible_height: usize) {
        let selected = self.selected_reminder;
        self.reminders_scroll = calculate_scroll_offset(selected, self.reminders_scroll, visible_height);
    }

    /// Update changes scroll to keep selected item visible
    pub fn update_changes_scroll(&mut self, visible_height: usize) {
        let selected = self.selected_change;
        self.changes_scroll = calculate_scroll_offset(selected, self.changes_scroll, visible_height);
    }

    /// Update logs scroll to keep selected item visible
    pub fn update_logs_scroll(&mut self, visible_height: usize) {
        let selected = self.selected_log;
        self.logs_scroll = calculate_scroll_offset(selected, self.logs_scroll, visible_height);
    }
}

/// Calculate scroll offset to keep selected item visible
fn calculate_scroll_offset(selected: usize, current_scroll: usize, visible_height: usize) -> usize {
    if visible_height == 0 {
        return 0;
    }

    // If selected is above visible window, scroll up
    if selected < current_scroll {
        return selected;
    }

    // If selected is below visible window, scroll down
    if selected >= current_scroll + visible_height {
        return selected.saturating_sub(visible_height - 1);
    }

    // Selected is within visible window, keep current scroll
    current_scroll
}

// === Edit State ===

#[derive(Clone)]
pub struct EditState {
    pub field: EditField,
    pub name: String,
    pub interval_secs: u64,
    pub interval_input: String, // For typing custom intervals
    pub engine: Engine,
    pub extraction: String, // "auto", "css:selector", "full", "rss"
    pub enabled: bool,
    pub agent_enabled: bool,
    pub agent_instructions: String,
    pub use_profile: bool,
    pub notify_use_global: bool,
    pub notify_type: NotifyType,
    pub notify_value: String,
    pub watch_id: uuid::Uuid,
}

impl EditState {
    pub fn from_watch(watch: &Watch) -> Self {
        // Convert Extraction to string representation
        let extraction = match &watch.extraction {
            crate::watch::Extraction::Auto => "auto".to_string(),
            crate::watch::Extraction::Selector { selector } => format!("css:{}", selector),
            crate::watch::Extraction::Full => "full".to_string(),
            crate::watch::Extraction::Meta { tags } => format!("meta:{}", tags.join(",")),
            crate::watch::Extraction::Rss => "rss".to_string(),
            crate::watch::Extraction::JsonLd { types } => {
                match types {
                    Some(t) if !t.is_empty() => format!("jsonld:{}", t.join(",")),
                    _ => "jsonld".to_string(),
                }
            }
        };

        // Convert NotifyTarget to structured representation
        let (notify_use_global, notify_type, notify_value) = match &watch.notify_target {
            None => (true, NotifyType::Ntfy, String::new()),
            Some(crate::config::NotifyTarget::Ntfy { topic, .. }) =>
                (false, NotifyType::Ntfy, topic.clone()),
            Some(crate::config::NotifyTarget::Gotify { server, token }) =>
                (false, NotifyType::Gotify, format!("{}|{}", server, token)),
            Some(crate::config::NotifyTarget::Slack { webhook_url }) =>
                (false, NotifyType::Slack, webhook_url.clone()),
            Some(crate::config::NotifyTarget::Discord { webhook_url }) =>
                (false, NotifyType::Discord, webhook_url.clone()),
            Some(crate::config::NotifyTarget::Telegram { chat_id, bot_token }) =>
                (false, NotifyType::Telegram, format!("{}|{}", chat_id, bot_token)),
            Some(crate::config::NotifyTarget::Pushover { user_key, api_token }) =>
                (false, NotifyType::Pushover, format!("{}|{}", user_key, api_token)),
            Some(crate::config::NotifyTarget::Command { command }) =>
                (false, NotifyType::Command, command.clone()),
            Some(crate::config::NotifyTarget::Email { .. }) =>
                (true, NotifyType::Ntfy, String::new()), // Email not supported in TUI yet
            Some(crate::config::NotifyTarget::Matrix { .. }) =>
                (true, NotifyType::Ntfy, String::new()), // Matrix not supported in TUI yet
        };

        Self {
            field: EditField::Name,
            name: watch.name.clone(),
            interval_secs: watch.interval_secs,
            interval_input: watch.interval_secs.to_string(),
            engine: watch.engine.clone(),
            extraction,
            enabled: watch.enabled,
            agent_enabled: watch.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false),
            agent_instructions: watch.agent_config.as_ref()
                .and_then(|c| c.instructions.clone())
                .unwrap_or_default(),
            use_profile: watch.use_profile,
            notify_use_global,
            notify_type,
            notify_value,
            watch_id: watch.id,
        }
    }

    pub fn next_field(&mut self) {
        self.field = match self.field {
            EditField::Name => EditField::Interval,
            EditField::Interval => EditField::Engine,
            EditField::Engine => EditField::Extraction,
            EditField::Extraction => EditField::Enabled,
            EditField::Enabled => EditField::Agent,
            EditField::Agent => EditField::AgentInstructions,
            EditField::AgentInstructions => EditField::UseProfile,
            EditField::UseProfile => EditField::Filters,
            EditField::Filters => EditField::Notify,
            EditField::Notify => {
                if self.notify_use_global {
                    EditField::Name
                } else {
                    EditField::NotifyCustom
                }
            }
            EditField::NotifyCustom => EditField::Name,
        };
    }

    pub fn prev_field(&mut self) {
        self.field = match self.field {
            EditField::Name => {
                if self.notify_use_global {
                    EditField::Notify
                } else {
                    EditField::NotifyCustom
                }
            }
            EditField::Interval => EditField::Name,
            EditField::Engine => EditField::Interval,
            EditField::Extraction => EditField::Engine,
            EditField::Enabled => EditField::Extraction,
            EditField::Agent => EditField::Enabled,
            EditField::AgentInstructions => EditField::Agent,
            EditField::UseProfile => EditField::AgentInstructions,
            EditField::Filters => EditField::UseProfile,
            EditField::Notify => EditField::Filters,
            EditField::NotifyCustom => EditField::Notify,
        };
    }
}

// === Reminder Edit State ===

#[derive(Clone)]
pub struct ReminderEditState {
    pub field: ReminderEditField,
    pub name: String,
    pub trigger_time: String,  // HH:MM format for editing
    pub recurring: bool,
    pub interval_input: String,  // For typing interval like "1d" or "2h"
    pub enabled: bool,
    pub reminder_id: uuid::Uuid,
}

impl ReminderEditState {
    pub fn from_reminder(reminder: &Reminder) -> Self {
        let local_time: chrono::DateTime<chrono::Local> = reminder.trigger_at.into();
        Self {
            field: ReminderEditField::Name,
            name: reminder.name.clone(),
            trigger_time: local_time.format("%H:%M").to_string(),
            recurring: reminder.interval_secs.is_some(),
            interval_input: reminder.interval_secs
                .map(|s| format_interval(s))
                .unwrap_or_else(|| "1d".to_string()),
            enabled: reminder.enabled,
            reminder_id: reminder.id,
        }
    }

    pub fn next_field(&mut self) {
        self.field = match self.field {
            ReminderEditField::Name => ReminderEditField::TriggerTime,
            ReminderEditField::TriggerTime => ReminderEditField::Recurring,
            ReminderEditField::Recurring => {
                if self.recurring {
                    ReminderEditField::Interval
                } else {
                    ReminderEditField::Enabled
                }
            }
            ReminderEditField::Interval => ReminderEditField::Enabled,
            ReminderEditField::Enabled => ReminderEditField::Name,
        };
    }

    pub fn prev_field(&mut self) {
        self.field = match self.field {
            ReminderEditField::Name => ReminderEditField::Enabled,
            ReminderEditField::TriggerTime => ReminderEditField::Name,
            ReminderEditField::Recurring => ReminderEditField::TriggerTime,
            ReminderEditField::Interval => ReminderEditField::Recurring,
            ReminderEditField::Enabled => {
                if self.recurring {
                    ReminderEditField::Interval
                } else {
                    ReminderEditField::Recurring
                }
            }
        };
    }
}

// === Filter Edit State ===

#[derive(Clone)]
pub struct FilterEditState {
    pub field: FilterEditField,
    pub filter_idx: Option<usize>,  // None = adding new filter
    pub target: FilterTarget,
    pub condition: FilterCondition,
    pub value: String,
}

impl FilterEditState {
    pub fn new() -> Self {
        Self {
            field: FilterEditField::Target,
            filter_idx: None,
            target: FilterTarget::New,
            condition: FilterCondition::Contains,
            value: String::new(),
        }
    }

    pub fn from_filter(idx: usize, filter: &Filter) -> Self {
        let (condition, value) = if let Some(ref v) = filter.contains {
            (FilterCondition::Contains, v.clone())
        } else if let Some(ref v) = filter.not_contains {
            (FilterCondition::NotContains, v.clone())
        } else if let Some(ref v) = filter.matches {
            (FilterCondition::Matches, v.clone())
        } else if let Some(n) = filter.size_gt {
            (FilterCondition::SizeGt, n.to_string())
        } else {
            (FilterCondition::Contains, String::new())
        };

        Self {
            field: FilterEditField::Target,
            filter_idx: Some(idx),
            target: filter.on.clone(),
            condition,
            value,
        }
    }

    pub fn to_filter(&self) -> Filter {
        let mut filter = Filter {
            on: self.target.clone(),
            contains: None,
            not_contains: None,
            matches: None,
            size_gt: None,
        };

        match self.condition {
            FilterCondition::Contains => filter.contains = Some(self.value.clone()),
            FilterCondition::NotContains => filter.not_contains = Some(self.value.clone()),
            FilterCondition::Matches => filter.matches = Some(self.value.clone()),
            FilterCondition::SizeGt => filter.size_gt = self.value.parse().ok(),
        }

        filter
    }

    pub fn next_field(&mut self) {
        self.field = match self.field {
            FilterEditField::Target => FilterEditField::Condition,
            FilterEditField::Condition => FilterEditField::Value,
            FilterEditField::Value => FilterEditField::Target,
        };
    }

    pub fn prev_field(&mut self) {
        self.field = match self.field {
            FilterEditField::Target => FilterEditField::Value,
            FilterEditField::Condition => FilterEditField::Target,
            FilterEditField::Value => FilterEditField::Condition,
        };
    }
}

// === Memory Inspector State ===

#[derive(Clone)]
pub struct MemoryInspectorState {
    pub watch_id: Uuid,
    pub watch_name: String,
    pub memory: AgentMemory,
    pub section: MemorySection,
    pub selected_item: usize,
}

impl MemoryInspectorState {
    pub fn next_section(&mut self) {
        self.section = match self.section {
            MemorySection::Counters => MemorySection::LastValues,
            MemorySection::LastValues => MemorySection::Notes,
            MemorySection::Notes => MemorySection::Counters,
        };
        self.selected_item = 0;
    }
}

// === Profile Inspector State ===

#[derive(Clone)]
pub struct ProfileInspectorState {
    pub profile: InterestProfile,
    pub global_memory: GlobalMemory,
    pub section: ProfileSection,
    pub selected_item: usize,
    pub scroll_offset: usize,
}

impl ProfileInspectorState {
    pub fn next_section(&mut self) {
        self.section = match self.section {
            ProfileSection::Description => ProfileSection::Interests,
            ProfileSection::Interests => ProfileSection::GlobalMemory,
            ProfileSection::GlobalMemory => ProfileSection::Description,
        };
        self.selected_item = 0;
    }
}

// === Wizard State ===

#[derive(Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub template: WatchTemplate,
    pub url: String,
    pub engine: Engine,  // Fetch engine
    pub name: String,
    pub extraction: String, // "auto", "css:<selector>", "xpath:<path>", "rss"
    pub interval_input: String, // Human-readable like "5m", "2h", "1d"
    pub interval_secs: u64,
    pub agent_enabled: bool,
    pub agent_instructions: String,
    pub test_result: Option<String>,
    /// Transform suggestion if detected (e.g., GitHub releases.atom)
    pub transform_suggestion: Option<TransformSuggestion>,
}

/// Simplified transform suggestion for TUI wizard
#[derive(Clone)]
pub struct TransformSuggestion {
    pub original_url: String,
    pub transformed_url: String,
    pub engine: Engine,
    pub description: &'static str,
    pub confidence: f32,
}

impl WizardState {
    pub fn new() -> Self {
        Self {
            step: WizardStep::Template,
            template: WatchTemplate::Custom,
            url: String::new(),
            engine: Engine::Http,
            name: String::new(),
            extraction: "auto".to_string(),
            interval_input: "5m".to_string(),
            interval_secs: 300,
            agent_enabled: false,
            agent_instructions: String::new(),
            test_result: None,
            transform_suggestion: None,
        }
    }

    /// Apply template settings
    pub fn apply_template(&mut self) {
        if let Some(instructions) = self.template.agent_instructions() {
            self.agent_enabled = true;
            self.agent_instructions = instructions.to_string();
        }
    }

    /// Auto-detect engine from URL patterns and check for URL transforms
    pub fn detect_engine(&mut self) {
        // First, check for URL transforms based on template intent
        let intent = self.template.to_intent();

        if intent != Intent::Generic {
            if let Ok(parsed_url) = url::Url::parse(&self.url) {
                if let Some(transform_match) = crate::transforms::match_transform(&parsed_url, intent) {
                    if transform_match.confidence >= 0.8 {
                        // Store the suggestion - user will see it in engine step
                        self.transform_suggestion = Some(TransformSuggestion {
                            original_url: self.url.clone(),
                            transformed_url: transform_match.url.to_string(),
                            engine: transform_match.engine.clone(),
                            description: transform_match.description,
                            confidence: transform_match.confidence,
                        });

                        // Pre-fill with transform suggestion
                        self.url = transform_match.url.to_string();
                        self.engine = transform_match.engine;
                        if self.engine == Engine::Rss {
                            self.extraction = "rss".to_string();
                        }

                        // Generate a name from the URL
                        self.name = generate_name_from_transform_url(&transform_match.url);
                        return;
                    }
                }
            }
        }

        // Fallback to simple URL pattern detection
        let url_lower = self.url.to_lowercase();
        if url_lower.ends_with(".rss")
            || url_lower.ends_with(".xml")
            || url_lower.ends_with("/feed")
            || url_lower.contains("/rss")
            || url_lower.contains("/feed/")
            || url_lower.contains("atom.xml")
            || url_lower.contains("/atom")
        {
            self.engine = Engine::Rss;
            self.extraction = "rss".to_string();
        }
    }

    /// Clear transform suggestion (e.g., when user declines it)
    pub fn clear_transform(&mut self) {
        if let Some(ref suggestion) = self.transform_suggestion {
            // Restore original URL
            self.url = suggestion.original_url.clone();
            self.engine = Engine::Http;
            self.extraction = "auto".to_string();
            self.name.clear();
        }
        self.transform_suggestion = None;
    }

    pub fn next_step(&mut self) {
        self.step = match self.step {
            WizardStep::Template => {
                // Apply template settings when leaving template step
                self.apply_template();
                WizardStep::Url
            }
            WizardStep::Url => {
                // Auto-detect RSS when leaving URL step
                self.detect_engine();
                WizardStep::Engine
            }
            WizardStep::Engine => WizardStep::Name,
            WizardStep::Name => WizardStep::Extraction,
            WizardStep::Extraction => WizardStep::Interval,
            WizardStep::Interval => WizardStep::Agent,
            WizardStep::Agent => WizardStep::Review,
            WizardStep::Review => WizardStep::Review,
        };
    }

    pub fn prev_step(&mut self) {
        self.step = match self.step {
            WizardStep::Template => WizardStep::Template,
            WizardStep::Url => WizardStep::Template,
            WizardStep::Engine => WizardStep::Url,
            WizardStep::Name => WizardStep::Engine,
            WizardStep::Extraction => WizardStep::Name,
            WizardStep::Interval => WizardStep::Extraction,
            WizardStep::Agent => WizardStep::Interval,
            WizardStep::Review => WizardStep::Agent,
        };
    }
}

// === Reminder Wizard State ===

#[derive(Clone)]
pub struct ReminderWizardState {
    pub step: ReminderWizardStep,
    pub name: String,
    pub when_type: String,  // "in" or "at"
    pub when_value: String, // e.g., "1h" or "14:30"
    pub recurring: bool,
    pub interval: String,   // e.g., "1d" for daily
}

impl ReminderWizardState {
    pub fn new() -> Self {
        Self {
            step: ReminderWizardStep::Name,
            name: String::new(),
            when_type: "in".to_string(),
            when_value: "1h".to_string(),
            recurring: false,
            interval: "1d".to_string(),
        }
    }

    pub fn next_step(&mut self) {
        self.step = match self.step {
            ReminderWizardStep::Name => ReminderWizardStep::When,
            ReminderWizardStep::When => ReminderWizardStep::Recurring,
            ReminderWizardStep::Recurring => ReminderWizardStep::Review,
            ReminderWizardStep::Review => ReminderWizardStep::Review,
        };
    }

    pub fn prev_step(&mut self) {
        self.step = match self.step {
            ReminderWizardStep::Name => ReminderWizardStep::Name,
            ReminderWizardStep::When => ReminderWizardStep::Name,
            ReminderWizardStep::Recurring => ReminderWizardStep::When,
            ReminderWizardStep::Review => ReminderWizardStep::Recurring,
        };
    }
}

// === Notify Setup State ===

#[derive(Clone)]
pub struct NotifySetupState {
    pub notify_type: NotifyType,
    pub field1: String,  // topic/webhook/server/command
    pub field2: String,  // token (for gotify)
    pub step: usize,     // 0 = select type, 1 = enter details
}

impl NotifySetupState {
    pub fn new() -> Self {
        Self {
            notify_type: NotifyType::Ntfy,
            field1: String::new(),
            field2: String::new(),
            step: 0,
        }
    }
}

// === Health Dashboard State ===

#[derive(Clone)]
pub struct HealthDashboardState {
    pub daemon_running: bool,
    pub daemon_pid: Option<u32>,
    pub last_check: Option<chrono::DateTime<Utc>>,
    pub healthy_watches: usize,
    pub stale_watches: usize,
    pub error_watches: usize,
    pub notifications_24h: usize,
}

impl HealthDashboardState {
    pub fn new() -> Self {
        Self {
            daemon_running: false,
            daemon_pid: None,
            last_check: None,
            healthy_watches: 0,
            stale_watches: 0,
            error_watches: 0,
            notifications_24h: 0,
        }
    }
}

// === Watch Templates ===

#[derive(Clone, Debug, PartialEq)]
pub enum WatchTemplate {
    Custom,
    PriceDrop,
    BackInStock,
    JobPostings,
    Changelog,
}

impl WatchTemplate {
    pub fn all() -> Vec<WatchTemplate> {
        vec![
            WatchTemplate::Custom,
            WatchTemplate::PriceDrop,
            WatchTemplate::BackInStock,
            WatchTemplate::JobPostings,
            WatchTemplate::Changelog,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            WatchTemplate::Custom => "Custom (no template)",
            WatchTemplate::PriceDrop => "Price Drop Monitor",
            WatchTemplate::BackInStock => "Back-in-Stock Alert",
            WatchTemplate::JobPostings => "Job Posting Tracker",
            WatchTemplate::Changelog => "Changelog/Release Watcher",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            WatchTemplate::Custom => "Start from scratch with full control",
            WatchTemplate::PriceDrop => "Alert when price drops below threshold",
            WatchTemplate::BackInStock => "Alert when item becomes available",
            WatchTemplate::JobPostings => "Track new job listings",
            WatchTemplate::Changelog => "Monitor software releases and updates",
        }
    }

    pub fn agent_instructions(&self) -> Option<&'static str> {
        match self {
            WatchTemplate::Custom => None,
            WatchTemplate::PriceDrop => Some(
                "Track the current price. Alert me when the price drops significantly. \
                 Note the previous price and new price in the notification."
            ),
            WatchTemplate::BackInStock => Some(
                "Alert me when the item becomes available or back in stock. \
                 Ignore 'out of stock' or 'unavailable' status unless it changes to available."
            ),
            WatchTemplate::JobPostings => Some(
                "Alert me on NEW job postings only. Ignore updates to existing listings. \
                 Include the job title and key requirements in the notification."
            ),
            WatchTemplate::Changelog => Some(
                "Summarize new releases and version updates. Alert on major version bumps. \
                 Include the version number and key changes."
            ),
        }
    }

    /// Map template to Intent for URL transform detection
    pub fn to_intent(&self) -> Intent {
        match self {
            WatchTemplate::Custom => Intent::Generic,
            WatchTemplate::PriceDrop => Intent::Price,
            WatchTemplate::BackInStock => Intent::Stock,
            WatchTemplate::JobPostings => Intent::Jobs,
            WatchTemplate::Changelog => Intent::Release,
        }
    }
}

/// Generate a human-readable name from a transformed URL
fn generate_name_from_transform_url(url: &url::Url) -> String {
    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if let Some(host) = url.host_str() {
        // GitHub/GitLab/Codeberg: "owner/repo/releases.atom" -> "owner/repo"
        if (host == "github.com" || host == "gitlab.com" || host == "codeberg.org")
            && segments.len() >= 2
        {
            let owner = segments[0];
            let repo = segments[1];
            return format!("{}/{}", owner, repo);
        }

        // Reddit: "r/subreddit.rss" -> "r/subreddit"
        if host.contains("reddit.com") && segments.len() >= 2 && segments[0] == "r" {
            let subreddit = segments[1].trim_end_matches(".rss");
            return format!("r/{}", subreddit);
        }

        // Hacker News
        if host == "news.ycombinator.com" {
            return "Hacker News".to_string();
        }

        // PyPI
        if host == "pypi.org" && segments.len() >= 2 && segments[0] == "project" {
            let package = segments[1];
            return format!("PyPI: {}", package);
        }
    }

    // Fallback: use host
    url.host_str().unwrap_or("Watch").to_string()
}
