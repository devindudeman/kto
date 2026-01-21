//! TUI rendering - all UI drawing functions

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::watch::Engine;

use super::state::*;
use super::types::*;
use super::utils::{format_interval, centered_rect};

/// Main UI entry point - called from the main loop
pub fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Min(10),    // Main content
            Constraint::Length(3),  // Status bar
        ])
        .split(f.area());

    // Title with version
    let version = env!("CARGO_PKG_VERSION");
    let title = Paragraph::new(format!(" kto v{} - Web Change Watcher", version))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    // Main content split
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Left panel: Watches (top) + Reminders (bottom)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(main_chunks[0]);

    // Right panel: Details (top) + Changes (bottom)
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(main_chunks[1]);

    // Store layout areas for click detection
    app.layout_areas = LayoutAreas {
        watches: left_chunks[0],
        reminders: left_chunks[1],
        details: right_chunks[0],
        changes: right_chunks[1],
        logs_modal: Rect::default(),
        wizard_modal: Rect::default(),
        reminder_wizard_modal: Rect::default(),
    };

    // Watches list
    render_watches(f, app, left_chunks[0]);

    // Reminders list
    render_reminders(f, app, left_chunks[1]);

    // Details/Changes panel
    render_details(f, app, main_chunks[1]);

    // Status bar
    render_status_bar(f, app, chunks[2]);

    // Overlays
    let mode = app.mode.clone();
    match mode {
        Mode::Help => render_help(f),
        Mode::Confirm(action) => render_confirm(f, action, app),
        Mode::Edit => render_edit(f, app),
        Mode::EditReminder => render_reminder_edit(f, app),
        Mode::ViewChange => render_view_change(f, app),
        Mode::Wizard => {
            app.layout_areas.wizard_modal = centered_rect(70, 60, f.area());
            render_wizard(f, app);
        }
        Mode::ReminderWizard => {
            app.layout_areas.reminder_wizard_modal = centered_rect(60, 50, f.area());
            render_reminder_wizard(f, app);
        }
        Mode::Describe => render_describe(f, app),
        Mode::Logs => {
            app.layout_areas.logs_modal = centered_rect(85, 85, f.area());
            render_logs(f, app);
        }
        Mode::NotifySetup => render_notify_setup(f, app),
        Mode::FilterList => render_filter_list(f, app),
        Mode::FilterEdit => render_filter_edit(f, app),
        Mode::MemoryInspector => render_memory_inspector(f, app),
        Mode::ProfileInspector => render_profile_inspector(f, app),
        Mode::Normal | Mode::Search => {}
    }
}

fn render_watches(f: &mut Frame, app: &App, area: Rect) {
    let filtered = app.filtered_watches();

    if filtered.is_empty() && app.filter_text.is_empty() {
        let border_style = if app.focus == Pane::Watches {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let text = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  No watches yet", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from("  Get started:"),
            Line::from(vec![
                Span::styled("    n", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  Create new watch"),
            ]),
            Line::from(vec![
                Span::styled("    ?", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  Show all keybindings"),
            ]),
        ];

        let paragraph = Paragraph::new(text)
            .block(Block::default()
                .borders(Borders::ALL)
                .title(" Watches ")
                .border_style(border_style));

        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, watch)| {
            let has_error = app.watch_errors.contains_key(&watch.id);

            let (status, status_color) = if has_error {
                ("✗", Color::Red)
            } else if watch.enabled {
                ("●", Color::Green)
            } else {
                ("○", Color::Yellow)
            };

            let engine_badge = match watch.engine {
                Engine::Rss => " RSS",
                Engine::Playwright => " JS",
                Engine::Http => "",
                Engine::Shell { .. } => " SH",
            };
            let engine_color = match watch.engine {
                Engine::Rss => Color::Magenta,
                Engine::Playwright => Color::Blue,
                Engine::Http => Color::White,
                Engine::Shell { .. } => Color::Cyan,
            };

            let ai_badge = if watch.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false) {
                " AI"
            } else {
                ""
            };

            let is_selected = i == app.selected_watch;
            let is_focused = app.focus == Pane::Watches;

            let base_style = if is_selected && is_focused {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if has_error {
                Style::default().fg(Color::Red)
            } else if !watch.enabled {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };

            let mut spans = vec![
                Span::styled(format!(" {} ", status), base_style.fg(status_color)),
                Span::styled(&watch.name, base_style),
            ];

            if !engine_badge.is_empty() {
                spans.push(Span::styled(engine_badge, base_style.fg(engine_color)));
            }
            if !ai_badge.is_empty() {
                spans.push(Span::styled(ai_badge, base_style.fg(Color::Cyan)));
            }
            if watch.use_profile {
                spans.push(Span::styled(" PRO", base_style.fg(Color::Green)));
            }
            if has_error {
                spans.push(Span::styled(" ERR", base_style.fg(Color::Red)));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = if filtered.len() > (area.height as usize - 2) {
        format!(" Watches ({}) ↕ ", filtered.len())
    } else {
        format!(" Watches ({}) ", filtered.len())
    };

    let border_style = if app.focus == Pane::Watches {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style));

    f.render_widget(list, area);
}

fn render_reminders(f: &mut Frame, app: &App, area: Rect) {
    if app.reminders.is_empty() {
        let border_style = if app.focus == Pane::Reminders {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let text = vec![
            Line::from(""),
            Line::from("  No reminders"),
            Line::from(""),
            Line::from(vec![
                Span::styled("    n", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  Create new reminder"),
            ]),
        ];

        let paragraph = Paragraph::new(text)
            .block(Block::default()
                .borders(Borders::ALL)
                .title(" Reminders ")
                .border_style(border_style));

        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = app
        .reminders
        .iter()
        .enumerate()
        .map(|(i, reminder)| {
            let (status, status_color) = if reminder.enabled {
                ("●", Color::Green)
            } else {
                ("○", Color::Yellow)
            };

            let now = chrono::Utc::now();
            let time_info = if reminder.trigger_at > now {
                let duration = reminder.trigger_at.signed_duration_since(now);
                if duration.num_seconds() < 60 {
                    format!("in {}s", duration.num_seconds())
                } else if duration.num_minutes() < 60 {
                    format!("in {}m", duration.num_minutes())
                } else if duration.num_hours() < 24 {
                    format!("in {}h", duration.num_hours())
                } else {
                    format!("in {}d", duration.num_days())
                }
            } else {
                "due".to_string()
            };

            let recurring_badge = if reminder.interval_secs.is_some() { " ↻" } else { "" };

            let is_selected = i == app.selected_reminder;
            let is_focused = app.focus == Pane::Reminders;

            let base_style = if is_selected && is_focused {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if !reminder.enabled {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };

            let spans = vec![
                Span::styled(format!(" {} ", status), base_style.fg(status_color)),
                Span::styled(&reminder.name, base_style),
                Span::styled(recurring_badge, base_style.fg(Color::Cyan)),
                Span::styled(format!(" {}", time_info), base_style.fg(Color::DarkGray)),
            ];

            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = format!(" Reminders ({}) ", app.reminders.len());
    let border_style = if app.focus == Pane::Reminders {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style));

    f.render_widget(list, area);
}

fn render_details(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(area);

    if let Some(watch) = app.selected_watch() {
        let interval = format_interval(watch.interval_secs);
        let status = if watch.enabled { "Active" } else { "Paused" };
        let engine = match &watch.engine {
            Engine::Http => "HTTP".to_string(),
            Engine::Playwright => "Playwright (JS)".to_string(),
            Engine::Rss => "RSS/Atom".to_string(),
            Engine::Shell { command } => {
                let cmd = if command.len() > 30 { format!("{}...", &command[..27]) } else { command.clone() };
                format!("Shell: {}", cmd)
            }
        };
        let agent = if watch.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false) { "Enabled" } else { "Disabled" };

        let last_check = if let Ok(snapshot) = app.db.get_latest_snapshot(&watch.id) {
            if let Some(s) = snapshot {
                let ago = chrono::Utc::now().signed_duration_since(s.fetched_at);
                if ago.num_seconds() < 60 {
                    format!("{}s ago", ago.num_seconds())
                } else if ago.num_minutes() < 60 {
                    format!("{}m ago", ago.num_minutes())
                } else {
                    format!("{}h ago", ago.num_hours())
                }
            } else {
                "Never".to_string()
            }
        } else {
            "Unknown".to_string()
        };

        let max_url_len = area.width.saturating_sub(10) as usize;
        let url_display = if watch.url.len() > max_url_len {
            format!("{}...", &watch.url[..max_url_len.saturating_sub(3)])
        } else {
            watch.url.clone()
        };

        let details = vec![
            Line::from(vec![
                Span::styled(" URL: ", Style::default().fg(Color::DarkGray)),
                Span::raw(url_display),
            ]),
            Line::from(vec![
                Span::styled(" Status: ", Style::default().fg(Color::DarkGray)),
                Span::styled(status, if watch.enabled { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Yellow) }),
            ]),
            Line::from(vec![
                Span::styled(" Interval: ", Style::default().fg(Color::DarkGray)),
                Span::raw(interval),
            ]),
            Line::from(vec![
                Span::styled(" Engine: ", Style::default().fg(Color::DarkGray)),
                Span::raw(engine),
            ]),
            Line::from(vec![
                Span::styled(" AI Agent: ", Style::default().fg(Color::DarkGray)),
                Span::raw(agent),
            ]),
            Line::from(vec![
                Span::styled(" Last check: ", Style::default().fg(Color::DarkGray)),
                Span::raw(last_check),
            ]),
        ];

        let details_widget = Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title(" Details "));
        f.render_widget(details_widget, chunks[0]);
    } else {
        let empty = Paragraph::new(" No watch selected")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title(" Details "));
        f.render_widget(empty, chunks[0]);
    }

    // Recent changes
    let changes: Vec<ListItem> = app
        .changes
        .iter()
        .enumerate()
        .map(|(i, change)| {
            let time = change.detected_at.format("%m/%d %H:%M").to_string();
            let status = if change.notified { "✓" } else { "○" };
            let status_color = if change.notified { Color::Green } else { Color::DarkGray };

            let is_selected = i == app.selected_change;
            let is_focused = app.focus == Pane::Changes;

            let base_style = if is_selected && is_focused {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };

            let summary_preview = change.agent_response.as_ref()
                .and_then(|r| r.get("summary"))
                .and_then(|v| v.as_str())
                .map(|s| {
                    let max_len = 25;
                    if s.len() > max_len { format!(" {:.width$}...", s, width = max_len - 3) } else { format!(" {}", s) }
                })
                .unwrap_or_default();

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", status), base_style.fg(status_color)),
                Span::styled(time, base_style.fg(Color::DarkGray)),
                Span::styled(format!(" ({} chars)", change.diff.len()), base_style),
                Span::styled(summary_preview, base_style.fg(Color::Cyan)),
            ]))
        })
        .collect();

    let changes_title = if app.focus == Pane::Changes { " Recent Changes (Enter to view) " } else { " Recent Changes " };
    let border_style = if app.focus == Pane::Changes { Style::default().fg(Color::Cyan) } else { Style::default() };

    let changes_widget = List::new(changes)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(changes_title)
            .border_style(border_style));
    f.render_widget(changes_widget, chunks[1]);
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let active_watches = app.watches.iter().filter(|w| w.enabled).count();
    let ai_watches = app.watches.iter().filter(|w| w.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false)).count();
    let active_reminders = app.reminders.iter().filter(|r| r.enabled).count();

    let (status, help_hint) = match &app.mode {
        Mode::Search => (format!("/{}", app.filter_text), "<Enter> apply  <Esc> cancel".to_string()),
        Mode::Normal => {
            let status = if let Some(ref msg) = app.status_message {
                msg.clone()
            } else {
                match app.focus {
                    Pane::Watches => format!("{} watches ({} active, {} AI)", app.watches.len(), active_watches, ai_watches),
                    Pane::Changes => format!("{} changes", app.changes.len()),
                    Pane::Reminders => format!("{} reminders ({} active)", app.reminders.len(), active_reminders),
                }
            };
            let hint = match app.focus {
                Pane::Watches => "<n> new  <D> describe  <e> edit  </> filter  <?> help",
                Pane::Changes => "<Enter> view diff  <j/k> navigate  <?> help",
                Pane::Reminders => "<n> new  <e> edit  <p> pause  <d> delete  <?> help",
            };
            (status, hint.to_string())
        }
        Mode::ViewChange => {
            let watch_name = app.selected_watch().map(|w| w.name.as_str()).unwrap_or("?");
            (format!("{} > Change #{}", watch_name, app.selected_change + 1), "<j/k> scroll  <Esc> close".to_string())
        }
        Mode::Describe => {
            let watch_name = app.selected_watch().map(|w| w.name.as_str()).unwrap_or("?");
            (format!("Describing: {}", watch_name), "<j/k> scroll  <Esc> close".to_string())
        }
        Mode::Logs => (format!("Activity Log ({} entries)", app.all_changes.len()), "<j/k> navigate  <Esc> close".to_string()),
        Mode::NotifySetup => ("Notification Setup".to_string(), "<Tab> switch  <Enter> save  <Esc> cancel".to_string()),
        _ => (String::new(), String::new())
    };

    let status_line = Line::from(vec![
        Span::styled(format!(" {} ", status), Style::default().fg(Color::Cyan)),
        Span::raw(" ".repeat(area.width.saturating_sub(status.len() as u16 + help_hint.len() as u16 + 4) as usize)),
        Span::styled(&help_hint, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
    ]);

    let status_bar = Paragraph::new(status_line).block(Block::default().borders(Borders::ALL));
    f.render_widget(status_bar, area);
}

fn render_help(f: &mut Frame) {
    let area = centered_rect(55, 80, f.area());
    f.render_widget(Clear, area);

    let help_text = vec![
        Line::from(""),
        Line::from(Span::styled(" Navigation", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(vec![Span::styled(" Tab     ", Style::default().fg(Color::Yellow)), Span::raw("Cycle panes")]),
        Line::from(vec![Span::styled(" j/k     ", Style::default().fg(Color::Yellow)), Span::raw("Navigate up/down")]),
        Line::from(vec![Span::styled(" g/G     ", Style::default().fg(Color::Yellow)), Span::raw("Jump to first/last")]),
        Line::from(vec![Span::styled(" /       ", Style::default().fg(Color::Yellow)), Span::raw("Filter watches")]),
        Line::from(""),
        Line::from(Span::styled(" Watches", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(vec![Span::styled(" n       ", Style::default().fg(Color::Yellow)), Span::raw("New watch (wizard)")]),
        Line::from(vec![Span::styled(" D       ", Style::default().fg(Color::Yellow)), Span::raw("Describe (view config)")]),
        Line::from(vec![Span::styled(" e       ", Style::default().fg(Color::Yellow)), Span::raw("Edit watch")]),
        Line::from(vec![Span::styled(" p       ", Style::default().fg(Color::Yellow)), Span::raw("Pause/Resume")]),
        Line::from(vec![Span::styled(" t       ", Style::default().fg(Color::Yellow)), Span::raw("Test (read-only)")]),
        Line::from(vec![Span::styled(" c       ", Style::default().fg(Color::Yellow)), Span::raw("Force check")]),
        Line::from(vec![Span::styled(" d       ", Style::default().fg(Color::Yellow)), Span::raw("Delete")]),
        Line::from(vec![Span::styled(" M       ", Style::default().fg(Color::Yellow)), Span::raw("View agent memory")]),
        Line::from(""),
        Line::from(Span::styled(" Edit Mode", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(vec![Span::styled(" f       ", Style::default().fg(Color::Yellow)), Span::raw("Manage filters")]),
        Line::from(vec![Span::styled(" T       ", Style::default().fg(Color::Yellow)), Span::raw("Test notification")]),
        Line::from(""),
        Line::from(Span::styled(" Global", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(vec![Span::styled(" L       ", Style::default().fg(Color::Yellow)), Span::raw("Activity logs")]),
        Line::from(vec![Span::styled(" N       ", Style::default().fg(Color::Yellow)), Span::raw("Notification setup")]),
        Line::from(vec![Span::styled(" P       ", Style::default().fg(Color::Yellow)), Span::raw("View profile")]),
        Line::from(vec![Span::styled(" r       ", Style::default().fg(Color::Yellow)), Span::raw("Refresh")]),
        Line::from(vec![Span::styled(" q       ", Style::default().fg(Color::Yellow)), Span::raw("Quit")]),
        Line::from(""),
        Line::from(Span::styled(" Press any key to close ", Style::default().fg(Color::DarkGray))),
    ];

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .style(Style::default().bg(Color::Black));
    f.render_widget(help, area);
}

fn render_wizard(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 60, f.area());
    f.render_widget(Clear, area);

    if let Some(ref wizard) = app.wizard_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" New Watch Wizard ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));

        let step_num = match wizard.step {
            WizardStep::Url => 1,
            WizardStep::Engine => 2,
            WizardStep::Name => 3,
            WizardStep::Extraction => 4,
            WizardStep::Interval => 5,
            WizardStep::Agent => 6,
            WizardStep::Review => 7,
        };
        lines.push(Line::from(Span::styled(format!(" Step {} of 7 ", step_num), Style::default().fg(Color::DarkGray))));
        lines.push(Line::from(""));

        match wizard.step {
            WizardStep::Url => {
                lines.push(Line::from(Span::styled(" URL to watch:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {}_", wizard.url))));
            }
            WizardStep::Engine => {
                lines.push(Line::from(Span::styled(" Fetch engine (space to cycle):", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {:?}", wizard.engine))));
            }
            WizardStep::Name => {
                lines.push(Line::from(Span::styled(" Watch name:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {}_", wizard.name))));
            }
            WizardStep::Extraction => {
                lines.push(Line::from(Span::styled(" Extraction method:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {}_", wizard.extraction))));
            }
            WizardStep::Interval => {
                lines.push(Line::from(Span::styled(" Check interval:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {}_", wizard.interval_input))));
            }
            WizardStep::Agent => {
                lines.push(Line::from(Span::styled(" AI Agent (space to toggle):", Style::default().fg(Color::Yellow))));
                let status = if wizard.agent_enabled { "[x] Enabled" } else { "[ ] Disabled" };
                lines.push(Line::from(Span::raw(format!(" > {}", status))));
            }
            WizardStep::Review => {
                lines.push(Line::from(Span::styled(" Review:", Style::default().add_modifier(Modifier::BOLD))));
                lines.push(Line::from(Span::raw(format!("  URL: {}", wizard.url))));
                lines.push(Line::from(Span::raw(format!("  Name: {}", wizard.name))));
                lines.push(Line::from(Span::raw(format!("  Interval: {}", format_interval(wizard.interval_secs)))));
                lines.push(Line::from(Span::styled(" Press Enter to create ", Style::default().fg(Color::Green))));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Tab/Enter: Next  BackTab: Back  Esc: Cancel ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" New Watch "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_reminder_wizard(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 50, f.area());
    f.render_widget(Clear, area);

    if let Some(ref wizard) = app.reminder_wizard_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" New Reminder ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));

        match wizard.step {
            ReminderWizardStep::Name => {
                lines.push(Line::from(Span::styled(" Reminder message:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {}_", wizard.name))));
            }
            ReminderWizardStep::When => {
                lines.push(Line::from(Span::styled(" When (space to toggle in/at):", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {} {}_", wizard.when_type, wizard.when_value))));
            }
            ReminderWizardStep::Recurring => {
                lines.push(Line::from(Span::styled(" Recurring (space to toggle):", Style::default().fg(Color::Yellow))));
                let status = if wizard.recurring { "[x] Yes" } else { "[ ] No" };
                lines.push(Line::from(Span::raw(format!(" > {}", status))));
            }
            ReminderWizardStep::Review => {
                lines.push(Line::from(Span::styled(" Review:", Style::default().add_modifier(Modifier::BOLD))));
                lines.push(Line::from(Span::raw(format!("  Message: {}", wizard.name))));
                lines.push(Line::from(Span::raw(format!("  When: {} {}", wizard.when_type, wizard.when_value))));
                lines.push(Line::from(Span::styled(" Press Enter to create ", Style::default().fg(Color::Green))));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Tab: Next  Esc: Cancel ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" New Reminder "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_describe(f: &mut Frame, app: &App) {
    let area = centered_rect(80, 80, f.area());
    f.render_widget(Clear, area);

    if let Some(watch) = app.selected_watch() {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Watch: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&watch.name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::raw(format!("   URL: {}", watch.url))));
        lines.push(Line::from(Span::raw(format!("   Status: {}", if watch.enabled { "Active" } else { "Paused" }))));
        lines.push(Line::from(Span::raw(format!("   Interval: {}", format_interval(watch.interval_secs)))));
        lines.push(Line::from(Span::raw(format!("   Engine: {:?}", watch.engine))));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Press Esc to close ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Watch Details "))
            .style(Style::default().bg(Color::Black))
            .scroll((app.scroll_offset as u16, 0));
        f.render_widget(widget, area);
    }
}

fn render_logs(f: &mut Frame, app: &App) {
    let area = centered_rect(85, 85, f.area());
    f.render_widget(Clear, area);

    let mut lines = Vec::new();
    lines.push(Line::from(""));

    if app.all_changes.is_empty() {
        lines.push(Line::from(Span::styled("  No changes recorded yet", Style::default().fg(Color::DarkGray))));
    } else {
        for (i, (change, watch_name)) in app.all_changes.iter().enumerate() {
            let is_selected = i == app.selected_log;
            let base_style = if is_selected {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let time_str = change.detected_at.format("%Y-%m-%d %H:%M").to_string();
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", time_str), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<20}", watch_name), base_style.fg(Color::Cyan)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" j/k navigate  Enter view  Esc close ", Style::default().fg(Color::DarkGray))));

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(format!(" Activity Log ({}) ", app.all_changes.len())))
        .style(Style::default().bg(Color::Black));
    f.render_widget(widget, area);
}

fn render_notify_setup(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 50, f.area());
    f.render_widget(Clear, area);

    if let Some(state) = &app.notify_setup_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Configure Notifications", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));

        if state.step == 0 {
            lines.push(Line::from(" Select type (j/k to navigate, Enter to select):"));
            let types = ["ntfy", "Slack", "Discord", "Gotify", "Command"];
            for (i, name) in types.iter().enumerate() {
                let is_selected = i == 0; // Simplified
                let marker = if is_selected { ">" } else { " " };
                lines.push(Line::from(Span::raw(format!(" {} {}", marker, name))));
            }
        } else {
            lines.push(Line::from(Span::styled(" Enter value:", Style::default().fg(Color::Yellow))));
            lines.push(Line::from(Span::raw(format!(" > {}_", state.field1))));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Enter: Confirm  Esc: Cancel ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Notification Setup "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_filter_list(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 50, f.area());
    f.render_widget(Clear, area);

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" Filters", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(""));

    if let Some(watch) = app.selected_watch() {
        if watch.filters.is_empty() {
            lines.push(Line::from("  No filters defined"));
        } else {
            for (i, filter) in watch.filters.iter().enumerate() {
                let is_selected = i == app.selected_filter;
                let marker = if is_selected { ">" } else { " " };
                lines.push(Line::from(Span::raw(format!(" {} {:?}", marker, filter))));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" n: new  e: edit  d: delete  Esc: back ", Style::default().fg(Color::DarkGray))));

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Filter List "))
        .style(Style::default().bg(Color::Black));
    f.render_widget(widget, area);
}

fn render_filter_edit(f: &mut Frame, app: &App) {
    let area = centered_rect(55, 40, f.area());
    f.render_widget(Clear, area);

    if let Some(ref state) = app.filter_edit_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Edit Filter", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::raw(format!(" Target: {:?} (space to cycle)", state.target))));
        lines.push(Line::from(Span::raw(format!(" Condition: {:?} (space to cycle)", state.condition))));
        lines.push(Line::from(Span::raw(format!(" Value: {}_", state.value))));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Enter: Save  Esc: Cancel ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Filter Edit "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_memory_inspector(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 70, f.area());
    f.render_widget(Clear, area);

    if let Some(ref state) = app.memory_inspector_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Agent Memory: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.watch_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(""));

        let section_label = match state.section {
            MemorySection::Counters => "Counters",
            MemorySection::LastValues => "Last Values",
            MemorySection::Notes => "Notes",
        };
        lines.push(Line::from(Span::styled(format!(" {} (Tab to switch)", section_label), Style::default().fg(Color::Yellow))));
        lines.push(Line::from(""));

        match state.section {
            MemorySection::Counters => {
                for (i, (key, value)) in state.memory.counters.iter().enumerate() {
                    let marker = if i == state.selected_item { ">" } else { " " };
                    lines.push(Line::from(Span::raw(format!(" {} {}: {}", marker, key, value))));
                }
            }
            MemorySection::LastValues => {
                for (i, (key, value)) in state.memory.last_values.iter().enumerate() {
                    let marker = if i == state.selected_item { ">" } else { " " };
                    lines.push(Line::from(Span::raw(format!(" {} {}: {}", marker, key, value))));
                }
            }
            MemorySection::Notes => {
                for (i, note) in state.memory.notes.iter().enumerate() {
                    let marker = if i == state.selected_item { ">" } else { " " };
                    let preview = if note.len() > 50 { format!("{}...", &note[..47]) } else { note.clone() };
                    lines.push(Line::from(Span::raw(format!(" {} {}", marker, preview))));
                }
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" d: delete  C: clear all  Esc: close ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Memory Inspector "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_profile_inspector(f: &mut Frame, app: &App) {
    let area = centered_rect(75, 75, f.area());
    f.render_widget(Clear, area);

    if let Some(ref state) = app.profile_inspector_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Interest Profile", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));

        let section_label = match state.section {
            ProfileSection::Description => "Description",
            ProfileSection::Interests => "Interests",
            ProfileSection::GlobalMemory => "Global Memory",
        };
        lines.push(Line::from(Span::styled(format!(" {} (Tab to switch)", section_label), Style::default().fg(Color::Yellow))));
        lines.push(Line::from(""));

        match state.section {
            ProfileSection::Description => {
                let desc = &state.profile.profile.description;
                if !desc.is_empty() {
                    for line in desc.lines().take(10) {
                        lines.push(Line::from(Span::raw(format!("  {}", line))));
                    }
                } else {
                    lines.push(Line::from(Span::styled("  (no description)", Style::default().fg(Color::DarkGray))));
                }
            }
            ProfileSection::Interests => {
                for interest in &state.profile.interests {
                    lines.push(Line::from(Span::raw(format!("  {} (weight: {})", interest.name, interest.weight))));
                }
            }
            ProfileSection::GlobalMemory => {
                for observation in &state.global_memory.observations {
                    lines.push(Line::from(Span::raw(format!("  {}", observation.text))));
                }
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Esc: close ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Profile Inspector "))
            .style(Style::default().bg(Color::Black))
            .scroll((state.scroll_offset as u16, 0));
        f.render_widget(widget, area);
    }
}

fn render_view_change(f: &mut Frame, app: &App) {
    let area = centered_rect(85, 85, f.area());
    f.render_widget(Clear, area);

    if let Some(change) = app.changes.get(app.selected_change) {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Change detected: ", Style::default().fg(Color::DarkGray)),
            Span::styled(change.detected_at.format("%Y-%m-%d %H:%M:%S").to_string(), Style::default().fg(Color::Cyan)),
        ]));
        lines.push(Line::from(""));

        if let Some(ref response) = change.agent_response {
            if let Some(title) = response.get("title").and_then(|v| v.as_str()) {
                lines.push(Line::from(Span::styled(format!(" {}", title), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
                lines.push(Line::from(""));
            }
            if let Some(summary) = response.get("summary").and_then(|v| v.as_str()) {
                lines.push(Line::from(Span::raw(format!(" {}", summary))));
                lines.push(Line::from(""));
            }
        }

        lines.push(Line::from(Span::styled(" Diff:", Style::default().fg(Color::Yellow))));
        for line in change.diff.lines().take(30) {
            let style = if line.starts_with("[+") || line.starts_with("+") {
                Style::default().fg(Color::Green)
            } else if line.starts_with("[-") || line.starts_with("-") {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(format!(" {}", line), style)));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" j/k: scroll  u: toggle format  Esc: close ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Change Details "))
            .style(Style::default().bg(Color::Black))
            .scroll((app.scroll_offset as u16, 0));
        f.render_widget(widget, area);
    }
}

fn render_edit(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 55, f.area());
    f.render_widget(Clear, area);

    if let Some(ref edit_state) = app.edit_state {
        let field_style = |field: EditField| {
            if edit_state.field == field {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }
        };

        let enabled_str = if edit_state.enabled { "[x] Active" } else { "[ ] Paused" };
        let agent_str = if edit_state.agent_enabled { "[x] Enabled" } else { "[ ] Disabled" };
        let use_profile_str = if edit_state.use_profile { "[x] Enabled" } else { "[ ] Disabled" };

        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(" Name:         ", Style::default().fg(Color::DarkGray)),
                Span::styled(&edit_state.name, field_style(EditField::Name)),
            ]),
            Line::from(vec![
                Span::styled(" Interval:     ", Style::default().fg(Color::DarkGray)),
                Span::styled(format_interval(edit_state.interval_secs), field_style(EditField::Interval)),
                Span::styled(" (-/+ to change)", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled(" Engine:       ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:?} (space)", edit_state.engine), field_style(EditField::Engine)),
            ]),
            Line::from(vec![
                Span::styled(" Extraction:   ", Style::default().fg(Color::DarkGray)),
                Span::styled(&edit_state.extraction, field_style(EditField::Extraction)),
            ]),
            Line::from(vec![
                Span::styled(" Status:       ", Style::default().fg(Color::DarkGray)),
                Span::styled(enabled_str, field_style(EditField::Enabled)),
            ]),
            Line::from(vec![
                Span::styled(" AI Agent:     ", Style::default().fg(Color::DarkGray)),
                Span::styled(agent_str, field_style(EditField::Agent)),
            ]),
            Line::from(vec![
                Span::styled(" Instructions: ", Style::default().fg(Color::DarkGray)),
                Span::styled("(press 'e' to edit)", field_style(EditField::AgentInstructions)),
            ]),
            Line::from(vec![
                Span::styled(" Use Profile:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(use_profile_str, field_style(EditField::UseProfile)),
            ]),
            Line::from(vec![
                Span::styled(" Filters:      ", Style::default().fg(Color::DarkGray)),
                Span::styled("(press 'f' to manage)", field_style(EditField::Filters)),
            ]),
            Line::from(vec![
                Span::styled(" Notify:       ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    if edit_state.notify_use_global { "[x] Global" } else { "[ ] Custom" },
                    field_style(EditField::Notify)
                ),
                Span::styled(" (T to test)", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        if !edit_state.notify_use_global {
            lines.push(Line::from(vec![
                Span::styled("   Type:       ", Style::default().fg(Color::DarkGray)),
                Span::styled(edit_state.notify_type.label(), field_style(EditField::NotifyCustom)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("   Value:      ", Style::default().fg(Color::DarkGray)),
                Span::styled(&edit_state.notify_value, field_style(EditField::NotifyCustom)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Tab/j/k ", Style::default().fg(Color::DarkGray)),
            Span::raw("Navigate  "),
            Span::styled(" Enter ", Style::default().fg(Color::Green)),
            Span::raw("Save  "),
            Span::styled(" Esc ", Style::default().fg(Color::Red)),
            Span::raw("Cancel"),
        ]));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Edit Watch "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_reminder_edit(f: &mut Frame, app: &App) {
    let area = centered_rect(55, 45, f.area());
    f.render_widget(Clear, area);

    if let Some(ref edit_state) = app.reminder_edit_state {
        let field_style = |field: ReminderEditField| {
            if edit_state.field == field {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }
        };

        let recurring_str = if edit_state.recurring { "[x] Yes" } else { "[ ] No" };
        let enabled_str = if edit_state.enabled { "[x] Active" } else { "[ ] Paused" };

        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(" Name:      ", Style::default().fg(Color::DarkGray)),
                Span::styled(&edit_state.name, field_style(ReminderEditField::Name)),
            ]),
            Line::from(vec![
                Span::styled(" Time:      ", Style::default().fg(Color::DarkGray)),
                Span::styled(&edit_state.trigger_time, field_style(ReminderEditField::TriggerTime)),
                Span::styled(" (HH:MM)", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled(" Recurring: ", Style::default().fg(Color::DarkGray)),
                Span::styled(recurring_str, field_style(ReminderEditField::Recurring)),
            ]),
        ];

        if edit_state.recurring {
            lines.push(Line::from(vec![
                Span::styled(" Interval:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(&edit_state.interval_input, field_style(ReminderEditField::Interval)),
            ]));
        }

        lines.push(Line::from(vec![
            Span::styled(" Status:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(enabled_str, field_style(ReminderEditField::Enabled)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Tab ", Style::default().fg(Color::DarkGray)),
            Span::raw("Navigate  "),
            Span::styled(" Enter ", Style::default().fg(Color::Green)),
            Span::raw("Save  "),
            Span::styled(" Esc ", Style::default().fg(Color::Red)),
            Span::raw("Cancel"),
        ]));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Edit Reminder "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_confirm(f: &mut Frame, action: ConfirmAction, app: &App) {
    let area = centered_rect(40, 20, f.area());
    f.render_widget(Clear, area);

    let message = match action {
        ConfirmAction::Delete => {
            let name = app.selected_watch().map(|w| w.name.as_str()).unwrap_or("?");
            format!("Delete '{}'?", name)
        }
        ConfirmAction::DeleteReminder => {
            let name = app.selected_reminder().map(|r| r.name.as_str()).unwrap_or("?");
            format!("Delete reminder '{}'?", name)
        }
        ConfirmAction::Test => {
            let name = app.selected_watch().map(|w| w.name.as_str()).unwrap_or("?");
            format!("Test '{}'?", name)
        }
        ConfirmAction::ForceCheck => {
            let name = app.selected_watch().map(|w| w.name.as_str()).unwrap_or("?");
            format!("Force check '{}'?", name)
        }
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::raw(format!(" {} ", message))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y ", Style::default().fg(Color::Green)),
            Span::raw("Yes  "),
            Span::styled(" n ", Style::default().fg(Color::Red)),
            Span::raw("No"),
        ]),
    ];

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Confirm "))
        .style(Style::default().bg(Color::Black));
    f.render_widget(widget, area);
}
