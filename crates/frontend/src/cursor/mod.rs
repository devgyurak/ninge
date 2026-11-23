// SPDX-License-Identifier: Apache-2.0
//! The keyboard's virtual cursor: a crosshair on the bottom screen that a player without a mouse
//! moves with `h` `j` `k` `l` and presses with Space, as a stylus. Pure state and arithmetic; the
//! terminal never appears here (#44).
//!
//! `Tab` turns cursor mode on and off. In the mode `h` `j` `k` `l` move the cursor by [`STEP`]
//! panel pixels, Space held puts the pen down at the cursor, and `d` toggles a drag: the pen stays
//! down while the cursor moves, until it is toggled off. Outside the mode none of these keys does
//! anything, and the game's keys work in both. The mouse's pen wins while its button is down
//! (`App::graphics`).
//!
//! The crosshair is drawn by the frontend over a copy of the bottom screen's pixels, with inverted
//! colors so it shows on any background ([`overlay`]); the core's frame buffers are never touched.

use crate::input::Hold;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH, Touch};

/// Panel pixels the cursor moves by on one key press (or repeat): coarse enough to cross the
/// screen in a few seconds of key repeat, fine enough to hit a small target in a few presses.
pub const STEP: u8 = 4;

/// Pixels an arm of the crosshair reaches beyond its center.
const ARM: usize = 3;

/// The cursor's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// Whether cursor mode is on.
    active: bool,
    /// Where the cursor is, in panel pixels.
    x: u8,
    y: u8,
    /// Whether a drag is toggled on: the pen stays down.
    drag: bool,
    /// The Space key.
    space: Hold,
    /// The drag-toggle key, held, so a legacy terminal's repeat presses toggle once.
    drag_key: Hold,
    /// The mode key, held, for the same reason.
    mode_key: Hold,
    /// Whether the terminal reports key release.
    release_reported: bool,
}

impl Cursor {
    /// Cursor mode off, the cursor at the center of the panel.
    pub const fn new(release_reported: bool) -> Self {
        Self {
            active: false,
            x: (SCREEN_WIDTH / 2) as u8,
            y: (SCREEN_HEIGHT / 2) as u8,
            drag: false,
            space: Hold::Up,
            drag_key: Hold::Up,
            mode_key: Hold::Up,
            release_reported,
        }
    }

    /// Whether cursor mode is on.
    pub const fn active(&self) -> bool {
        self.active
    }

    /// Where the cursor is, in panel pixels.
    pub const fn position(&self) -> (u8, u8) {
        (self.x, self.y)
    }

    /// Takes a key event. Tab toggles the mode; in the mode `h` `j` `k` `l` move (a repeat moves
    /// again), Space holds the pen down while it is held and `d` toggles the drag. Keys with Ctrl,
    /// Alt or Super are not the cursor's.
    pub fn handle(&mut self, event: &KeyEvent) {
        let key = match event.code {
            KeyCode::Tab => CursorKey::Mode,
            KeyCode::Char(' ') => CursorKey::Space,
            KeyCode::Char(c) => match c.to_ascii_lowercase() {
                'h' => CursorKey::Move(-1, 0),
                'j' => CursorKey::Move(0, 1),
                'k' => CursorKey::Move(0, -1),
                'l' => CursorKey::Move(1, 0),
                'd' => CursorKey::Drag,
                _ => return,
            },
            _ => return,
        };
        if event.kind == KeyEventKind::Release {
            // Honored whatever the modifiers are now, so a key held before Ctrl is not stuck.
            match key {
                CursorKey::Space => self.space = Hold::Up,
                CursorKey::Drag => self.drag_key = Hold::Up,
                CursorKey::Mode => self.mode_key = Hold::Up,
                CursorKey::Move(..) => {}
            }
            return;
        }
        let others = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER;
        if event.modifiers.intersects(others) {
            return;
        }
        let press = event.kind == KeyEventKind::Press;
        match key {
            CursorKey::Mode if press && !self.mode_key.is_held() => {
                self.mode_key = Hold::pressed(self.release_reported);
                self.active = !self.active;
                if !self.active {
                    // Leaving the mode puts the pen up.
                    self.drag = false;
                    self.space = Hold::Up;
                }
            }
            CursorKey::Mode if press => self.mode_key = Hold::pressed(self.release_reported),
            CursorKey::Space if press && self.active => {
                self.space = Hold::pressed(self.release_reported);
            }
            CursorKey::Drag if press && self.active => {
                if !self.drag_key.is_held() {
                    self.drag = !self.drag;
                }
                self.drag_key = Hold::pressed(self.release_reported);
            }
            CursorKey::Move(dx, dy) if self.active => {
                self.x = step(self.x, dx, SCREEN_WIDTH);
                self.y = step(self.y, dy, SCREEN_HEIGHT);
            }
            _ => {}
        }
    }

    /// Lets the keys go and lifts a toggled drag, as when the terminal loses focus. The mode and
    /// the position stay.
    pub fn release_all(&mut self) {
        self.space = Hold::Up;
        self.drag_key = Hold::Up;
        self.mode_key = Hold::Up;
        self.drag = false;
    }

    /// One frame has passed: holds without a release event run down.
    pub fn end_frame(&mut self) {
        self.space = self.space.after_frame();
        self.drag_key = self.drag_key.after_frame();
        self.mode_key = self.mode_key.after_frame();
    }

    /// Where the cursor touches the panel: in the mode, while Space is held or a drag is on.
    pub fn touch(&self) -> Option<Touch> {
        (self.active && (self.space.is_held() || self.drag)).then_some(Touch {
            x: self.x,
            y: self.y,
        })
    }
}

/// What a key means to the cursor.
#[derive(Clone, Copy)]
enum CursorKey {
    Mode,
    Space,
    Drag,
    Move(i8, i8),
}

/// `at` moved by [`STEP`] times `delta`, kept inside 0..`len`.
fn step(at: u8, delta: i8, len: usize) -> u8 {
    let moved = i32::from(at) + i32::from(delta) * i32::from(STEP);
    moved.clamp(0, len as i32 - 1) as u8
}

/// Draws the crosshair at (`x`, `y`) over `pixels`, the bottom screen's `SCREEN_WIDTH` x
/// `SCREEN_HEIGHT` xRGB pixels: a plus of arms [`ARM`] pixels long, each pixel inverted so it shows
/// on any background. The part past an edge is clipped.
pub fn overlay(pixels: &mut [u32], (x, y): (u8, u8)) {
    let (x, y) = (usize::from(x), usize::from(y));
    let mut invert = |px: usize, py: usize| {
        if let Some(p) = pixels
            .get_mut(py * SCREEN_WIDTH + px)
            .filter(|_| px < SCREEN_WIDTH)
        {
            *p ^= 0x00FF_FFFF;
        }
    };
    for px in x.saturating_sub(ARM)..=x + ARM {
        invert(px, y);
    }
    for py in y.saturating_sub(ARM)..=y + ARM {
        // The center was inverted by the horizontal arm.
        if py != y {
            invert(x, py);
        }
    }
}

#[cfg(test)]
mod tests;
