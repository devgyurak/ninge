// SPDX-License-Identifier: Apache-2.0
//! Application state, input mapping, and one frame of output.

use crate::kitty;
use crate::layout::{Areas, TermSize, compute};
use crate::pattern::dummy_frame;
use crate::placeholder::ScreenImage;
use crate::source::Source;
use crate::transfer::{NoShm, ShmSink, Transfer};
use anyhow::anyhow;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ninge_core::{Machine, Screen};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::io::Write;
use std::time::{Duration, Instant};

/// Image id of the top screen.
pub const TOP_IMAGE_ID: u32 = 1;
/// Image id of the bottom screen.
pub const BOTTOM_IMAGE_ID: u32 = 2;

/// What an input event asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do.
    None,
    /// Leave the program.
    Quit,
    /// The terminal changed size; the caller re-queries it and calls [`App::resize`].
    Resized,
}

/// Maps a terminal event to an [`Action`]. `q`, Esc, and Ctrl-C quit (raw mode delivers
/// Ctrl-C as a key).
pub fn map_event(event: &Event) -> Action {
    match event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) => {
            let ctrl_c = *code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL);
            if ctrl_c || matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
                Action::Quit
            } else {
                Action::None
            }
        }
        Event::Resize(..) => Action::Resized,
        _ => Action::None,
    }
}

/// Frontend state. `S` is where shared-memory frames are written.
pub struct App<S = NoShm> {
    size: TermSize,
    areas: Option<Areas>,
    frame_no: u64,
    source: Source,
    quit: bool,
    transfer: Transfer,
    sink: S,
}

impl App<NoShm> {
    /// A new app for a terminal of `size` that sends frames as zlib.
    pub fn new(size: TermSize) -> Self {
        Self::with_transfer(size, NoShm, Transfer::Direct { reason: None })
    }
}

impl<S: ShmSink> App<S> {
    /// A new app that sends frames the `transfer` way, writing shared-memory frames
    /// through `sink`.
    pub fn with_transfer(size: TermSize, sink: S, transfer: Transfer) -> Self {
        Self {
            size,
            areas: compute(size),
            frame_no: 0,
            source: Source::Pattern,
            quit: false,
            transfer,
            sink,
        }
    }

    /// Takes frames from `source` instead of the dummy pattern.
    #[must_use]
    pub fn with_source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    /// The machine behind the screens, `None` while the dummy pattern is shown.
    pub fn machine(&self) -> Option<&Machine> {
        match &self.source {
            Source::Machine(machine) => Some(machine),
            Source::Pattern => None,
        }
    }

    /// The medium frames go over now; it turns to zlib for good if a shared-memory write
    /// fails.
    pub fn transfer(&self) -> &Transfer {
        &self.transfer
    }

    /// Records that the terminal is now `size`; the next frame places the images anew.
    pub fn resize(&mut self, size: TermSize) {
        self.size = size;
        self.areas = compute(size);
    }

    /// Applies an [`Action`]; `Resized` is handled by the caller through [`App::resize`].
    pub fn apply(&mut self, action: Action) {
        self.quit |= action == Action::Quit;
    }

    /// Whether the user asked to quit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// The current layout, `None` when the terminal is too small.
    pub fn areas(&self) -> Option<Areas> {
        self.areas
    }

    /// The graphics bytes for this frame: each screen's pixels with its virtual
    /// placement. Re-transmitting an image deletes its placements, so every frame
    /// places both again. Advances the frame counter and, with a machine, runs one of
    /// its frames even when the terminal is too small to show it.
    pub fn graphics(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Source::Machine(machine) = &mut self.source {
            machine.run_frame();
        }
        if let Some(areas) = self.areas {
            let (top, bottom) = self.screens();
            out.extend(self.send(TOP_IMAGE_ID, &top, areas.top));
            out.extend(self.send(BOTTOM_IMAGE_ID, &bottom, areas.bottom));
        }
        self.frame_no += 1;
        out
    }

    /// The pixels of both screens for this tick.
    fn screens(&self) -> (Vec<u32>, Vec<u32>) {
        match &self.source {
            Source::Machine(machine) => (
                machine.frame(Screen::Top).to_vec(),
                machine.frame(Screen::Bottom).to_vec(),
            ),
            Source::Pattern => (dummy_frame(self.frame_no, 0), dummy_frame(self.frame_no, 1)),
        }
    }

    /// One screen over the current medium. A failed shared-memory write is not retried:
    /// this frame and every later one go as zlib, and the panel says why.
    fn send(&mut self, image_id: u32, pixels: &[u32], area: Rect) -> Vec<u8> {
        if self.transfer == Transfer::Shm {
            match self.sink.put(&kitty::rgb24(pixels)) {
                Ok(name) => {
                    return kitty::transmit_shm(image_id, &name, area.width, area.height);
                }
                Err(e) => self.transfer = Transfer::direct(format!("shm write failed: {e}")),
            }
        }
        kitty::transmit(image_id, pixels, area.width, area.height)
    }

    /// Draws the screens and panels.
    pub fn draw(&self, frame: &mut Frame) {
        let Some(areas) = self.areas else {
            frame.render_widget(Paragraph::new("terminal too small"), frame.area());
            return;
        };
        frame.render_widget(
            ScreenImage {
                image_id: TOP_IMAGE_ID,
            },
            areas.top,
        );
        frame.render_widget(
            ScreenImage {
                image_id: BOTTOM_IMAGE_ID,
            },
            areas.bottom,
        );
        let note = if self.size.px_known {
            ""
        } else {
            "\ncell size unknown, assuming 8x16"
        };
        // TODO(phase1): real register and disassembly panels (#56).
        frame.render_widget(
            Paragraph::new(format!(
                "TODO(phase1) (#56){note}\n{}",
                self.transfer.label()
            ))
            .wrap(Wrap { trim: false })
            .block(Block::new().borders(Borders::ALL).title("Registers")),
            areas.registers,
        );
        frame.render_widget(
            Paragraph::new("TODO(phase1) (#56)")
                .block(Block::new().borders(Borders::ALL).title("Disassembly")),
            areas.disasm,
        );
    }
}

/// How long to wait for input: the time left until `deadline`, or zero once it has
/// passed. Zero still polls, so pending keys and resizes are read every frame even when
/// drawing overran the frame time.
pub fn poll_timeout(deadline: Instant, now: Instant) -> Duration {
    deadline.saturating_duration_since(now)
}

/// One tick: write the graphics to `gfx`, then draw with ratatui.
pub fn frame_step<S: ShmSink, B: Backend, W: Write>(
    app: &mut App<S>,
    terminal: &mut Terminal<B>,
    gfx: &mut W,
) -> anyhow::Result<()> {
    gfx.write_all(&app.graphics())?;
    gfx.flush()?;
    terminal.draw(|f| app.draw(f)).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
