// SPDX-License-Identifier: Apache-2.0
//! The master brightness: MASTER_BRIGHT decoded, and what it does to a finished line
//! (`docs/hw/blending.md`, "Master brightness"). It works on 6-bit channels; a line whose engine has
//! it off keeps the plain 5-to-8-bit expansion, so no picture that does not use it changes.

use super::{BLUE_SHIFT, CHANNEL_MASK, GREEN_SHIFT, expand_color};

/// MASTER_BRIGHT, at offset `6Ch` of an engine's register block, a halfword in a 4-byte block of the
/// address map (`blending.md`, "Registers").
pub(super) const BRIGHT_OFFSET: u32 = 0x6C;
/// Bytes of that block in the address map.
pub(super) const BRIGHT_BLOCK_BYTES: u32 = 4;
/// MASTER_BRIGHT keeps the factor (bits 0-4) and the mode (bits 14-15).
// TODO(hw-unverified): the unused bits are cleared on write and read back 0, and the register reads
// back at all (`blending.md`, "Unverified").
pub(super) const BRIGHT_MASK: u16 = 0xC01F;
/// Bits 14-15: the mode.
const MODE_SHIFT: u32 = 14;
const MODE_MASK: u16 = 0x3;
/// The factor is in sixteenths, and a value above 16 counts as 16.
const FACTOR_ONE: u16 = 16;
/// A 6-bit channel holds up to 63.
const CHANNEL6_MAX: u16 = 63;
/// A channel of the 15-bit colour holds five bits; the unit works on six.
const CHANNEL6_MASK: u16 = 0x3F;

/// MASTER_BRIGHT bits 14-15 (`blending.md`, "MASTER_BRIGHT bits").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrightnessMode {
    /// The picture is not changed.
    Off,
    /// Every channel moves toward white.
    Up,
    /// Every channel moves toward black.
    Down,
    /// Reserved; the unit treats it as off.
    Reserved,
}

/// What an engine's MASTER_BRIGHT selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MasterBrightness {
    /// Bits 14-15.
    pub mode: BrightnessMode,
    /// Bits 0-4 in sixteenths, 0-16 (a register value above 16 reads as 16).
    pub factor: u8,
}

impl MasterBrightness {
    /// No change: what an engine whose brightness does not apply shows.
    pub(super) const OFF: Self = Self {
        mode: BrightnessMode::Off,
        factor: 0,
    };

    /// The brightness `register` holds.
    pub(super) fn decode(register: u16) -> Self {
        Self {
            mode: match (register >> MODE_SHIFT) & MODE_MASK {
                0 => BrightnessMode::Off,
                1 => BrightnessMode::Up,
                2 => BrightnessMode::Down,
                _ => BrightnessMode::Reserved,
            },
            factor: (register & CHANNEL_MASK).min(FACTOR_ONE) as u8,
        }
    }

    /// Whether the line goes through the 6-bit path: the mode is up or down, whatever the factor.
    fn is_on(self) -> bool {
        matches!(self.mode, BrightnessMode::Up | BrightnessMode::Down)
    }

    /// One 6-bit channel moved toward white or black by the factor.
    // TODO(hw-unverified): the truncation of the sixteenths (`blending.md`, "Unverified").
    fn channel(self, value: u16) -> u16 {
        let factor = u16::from(self.factor);
        match self.mode {
            BrightnessMode::Up => value + (CHANNEL6_MAX - value) * factor / FACTOR_ONE,
            BrightnessMode::Down => value - value * factor / FACTOR_ONE,
            BrightnessMode::Off | BrightnessMode::Reserved => value,
        }
    }

    /// The 15-bit colour as xRGB8888 after the brightness.
    // TODO(hw-unverified): a 5-bit channel is widened to six by repeating its top bit, and mode 3 is
    // taken as off (`blending.md`, "Unverified").
    pub(super) fn expand(self, color: u16) -> u32 {
        if !self.is_on() {
            return expand_color(color);
        }
        let host = |shift: u32| {
            let five = (color >> shift) & CHANNEL_MASK;
            let six = self.channel(((five << 1) | (five >> 4)) & CHANNEL6_MASK);
            // The 6-to-8-bit expansion is a host choice like the 5-to-8-bit one
            // (`blending.md`, "Colour depth").
            (u32::from(six) << 2) | (u32::from(six) >> 4)
        };
        (host(0) << 16) | (host(GREEN_SHIFT) << 8) | host(BLUE_SHIFT)
    }
}

#[cfg(test)]
mod tests;
