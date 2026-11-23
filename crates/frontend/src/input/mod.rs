// SPDX-License-Identifier: Apache-2.0
//! The keyboard as a host input: which console buttons are held, whether the player blows into
//! the microphone, and whether the lid is closed. Pure state, no terminal: [`Keys`] takes crossterm
//! key events and gives back the buttons, so a mapping is tested without a terminal and the poll
//! can move to an input thread (#218) without touching it.
//!
//! The keymap is fixed (#44): the arrow keys are the D-pad, `X` A, `Z` B, `S` X, `A` Y, `Q` L,
//! `W` R, Enter Start, Backspace Select, `M` the microphone (blow) while held, and the grave
//! accent key toggles the lid. Letters match in either case, so Shift never drops a button.
//!
//! # Key release
//!
//! A terminal reports key release only after the kitty keyboard protocol is on (`kitty`, #44). Without
//! it a press is all the app ever sees, so the key counts as held for [`FALLBACK_HOLD_FRAMES`]
//! frames, then lets go. A key held down keeps its button only until the terminal's first repeat,
//! which usually comes 250 to 600 ms after the press, much later than the hold ends: the button
//! drops for the gap, then each repeat press renews it. That is a limit of the fixed count, not
//! of the terminal; a terminal with the keyboard protocol has no gap. The lid key toggles on the
//! press that finds it up, so holding it past the hold toggles it again at the first repeat.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ninge_core::{Buttons, Input};

/// Frames a press stays held when the terminal does not report release: 6, about 100 ms at the 60
/// Hz output rate. A game polls the keys once a frame, so a tap must outlast a frame or two to be
/// seen; much longer and a tap feels sticky, because nothing ends it early. A held key is not kept
/// held across the delay before its first repeat (see the module doc).
pub const FALLBACK_HOLD_FRAMES: u8 = 6;

/// A key the keymap gives a meaning to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
    Select,
    /// The microphone: blowing while held.
    Mic,
    /// The lid: a press toggles it.
    Lid,
}

/// Every key, in the order of [`Keys::held`].
const KEYS: [Key; 14] = [
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::A,
    Key::B,
    Key::X,
    Key::Y,
    Key::L,
    Key::R,
    Key::Start,
    Key::Select,
    Key::Mic,
    Key::Lid,
];

impl Key {
    /// The key's place in [`KEYS`].
    const fn index(self) -> usize {
        self as usize
    }

    /// The console button the key stands for, none for the microphone and the lid.
    const fn button(self) -> Buttons {
        match self {
            Self::Up => Buttons::UP,
            Self::Down => Buttons::DOWN,
            Self::Left => Buttons::LEFT,
            Self::Right => Buttons::RIGHT,
            Self::A => Buttons::A,
            Self::B => Buttons::B,
            Self::X => Buttons::X,
            Self::Y => Buttons::Y,
            Self::L => Buttons::L,
            Self::R => Buttons::R,
            Self::Start => Buttons::START,
            Self::Select => Buttons::SELECT,
            Self::Mic | Self::Lid => Buttons::empty(),
        }
    }

    /// The key `code` stands for, if the keymap has one.
    fn from_code(code: KeyCode) -> Option<Self> {
        match code {
            KeyCode::Up => Some(Self::Up),
            KeyCode::Down => Some(Self::Down),
            KeyCode::Left => Some(Self::Left),
            KeyCode::Right => Some(Self::Right),
            KeyCode::Enter => Some(Self::Start),
            KeyCode::Backspace => Some(Self::Select),
            KeyCode::Char('`') => Some(Self::Lid),
            KeyCode::Char(c) => match c.to_ascii_lowercase() {
                'x' => Some(Self::A),
                'z' => Some(Self::B),
                's' => Some(Self::X),
                'a' => Some(Self::Y),
                'q' => Some(Self::L),
                'w' => Some(Self::R),
                'm' => Some(Self::Mic),
                _ => None,
            },
            _ => None,
        }
    }
}

/// How long a key stays held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Hold {
    /// Not held.
    #[default]
    Up,
    /// Held until the terminal reports its release.
    UntilRelease,
    /// Held for this many more frames (at least 1): the terminal does not report release.
    Frames(u8),
}

/// The keys held now, and the lid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    /// Whether the terminal reports key release (the kitty keyboard protocol is on).
    release_reported: bool,
    /// The hold of each key of [`KEYS`].
    held: [Hold; KEYS.len()],
    /// Whether the lid is closed.
    hinge_closed: bool,
}

impl Hold {
    /// The hold a press starts: until release when the terminal reports it, else a fixed count.
    pub(crate) const fn pressed(release_reported: bool) -> Self {
        if release_reported {
            Self::UntilRelease
        } else {
            Self::Frames(FALLBACK_HOLD_FRAMES)
        }
    }

    /// Whether the key is held.
    pub(crate) fn is_held(self) -> bool {
        self != Self::Up
    }

    /// The hold after one frame: a fixed count runs down.
    pub(crate) const fn after_frame(self) -> Self {
        match self {
            Self::Frames(1) => Self::Up,
            Self::Frames(n) => Self::Frames(n - 1),
            other => other,
        }
    }
}

impl Keys {
    /// No key held, the lid open, on a terminal that reports key release or not.
    pub const fn new(release_reported: bool) -> Self {
        Self {
            release_reported,
            held: [Hold::Up; KEYS.len()],
            hinge_closed: false,
        }
    }

    /// Whether the terminal reports key release.
    pub const fn release_reported(&self) -> bool {
        self.release_reported
    }

    /// Takes a key event: a press holds the key (and a press of the lid key toggles the lid), a
    /// release lets it go. A repeat changes nothing; a legacy terminal sends its repeats as
    /// presses, which renew the hold. A key with Ctrl, Alt or Super is not the player's.
    pub fn handle(&mut self, event: &KeyEvent) {
        let Some(key) = Key::from_code(event.code) else {
            return;
        };
        match event.kind {
            // A release is honored whatever the modifiers are now: Ctrl may have gone down after
            // the key did, and the key must not stay held.
            KeyEventKind::Release => self.held[key.index()] = Hold::Up,
            KeyEventKind::Repeat => {}
            KeyEventKind::Press => {
                let others = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER;
                if event.modifiers.intersects(others) {
                    return;
                }
                // The lid toggles on the press that finds it up, so a key's repeats that come
                // within the hold do not flip it back and forth. A hold that has run out before
                // the first repeat (see the module doc) lets that repeat toggle it again.
                if key == Key::Lid && self.held[key.index()] == Hold::Up {
                    self.hinge_closed = !self.hinge_closed;
                }
                self.held[key.index()] = Hold::pressed(self.release_reported);
            }
        }
    }

    /// Lets every key go, as when the terminal loses focus: the release of a key held while the
    /// window changed would never arrive. The lid keeps its state.
    pub fn release_all(&mut self) {
        self.held = [Hold::Up; KEYS.len()];
    }

    /// The console buttons held.
    pub fn buttons(&self) -> Buttons {
        KEYS.iter()
            .filter(|key| self.is_held(**key))
            .fold(Buttons::empty(), |all, key| all | key.button())
    }

    /// Whether the player is blowing into the microphone.
    pub fn mic(&self) -> bool {
        self.is_held(Key::Mic)
    }

    /// Whether the lid is closed.
    pub const fn hinge_closed(&self) -> bool {
        self.hinge_closed
    }

    /// The host input for the frame about to run, without the wall time.
    pub fn input(&self) -> Input {
        Input::new()
            .with_buttons(self.buttons())
            .with_mic(self.mic())
            .with_hinge_closed(self.hinge_closed)
    }

    /// One frame has passed: a hold without a release event runs down.
    pub fn end_frame(&mut self) {
        self.held
            .iter_mut()
            .for_each(|hold| *hold = hold.after_frame());
    }

    fn is_held(&self, key: Key) -> bool {
        self.held[key.index()].is_held()
    }
}

#[cfg(test)]
mod tests;
