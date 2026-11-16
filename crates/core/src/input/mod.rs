// SPDX-License-Identifier: Apache-2.0
//! What the host gives the machine for one frame: the buttons held, where the touch panel is
//! pressed and whether the hinge is closed (`docs/hw/keypad.md`, "Where the bits come from").
//!
//! The host builds an [`Input`] and hands it to `Machine::set_input`; the machine latches it when
//! `run_frame` begins, so one `Input` per frame is a complete input log (AGENTS.md §2 rule 6).

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// The buttons held.
    pub buttons: Buttons,
    /// Where the panel is pressed, if it is.
    // TODO(phase5): nothing reads the touch point until the touch controller and the pen bit
    // exist (#44, `keypad.md`, "What is still a stub").
    pub touch: Option<Touch>,
    /// Whether the hinge is closed (EXTKEYIN bit 7).
    pub hinge_closed: bool,
}

#[cfg(test)]
mod tests;
