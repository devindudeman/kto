//! TUI module - Terminal user interface for kto
//!
//! This module provides a ratatui-based terminal UI for managing watches,
//! viewing changes, and configuring notifications.

#![cfg(feature = "tui")]

mod types;
mod state;
mod utils;
mod editor;
mod render;
mod input;

// Re-export main types for external use
pub use types::*;
pub use state::*;
pub use utils::{format_interval, parse_duration_str, build_notify_target, parse_extraction_string, centered_rect};
pub use editor::open_in_editor;

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};

use crate::error::Result;

/// Run the TUI
pub fn run() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new()?;
    let mut last_refresh = Instant::now();
    let mut last_interaction = Instant::now();

    // Main loop
    loop {
        // Check if we need a full redraw (after returning from external editor)
        if app.needs_full_redraw {
            // Sync terminal size in case it was resized while in editor
            terminal.autoresize()?;
            terminal.clear()?;
            app.needs_full_redraw = false;
        }

        // Draw UI
        terminal.draw(|f| render::ui(f, &mut app))?;

        // Auto-refresh every 1 second, but debounce during user interaction
        // Skip refresh if user interacted within last 2 seconds to prevent UI jumpiness
        let interaction_debounce = last_interaction.elapsed() > Duration::from_secs(2);
        if last_refresh.elapsed() > Duration::from_secs(1) && interaction_debounce {
            let _ = app.refresh();
            last_refresh = Instant::now();
        }

        // Handle input with timeout
        if event::poll(Duration::from_millis(100))? {
            let event = event::read()?;
            // Track interaction for debounce
            last_interaction = Instant::now();
            match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    input::handle_key_event(&mut app, key.code, key.modifiers)?;
                }
                Event::Mouse(mouse) => {
                    input::handle_mouse_event(&mut app, mouse)?;
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
