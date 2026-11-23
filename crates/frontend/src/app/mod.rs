// SPDX-License-Identifier: Apache-2.0
//! Application state, input mapping, and one frame of output.
//!
//! # Three threads (#218)
//!
//! The frontend runs three threads (AGENTS.md §2 rule 12, plan §4.3), started by [`crate::term`]
//! and written as plain loops in [`crate::threads`]:
//!
//! - **emu** owns the [`App`]. At the start of each frame it applies every queued host event, runs
//!   [`App::step`], fills a [`Snapshot`] with [`App::fill`] and publishes it through the
//!   [`crate::handoff`] triple buffer, then waits as a [`crate::pacing::Pacer`] says. Audio pacing
//!   (#47) replaces the fixed-rate pacer without touching the loop.
//! - **render** owns the [`crate::render::Renderer`], the terminal and the graphics writer. It
//!   shows the newest published snapshot. If it is slower than the emu thread, a newer snapshot
//!   replaces one it never took: emulated frames are unchanged and only displayed frames are
//!   skipped.
//! - **input** owns the terminal's event reader and the gamepads, and sends the emu thread
//!   [`crate::threads::HostEvent`]s over a channel.
//!
//! The model and the renderer share nothing but the snapshot.
//!
//! # Input (#44)
//!
//! Input mapping is a pure function from a terminal event to an [`Action`] ([`map_event`]) and
//! from the keys held to the host [`Input`] ([`Keys`]). The input thread only maps and forwards
//! [`Action`]s; the emu thread applies them, because the keys' frame counts, the cursor and the
//! pen mapping count emu frames or need the layout.

use crate::clock::local_wall_time;
use crate::cursor::{self, Cursor};
use crate::gamepad::{Pad, PadEvent};
use crate::input::Keys;
use crate::layout::{Areas, TermSize, compute};
use crate::pattern::dummy_frame;
use crate::pen::{Pen, Target, Unit};
use crate::render::Snapshot;
use crate::savefile::SaveFile;
use crate::source::Source;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use ninge_core::cart::SaveType;
use ninge_core::{Input, Machine, Screen};

/// What an input event asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do.
    None,
    /// Leave the program.
    Quit,
    /// The terminal changed size; the input thread re-queries it and sends the new size, which
    /// the emu thread gives to [`App::resize`].
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

/// Fills `dst` with the pixels of a screen as shown: `src` unchanged while its backlight is on,
/// all black while it is off. The core's frame buffers are not dimmed; what an unlit screen looks
/// like is the host's choice (`touch-power.md`, "What the power register does"). `dst`'s buffer is
/// reused.
// TODO(hw-unverified): what an unlit screen shows (`touch-power.md`, "Unverified").
pub fn show_pixels(dst: &mut Vec<u32>, src: &[u32], backlight: bool) {
    dst.clear();
    if backlight {
        dst.extend_from_slice(src);
    } else {
        dst.resize(src.len(), 0);
    }
}

/// Frontend state: everything the emulation side owns, with no terminal in it.
pub struct App {
    size: TermSize,
    areas: Option<Areas>,
    frame_no: u64,
    source: Source,
    save: Option<SaveFile>,
    quit: bool,
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

impl App {
    /// A new app for a terminal of `size`.
    pub fn new(size: TermSize) -> Self {
        Self {
            size,
            areas: compute(size),
            frame_no: 0,
            source: Source::Pattern,
            save: None,
            quit: false,
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
    /// without it a press holds its key for [`crate::input::FALLBACK_HOLD_FRAMES`] frames. Resets
    /// the keys.
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

    /// Runs one frame: with a machine, one of its frames, even when the terminal is too small to
    /// show it; then the held keys age by a frame and the frame counter advances.
    pub fn step(&mut self) {
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
        self.frame_no += 1;
    }

    /// Fills `snapshot` with what the renderer needs for the frame [`App::step`] just ran. The
    /// pixels are filled only while the screens have somewhere to go, and `snapshot`'s buffers are
    /// reused.
    pub fn fill(&self, snapshot: &mut Snapshot) {
        snapshot.areas = self.areas;
        snapshot.px_known = self.size.px_known;
        snapshot.key_release_reported = self.keys.release_reported();
        snapshot.cursor = self.cursor.active().then(|| self.cursor.position());
        snapshot.pad_available = self.pad_available;
        snapshot.pads_connected = self.pads_connected;
        if self.areas.is_some() {
            self.fill_screens(snapshot);
        } else {
            snapshot.top.clear();
            snapshot.bottom.clear();
        }
    }

    /// The pixels of both screens for the frame just run.
    fn fill_screens(&self, snapshot: &mut Snapshot) {
        match &self.source {
            Source::Machine(machine) => {
                show_pixels(
                    &mut snapshot.top,
                    machine.frame(Screen::Top),
                    machine.backlight(Screen::Top),
                );
                show_pixels(
                    &mut snapshot.bottom,
                    machine.frame(Screen::Bottom),
                    machine.backlight(Screen::Bottom),
                );
            }
            Source::Pattern => {
                // `frame_no` already counts the frame just run; the pattern starts at phase 0.
                let phase = self.frame_no.saturating_sub(1);
                snapshot.top = dummy_frame(phase, 0);
                snapshot.bottom = dummy_frame(phase, 1);
            }
        }
        if self.cursor.active() {
            cursor::overlay(&mut snapshot.bottom, self.cursor.position());
        }
    }
}

#[cfg(test)]
mod tests;
