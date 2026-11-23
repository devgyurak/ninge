// SPDX-License-Identifier: Apache-2.0
//! Application state, input mapping, and one frame of output.
//!
//! # One loop, for now
//!
//! The frontend runs on a single loop: [`frame_step`] and [`poll_timeout`] run the machine, draw
//! and poll the terminal's input in turn, on one thread. AGENTS.md §2 rule 12 (and plan §4.3)
//! describes three threads (emu, render, input), a triple-buffered frame hand-off and audio-based
//! pacing; that is deferred, not dropped. Audio pacing (#47) is the first feature that needs the
//! split, so the split is its own `REF:` issue, #218, the last of Phase 5 and ahead of #47, and
//! the loop stays as it is until then. Nothing before #218 should add a thread.
//!
//! # Input (#44)
//!
//! Input mapping is a pure function from a terminal event to an [`Action`] ([`map_event`]) and
//! from the keys held to the host [`Input`] ([`Keys`]); the terminal poll is a thin adapter around
//! them. That holds whether the poll later runs on this loop or on an input thread (#218).

use crate::clock::local_wall_time;
use crate::cursor::{self, Cursor};
use crate::gamepad::{Pad, PadEvent};
use crate::input::{FALLBACK_HOLD_FRAMES, Keys};
use crate::kitty;
use crate::layout::{Areas, TermSize, compute};
use crate::pattern::dummy_frame;
use crate::pen::{Pen, Target, Unit};
use crate::placeholder::ScreenImage;
use crate::savefile::SaveFile;
use crate::source::Source;
use crate::transfer::{NoShm, ShmSink, Transfer};
use anyhow::anyhow;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use ninge_core::cart::SaveType;
use ninge_core::{Input, Machine, Screen};
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
    /// A key event for the console's keys.
    Key(KeyEvent),
    /// A mouse report: the left button is the touch pen.
    Mouse(MouseEvent),
    /// The terminal lost focus: every key lets go, since a release sent after it would never
    /// arrive.
    ReleaseAll,
}

/// Whether `event` is the press of a quit key: Ctrl-C or Ctrl-Q (raw mode delivers them as keys).
/// Neither `q` nor Esc quits: `q` is the L button, and Esc is too easy to hit in a game.
fn is_quit(event: &KeyEvent) -> bool {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let letter = |c: char| matches!(event.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c));
    event.kind == KeyEventKind::Press && ctrl && (letter('c') || letter('q'))
}

/// Maps a terminal event to an [`Action`]: Ctrl-C and Ctrl-Q quit, a resize and a loss of focus
/// have their own actions, and every other key goes to the console's keys.
pub fn map_event(event: &Event) -> Action {
    match event {
        Event::Key(key) if is_quit(key) => Action::Quit,
        Event::Key(key) => Action::Key(*key),
        Event::Mouse(mouse) => Action::Mouse(*mouse),
        Event::Resize(..) => Action::Resized,
        Event::FocusLost => Action::ReleaseAll,
        _ => Action::None,
    }
}

/// The pixels of a screen as shown: unchanged while its backlight is on, all black while it is
/// off. The core's frame buffers are not dimmed; what an unlit screen looks like is the host's
/// choice (`touch-power.md`, "What the power register does").
// TODO(hw-unverified): what an unlit screen shows (`touch-power.md`, "Unverified").
pub fn lit_or_black(pixels: Vec<u32>, backlight: bool) -> Vec<u32> {
    if backlight {
        pixels
    } else {
        vec![0; pixels.len()]
    }
}

/// Frontend state. `S` is where shared-memory frames are written.
pub struct App<S = NoShm> {
    size: TermSize,
    areas: Option<Areas>,
    frame_no: u64,
    source: Source,
    save: Option<SaveFile>,
    quit: bool,
    transfer: Transfer,
    sink: S,
    /// The host's wall-clock time, read once a frame for the machine's clock chip.
    clock: fn() -> u32,
    /// The keys held, from which each frame's input is built.
    keys: Keys,
    /// The keyboard's virtual cursor.
    cursor: Cursor,
    /// The gamepads' buttons.
    pad: Pad,
    /// Whether the gamepad library started; the panel says so when it did not.
    pad_available: bool,
    /// How many pads are plugged in, as the library last said.
    pads_connected: u32,
    /// Where the mouse holds the touch pen.
    pen: Pen,
    /// What a mouse report's column and row count.
    mouse_unit: Unit,
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
            save: None,
            quit: false,
            transfer,
            sink,
            clock: local_wall_time,
            keys: Keys::new(false),
            cursor: Cursor::new(false),
            pad: Pad::new(),
            pad_available: true,
            pads_connected: 0,
            pen: Pen::new(),
            mouse_unit: Unit::Cells,
        }
    }

    /// Tells the app whether the gamepad library started and how many pads are plugged in.
    #[must_use]
    pub fn with_gamepads(mut self, available: bool, connected: u32) -> Self {
        self.pad_available = available;
        self.pads_connected = connected;
        self
    }

    /// Records how many pads are plugged in now, for the panel.
    pub fn set_pads_connected(&mut self, connected: u32) {
        self.pads_connected = connected;
    }

    /// Takes a gamepad event.
    pub fn apply_pad(&mut self, event: PadEvent) {
        self.pad.handle(event);
    }

    /// Tells the app whether mouse reports count cells or pixels (SGR-Pixels is on).
    #[must_use]
    pub fn with_mouse_unit(mut self, unit: Unit) -> Self {
        self.mouse_unit = unit;
        self
    }

    /// Tells the app whether the terminal reports key release (the kitty keyboard protocol is on);
    /// without it a press holds its key for [`FALLBACK_HOLD_FRAMES`] frames. Resets the keys.
    #[must_use]
    pub fn with_key_release(mut self, reported: bool) -> Self {
        self.keys = Keys::new(reported);
        self.cursor = Cursor::new(reported);
        self
    }

    /// Reads the host's time from `clock` instead of the system clock, so a test controls it.
    #[must_use]
    pub fn with_clock(mut self, clock: fn() -> u32) -> Self {
        self.clock = clock;
        self
    }

    /// Takes frames from `source` instead of the dummy pattern.
    #[must_use]
    pub fn with_source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    /// Writes the machine's save memory to `save` as it changes, and at exit ([`App::finish`]).
    #[must_use]
    pub fn with_save(mut self, save: Option<SaveFile>) -> Self {
        self.save = save;
        self
    }

    /// Called when the main loop ends: writes any save memory the file does not hold yet.
    pub fn finish(&mut self) -> std::io::Result<()> {
        match (&mut self.save, &self.source) {
            (Some(save), Source::Machine(machine)) => save.finish(machine),
            _ => Ok(()),
        }
    }

    /// A line for the user once the program ends: the save chip and its file, `None` without one.
    pub fn save_note(&self) -> Option<String> {
        match (&self.save, &self.source) {
            (Some(save), Source::Machine(machine)) if machine.save_type() != SaveType::None => {
                Some(save.describe(machine))
            }
            _ => None,
        }
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
        // The bottom screen moved under the pen.
        self.pen.lift();
    }

    /// Applies an [`Action`]; `Resized` is handled by the caller through [`App::resize`].
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::Key(key) => {
                self.keys.handle(&key);
                self.cursor.handle(&key);
            }
            Action::Mouse(mouse) => {
                let bottom = self.areas.map(|areas| areas.bottom);
                match bottom {
                    Some(bottom) => self.pen.handle(
                        &mouse,
                        &Target {
                            unit: self.mouse_unit,
                            bottom,
                            cell: (self.size.cell_w, self.size.cell_h),
                        },
                    ),
                    None => self.pen.lift(),
                }
            }
            Action::ReleaseAll => {
                self.keys.release_all();
                self.cursor.release_all();
                self.pad.release_all();
                self.pen.lift();
            }
            Action::None | Action::Resized => {}
        }
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
            // The keys held make the buttons, the microphone and the lid; the host's local time
            // feeds the clock chip (`rtc.md`).
            let input: Input = self
                .keys
                .input()
                .with_buttons(self.keys.buttons() | self.pad.buttons())
                // The mouse's pen wins while its button is down; else the cursor's.
                .with_touch(self.pen.touch().or(self.cursor.touch()))
                .with_wall_time((self.clock)());
            machine.run_frame(input);
            if let Some(save) = &mut self.save {
                save.after_frame(machine);
            }
            // The program shut the console down: end the run the way a quit does, so `finish`
            // writes the save as usual. The frontend only reads what the machine reports
            // (`touch-power.md`, "What the power register does").
            self.quit |= machine.is_shut_down();
        }
        self.keys.end_frame();
        self.cursor.end_frame();
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
                lit_or_black(
                    machine.frame(Screen::Top).to_vec(),
                    machine.backlight(Screen::Top),
                ),
                self.with_cursor(lit_or_black(
                    machine.frame(Screen::Bottom).to_vec(),
                    machine.backlight(Screen::Bottom),
                )),
            ),
            Source::Pattern => (
                dummy_frame(self.frame_no, 0),
                self.with_cursor(dummy_frame(self.frame_no, 1)),
            ),
        }
    }

    /// The bottom screen's `pixels` with the crosshair over them in cursor mode.
    fn with_cursor(&self, mut pixels: Vec<u32>) -> Vec<u32> {
        if self.cursor.active() {
            cursor::overlay(&mut pixels, self.cursor.position());
        }
        pixels
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
        let keys_note = if self.keys.release_reported() {
            String::new()
        } else {
            format!("\nkey release not reported:\na press holds {FALLBACK_HOLD_FRAMES} frames")
        };
        let cursor_note = if self.cursor.active() {
            let (x, y) = self.cursor.position();
            format!("\ncursor mode: {x},{y}")
        } else {
            String::new()
        };
        let pad_note = if !self.pad_available {
            "\ngamepad unavailable".to_string()
        } else if self.pads_connected == 0 {
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
