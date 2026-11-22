// SPDX-License-Identifier: Apache-2.0
//! What the host gives the machine for one frame: the buttons held, where the touch panel is
//! pressed, whether the hinge is closed (`docs/hw/keypad.md`, "Where the bits come from") and the
//! time of day the clock chip shows (`docs/hw/rtc.md`, "How the model gets its time").
//!
//! The host builds an [`Input`] and hands it to `Machine::run_frame`; the machine latches it as
//! the frame begins, so one `Input` per frame is a complete input log (AGENTS.md §2 rule 6).

use serde::{Deserialize, Serialize};
use std::ops::BitOr;

/// The twelve buttons as a set, one bit each in the order of KEYINPUT (A is bit 0 ... L is bit 9)
/// and then X (bit 10) and Y (bit 11) (`keypad.md`, "KEYINPUT bits" and "EXTKEYIN bits").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buttons(u16);

impl Buttons {
    /// The A button.
    pub const A: Self = Self(1 << 0);
    /// The B button.
    pub const B: Self = Self(1 << 1);
    /// The Select button.
    pub const SELECT: Self = Self(1 << 2);
    /// The Start button.
    pub const START: Self = Self(1 << 3);
    /// The D-pad's right.
    pub const RIGHT: Self = Self(1 << 4);
    /// The D-pad's left.
    pub const LEFT: Self = Self(1 << 5);
    /// The D-pad's up.
    pub const UP: Self = Self(1 << 6);
    /// The D-pad's down.
    pub const DOWN: Self = Self(1 << 7);
    /// The right shoulder button.
    pub const R: Self = Self(1 << 8);
    /// The left shoulder button.
    pub const L: Self = Self(1 << 9);
    /// The X button (ARM7 side, EXTKEYIN).
    pub const X: Self = Self(1 << 10);
    /// The Y button (ARM7 side, EXTKEYIN).
    pub const Y: Self = Self(1 << 11);

    /// The bits that name a button.
    const ALL: u16 = 0x0FFF;

    /// No button held.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The buttons whose bits are set in `bits`; a bit above Y is dropped.
    pub const fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & Self::ALL)
    }

    /// The set as bits, A at bit 0.
    pub const fn bits(self) -> u16 {
        self.0 & Self::ALL
    }

    /// Whether every button of `other` is in the set.
    pub const fn contains(self, other: Self) -> bool {
        self.bits() & other.bits() == other.bits()
    }
}

impl BitOr for Buttons {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A press on the touch panel, in pixels of the lower screen: `x` 0-255, `y` 0-191.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Touch {
    /// Column, left to right.
    pub x: u8,
    /// Row, top to bottom.
    pub y: u8,
}

/// The host's state for one frame. The default is the console untouched with the lid open.
///
/// Non-exhaustive, so a field can be added without breaking a host: outside this crate build one
/// with [`Input::new`] and the `with_*` methods, and read the fields directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Input {
    /// The buttons held.
    pub buttons: Buttons,
    /// Where the panel is pressed, if it is. The touch controller converts it, and EXTKEYIN's pen
    /// bit follows it (`touch-power.md`).
    pub touch: Option<Touch>,
    /// Whether the hinge is closed (EXTKEYIN bit 7).
    pub hinge_closed: bool,
    /// The host's wall-clock time for this frame, in whole seconds since 2000-01-01 00:00:00 in the
    /// host's local time: a plain number, with no time zone in the core. The clock chip shows this
    /// plus whatever the program set it by (`rtc.md`, "How the model gets its time"); the default
    /// is 0, the first second of 2000, so a run with no setup is deterministic. A time past 2099
    /// wraps to 2000 inside the chip.
    pub wall_time: u32,
}

impl Input {
    /// The console untouched with the lid open: the same as [`Input::default`].
    pub const fn new() -> Self {
        Self {
            buttons: Buttons::empty(),
            touch: None,
            hinge_closed: false,
            wall_time: 0,
        }
    }

    /// This input with `buttons` held.
    #[must_use]
    pub const fn with_buttons(mut self, buttons: Buttons) -> Self {
        self.buttons = buttons;
        self
    }

    /// This input with the panel pressed at `touch`, or released for `None`.
    #[must_use]
    pub const fn with_touch(mut self, touch: Option<Touch>) -> Self {
        self.touch = touch;
        self
    }

    /// This input with the hinge closed or open.
    #[must_use]
    pub const fn with_hinge_closed(mut self, hinge_closed: bool) -> Self {
        self.hinge_closed = hinge_closed;
        self
    }

    /// This input with the host's wall-clock time `wall_time`: seconds since 2000-01-01 00:00:00,
    /// local time.
    #[must_use]
    pub const fn with_wall_time(mut self, wall_time: u32) -> Self {
        self.wall_time = wall_time;
        self
    }
}

#[cfg(test)]
mod tests;
