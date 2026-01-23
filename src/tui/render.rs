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
        Mode::Health => render_health_dashboard(f, app),
        Mode::Normal | Mode::Search => {}
    }
}

fn render_watches(f: &mut Frame, app: &mut App, area: Rect) {
    // Check if empty first
    let is_empty = app.filtered_watches().is_empty();
    let filter_empty = app.filter_text.is_empty();

    if is_empty && filter_empty {
        render_empty_watches_state(f, app, area);
        return;
    }

    // Calculate visible height (area minus borders)
    let visible_height = area.height.saturating_sub(2) as usize;
    let total_items = app.filtered_watches().len();

    // Update scroll offset to keep selection visible
    app.update_watches_scroll(visible_height);
    let scroll_offset = app.watches_scroll;

    // Now get filtered watches again for rendering (after scroll update)
    let filtered = app.filtered_watches();

    // Build items with scroll offset applied
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, watch)| {
            let has_error = app.watch_errors.contains_key(&watch.id);

            // Check if watch is stale (no check in 2x interval)
            let is_stale = is_watch_stale(&app.db, watch);

            // Determine status indicator
            // ● = active, checked recently
            // ◐ = active but stale (no check in 2x interval)
            // ○ = paused
            // ✗ = error
            let (status, status_color) = if has_error {
                ("✗", Color::Red)
            } else if !watch.enabled {
                ("○", Color::Yellow)
            } else if is_stale {
                ("◐", Color::Yellow)
            } else {
                ("●", Color::Green)
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

    // Build title with scroll indicator
    let title = if total_items > visible_height {
        let can_scroll_up = scroll_offset > 0;
        let can_scroll_down = scroll_offset + visible_height < total_items;
        let scroll_indicator = match (can_scroll_up, can_scroll_down) {
            (true, true) => "↕",
            (true, false) => "↑",
            (false, true) => "↓",
            (false, false) => "",
        };
        format!(" Watches ({}) {} ", total_items, scroll_indicator)
    } else {
        format!(" Watches ({}) ", total_items)
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

fn render_reminders(f: &mut Frame, app: &mut App, area: Rect) {
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

    // Calculate visible height (area minus borders)
    let visible_height = area.height.saturating_sub(2) as usize;
    let total_items = app.reminders.len();

    // Update scroll offset to keep selection visible
    app.update_reminders_scroll(visible_height);
    let scroll_offset = app.reminders_scroll;

    let items: Vec<ListItem> = app
        .reminders
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
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

    // Build title with scroll indicator
    let title = if total_items > visible_height {
        let can_scroll_up = scroll_offset > 0;
        let can_scroll_down = scroll_offset + visible_height < total_items;
        let scroll_indicator = match (can_scroll_up, can_scroll_down) {
            (true, true) => "↕",
            (true, false) => "↑",
            (false, true) => "↓",
            (false, false) => "",
        };
        format!(" Reminders ({}) {} ", total_items, scroll_indicator)
    } else {
        format!(" Reminders ({}) ", total_items)
    };

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

fn render_details(f: &mut Frame, app: &mut App, area: Rect) {
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

    // Calculate visible height for changes list (area minus borders)
    let changes_area = chunks[1];
    let visible_height = changes_area.height.saturating_sub(2) as usize;
    let total_changes = app.changes.len();

    // Update scroll offset to keep selection visible
    app.update_changes_scroll(visible_height);
    let scroll_offset = app.changes_scroll;

    // Recent changes with scrolling
    let changes: Vec<ListItem> = app
        .changes
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
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

    // Build title with scroll indicator
    let changes_title = if total_changes > visible_height {
        let can_scroll_up = scroll_offset > 0;
        let can_scroll_down = scroll_offset + visible_height < total_changes;
        let scroll_indicator = match (can_scroll_up, can_scroll_down) {
            (true, true) => "↕",
            (true, false) => "↑",
            (false, true) => "↓",
            (false, false) => "",
        };
        if app.focus == Pane::Changes {
            format!(" Recent Changes ({}) {} (Enter to view) ", total_changes, scroll_indicator)
        } else {
            format!(" Recent Changes ({}) {} ", total_changes, scroll_indicator)
        }
    } else if app.focus == Pane::Changes {
        " Recent Changes (Enter to view) ".to_string()
    } else {
        " Recent Changes ".to_string()
    };

    let border_style = if app.focus == Pane::Changes { Style::default().fg(Color::Cyan) } else { Style::default() };

    let changes_widget = List::new(changes)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(changes_title)
            .border_style(border_style));
    f.render_widget(changes_widget, changes_area);
}

fn render_status_bar(f: &mut Frame, app: &mut App, area: Rect) {
    let active_watches = app.watches.iter().filter(|w| w.enabled).count();
    let ai_watches = app.watches.iter().filter(|w| w.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false)).count();
    let active_reminders = app.reminders.iter().filter(|r| r.enabled).count();

    // Check if daemon is running
    let daemon_status = get_daemon_status(&app.db);

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

            let base_hint = match app.focus {
                Pane::Watches => "<n> new  <H> health  <?> help",
                Pane::Changes => "<Enter> view  <H> health  <?> help",
                Pane::Reminders => "<n> new  <H> health  <?> help",
            };
            (status, base_hint.to_string())
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
        Mode::Health => ("System Health".to_string(), "<Esc> close  <r> refresh".to_string()),
        _ => (String::new(), String::new())
    };

    // Build the status line with daemon status indicator
    let daemon_indicator = match &daemon_status {
        DaemonStatus::Running(info) => {
            Span::styled(
                format!(" {} {} ", "●", info),
                Style::default().fg(Color::Green)
            )
        }
        DaemonStatus::Stale(info) => {
            Span::styled(
                format!(" {} {} ", "◐", info),
                Style::default().fg(Color::Yellow)
            )
        }
        DaemonStatus::Stopped => {
            Span::styled(
                " ○ daemon stopped ",
                Style::default().fg(Color::Red)
            )
        }
    };

    let status_line = Line::from(vec![
        Span::styled(format!(" {} ", status), Style::default().fg(Color::Cyan)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        daemon_indicator,
        Span::raw(" ".repeat(
            area.width
                .saturating_sub(status.len() as u16 + help_hint.len() as u16 + 25) as usize
        )),
        Span::styled(&help_hint, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
    ]);

    let status_bar = Paragraph::new(status_line).block(Block::default().borders(Borders::ALL));
    f.render_widget(status_bar, area);
}

/// Daemon status for display
enum DaemonStatus {
    Running(String),  // e.g., "last check 2m ago"
    Stale(String),    // e.g., "last check 2h ago"
    Stopped,
}

/// Check if daemon is running and get last check info
fn get_daemon_status(db: &crate::db::Database) -> DaemonStatus {
    // Try to find when the last check occurred by looking at snapshots
    if let Ok(Some(last_snapshot)) = db.get_most_recent_snapshot() {
        let ago = chrono::Utc::now().signed_duration_since(last_snapshot.fetched_at);
        let ago_str = if ago.num_seconds() < 60 {
            format!("{}s ago", ago.num_seconds())
        } else if ago.num_minutes() < 60 {
            format!("{}m ago", ago.num_minutes())
        } else if ago.num_hours() < 24 {
            format!("{}h ago", ago.num_hours())
        } else {
            format!("{}d ago", ago.num_days())
        };

        // Consider "stale" if no check in last 30 minutes
        if ago.num_minutes() > 30 {
            DaemonStatus::Stale(format!("last: {}", ago_str))
        } else {
            DaemonStatus::Running(format!("last: {}", ago_str))
        }
    } else {
        DaemonStatus::Stopped
    }
}


fn render_help(f: &mut Frame) {
    let area = centered_rect(58, 85, f.area());
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
        Line::from(vec![Span::styled(" n       ", Style::default().fg(Color::Yellow)), Span::raw("New watch (with templates)")]),
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
        Line::from(vec![Span::styled(" H       ", Style::default().fg(Color::Yellow)), Span::raw("Health dashboard")]),
        Line::from(vec![Span::styled(" L       ", Style::default().fg(Color::Yellow)), Span::raw("Activity logs")]),
        Line::from(vec![Span::styled(" N       ", Style::default().fg(Color::Yellow)), Span::raw("Notification setup")]),
        Line::from(vec![Span::styled(" P       ", Style::default().fg(Color::Yellow)), Span::raw("View profile")]),
        Line::from(vec![Span::styled(" r       ", Style::default().fg(Color::Yellow)), Span::raw("Refresh")]),
        Line::from(vec![Span::styled(" q       ", Style::default().fg(Color::Yellow)), Span::raw("Quit")]),
        Line::from(""),
        Line::from(Span::styled(" Status Indicators", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled(" ●  ", Style::default().fg(Color::Green)),
            Span::raw("healthy   "),
            Span::styled("◐  ", Style::default().fg(Color::Yellow)),
            Span::raw("stale   "),
            Span::styled("○  ", Style::default().fg(Color::Yellow)),
            Span::raw("paused   "),
            Span::styled("✗  ", Style::default().fg(Color::Red)),
            Span::raw("error"),
        ]),
        Line::from(""),
        Line::from(Span::styled(" Press any key to close ", Style::default().fg(Color::DarkGray))),
    ];

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .style(Style::default().bg(Color::Black));
    f.render_widget(help, area);
}

fn render_wizard(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 65, f.area());
    f.render_widget(Clear, area);

    if let Some(ref wizard) = app.wizard_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" New Watch Wizard ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));

        let step_num = match wizard.step {
            WizardStep::Template => 1,
            WizardStep::Url => 2,
            WizardStep::Engine => 3,
            WizardStep::Name => 4,
            WizardStep::Extraction => 5,
            WizardStep::Interval => 6,
            WizardStep::Agent => 7,
            WizardStep::Review => 8,
        };
        lines.push(Line::from(Span::styled(format!(" Step {} of 8 ", step_num), Style::default().fg(Color::DarkGray))));
        lines.push(Line::from(""));

        match wizard.step {
            WizardStep::Template => {
                lines.push(Line::from(Span::styled(" Choose a template:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(""));

                for template in WatchTemplate::all() {
                    let is_selected = template == wizard.template;
                    let marker = if is_selected { ">" } else { " " };
                    let marker_style = if is_selected {
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    let name_style = if is_selected {
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    lines.push(Line::from(vec![
                        Span::styled(format!(" {} ", marker), marker_style),
                        Span::styled(template.name(), name_style),
                    ]));
                    lines.push(Line::from(vec![
                        Span::raw("     "),
                        Span::styled(template.description(), Style::default().fg(Color::DarkGray)),
                    ]));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" j/k: navigate  Space: select  Tab: continue ", Style::default().fg(Color::DarkGray))));
            }
            WizardStep::Url => {
                lines.push(Line::from(Span::styled(" URL to watch:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(Span::raw(format!(" > {}_", wizard.url))));
            }
            WizardStep::Engine => {
                // Show transform suggestion if detected
                if let Some(ref suggestion) = wizard.transform_suggestion {
                    lines.push(Line::from(Span::styled(" Feed URL detected:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))));
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled("   ", Style::default()),
                        Span::styled(suggestion.description, Style::default().fg(Color::Cyan)),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled("   URL: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&suggestion.transformed_url, Style::default().fg(Color::Green)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("   Engine: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(format!("{:?}", suggestion.engine), Style::default()),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(" Tab: accept  x: use original URL instead", Style::default().fg(Color::DarkGray))));
                } else {
                    lines.push(Line::from(Span::styled(" Fetch engine (space to cycle):", Style::default().fg(Color::Yellow))));
                    lines.push(Line::from(Span::raw(format!(" > {:?}", wizard.engine))));
                }
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
                lines.push(Line::from(""));

                // Show template if not custom
                if wizard.template != WatchTemplate::Custom {
                    lines.push(Line::from(vec![
                        Span::styled("  Template: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(wizard.template.name(), Style::default().fg(Color::Cyan)),
                    ]));
                }

                // URL (truncated if needed)
                let max_url_len = (area.width as usize).saturating_sub(12);
                let url_display = if wizard.url.len() > max_url_len {
                    format!("{}...", &wizard.url[..max_url_len.saturating_sub(3)])
                } else {
                    wizard.url.clone()
                };
                lines.push(Line::from(vec![
                    Span::styled("  URL: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(url_display),
                ]));

                lines.push(Line::from(vec![
                    Span::styled("  Name: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&wizard.name),
                ]));

                // Engine
                let engine_str = match &wizard.engine {
                    Engine::Http => "HTTP",
                    Engine::Playwright => "Playwright (JS rendering)",
                    Engine::Rss => "RSS/Atom feed",
                    Engine::Shell { command } => {
                        if command.len() > 30 { "Shell command" } else { "Shell" }
                    }
                };
                lines.push(Line::from(vec![
                    Span::styled("  Engine: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(engine_str),
                ]));

                // Extraction
                lines.push(Line::from(vec![
                    Span::styled("  Extraction: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&wizard.extraction),
                ]));

                lines.push(Line::from(vec![
                    Span::styled("  Interval: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(format_interval(wizard.interval_secs)),
                ]));

                // AI Agent status
                let agent_display = if wizard.agent_enabled {
                    if wizard.agent_instructions.is_empty() {
                        "Enabled (no instructions)".to_string()
                    } else {
                        let preview = if wizard.agent_instructions.len() > 30 {
                            format!("{}...", &wizard.agent_instructions[..30].replace('\n', " "))
                        } else {
                            wizard.agent_instructions.replace('\n', " ")
                        };
                        format!("Enabled: \"{}\"", preview)
                    }
                } else {
                    "Disabled".to_string()
                };
                lines.push(Line::from(vec![
                    Span::styled("  AI Agent: ", Style::default().fg(Color::DarkGray)),
                    if wizard.agent_enabled {
                        Span::styled(agent_display, Style::default().fg(Color::Cyan))
                    } else {
                        Span::raw(agent_display)
                    },
                ]));

                // Show test result if available
                if let Some(ref result) = wizard.test_result {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(" Test Result:", Style::default().fg(Color::Yellow))));
                    // Truncate result to fit
                    let max_len = (area.width as usize).saturating_sub(6);
                    let result_preview = if result.len() > max_len {
                        format!("{}...", &result[..max_len.saturating_sub(3)])
                    } else {
                        result.clone()
                    };
                    lines.push(Line::from(Span::styled(
                        format!("  {}", result_preview),
                        Style::default().fg(Color::DarkGray)
                    )));
                }

                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled(" Enter ", Style::default().fg(Color::Green)),
                    Span::raw("Create  "),
                    Span::styled(" t ", Style::default().fg(Color::Yellow)),
                    Span::raw("Test extraction first"),
                ]));
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
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("  Message: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&wizard.name),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  When: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(format!("{} {}", wizard.when_type, wizard.when_value)),
                ]));

                // Calculate and show actual trigger time
                let trigger_time_display = calculate_reminder_time(&wizard.when_type, &wizard.when_value);
                lines.push(Line::from(vec![
                    Span::styled("  Triggers: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(trigger_time_display, Style::default().fg(Color::Cyan)),
                ]));

                // Recurring status
                if wizard.recurring {
                    lines.push(Line::from(vec![
                        Span::styled("  Recurring: ", Style::default().fg(Color::DarkGray)),
                        Span::raw(format!("Every {}", wizard.interval)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("  Recurring: ", Style::default().fg(Color::DarkGray)),
                        Span::raw("No (one-time)"),
                    ]));
                }

                lines.push(Line::from(""));
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

        // Basic info
        lines.push(Line::from(vec![
            Span::styled("   URL: ", Style::default().fg(Color::DarkGray)),
            Span::raw(&watch.url),
        ]));
        lines.push(Line::from(vec![
            Span::styled("   Status: ", Style::default().fg(Color::DarkGray)),
            if watch.enabled {
                Span::styled("Active", Style::default().fg(Color::Green))
            } else {
                Span::styled("Paused", Style::default().fg(Color::Yellow))
            },
        ]));
        lines.push(Line::from(vec![
            Span::styled("   Interval: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format_interval(watch.interval_secs)),
        ]));

        // Engine (formatted nicely)
        let engine_str = match &watch.engine {
            Engine::Http => "HTTP".to_string(),
            Engine::Playwright => "Playwright (JS rendering)".to_string(),
            Engine::Rss => "RSS/Atom feed".to_string(),
            Engine::Shell { command } => format!("Shell: {}", command),
        };
        lines.push(Line::from(vec![
            Span::styled("   Engine: ", Style::default().fg(Color::DarkGray)),
            Span::raw(engine_str),
        ]));

        // Extraction (formatted nicely)
        let extraction_str = match &watch.extraction {
            crate::watch::Extraction::Auto => "Auto (readability)".to_string(),
            crate::watch::Extraction::Selector { selector } => format!("CSS: {}", selector),
            crate::watch::Extraction::Full => "Full page".to_string(),
            crate::watch::Extraction::Meta { tags } => format!("Meta tags: {}", tags.join(", ")),
            crate::watch::Extraction::Rss => "RSS items".to_string(),
            crate::watch::Extraction::JsonLd { types } => {
                match types {
                    Some(t) if !t.is_empty() => format!("JSON-LD: {}", t.join(", ")),
                    _ => "JSON-LD (all types)".to_string(),
                }
            }
        };
        lines.push(Line::from(vec![
            Span::styled("   Extraction: ", Style::default().fg(Color::DarkGray)),
            Span::raw(extraction_str),
        ]));

        // Tags
        if !watch.tags.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("   Tags: ", Style::default().fg(Color::DarkGray)),
                Span::raw(watch.tags.join(", ")),
            ]));
        }

        // Filters
        if !watch.filters.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("   Filters: ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{} filter{}", watch.filters.len(), if watch.filters.len() == 1 { "" } else { "s" })),
            ]));
            for filter in &watch.filters {
                lines.push(Line::from(vec![
                    Span::raw("     - "),
                    Span::styled(format_filter_readable(filter), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }

        lines.push(Line::from(""));

        // ID and created date (for debugging)
        lines.push(Line::from(Span::styled(" Metadata:", Style::default().fg(Color::Yellow))));
        lines.push(Line::from(vec![
            Span::styled("   ID: ", Style::default().fg(Color::DarkGray)),
            Span::styled(watch.id.to_string(), Style::default().fg(Color::DarkGray)),
        ]));
        let local_created: chrono::DateTime<chrono::Local> = watch.created_at.into();
        lines.push(Line::from(vec![
            Span::styled("   Created: ", Style::default().fg(Color::DarkGray)),
            Span::styled(local_created.format("%Y-%m-%d %H:%M").to_string(), Style::default().fg(Color::DarkGray)),
        ]));

        lines.push(Line::from(""));

        // AI Agent section
        let agent_enabled = watch.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false);
        lines.push(Line::from(vec![
            Span::styled(" AI Agent: ", Style::default().fg(Color::DarkGray)),
            if agent_enabled {
                Span::styled("Enabled", Style::default().fg(Color::Green))
            } else {
                Span::styled("Disabled", Style::default().fg(Color::DarkGray))
            },
        ]));

        // Show AI instructions if present
        if let Some(ref config) = watch.agent_config {
            if let Some(ref instructions) = config.instructions {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" AI Instructions:", Style::default().fg(Color::Yellow))));
                lines.push(Line::from(""));
                // Word wrap the instructions manually for display
                let wrap_width = area.width.saturating_sub(8) as usize; // Account for borders and padding
                for wrapped_line in wrap_text(instructions, wrap_width) {
                    lines.push(Line::from(Span::raw(format!("   {}", wrapped_line))));
                }
            }
        }

        // Profile usage
        if watch.use_profile {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("   Using interest profile", Style::default().fg(Color::Magenta))));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" j/k scroll  Esc close ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Watch Details "))
            .style(Style::default().bg(Color::Black))
            .scroll((app.scroll_offset as u16, 0));
        f.render_widget(widget, area);
    }
}

/// Simple word wrapping for text display
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current_line = String::new();
        for word in paragraph.split_whitespace() {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(current_line);
                current_line = word.to_string();
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }
    if lines.is_empty() {
        lines.push(text.to_string());
    }
    lines
}

fn render_logs(f: &mut Frame, app: &mut App) {
    let area = centered_rect(85, 85, f.area());
    f.render_widget(Clear, area);

    let mut lines = Vec::new();
    lines.push(Line::from(""));

    if app.all_changes.is_empty() {
        lines.push(Line::from(Span::styled("  No changes recorded yet", Style::default().fg(Color::DarkGray))));
    } else {
        // Calculate visible height (area minus borders and help line)
        let visible_height = area.height.saturating_sub(5) as usize; // -2 borders, -1 empty, -2 help
        let total_items = app.all_changes.len();

        // Update scroll offset to keep selection visible
        app.update_logs_scroll(visible_height);
        let scroll_offset = app.logs_scroll;

        for (i, (change, watch_name)) in app.all_changes.iter().enumerate().skip(scroll_offset).take(visible_height) {
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

        // Show scroll position if needed
        if total_items > visible_height {
            let can_scroll_up = scroll_offset > 0;
            let can_scroll_down = scroll_offset + visible_height < total_items;
            let scroll_indicator = match (can_scroll_up, can_scroll_down) {
                (true, true) => format!(" [{}-{} of {}] ↕", scroll_offset + 1, (scroll_offset + visible_height).min(total_items), total_items),
                (true, false) => format!(" [{}-{} of {}] ↑", scroll_offset + 1, total_items, total_items),
                (false, true) => format!(" [1-{} of {}] ↓", visible_height, total_items),
                (false, false) => String::new(),
            };
            if !scroll_indicator.is_empty() {
                lines.push(Line::from(Span::styled(scroll_indicator, Style::default().fg(Color::DarkGray))));
            }
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
    let area = centered_rect(65, 65, f.area());
    f.render_widget(Clear, area);

    if let Some(state) = &app.notify_setup_state {
        use super::types::NotifyType;

        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Configure Notifications", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));

        // Show current configuration
        let config = crate::config::Config::load().unwrap_or_default();
        if let Some(ref target) = config.default_notify {
            let description = describe_notify_target(target);
            lines.push(Line::from(vec![
                Span::styled(" Current: ", Style::default().fg(Color::DarkGray)),
                Span::styled(description, Style::default().fg(Color::Green)),
            ]));
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled(" Current: Not configured", Style::default().fg(Color::Yellow))));
            lines.push(Line::from(""));
        }

        if state.step == 0 {
            lines.push(Line::from(Span::styled(" Select notification service:", Style::default().fg(Color::Yellow))));
            lines.push(Line::from(""));

            let types = [
                (NotifyType::Ntfy, "ntfy", "Free, open source push notifications"),
                (NotifyType::Gotify, "Gotify", "Self-hosted push notification server"),
                (NotifyType::Slack, "Slack", "Slack incoming webhook"),
                (NotifyType::Discord, "Discord", "Discord webhook"),
                (NotifyType::Telegram, "Telegram", "Telegram bot notifications"),
                (NotifyType::Pushover, "Pushover", "Pushover notifications"),
                (NotifyType::Command, "Command", "Run a shell command"),
            ];

            for (notify_type, name, desc) in types.iter() {
                let is_selected = std::mem::discriminant(&state.notify_type) == std::mem::discriminant(notify_type);
                if is_selected {
                    lines.push(Line::from(vec![
                        Span::styled(" > ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::styled(*name, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" - {}", desc), Style::default().fg(Color::DarkGray)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw("   "),
                        Span::raw(*name),
                        Span::styled(format!(" - {}", desc), Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(" ↑/↓ or j/k: Navigate  Enter: Select  Esc: Cancel", Style::default().fg(Color::DarkGray))));
        } else {
            // Step 1: Enter value based on selected type
            let (prompt, hint, example) = match state.notify_type {
                NotifyType::Ntfy => (
                    "Enter ntfy topic name:",
                    "The topic others subscribe to",
                    "my-alerts",
                ),
                NotifyType::Gotify => (
                    "Enter Gotify server and token:",
                    "Format: server|token",
                    "gotify.example.com|AbCdEf123456",
                ),
                NotifyType::Slack => (
                    "Enter Slack webhook URL:",
                    "From Slack app > Incoming Webhooks",
                    "https://hooks.slack.com/services/T.../B.../...",
                ),
                NotifyType::Discord => (
                    "Enter Discord webhook URL:",
                    "Server Settings > Integrations > Webhooks",
                    "https://discord.com/api/webhooks/...",
                ),
                NotifyType::Telegram => (
                    "Enter Telegram chat ID and bot token:",
                    "Format: chat_id|bot_token",
                    "123456789|123456:ABC-DEF...",
                ),
                NotifyType::Pushover => (
                    "Enter Pushover user key and API token:",
                    "Format: user_key|api_token",
                    "uQiRzpo4DXgh...|azGDORePK8gMa...",
                ),
                NotifyType::Command => (
                    "Enter command to run:",
                    "Receives JSON payload via stdin",
                    "notify-send \"$KTO_TITLE\"",
                ),
            };

            lines.push(Line::from(Span::styled(format!(" {}", prompt), Style::default().fg(Color::Yellow))));
            lines.push(Line::from(Span::styled(format!(" {}", hint), Style::default().fg(Color::DarkGray))));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw(" > "),
                Span::styled(&state.field1, Style::default().fg(Color::White)),
                Span::styled("_", Style::default().fg(Color::White).add_modifier(Modifier::SLOW_BLINK)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(format!(" Example: {}", example), Style::default().fg(Color::DarkGray))));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(" Enter: Save  Esc: Back", Style::default().fg(Color::DarkGray))));
        }

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Notification Setup "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_filter_list(f: &mut Frame, app: &App) {
    let area = centered_rect(65, 50, f.area());
    f.render_widget(Clear, area);

    let mut lines = Vec::new();
    lines.push(Line::from(""));

    if let Some(watch) = app.selected_watch() {
        lines.push(Line::from(vec![
            Span::styled(" Filters for: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&watch.name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(""));

        if watch.filters.is_empty() {
            lines.push(Line::from(Span::styled("  No filters defined", Style::default().fg(Color::DarkGray))));
            lines.push(Line::from(""));
            lines.push(Line::from("  All changes will trigger notifications."));
            lines.push(Line::from("  Add filters to narrow which changes notify."));
        } else {
            lines.push(Line::from(Span::styled(
                format!("  {} filter{} (all must pass):", watch.filters.len(), if watch.filters.len() == 1 { "" } else { "s" }),
                Style::default().fg(Color::DarkGray)
            )));
            lines.push(Line::from(""));

            for (i, filter) in watch.filters.iter().enumerate() {
                let is_selected = i == app.selected_filter;
                let marker_style = if is_selected {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let filter_style = if is_selected {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", if is_selected { ">" } else { " " }), marker_style),
                    Span::styled(format!("{}. ", i + 1), Style::default().fg(Color::DarkGray)),
                    Span::styled(format_filter_readable(filter), filter_style),
                ]));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(" Filters", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));
        lines.push(Line::from("  No watch selected"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" n: new  e/Enter: edit  d: delete  Esc: back ", Style::default().fg(Color::DarkGray))));

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Filter List "))
        .style(Style::default().bg(Color::Black));
    f.render_widget(widget, area);
}

fn render_filter_edit(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 55, f.area());
    f.render_widget(Clear, area);

    if let Some(ref state) = app.filter_edit_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));

        // Show if editing or adding new
        let title = if let Some(idx) = state.filter_idx {
            let total = app.selected_watch().map(|w| w.filters.len()).unwrap_or(0);
            format!("Editing filter {} of {}", idx + 1, total)
        } else {
            "Adding new filter".to_string()
        };
        lines.push(Line::from(Span::styled(format!(" {}", title), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));

        // Field highlighting
        let field_style = |field: FilterEditField| {
            if state.field == field {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }
        };

        // Target field
        let target_str = match state.target {
            crate::watch::FilterTarget::New => "new content",
            crate::watch::FilterTarget::Diff => "diff",
            crate::watch::FilterTarget::Old => "old content",
        };
        lines.push(Line::from(vec![
            Span::styled(" Target:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} (space to cycle)", target_str), field_style(FilterEditField::Target)),
        ]));

        // Condition field
        let condition_str = match state.condition {
            FilterCondition::Contains => "contains",
            FilterCondition::NotContains => "not contains",
            FilterCondition::Matches => "matches (regex)",
            FilterCondition::SizeGt => "size greater than",
        };
        lines.push(Line::from(vec![
            Span::styled(" Condition: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} (space to cycle)", condition_str), field_style(FilterEditField::Condition)),
        ]));

        // Value field
        let value_hint = match state.condition {
            FilterCondition::Contains | FilterCondition::NotContains => "text to match",
            FilterCondition::Matches => "regex pattern",
            FilterCondition::SizeGt => "number of chars",
        };
        lines.push(Line::from(vec![
            Span::styled(" Value:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}_", state.value), field_style(FilterEditField::Value)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("            ", Style::default()),
            Span::styled(format!("({})", value_hint), Style::default().fg(Color::DarkGray)),
        ]));

        lines.push(Line::from(""));

        // Preview of this filter
        lines.push(Line::from(Span::styled(" Preview:", Style::default().fg(Color::DarkGray))));
        let preview_filter = state.to_filter();
        lines.push(Line::from(Span::styled(
            format!("   {}", format_filter_readable(&preview_filter)),
            Style::default().fg(Color::Cyan)
        )));

        // Show other filters if any
        if let Some(watch) = app.selected_watch() {
            if !watch.filters.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" Other filters:", Style::default().fg(Color::DarkGray))));
                for (i, filter) in watch.filters.iter().enumerate() {
                    // Skip the filter we're currently editing
                    if Some(i) == state.filter_idx {
                        continue;
                    }
                    lines.push(Line::from(Span::styled(
                        format!("   {}. {}", i + 1, format_filter_brief(filter)),
                        Style::default().fg(Color::DarkGray)
                    )));
                }
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Tab: next field  Enter: Save  Esc: Cancel ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Filter Edit "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

fn render_memory_inspector(f: &mut Frame, app: &App) {
    let area = centered_rect(75, 75, f.area());
    f.render_widget(Clear, area);

    if let Some(ref state) = app.memory_inspector_state {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Agent Memory: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.watch_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(""));

        // Show section tabs
        let sections = [
            (MemorySection::Counters, "Counters"),
            (MemorySection::LastValues, "Values"),
            (MemorySection::Notes, "Notes"),
        ];
        let mut tab_spans = vec![Span::raw(" ")];
        for (i, (section, label)) in sections.iter().enumerate() {
            if i > 0 {
                tab_spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            }
            if std::mem::discriminant(section) == std::mem::discriminant(&state.section) {
                tab_spans.push(Span::styled(
                    format!("[{}]", label),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                ));
            } else {
                tab_spans.push(Span::styled(*label, Style::default().fg(Color::DarkGray)));
            }
        }
        tab_spans.push(Span::styled("  (Tab to switch)", Style::default().fg(Color::DarkGray)));
        lines.push(Line::from(tab_spans));
        lines.push(Line::from(""));

        match state.section {
            MemorySection::Counters => {
                if state.memory.counters.is_empty() {
                    lines.push(Line::from(Span::styled("  No counters stored", Style::default().fg(Color::DarkGray))));
                    lines.push(Line::from(""));
                    lines.push(Line::from("  Counters track numeric values across checks."));
                    lines.push(Line::from("  Example: price_checks, version_bumps"));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("  {} counter{}:", state.memory.counters.len(), if state.memory.counters.len() == 1 { "" } else { "s" }),
                        Style::default().fg(Color::DarkGray)
                    )));
                    lines.push(Line::from(""));
                    for (i, (key, value)) in state.memory.counters.iter().enumerate() {
                        let is_selected = i == state.selected_item;
                        let marker_style = if is_selected {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        let key_style = if is_selected {
                            Style::default().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        lines.push(Line::from(vec![
                            Span::styled(format!("  {} ", if is_selected { ">" } else { " " }), marker_style),
                            Span::styled(key, key_style),
                            Span::styled(": ", Style::default().fg(Color::DarkGray)),
                            Span::styled(value.to_string(), Style::default().fg(Color::Cyan)),
                        ]));
                    }
                }
            }
            MemorySection::LastValues => {
                if state.memory.last_values.is_empty() {
                    lines.push(Line::from(Span::styled("  No values stored", Style::default().fg(Color::DarkGray))));
                    lines.push(Line::from(""));
                    lines.push(Line::from("  Values store the last seen state of something."));
                    lines.push(Line::from("  Example: last_price, current_version"));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("  {} value{}:", state.memory.last_values.len(), if state.memory.last_values.len() == 1 { "" } else { "s" }),
                        Style::default().fg(Color::DarkGray)
                    )));
                    lines.push(Line::from(""));
                    for (i, (key, value)) in state.memory.last_values.iter().enumerate() {
                        let is_selected = i == state.selected_item;
                        let marker_style = if is_selected {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        let key_style = if is_selected {
                            Style::default().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        // Format value - truncate if too long
                        let value_str = value.to_string();
                        let value_display = if value_str.len() > 40 {
                            format!("{}...", &value_str[..37])
                        } else {
                            value_str
                        };
                        lines.push(Line::from(vec![
                            Span::styled(format!("  {} ", if is_selected { ">" } else { " " }), marker_style),
                            Span::styled(key, key_style),
                            Span::styled(": ", Style::default().fg(Color::DarkGray)),
                            Span::styled(value_display, Style::default().fg(Color::Cyan)),
                        ]));
                    }
                }
            }
            MemorySection::Notes => {
                if state.memory.notes.is_empty() {
                    lines.push(Line::from(Span::styled("  No notes stored", Style::default().fg(Color::DarkGray))));
                    lines.push(Line::from(""));
                    lines.push(Line::from("  Notes are timestamped observations from the AI."));
                    lines.push(Line::from("  Notes older than 7 days are automatically removed."));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("  {} note{} (expire after 7 days):", state.memory.notes.len(), if state.memory.notes.len() == 1 { "" } else { "s" }),
                        Style::default().fg(Color::DarkGray)
                    )));
                    lines.push(Line::from(""));
                    for (i, note) in state.memory.notes.iter().enumerate() {
                        let is_selected = i == state.selected_item;
                        let marker_style = if is_selected {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };

                        // Try to extract timestamp from note
                        let (timestamp, content) = extract_note_timestamp(note);
                        let content_style = if is_selected {
                            Style::default().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };

                        // Truncate content if needed
                        let max_len = (area.width as usize).saturating_sub(20);
                        let content_display = if content.len() > max_len {
                            format!("{}...", &content[..max_len.saturating_sub(3)])
                        } else {
                            content.to_string()
                        };

                        if let Some(ts) = timestamp {
                            lines.push(Line::from(vec![
                                Span::styled(format!("  {} ", if is_selected { ">" } else { " " }), marker_style),
                                Span::styled(format!("{} ", ts), Style::default().fg(Color::DarkGray)),
                                Span::styled(content_display, content_style),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::styled(format!("  {} ", if is_selected { ">" } else { " " }), marker_style),
                                Span::styled(content_display, content_style),
                            ]));
                        }
                    }
                }
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" j/k: navigate  d: delete  C: clear all  r: refresh  Esc: close ", Style::default().fg(Color::DarkGray))));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Memory Inspector "))
            .style(Style::default().bg(Color::Black));
        f.render_widget(widget, area);
    }
}

/// Extract timestamp from a note if present
fn extract_note_timestamp(note: &str) -> (Option<&str>, &str) {
    // Notes may have format: "2026-01-15T03:43:33: content"
    // or "CRITICAL: 2026-01-15T03:43:33: content"
    if let Some(colon_idx) = note.find(':') {
        let prefix = &note[..colon_idx];
        // Check if prefix looks like a timestamp (starts with year)
        if prefix.len() >= 10 && prefix.starts_with("202") {
            let rest = note[colon_idx + 1..].trim_start();
            return (Some(prefix), rest);
        }
        // Check for "CRITICAL: timestamp: content" pattern
        if prefix == "CRITICAL" || prefix == "WARNING" || prefix == "INFO" {
            let rest = &note[colon_idx + 1..].trim_start();
            if let Some(next_colon) = rest.find(':') {
                let timestamp = &rest[..next_colon];
                if timestamp.len() >= 10 && timestamp.starts_with("202") {
                    let content = rest[next_colon + 1..].trim_start();
                    return (Some(timestamp), content);
                }
            }
        }
    }
    (None, note)
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
                Span::styled(format_extraction_display(&edit_state.extraction), field_style(EditField::Extraction)),
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
                Span::styled(
                    format_instructions_preview(&edit_state.agent_instructions),
                    field_style(EditField::AgentInstructions)
                ),
            ]),
            Line::from(vec![
                Span::styled(" Use Profile:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(use_profile_str, field_style(EditField::UseProfile)),
            ]),
            Line::from(vec![
                Span::styled(" Filters:      ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format_filters_preview(app.selected_watch()),
                    field_style(EditField::Filters)
                ),
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
    // Adjust modal size based on action type
    let (width, height) = match action {
        ConfirmAction::Delete | ConfirmAction::DeleteReminder => (55, 35),
        ConfirmAction::Test | ConfirmAction::ForceCheck => (55, 25),
    };
    let area = centered_rect(width, height, f.area());
    f.render_widget(Clear, area);

    let mut lines = vec![Line::from("")];

    match action {
        ConfirmAction::Delete => {
            if let Some(watch) = app.selected_watch() {
                lines.push(Line::from(Span::styled(
                    format!(" Delete '{}'?", watch.name),
                    Style::default().add_modifier(Modifier::BOLD)
                )));
                lines.push(Line::from(""));

                // URL (truncated if needed)
                let max_url_len = (area.width as usize).saturating_sub(10);
                let url_display = if watch.url.len() > max_url_len {
                    format!("{}...", &watch.url[..max_url_len.saturating_sub(3)])
                } else {
                    watch.url.clone()
                };
                lines.push(Line::from(vec![
                    Span::styled("   URL: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(url_display),
                ]));

                // Engine
                let engine = match &watch.engine {
                    Engine::Http => "HTTP",
                    Engine::Playwright => "Playwright (JS)",
                    Engine::Rss => "RSS/Atom",
                    Engine::Shell { .. } => "Shell",
                };
                lines.push(Line::from(vec![
                    Span::styled("   Engine: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(engine),
                ]));

                // AI Agent status
                let agent_status = if watch.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false) {
                    "Enabled"
                } else {
                    "Disabled"
                };
                lines.push(Line::from(vec![
                    Span::styled("   AI Agent: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(agent_status),
                ]));

                // Change count
                let change_count = app.changes.len();
                lines.push(Line::from(vec![
                    Span::styled("   History: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(format!("{} changes tracked", change_count)),
                ]));

                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "   This will delete all snapshots and history!",
                    Style::default().fg(Color::Red)
                )));
            } else {
                lines.push(Line::from(" Delete watch?"));
            }
        }
        ConfirmAction::DeleteReminder => {
            if let Some(reminder) = app.selected_reminder() {
                lines.push(Line::from(Span::styled(
                    format!(" Delete reminder '{}'?", reminder.name),
                    Style::default().add_modifier(Modifier::BOLD)
                )));
                lines.push(Line::from(""));

                // Scheduled time
                let local_time: chrono::DateTime<chrono::Local> = reminder.trigger_at.into();
                lines.push(Line::from(vec![
                    Span::styled("   Scheduled: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(local_time.format("%Y-%m-%d %H:%M").to_string()),
                ]));

                // Recurring status
                let recurring = if let Some(interval) = reminder.interval_secs {
                    format!("Every {}", format_interval(interval))
                } else {
                    "One-time".to_string()
                };
                lines.push(Line::from(vec![
                    Span::styled("   Recurring: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(recurring),
                ]));
            } else {
                lines.push(Line::from(" Delete reminder?"));
            }
        }
        ConfirmAction::Test => {
            if let Some(watch) = app.selected_watch() {
                lines.push(Line::from(Span::styled(
                    format!(" Test '{}'?", watch.name),
                    Style::default().add_modifier(Modifier::BOLD)
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "   Fetches page and compares to last snapshot",
                    Style::default().fg(Color::Cyan)
                )));
                lines.push(Line::from(Span::styled(
                    "   Read-only: does NOT save or notify",
                    Style::default().fg(Color::DarkGray)
                )));
            } else {
                lines.push(Line::from(" Test watch?"));
            }
        }
        ConfirmAction::ForceCheck => {
            if let Some(watch) = app.selected_watch() {
                lines.push(Line::from(Span::styled(
                    format!(" Force check '{}'?", watch.name),
                    Style::default().add_modifier(Modifier::BOLD)
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "   Fetches page and SAVES new snapshot",
                    Style::default().fg(Color::Yellow)
                )));
                lines.push(Line::from(Span::styled(
                    "   May trigger notification if change detected",
                    Style::default().fg(Color::DarkGray)
                )));
            } else {
                lines.push(Line::from(" Force check watch?"));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" y ", Style::default().fg(Color::Green)),
        Span::raw("Yes  "),
        Span::styled(" n ", Style::default().fg(Color::Red)),
        Span::raw("No"),
    ]));

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Confirm "))
        .style(Style::default().bg(Color::Black));
    f.render_widget(widget, area);
}

/// Calculate the actual trigger time for a reminder
fn calculate_reminder_time(when_type: &str, when_value: &str) -> String {
    use chrono::{Local, NaiveTime, Duration};

    let now = Local::now();

    if when_type == "at" {
        // Parse time like "14:30"
        if let Ok(time) = NaiveTime::parse_from_str(when_value, "%H:%M") {
            let today = now.date_naive().and_time(time);
            let today_local = today.and_local_timezone(Local).unwrap();

            let trigger = if today_local > now {
                today_local
            } else {
                // Tomorrow
                today_local + Duration::days(1)
            };

            trigger.format("%a %b %d at %H:%M").to_string()
        } else {
            format!("at {}", when_value)
        }
    } else {
        // Parse duration like "1h", "30m", "2d"
        let value = when_value.trim();
        let (num_str, unit) = if value.ends_with('h') || value.ends_with('m') || value.ends_with('d') || value.ends_with('s') {
            let len = value.len();
            (&value[..len - 1], &value[len - 1..])
        } else {
            (value, "m") // default to minutes
        };

        if let Ok(num) = num_str.parse::<i64>() {
            let duration = match unit {
                "s" => Duration::seconds(num),
                "m" => Duration::minutes(num),
                "h" => Duration::hours(num),
                "d" => Duration::days(num),
                _ => Duration::minutes(num),
            };

            let trigger = now + duration;
            trigger.format("%a %b %d at %H:%M").to_string()
        } else {
            format!("in {}", when_value)
        }
    }
}

/// Format extraction string for display
fn format_extraction_display(extraction: &str) -> String {
    if extraction.starts_with("css:") {
        // Show full CSS selector
        extraction.to_string()
    } else if extraction.starts_with("jsonld:") {
        extraction.to_string()
    } else if extraction.starts_with("meta:") {
        extraction.to_string()
    } else {
        // auto, full, rss - just show as-is
        extraction.to_string()
    }
}

/// Format AI instructions preview for edit mode
fn format_instructions_preview(instructions: &str) -> String {
    if instructions.is_empty() {
        "(none) (e to edit)".to_string()
    } else {
        // Show truncated preview + hint
        let preview = if instructions.len() > 40 {
            format!("\"{}...\"", &instructions[..40].replace('\n', " "))
        } else {
            format!("\"{}\"", instructions.replace('\n', " "))
        };
        format!("{} (e to edit)", preview)
    }
}

/// Format filters preview for edit mode
fn format_filters_preview(watch: Option<&crate::watch::Watch>) -> String {
    let filters = watch.map(|w| &w.filters);
    match filters {
        None => "(no watch) (f to manage)".to_string(),
        Some(f) if f.is_empty() => "(none) (f to manage)".to_string(),
        Some(f) => {
            let count = f.len();
            let preview = f.iter()
                .take(2)
                .map(|filter| format_filter_brief(filter))
                .collect::<Vec<_>>()
                .join(", ");

            if count > 2 {
                format!("{}: {}, ... (f to manage)", count, preview)
            } else {
                format!("{}: {} (f to manage)", count, preview)
            }
        }
    }
}

/// Format a single filter as a brief human-readable string
fn format_filter_brief(filter: &crate::watch::Filter) -> String {
    let target = match filter.on {
        crate::watch::FilterTarget::New => "new",
        crate::watch::FilterTarget::Diff => "diff",
        crate::watch::FilterTarget::Old => "old",
    };

    if let Some(ref v) = filter.contains {
        let display_v = if v.len() > 15 { format!("{}...", &v[..15]) } else { v.clone() };
        format!("contains \"{}\"", display_v)
    } else if let Some(ref v) = filter.not_contains {
        let display_v = if v.len() > 15 { format!("{}...", &v[..15]) } else { v.clone() };
        format!("!contains \"{}\"", display_v)
    } else if let Some(ref v) = filter.matches {
        let display_v = if v.len() > 15 { format!("{}...", &v[..15]) } else { v.clone() };
        format!("matches /{}/", display_v)
    } else if let Some(n) = filter.size_gt {
        format!("size > {}", n)
    } else {
        format!("on {}", target)
    }
}

/// Format a filter as a full human-readable string
fn format_filter_readable(filter: &crate::watch::Filter) -> String {
    let target = match filter.on {
        crate::watch::FilterTarget::New => "new content",
        crate::watch::FilterTarget::Diff => "diff",
        crate::watch::FilterTarget::Old => "old content",
    };

    if let Some(ref v) = filter.contains {
        format!("contains \"{}\" (on {})", v, target)
    } else if let Some(ref v) = filter.not_contains {
        format!("not contains \"{}\" (on {})", v, target)
    } else if let Some(ref v) = filter.matches {
        format!("matches /{}/ (on {})", v, target)
    } else if let Some(n) = filter.size_gt {
        format!("size > {} chars (on {})", n, target)
    } else {
        format!("(empty filter on {})", target)
    }
}

/// Describe a notification target for display
fn describe_notify_target(target: &crate::config::NotifyTarget) -> String {
    use crate::config::NotifyTarget;
    match target {
        NotifyTarget::Ntfy { topic, server } => {
            let host = server.as_deref().unwrap_or("ntfy.sh");
            format!("ntfy ({}/{})", host, topic)
        }
        NotifyTarget::Slack { webhook_url } => {
            format!("Slack ({}...)", &webhook_url[..50.min(webhook_url.len())])
        }
        NotifyTarget::Discord { webhook_url } => {
            format!("Discord ({}...)", &webhook_url[..50.min(webhook_url.len())])
        }
        NotifyTarget::Gotify { server, token: _ } => {
            format!("Gotify ({})", server)
        }
        NotifyTarget::Command { command } => {
            format!("Command: {}", command)
        }
        NotifyTarget::Telegram { chat_id, bot_token: _ } => {
            format!("Telegram (chat: {})", chat_id)
        }
        NotifyTarget::Pushover { user_key, api_token: _ } => {
            format!("Pushover ({}...)", &user_key[..8.min(user_key.len())])
        }
        NotifyTarget::Matrix { homeserver, room_id, access_token: _ } => {
            format!("Matrix ({}: {})", homeserver, room_id)
        }
        NotifyTarget::Email { smtp_server, from, to, .. } => {
            format!("Email ({} -> {} via {})", from, to, smtp_server)
        }
    }
}

/// Check if a watch is stale (no check in 2x its interval)
fn is_watch_stale(db: &crate::db::Database, watch: &crate::watch::Watch) -> bool {
    if !watch.enabled {
        return false;
    }

    if let Ok(Some(snapshot)) = db.get_latest_snapshot(&watch.id) {
        let ago = chrono::Utc::now().signed_duration_since(snapshot.fetched_at);
        let stale_threshold = (watch.interval_secs * 2) as i64;
        ago.num_seconds() > stale_threshold
    } else {
        // No snapshot yet - consider stale if watch was created more than 2x interval ago
        let ago = chrono::Utc::now().signed_duration_since(watch.created_at);
        let stale_threshold = (watch.interval_secs * 2) as i64;
        ago.num_seconds() > stale_threshold
    }
}

/// Render enhanced empty state for watches pane
fn render_empty_watches_state(f: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.focus == Pane::Watches {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Welcome to kto!",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Let's set up your first watch.",
            Style::default().fg(Color::Yellow)
        )),
        Line::from(""),
        Line::from("  Quick Start:"),
        Line::from(vec![
            Span::styled("    n", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("  Create new watch (with templates)"),
        ]),
        Line::from(vec![
            Span::styled("    N", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("  Set up notifications first"),
        ]),
        Line::from(vec![
            Span::styled("    ?", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("  View all keybindings"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Available templates:",
            Style::default().fg(Color::DarkGray)
        )),
        Line::from(Span::styled(
            "    - Price Drop Monitor",
            Style::default().fg(Color::DarkGray)
        )),
        Line::from(Span::styled(
            "    - Back-in-Stock Alert",
            Style::default().fg(Color::DarkGray)
        )),
        Line::from(Span::styled(
            "    - Job Posting Tracker",
            Style::default().fg(Color::DarkGray)
        )),
        Line::from(Span::styled(
            "    - Changelog Watcher",
            Style::default().fg(Color::DarkGray)
        )),
    ];

    let paragraph = Paragraph::new(text)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(" Watches ")
            .border_style(border_style));

    f.render_widget(paragraph, area);
}

/// Render the health dashboard
fn render_health_dashboard(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 60, f.area());
    f.render_widget(Clear, area);

    // Gather health data
    let daemon_status = get_daemon_status(&app.db);
    let total_watches = app.watches.len();
    let active_watches = app.watches.iter().filter(|w| w.enabled).count();
    let ai_watches = app.watches.iter().filter(|w| w.agent_config.as_ref().map(|c| c.enabled).unwrap_or(false)).count();
    let error_watches = app.watch_errors.len();
    let stale_watches = app.watches.iter().filter(|w| is_watch_stale(&app.db, w)).count();
    let healthy_watches = active_watches.saturating_sub(error_watches).saturating_sub(stale_watches);

    // Get notification config
    let config = crate::config::Config::load().unwrap_or_default();
    let notify_status = if let Some(ref target) = config.default_notify {
        describe_notify_target(target)
    } else {
        "Not configured".to_string()
    };

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " System Health Dashboard",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    )));
    lines.push(Line::from(""));

    // Daemon status
    lines.push(Line::from(Span::styled(" Daemon:", Style::default().fg(Color::Yellow))));
    match &daemon_status {
        DaemonStatus::Running(info) => {
            lines.push(Line::from(vec![
                Span::styled("   ● Running ", Style::default().fg(Color::Green)),
                Span::styled(format!("({})", info), Style::default().fg(Color::DarkGray)),
            ]));
        }
        DaemonStatus::Stale(info) => {
            lines.push(Line::from(vec![
                Span::styled("   ◐ Stale ", Style::default().fg(Color::Yellow)),
                Span::styled(format!("({})", info), Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(Span::styled(
                "   Consider running: kto daemon",
                Style::default().fg(Color::DarkGray)
            )));
        }
        DaemonStatus::Stopped => {
            lines.push(Line::from(vec![
                Span::styled("   ○ Not running", Style::default().fg(Color::Red)),
            ]));
            lines.push(Line::from(Span::styled(
                "   Start with: kto daemon &",
                Style::default().fg(Color::DarkGray)
            )));
        }
    }
    lines.push(Line::from(""));

    // Watch stats
    lines.push(Line::from(Span::styled(" Watches:", Style::default().fg(Color::Yellow))));
    lines.push(Line::from(vec![
        Span::styled("   Total: ", Style::default().fg(Color::DarkGray)),
        Span::raw(format!("{}", total_watches)),
    ]));

    if healthy_watches > 0 {
        lines.push(Line::from(vec![
            Span::styled("   ● ", Style::default().fg(Color::Green)),
            Span::raw(format!("{} healthy", healthy_watches)),
        ]));
    }

    if stale_watches > 0 {
        lines.push(Line::from(vec![
            Span::styled("   ◐ ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{} stale (not checked in 2x interval)", stale_watches)),
        ]));
    }

    if error_watches > 0 {
        lines.push(Line::from(vec![
            Span::styled("   ✗ ", Style::default().fg(Color::Red)),
            Span::raw(format!("{} with errors", error_watches)),
        ]));
    }

    if ai_watches > 0 {
        lines.push(Line::from(vec![
            Span::styled("   AI: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{} with AI agent enabled", ai_watches)),
        ]));
    }

    let paused = total_watches - active_watches;
    if paused > 0 {
        lines.push(Line::from(vec![
            Span::styled("   ○ ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{} paused", paused)),
        ]));
    }

    lines.push(Line::from(""));

    // Notifications
    lines.push(Line::from(Span::styled(" Notifications:", Style::default().fg(Color::Yellow))));
    lines.push(Line::from(vec![
        Span::styled("   Target: ", Style::default().fg(Color::DarkGray)),
        Span::raw(&notify_status),
    ]));

    // Quiet hours
    if let Some(ref quiet) = config.quiet_hours {
        lines.push(Line::from(vec![
            Span::styled("   Quiet hours: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{} - {}", quiet.start, quiet.end)),
        ]));
        if quiet.is_quiet_now() {
            lines.push(Line::from(Span::styled(
                "   (Currently in quiet hours)",
                Style::default().fg(Color::Yellow)
            )));
        }
    }

    lines.push(Line::from(""));

    // Status indicator legend
    lines.push(Line::from(Span::styled(" Status Indicators:", Style::default().fg(Color::DarkGray))));
    lines.push(Line::from(vec![
        Span::styled("   ● ", Style::default().fg(Color::Green)),
        Span::styled("healthy  ", Style::default().fg(Color::DarkGray)),
        Span::styled("◐ ", Style::default().fg(Color::Yellow)),
        Span::styled("stale  ", Style::default().fg(Color::DarkGray)),
        Span::styled("○ ", Style::default().fg(Color::Yellow)),
        Span::styled("paused  ", Style::default().fg(Color::DarkGray)),
        Span::styled("✗ ", Style::default().fg(Color::Red)),
        Span::styled("error", Style::default().fg(Color::DarkGray)),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Esc close  r refresh  L view logs ",
        Style::default().fg(Color::DarkGray)
    )));

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Health Dashboard "))
        .style(Style::default().bg(Color::Black));
    f.render_widget(widget, area);
}
