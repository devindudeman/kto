//! TUI input handling - keyboard and mouse event handlers

use std::io;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, Clear as TermClear, ClearType},
    cursor,
};
use ratatui::layout::{Margin, Rect};
use chrono::Utc;
use uuid::Uuid;

use crate::error::Result;
use crate::interests::InterestProfile;
use crate::watch::{AgentConfig, Engine, Reminder};

use super::state::*;
use super::types::*;
use super::utils::*;
use super::editor::open_in_editor;

/// Main key event dispatcher
pub fn handle_key_event(app: &mut App, key: KeyCode, modifiers: KeyModifiers) -> Result<()> {
    let mode = app.mode.clone();
    match mode {
        Mode::Normal => handle_normal_input(app, key, modifiers),
        Mode::Help => {
            app.mode = Mode::Normal;
            Ok(())
        }
        Mode::Confirm(action) => handle_confirm_input(app, key, action),
        Mode::Edit => handle_edit_input(app, key),
        Mode::EditReminder => handle_reminder_edit_input(app, key),
        Mode::Search => handle_search_input(app, key),
        Mode::ViewChange => handle_view_change_input(app, key),
        Mode::Wizard => handle_wizard_input(app, key),
        Mode::ReminderWizard => handle_reminder_wizard_input(app, key),
        Mode::Describe => handle_describe_input(app, key),
        Mode::Logs => handle_logs_input(app, key),
        Mode::NotifySetup => handle_notify_setup_input(app, key),
        Mode::FilterList => handle_filter_list_input(app, key),
        Mode::FilterEdit => handle_filter_edit_input(app, key),
        Mode::MemoryInspector => handle_memory_inspector_input(app, key),
        Mode::ProfileInspector => handle_profile_inspector_input(app, key),
    }
}

/// Handle mouse events
pub fn handle_mouse_event(app: &mut App, mouse: MouseEvent) -> Result<()> {
    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        let row = mouse.row as usize;
        let col = mouse.column as usize;

        match app.mode {
            Mode::Normal => {
                let areas = app.layout_areas;

                let inside = |area: Rect| -> bool {
                    col >= area.x as usize && col < (area.x + area.width) as usize &&
                    row >= area.y as usize && row < (area.y + area.height) as usize
                };

                if inside(areas.watches) {
                    let list_idx = row.saturating_sub(areas.watches.y as usize + 1);
                    let filtered = app.filtered_watches();
                    if list_idx < filtered.len() {
                        app.selected_watch = list_idx;
                        app.update_changes();
                        app.focus = Pane::Watches;
                    }
                } else if inside(areas.reminders) {
                    let list_idx = row.saturating_sub(areas.reminders.y as usize + 1);
                    if list_idx < app.reminders.len() {
                        app.selected_reminder = list_idx;
                        app.focus = Pane::Reminders;
                    }
                } else if inside(areas.changes) {
                    let list_idx = row.saturating_sub(areas.changes.y as usize + 1);
                    if list_idx < app.changes.len() {
                        app.selected_change = list_idx;
                        app.focus = Pane::Changes;
                    }
                }
            }
            Mode::Wizard => {
                let area = app.layout_areas.wizard_modal;
                let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
                let content_left = inner.x as usize;
                let button_zone_start = (inner.y + inner.height / 2) as usize;
                let button_zone_end = (inner.y + inner.height) as usize;

                if let Some(ref mut wizard) = app.wizard_state {
                    if row >= button_zone_start && row < button_zone_end {
                        if col >= content_left + 2 && col < content_left + 12 {
                            wizard.prev_step();
                        } else if col >= content_left + 12 && col < content_left + 26 {
                            wizard.next_step();
                        }
                    }
                }
            }
            Mode::ReminderWizard => {
                let area = app.layout_areas.reminder_wizard_modal;
                let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
                let content_left = inner.x as usize;
                let button_zone_start = (inner.y + inner.height / 2) as usize;
                let button_zone_end = (inner.y + inner.height) as usize;

                if let Some(ref mut wizard) = app.reminder_wizard_state {
                    if row >= button_zone_start && row < button_zone_end {
                        if col >= content_left + 2 && col < content_left + 12 {
                            wizard.prev_step();
                        } else if col >= content_left + 12 && col < content_left + 26 {
                            wizard.next_step();
                        }
                    }
                }
            }
            Mode::Logs => {
                let area = app.layout_areas.logs_modal;
                let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
                let content_top = inner.y as usize;
                let content_bottom = (inner.y + inner.height) as usize;

                if row >= content_top + 2 && row < content_bottom.saturating_sub(2) {
                    let list_idx = row.saturating_sub(content_top + 2);
                    if list_idx < app.all_changes.len() {
                        app.selected_log = list_idx;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_normal_input(app: &mut App, key: KeyCode, modifiers: KeyModifiers) -> Result<()> {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('?') => app.mode = Mode::Help,
        KeyCode::Char('/') => {
            app.mode = Mode::Search;
            app.status_message = Some("Type to filter...".to_string());
        }
        KeyCode::Char('h') | KeyCode::Left => app.focus = Pane::Watches,
        KeyCode::Char('l') | KeyCode::Right => app.focus = Pane::Changes,
        KeyCode::Tab => {
            app.focus = match app.focus {
                Pane::Watches => Pane::Changes,
                Pane::Changes => Pane::Reminders,
                Pane::Reminders => Pane::Watches,
            };
        }
        KeyCode::Char('j') | KeyCode::Down => {
            match app.focus {
                Pane::Watches => app.next_watch(),
                Pane::Changes => app.next_change(),
                Pane::Reminders => app.next_reminder(),
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            match app.focus {
                Pane::Watches => app.previous_watch(),
                Pane::Changes => app.previous_change(),
                Pane::Reminders => app.previous_reminder(),
            }
        }
        KeyCode::Char('g') => {
            match app.focus {
                Pane::Watches => app.first_watch(),
                Pane::Changes => app.first_change(),
                Pane::Reminders => app.selected_reminder = 0,
            }
        }
        KeyCode::Char('G') => {
            match app.focus {
                Pane::Watches => app.last_watch(),
                Pane::Changes => app.last_change(),
                Pane::Reminders => app.selected_reminder = app.reminders.len().saturating_sub(1),
            }
        }
        KeyCode::PageUp => {
            match app.focus {
                Pane::Watches => { for _ in 0..5 { app.previous_watch(); } }
                Pane::Changes => { for _ in 0..5 { app.previous_change(); } }
                Pane::Reminders => { for _ in 0..5 { app.previous_reminder(); } }
            }
        }
        KeyCode::PageDown => {
            match app.focus {
                Pane::Watches => { for _ in 0..5 { app.next_watch(); } }
                Pane::Changes => { for _ in 0..5 { app.next_change(); } }
                Pane::Reminders => { for _ in 0..5 { app.next_reminder(); } }
            }
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            match app.focus {
                Pane::Watches => { for _ in 0..5 { app.previous_watch(); } }
                Pane::Changes => { for _ in 0..5 { app.previous_change(); } }
                Pane::Reminders => { for _ in 0..5 { app.previous_reminder(); } }
            }
        }
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            match app.focus {
                Pane::Watches => { for _ in 0..5 { app.next_watch(); } }
                Pane::Changes => { for _ in 0..5 { app.next_change(); } }
                Pane::Reminders => { for _ in 0..5 { app.next_reminder(); } }
            }
        }
        KeyCode::Enter => {
            if app.focus == Pane::Changes && !app.changes.is_empty() {
                app.scroll_offset = 0;
                app.mode = Mode::ViewChange;
            }
        }
        KeyCode::Char('p') => {
            match app.focus {
                Pane::Watches => app.toggle_pause()?,
                Pane::Reminders => app.toggle_reminder_enabled()?,
                _ => {}
            }
        }
        KeyCode::Char('e') => {
            if app.focus == Pane::Watches {
                app.start_edit();
            } else if app.focus == Pane::Reminders {
                app.start_reminder_edit();
            }
        }
        KeyCode::Char('d') => {
            match app.focus {
                Pane::Watches => app.mode = Mode::Confirm(ConfirmAction::Delete),
                Pane::Reminders => app.mode = Mode::Confirm(ConfirmAction::DeleteReminder),
                _ => {}
            }
        }
        KeyCode::Char('D') => {
            if app.focus == Pane::Watches && !app.watches.is_empty() {
                app.mode = Mode::Describe;
            }
        }
        KeyCode::Char('E') => {
            if app.focus == Pane::Watches {
                if let Some(watch) = app.selected_watch() {
                    if let Some(error) = app.watch_errors.get(&watch.id) {
                        app.status_message = Some(format!("Error: {}", error));
                    } else {
                        app.status_message = Some("No errors for this watch".to_string());
                    }
                }
            }
        }
        KeyCode::Char('t') => app.mode = Mode::Confirm(ConfirmAction::Test),
        KeyCode::Char('c') => app.mode = Mode::Confirm(ConfirmAction::ForceCheck),
        KeyCode::Char('n') => {
            match app.focus {
                Pane::Watches => {
                    app.wizard_state = Some(WizardState::new());
                    app.mode = Mode::Wizard;
                }
                Pane::Reminders => {
                    app.reminder_wizard_state = Some(ReminderWizardState::new());
                    app.mode = Mode::ReminderWizard;
                }
                _ => {}
            }
        }
        KeyCode::Char('L') => {
            app.all_changes = app.db.get_all_recent_changes(50)?;
            app.selected_log = 0;
            app.scroll_offset = 0;
            app.mode = Mode::Logs;
        }
        KeyCode::Char('N') => {
            app.notify_setup_state = Some(NotifySetupState::new());
            app.mode = Mode::NotifySetup;
        }
        KeyCode::Char('M') => {
            if app.focus == Pane::Watches {
                if let Some(watch) = app.selected_watch() {
                    let watch_id = watch.id;
                    let watch_name = watch.name.clone();
                    match app.db.get_agent_memory(&watch_id) {
                        Ok(memory) => {
                            app.memory_inspector_state = Some(MemoryInspectorState {
                                watch_id,
                                watch_name,
                                memory,
                                section: MemorySection::Counters,
                                selected_item: 0,
                            });
                            app.mode = Mode::MemoryInspector;
                        }
                        Err(e) => {
                            app.status_message = Some(format!("Failed to load memory: {}", e));
                        }
                    }
                }
            }
        }
        KeyCode::Char('P') => {
            match InterestProfile::load() {
                Ok(profile) => {
                    let global_memory = app.db.get_global_memory().unwrap_or_default();
                    app.profile_inspector_state = Some(ProfileInspectorState {
                        profile,
                        global_memory,
                        section: ProfileSection::Description,
                        selected_item: 0,
                        scroll_offset: 0,
                    });
                    app.mode = Mode::ProfileInspector;
                }
                Err(e) => {
                    app.status_message = Some(format!("Failed to load profile: {}", e));
                }
            }
        }
        KeyCode::Char('r') => {
            app.refresh()?;
            app.status_message = Some("Refreshed".to_string());
        }
        _ => {}
    }
    Ok(())
}

fn handle_search_input(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc => {
            app.filter_text.clear();
            app.mode = Mode::Normal;
            app.status_message = None;
        }
        KeyCode::Enter => {
            app.mode = Mode::Normal;
            if app.filter_text.is_empty() {
                app.status_message = None;
            } else {
                let count = app.filtered_watches().len();
                app.status_message = Some(format!("Filter: \"{}\" ({} matches)", app.filter_text, count));
            }
        }
        KeyCode::Backspace => {
            app.filter_text.pop();
            if let Some(watch) = app.selected_watch() {
                app.changes = app.db.get_recent_changes(&watch.id, 20).unwrap_or_default();
            } else {
                app.changes.clear();
            }
            app.selected_change = 0;
        }
        KeyCode::Char(c) => {
            app.filter_text.push(c);
            if let Some(watch) = app.selected_watch() {
                app.changes = app.db.get_recent_changes(&watch.id, 20).unwrap_or_default();
            } else {
                app.changes.clear();
            }
            app.selected_change = 0;
        }
        _ => {}
    }
    Ok(())
}

fn handle_view_change_input(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = app.previous_mode.take().unwrap_or(Mode::Normal);
            app.scroll_offset = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.scroll_offset = app.scroll_offset.saturating_add(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.scroll_offset = app.scroll_offset.saturating_sub(1);
        }
        KeyCode::Char('g') => app.scroll_offset = 0,
        KeyCode::Char('G') => app.scroll_offset = 1000,
        KeyCode::PageDown => {
            app.scroll_offset = app.scroll_offset.saturating_add(10);
        }
        KeyCode::PageUp => {
            app.scroll_offset = app.scroll_offset.saturating_sub(10);
        }
        KeyCode::Char('u') => {
            app.diff_view_mode = match app.diff_view_mode {
                DiffViewMode::Inline => DiffViewMode::Unified,
                DiffViewMode::Unified => DiffViewMode::Inline,
            };
        }
        _ => {}
    }
    Ok(())
}

fn handle_confirm_input(app: &mut App, key: KeyCode, action: ConfirmAction) -> Result<()> {
    match key {
        KeyCode::Char('y') | KeyCode::Enter => {
            match action {
                ConfirmAction::Delete => {
                    app.delete_selected()?;
                }
                ConfirmAction::DeleteReminder => {
                    app.delete_selected_reminder()?;
                }
                ConfirmAction::Test => {
                    app.test_selected()?;
                }
                ConfirmAction::ForceCheck => {
                    app.check_selected()?;
                }
            }
            app.mode = Mode::Normal;
        }
        _ => {
            app.mode = Mode::Normal;
        }
    }
    Ok(())
}

fn handle_wizard_input(app: &mut App, key: KeyCode) -> Result<()> {
    if let Some(ref mut wizard) = app.wizard_state {
        match key {
            KeyCode::Esc => {
                app.wizard_state = None;
                app.mode = Mode::Normal;
            }
            KeyCode::Tab | KeyCode::Enter => {
                if wizard.step == WizardStep::Review {
                    let wizard_data = wizard.clone();
                    if let Err(e) = create_watch_from_wizard(app, &wizard_data) {
                        app.status_message = Some(format!("Failed to create watch: {}", e));
                    } else {
                        app.status_message = Some("Watch created!".to_string());
                        app.wizard_state = None;
                        app.mode = Mode::Normal;
                        let _ = app.refresh();
                    }
                    return Ok(());
                } else {
                    wizard.next_step();
                }
            }
            KeyCode::BackTab => wizard.prev_step(),
            KeyCode::Char(' ') if wizard.step == WizardStep::Agent => {
                wizard.agent_enabled = !wizard.agent_enabled;
            }
            KeyCode::Char(' ') if wizard.step == WizardStep::Engine => {
                wizard.engine = match wizard.engine {
                    Engine::Http => Engine::Playwright,
                    Engine::Playwright => Engine::Rss,
                    Engine::Rss => Engine::Http,
                    Engine::Shell { .. } => Engine::Http,
                };
                if wizard.engine == Engine::Rss {
                    wizard.extraction = "rss".to_string();
                } else if wizard.extraction == "rss" {
                    wizard.extraction = "auto".to_string();
                }
            }
            KeyCode::Char('e') if wizard.step == WizardStep::Agent => {
                disable_raw_mode()?;
                execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
                if let Ok(edited) = open_in_editor(&wizard.agent_instructions) {
                    wizard.agent_instructions = edited;
                }
                enable_raw_mode()?;
                execute!(
                    io::stdout(),
                    EnterAlternateScreen,
                    TermClear(ClearType::All),
                    cursor::MoveTo(0, 0),
                    EnableMouseCapture
                )?;
                app.needs_full_redraw = true;
            }
            KeyCode::Char(c) => {
                match wizard.step {
                    WizardStep::Url => wizard.url.push(c),
                    WizardStep::Name => wizard.name.push(c),
                    WizardStep::Extraction => wizard.extraction.push(c),
                    WizardStep::Interval => {
                        if c.is_ascii_digit() || "smhdw".contains(c) {
                            wizard.interval_input.push(c);
                            if let Some(secs) = parse_duration_str(&wizard.interval_input) {
                                wizard.interval_secs = secs;
                            } else if let Ok(secs) = wizard.interval_input.parse::<u64>() {
                                wizard.interval_secs = secs;
                            }
                        }
                    }
                    WizardStep::Agent => {
                        if wizard.agent_instructions.len() < 500 {
                            wizard.agent_instructions.push(c);
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                match wizard.step {
                    WizardStep::Url => { wizard.url.pop(); }
                    WizardStep::Name => { wizard.name.pop(); }
                    WizardStep::Extraction => { wizard.extraction.pop(); }
                    WizardStep::Interval => {
                        wizard.interval_input.pop();
                        if let Some(secs) = parse_duration_str(&wizard.interval_input) {
                            wizard.interval_secs = secs;
                        }
                    }
                    WizardStep::Agent => { wizard.agent_instructions.pop(); }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn create_watch_from_wizard(app: &mut App, wizard: &WizardState) -> Result<()> {
    use crate::watch::Watch;

    let watch = Watch {
        id: Uuid::new_v4(),
        name: if wizard.name.is_empty() {
            wizard.url.split('/').last().unwrap_or("New Watch").to_string()
        } else {
            wizard.name.clone()
        },
        url: wizard.url.clone(),
        enabled: true,
        interval_secs: wizard.interval_secs.max(10),
        engine: wizard.engine.clone(),
        extraction: parse_extraction_string(&wizard.extraction),
        normalization: crate::watch::Normalization::default(),
        filters: Vec::new(),
        headers: std::collections::HashMap::new(),
        cookie_file: None,
        storage_state: None,
        notify_target: None,
        use_profile: false,
        tags: Vec::new(),
        agent_config: if wizard.agent_enabled {
            Some(AgentConfig {
                enabled: true,
                prompt_template: None,
                instructions: if wizard.agent_instructions.is_empty() {
                    None
                } else {
                    Some(wizard.agent_instructions.clone())
                },
            })
        } else {
            None
        },
        created_at: Utc::now(),
    };

    app.db.insert_watch(&watch)?;
    Ok(())
}

fn handle_reminder_wizard_input(app: &mut App, key: KeyCode) -> Result<()> {
    if let Some(ref mut wizard) = app.reminder_wizard_state {
        match key {
            KeyCode::Esc => {
                app.reminder_wizard_state = None;
                app.mode = Mode::Normal;
            }
            KeyCode::Tab | KeyCode::Enter => {
                if wizard.step == ReminderWizardStep::Review {
                    let wizard_data = wizard.clone();
                    if let Err(e) = create_reminder_from_wizard(app, &wizard_data) {
                        app.status_message = Some(format!("Failed to create reminder: {}", e));
                    } else {
                        app.status_message = Some("Reminder created!".to_string());
                        app.reminder_wizard_state = None;
                        app.mode = Mode::Normal;
                        let _ = app.refresh();
                    }
                    return Ok(());
                } else {
                    wizard.next_step();
                }
            }
            KeyCode::BackTab => wizard.prev_step(),
            KeyCode::Char(' ') => {
                match wizard.step {
                    ReminderWizardStep::When => {
                        wizard.when_type = if wizard.when_type == "in" { "at".to_string() } else { "in".to_string() };
                        wizard.when_value = if wizard.when_type == "in" { "1h".to_string() } else { "14:00".to_string() };
                    }
                    ReminderWizardStep::Recurring => {
                        wizard.recurring = !wizard.recurring;
                    }
                    _ => {}
                }
            }
            KeyCode::Char(c) => {
                match wizard.step {
                    ReminderWizardStep::Name => wizard.name.push(c),
                    ReminderWizardStep::When => wizard.when_value.push(c),
                    ReminderWizardStep::Recurring if wizard.recurring => wizard.interval.push(c),
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                match wizard.step {
                    ReminderWizardStep::Name => { wizard.name.pop(); }
                    ReminderWizardStep::When => { wizard.when_value.pop(); }
                    ReminderWizardStep::Recurring if wizard.recurring => { wizard.interval.pop(); }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn create_reminder_from_wizard(app: &mut App, wizard: &ReminderWizardState) -> Result<()> {
    let trigger_at = if wizard.when_type == "in" {
        let secs = parse_duration_str(&wizard.when_value).unwrap_or(3600);
        Utc::now() + chrono::Duration::seconds(secs as i64)
    } else {
        parse_time_to_datetime(&wizard.when_value).unwrap_or_else(|| Utc::now() + chrono::Duration::hours(1))
    };

    let interval_secs = if wizard.recurring {
        parse_duration_str(&wizard.interval)
    } else {
        None
    };

    let reminder = Reminder {
        id: Uuid::new_v4(),
        name: wizard.name.clone(),
        message: None,
        trigger_at,
        interval_secs,
        enabled: true,
        notify_target: None,
        created_at: Utc::now(),
    };

    app.db.insert_reminder(&reminder)?;
    Ok(())
}

fn handle_describe_input(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('D') => {
            app.mode = Mode::Normal;
            app.scroll_offset = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.scroll_offset = app.scroll_offset.saturating_add(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.scroll_offset = app.scroll_offset.saturating_sub(1);
        }
        _ => {}
    }
    Ok(())
}

fn handle_logs_input(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = Mode::Normal;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.selected_log < app.all_changes.len().saturating_sub(1) {
                app.selected_log += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected_log = app.selected_log.saturating_sub(1);
        }
        KeyCode::Char('r') => {
            app.all_changes = app.db.get_all_recent_changes(50)?;
            app.status_message = Some("Logs refreshed".to_string());
        }
        KeyCode::Enter => {
            if let Some((change, _)) = app.all_changes.get(app.selected_log) {
                if let Some(idx) = app.watches.iter().position(|w| w.id == change.watch_id) {
                    app.selected_watch = idx;
                    app.changes = app.db.get_recent_changes(&change.watch_id, 20)?;
                    if let Some(change_idx) = app.changes.iter().position(|c| c.id == change.id) {
                        app.selected_change = change_idx;
                    }
                    app.scroll_offset = 0;
                    app.previous_mode = Some(Mode::Logs);
                    app.mode = Mode::ViewChange;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_notify_setup_input(app: &mut App, key: KeyCode) -> Result<()> {
    if let Some(ref mut state) = app.notify_setup_state {
        if state.step == 0 {
            match key {
                KeyCode::Esc => {
                    app.notify_setup_state = None;
                    app.mode = Mode::Normal;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    state.notify_type = state.notify_type.next();
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.notify_type = match state.notify_type {
                        NotifyType::Ntfy => NotifyType::Command,
                        NotifyType::Gotify => NotifyType::Ntfy,
                        NotifyType::Slack => NotifyType::Gotify,
                        NotifyType::Discord => NotifyType::Slack,
                        NotifyType::Telegram => NotifyType::Discord,
                        NotifyType::Pushover => NotifyType::Telegram,
                        NotifyType::Command => NotifyType::Pushover,
                    };
                }
                KeyCode::Enter => {
                    state.step = 1;
                }
                _ => {}
            }
        } else {
            match key {
                KeyCode::Esc => {
                    state.step = 0;
                }
                KeyCode::Enter => {
                    if let Some(target) = build_notify_target(&state.notify_type, &state.field1) {
                        let mut config = crate::config::Config::load().unwrap_or_default();
                        config.default_notify = Some(target);
                        if let Err(e) = config.save() {
                            app.status_message = Some(format!("Failed to save: {}", e));
                        } else {
                            app.status_message = Some("Notifications configured!".to_string());
                            app.notify_setup_state = None;
                            app.mode = Mode::Normal;
                        }
                    } else {
                        app.status_message = Some("Invalid configuration".to_string());
                    }
                }
                KeyCode::Char(c) => {
                    state.field1.push(c);
                }
                KeyCode::Backspace => {
                    state.field1.pop();
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn handle_edit_input(app: &mut App, key: KeyCode) -> Result<()> {
    if let Some(ref mut edit_state) = app.edit_state {
        match key {
            KeyCode::Esc => app.cancel_edit(),
            KeyCode::Enter => app.save_edit()?,
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => edit_state.next_field(),
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => edit_state.prev_field(),
            KeyCode::Char(' ') => {
                match edit_state.field {
                    EditField::Enabled => edit_state.enabled = !edit_state.enabled,
                    EditField::Agent => edit_state.agent_enabled = !edit_state.agent_enabled,
                    EditField::UseProfile => edit_state.use_profile = !edit_state.use_profile,
                    EditField::Engine => {
                        edit_state.engine = match edit_state.engine {
                            Engine::Http => Engine::Playwright,
                            Engine::Playwright => Engine::Rss,
                            Engine::Rss => Engine::Http,
                            Engine::Shell { .. } => Engine::Http,
                        };
                    }
                    EditField::Extraction => {
                        edit_state.extraction = match edit_state.extraction.as_str() {
                            "auto" => "full".to_string(),
                            "full" => "rss".to_string(),
                            "rss" => "auto".to_string(),
                            _ => "auto".to_string(),
                        };
                    }
                    EditField::Notify => {
                        edit_state.notify_use_global = !edit_state.notify_use_global;
                    }
                    EditField::NotifyCustom => {
                        edit_state.notify_type = edit_state.notify_type.next();
                        edit_state.notify_value.clear();
                    }
                    _ => {}
                }
            }
            KeyCode::Char('T') if edit_state.field == EditField::Notify || edit_state.field == EditField::NotifyCustom => {
                use crate::notify::{send_notification, NotificationPayload};

                let target = if edit_state.notify_use_global {
                    crate::config::Config::load().ok().and_then(|c| c.default_notify)
                } else {
                    build_notify_target(&edit_state.notify_type, &edit_state.notify_value)
                };

                if let Some(target) = target {
                    let watch_url = app.watches
                        .iter()
                        .find(|w| w.id == edit_state.watch_id)
                        .map(|w| w.url.clone())
                        .unwrap_or_else(|| "https://example.com/test".to_string());

                    let payload = NotificationPayload {
                        watch_id: edit_state.watch_id.to_string(),
                        watch_name: edit_state.name.clone(),
                        url: watch_url,
                        old_content: "Previous content".to_string(),
                        new_content: "Updated content".to_string(),
                        diff: "[-Previous][+Updated] content".to_string(),
                        smart_summary: Some("Test notification from TUI".to_string()),
                        agent_title: Some(format!("Test: {}", edit_state.name)),
                        agent_bullets: Some(vec!["Testing notification setup".to_string()]),
                        agent_summary: None,
                        agent_analysis: None,
                        agent_error: None,
                        detected_at: Utc::now(),
                    };

                    match send_notification(&target, &payload) {
                        Ok(()) => app.status_message = Some("Test notification sent!".to_string()),
                        Err(e) => app.status_message = Some(format!("Notification failed: {}", e)),
                    }
                } else {
                    app.status_message = Some("No notification target configured".to_string());
                }
            }
            KeyCode::Char('e') if edit_state.field == EditField::AgentInstructions => {
                disable_raw_mode()?;
                execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
                let current_instructions = edit_state.agent_instructions.clone();
                match open_in_editor(&current_instructions) {
                    Ok(new_instructions) => {
                        edit_state.agent_instructions = new_instructions;
                        app.status_message = Some("Instructions updated".to_string());
                    }
                    Err(e) => {
                        app.status_message = Some(format!("Editor error: {}", e));
                    }
                }
                enable_raw_mode()?;
                execute!(
                    io::stdout(),
                    EnterAlternateScreen,
                    TermClear(ClearType::All),
                    cursor::MoveTo(0, 0),
                    EnableMouseCapture
                )?;
                app.needs_full_redraw = true;
            }
            KeyCode::Char('f') if edit_state.field == EditField::Filters => {
                app.selected_filter = 0;
                app.mode = Mode::FilterList;
            }
            KeyCode::Char('+') | KeyCode::Right => {
                if edit_state.field == EditField::Interval {
                    edit_state.interval_secs = next_interval_preset(edit_state.interval_secs);
                    edit_state.interval_input = edit_state.interval_secs.to_string();
                }
            }
            KeyCode::Char('-') | KeyCode::Left => {
                if edit_state.field == EditField::Interval {
                    edit_state.interval_secs = prev_interval_preset(edit_state.interval_secs);
                    edit_state.interval_input = edit_state.interval_secs.to_string();
                }
            }
            KeyCode::Backspace => {
                match edit_state.field {
                    EditField::Name => { edit_state.name.pop(); }
                    EditField::Interval => {
                        edit_state.interval_input.pop();
                        if let Ok(secs) = edit_state.interval_input.parse::<u64>() {
                            if secs > 0 { edit_state.interval_secs = secs; }
                        }
                    }
                    EditField::Extraction => { edit_state.extraction.pop(); }
                    EditField::NotifyCustom => { edit_state.notify_value.pop(); }
                    _ => {}
                }
            }
            KeyCode::Char(c) => {
                match edit_state.field {
                    EditField::Name => {
                        if edit_state.name.len() < 50 { edit_state.name.push(c); }
                    }
                    EditField::Interval => {
                        if c.is_ascii_digit() && edit_state.interval_input.len() < 8 {
                            edit_state.interval_input.push(c);
                            if let Ok(secs) = edit_state.interval_input.parse::<u64>() {
                                if secs > 0 { edit_state.interval_secs = secs; }
                            }
                        }
                    }
                    EditField::Extraction => {
                        if edit_state.extraction.len() < 100 { edit_state.extraction.push(c); }
                    }
                    EditField::NotifyCustom => {
                        if edit_state.notify_value.len() < 200 { edit_state.notify_value.push(c); }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_reminder_edit_input(app: &mut App, key: KeyCode) -> Result<()> {
    if let Some(ref mut edit_state) = app.reminder_edit_state {
        match key {
            KeyCode::Esc => app.cancel_reminder_edit(),
            KeyCode::Enter => app.save_reminder_edit()?,
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => edit_state.next_field(),
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => edit_state.prev_field(),
            KeyCode::Char(' ') => {
                match edit_state.field {
                    ReminderEditField::Recurring => {
                        edit_state.recurring = !edit_state.recurring;
                        if !edit_state.recurring && edit_state.field == ReminderEditField::Interval {
                            edit_state.field = ReminderEditField::Enabled;
                        }
                    }
                    ReminderEditField::Enabled => edit_state.enabled = !edit_state.enabled,
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                match edit_state.field {
                    ReminderEditField::Name => { edit_state.name.pop(); }
                    ReminderEditField::TriggerTime => { edit_state.trigger_time.pop(); }
                    ReminderEditField::Interval => { edit_state.interval_input.pop(); }
                    _ => {}
                }
            }
            KeyCode::Char(c) => {
                match edit_state.field {
                    ReminderEditField::Name => {
                        if edit_state.name.len() < 100 { edit_state.name.push(c); }
                    }
                    ReminderEditField::TriggerTime => {
                        if edit_state.trigger_time.len() < 5 { edit_state.trigger_time.push(c); }
                    }
                    ReminderEditField::Interval => {
                        if edit_state.interval_input.len() < 10 { edit_state.interval_input.push(c); }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_filter_list_input(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc => {
            app.mode = Mode::Edit;
        }
        KeyCode::Char('n') => {
            app.filter_edit_state = Some(FilterEditState::new());
            app.mode = Mode::FilterEdit;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(watch) = app.selected_watch() {
                if app.selected_filter < watch.filters.len().saturating_sub(1) {
                    app.selected_filter += 1;
                }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected_filter = app.selected_filter.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Char('e') => {
            if let Some(watch) = app.selected_watch() {
                if let Some(filter) = watch.filters.get(app.selected_filter) {
                    app.filter_edit_state = Some(FilterEditState::from_filter(app.selected_filter, filter));
                    app.mode = Mode::FilterEdit;
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(watch_ref) = app.selected_watch() {
                let watch_id = watch_ref.id;
                if let Some(watch) = app.watches.iter_mut().find(|w| w.id == watch_id) {
                    if app.selected_filter < watch.filters.len() {
                        watch.filters.remove(app.selected_filter);
                        let _ = app.db.update_watch(watch);
                        if app.selected_filter > 0 {
                            app.selected_filter -= 1;
                        }
                        app.status_message = Some("Filter deleted".to_string());
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_filter_edit_input(app: &mut App, key: KeyCode) -> Result<()> {
    // Get the watch_id before borrowing filter_edit_state mutably
    let watch_id = app.selected_watch().map(|w| w.id);

    if let Some(ref mut filter_state) = app.filter_edit_state {
        match key {
            KeyCode::Esc => {
                app.filter_edit_state = None;
                app.mode = Mode::FilterList;
            }
            KeyCode::Enter => {
                let filter = filter_state.to_filter();
                let filter_idx = filter_state.filter_idx;
                if let Some(wid) = watch_id {
                    if let Some(watch) = app.watches.iter_mut().find(|w| w.id == wid) {
                        if let Some(idx) = filter_idx {
                            watch.filters[idx] = filter;
                        } else {
                            watch.filters.push(filter);
                        }
                        let _ = app.db.update_watch(watch);
                        app.status_message = Some("Filter saved".to_string());
                    }
                }
                app.filter_edit_state = None;
                app.mode = Mode::FilterList;
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => filter_state.next_field(),
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => filter_state.prev_field(),
            KeyCode::Char(' ') => {
                match filter_state.field {
                    FilterEditField::Target => {
                        use crate::watch::FilterTarget;
                        filter_state.target = match filter_state.target {
                            FilterTarget::New => FilterTarget::Old,
                            FilterTarget::Old => FilterTarget::Diff,
                            FilterTarget::Diff => FilterTarget::New,
                        };
                    }
                    FilterEditField::Condition => {
                        filter_state.condition = match filter_state.condition {
                            FilterCondition::Contains => FilterCondition::NotContains,
                            FilterCondition::NotContains => FilterCondition::Matches,
                            FilterCondition::Matches => FilterCondition::SizeGt,
                            FilterCondition::SizeGt => FilterCondition::Contains,
                        };
                    }
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                if filter_state.field == FilterEditField::Value {
                    filter_state.value.pop();
                }
            }
            KeyCode::Char(c) => {
                if filter_state.field == FilterEditField::Value && filter_state.value.len() < 100 {
                    filter_state.value.push(c);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_memory_inspector_input(app: &mut App, key: KeyCode) -> Result<()> {
    if let Some(ref mut state) = app.memory_inspector_state {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.memory_inspector_state = None;
                app.mode = Mode::Normal;
            }
            KeyCode::Tab => state.next_section(),
            KeyCode::Char('j') | KeyCode::Down => {
                state.selected_item = state.selected_item.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.selected_item = state.selected_item.saturating_sub(1);
            }
            KeyCode::Char('d') => {
                let watch_id = state.watch_id;
                match state.section {
                    MemorySection::Counters => {
                        let keys: Vec<_> = state.memory.counters.keys().cloned().collect();
                        if let Some(key) = keys.get(state.selected_item) {
                            state.memory.counters.remove(key);
                            let _ = app.db.update_agent_memory(&watch_id, &state.memory);
                            app.status_message = Some(format!("Deleted counter: {}", key));
                        }
                    }
                    MemorySection::LastValues => {
                        let keys: Vec<_> = state.memory.last_values.keys().cloned().collect();
                        if let Some(key) = keys.get(state.selected_item) {
                            state.memory.last_values.remove(key);
                            let _ = app.db.update_agent_memory(&watch_id, &state.memory);
                            app.status_message = Some(format!("Deleted value: {}", key));
                        }
                    }
                    MemorySection::Notes => {
                        if state.selected_item < state.memory.notes.len() {
                            state.memory.notes.remove(state.selected_item);
                            let _ = app.db.update_agent_memory(&watch_id, &state.memory);
                            app.status_message = Some("Deleted note".to_string());
                        }
                    }
                }
            }
            KeyCode::Char('C') => {
                let watch_id = state.watch_id;
                state.memory = Default::default();
                let _ = app.db.update_agent_memory(&watch_id, &state.memory);
                app.status_message = Some("Memory cleared".to_string());
            }
            KeyCode::Char('r') => {
                let watch_id = state.watch_id;
                if let Ok(memory) = app.db.get_agent_memory(&watch_id) {
                    state.memory = memory;
                    app.status_message = Some("Memory refreshed".to_string());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_profile_inspector_input(app: &mut App, key: KeyCode) -> Result<()> {
    if let Some(ref mut state) = app.profile_inspector_state {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.profile_inspector_state = None;
                app.mode = Mode::Normal;
            }
            KeyCode::Tab => state.next_section(),
            KeyCode::Char('j') | KeyCode::Down => {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            }
            _ => {}
        }
    }
    Ok(())
}
