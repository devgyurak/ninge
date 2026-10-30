// SPDX-License-Identifier: Apache-2.0
//! The terminal adapter: raw mode, alternate screen, size queries, and the main loop.
//!
//! This is the only module that touches the OS terminal. Everything it drives is
//! tested through [`crate::app`] with a test backend. OS-specific pieces stay here so a
//! Windows backend is an addition (AGENTS.md §2).

use crate::app::{Action, App, frame_step, map_event};
use crate::layout::TermSize;
use anyhow::Context as _;
use crossterm::cursor::{Hide, Show};
use crossterm::event;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, window_size,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Write};
use std::time::{Duration, Instant};

/// Fixed output rate; adaptive rate is #63.
const FRAME_TIME: Duration = Duration::from_micros(16_667);

/// Restores the terminal on drop.
struct Restore;

fn restore() {
    let mut out = io::stdout();
    let _ = crossterm::execute!(out, Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

impl Drop for Restore {
    fn drop(&mut self) {
        restore();
    }
}

/// The current terminal size, with the cell pixel size when the terminal reports it.
pub fn size() -> io::Result<TermSize> {
    let w = window_size()?;
    Ok(TermSize::from_window(w.columns, w.rows, w.width, w.height))
}

/// Runs the TUI until the user quits.
pub fn run() -> anyhow::Result<()> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    enable_raw_mode().context("enabling raw mode")?;
    let _restore = Restore;
    let mut out = io::stdout();
    crossterm::execute!(out, EnterAlternateScreen, Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new(size()?);
    let mut gfx = io::stdout();
    while !app.should_quit() {
        let deadline = Instant::now() + FRAME_TIME;
        frame_step(&mut app, &mut terminal, &mut gfx)?;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if event::poll(left)? {
                match map_event(&event::read()?) {
                    Action::Resized => app.resize(size()?),
                    other => app.apply(other),
                }
            }
        }
    }
    gfx.flush()?;
    Ok(())
}
