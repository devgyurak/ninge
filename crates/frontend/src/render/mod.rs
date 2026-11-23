// SPDX-License-Identifier: Apache-2.0
//! The renderer: a [`Snapshot`] of what the emu side decided, written to the terminal.
//!
//! The renderer makes no emulation decision (AGENTS.md §2 rule 12). The pixels it sends are the
//! ones the model chose to show (backlight, cursor overlay), the layout is the model's, and what
//! it adds is only what it owns: the transfer medium and its label.

use crate::input::FALLBACK_HOLD_FRAMES;
use crate::kitty;
use crate::layout::Areas;
use crate::placeholder::ScreenImage;
use crate::transfer::{NoShm, ShmSink, Transfer};
use anyhow::anyhow;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::io::Write;

/// Image id of the top screen.
pub const TOP_IMAGE_ID: u32 = 1;
/// Image id of the bottom screen.
pub const BOTTOM_IMAGE_ID: u32 = 2;

/// Everything the renderer needs for one frame, as plain data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The top screen as shown: black while its backlight is off.
    pub top: Vec<u32>,
    /// The bottom screen as shown: black while its backlight is off, with the cursor's crosshair
    /// over it. Both are empty while `areas` is `None`, since nothing is sent then.
    pub bottom: Vec<u32>,
    /// The layout, `None` when the terminal is too small to show a screen.
    pub areas: Option<Areas>,
    /// False when the cell size is a guess, which the panel says.
    pub px_known: bool,
    /// Whether the terminal reports key release; without it the panel warns.
    pub key_release_reported: bool,
    /// Where the virtual cursor is while cursor mode is on, for the panel.
    pub cursor: Option<(u8, u8)>,
    /// Whether the gamepad library started.
    pub pad_available: bool,
    /// How many pads are plugged in.
    pub pads_connected: u32,
}

/// Writes snapshots to the terminal. `S` is where shared-memory frames are written.
pub struct Renderer<S = NoShm> {
    transfer: Transfer,
    sink: S,
}

impl<S: ShmSink> Renderer<S> {
    /// A renderer that sends frames the `transfer` way, writing shared-memory frames through
    /// `sink`.
    pub fn new(sink: S, transfer: Transfer) -> Self {
        Self { transfer, sink }
    }

    /// The medium frames go over now; it turns to zlib for good if a shared-memory write
    /// fails.
    pub fn transfer(&self) -> &Transfer {
        &self.transfer
    }

    /// The graphics bytes for `snapshot`: each screen's pixels with its virtual placement.
    /// Re-transmitting an image deletes its placements, so every frame places both again.
    /// Nothing is sent when the terminal is too small to show the screens.
    pub fn graphics(&mut self, snapshot: &Snapshot) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(areas) = snapshot.areas {
            out.extend(self.send(TOP_IMAGE_ID, &snapshot.top, areas.top));
            out.extend(self.send(BOTTOM_IMAGE_ID, &snapshot.bottom, areas.bottom));
        }
        out
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
    pub fn draw(&self, snapshot: &Snapshot, frame: &mut Frame) {
        let Some(areas) = snapshot.areas else {
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
        let note = if snapshot.px_known {
            ""
        } else {
            "\ncell size unknown, assuming 8x16"
        };
        let keys_note = if snapshot.key_release_reported {
            String::new()
        } else {
            format!("\nkey release not reported:\na press holds {FALLBACK_HOLD_FRAMES} frames")
        };
        let cursor_note = match snapshot.cursor {
            Some((x, y)) => format!("\ncursor mode: {x},{y}"),
            None => String::new(),
        };
        let pad_note = if !snapshot.pad_available {
            "\ngamepad unavailable".to_string()
        } else if snapshot.pads_connected == 0 {
            "\nno gamepad connected".to_string()
        } else {
            "\ngamepad connected".to_string()
        };
        // TODO(phase1): real register and disassembly panels (#56).
        frame.render_widget(
            Paragraph::new(format!(
                "TODO(phase1) (#56){note}{keys_note}{cursor_note}{pad_note}\n{}",
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

    /// One tick: write the graphics to `gfx`, then draw with ratatui.
    pub fn present<B: Backend, W: Write>(
        &mut self,
        snapshot: &Snapshot,
        terminal: &mut Terminal<B>,
        gfx: &mut W,
    ) -> anyhow::Result<()> {
        gfx.write_all(&self.graphics(snapshot))?;
        gfx.flush()?;
        terminal
            .draw(|f| self.draw(snapshot, f))
            .map_err(|e| anyhow!("{e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
