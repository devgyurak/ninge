// SPDX-License-Identifier: Apache-2.0
//! The touch-screen controller on SPI device select 2 (`docs/hw/touch-power.md`): the control byte,
//! the eight channels, the reply a program clocks out after it, and the power-down mode that
//! decides whether the chip's pen output is enabled.
//!
//! A plain device that sees one CPU through the bus (`AGENTS.md` §4): it holds no host data. The
//! panel state arrives with each transfer, and the pen line is a pure function of the mode, so a
//! device never reaches another one (`io::spi` hands the line on).

use super::ADC_PER_PIXEL;
use crate::input::Touch;
use serde::{Deserialize, Serialize};

/// Control byte bit 7: the start bit. A byte without it is padding, not a command.
const START: u8 = 0x80;
/// Control byte bits 0-1: the power-down mode.
const MODE_MASK: u8 = 0b11;
/// Control byte bit 2: 1 = single-ended reference, 0 = differential.
const SINGLE_ENDED: u8 = 1 << 2;
/// Control byte bit 3: 1 = 8-bit conversion, 0 = 12-bit.
const EIGHT_BIT: u8 = 1 << 3;
/// Control byte bits 4-6: the channel.
const CHANNEL_SHIFT: u32 = 4;
const CHANNEL_MASK: u8 = 0b111;

/// Channel 0: temperature 0.
const CH_TEMP0: u8 = 0;
/// Channel 1: the Y position.
const CH_Y: u8 = 1;
/// Channel 2: the battery voltage, tied to ground on this console.
const CH_BATTERY: u8 = 2;
/// Channel 3: Z1, for the pressure.
const CH_Z1: u8 = 3;
/// Channel 4: Z2, for the pressure.
const CH_Z2: u8 = 4;
/// Channel 5: the X position.
const CH_X: u8 = 5;
/// Channel 6: AUX, the microphone.
const CH_AUX: u8 = 6;

/// The all-ones 12-bit value: what Y reads when the panel is released.
const FULL_SCALE: u16 = 0xFFF;
/// A 12-bit result is sent after a dummy zero bit, so it sits three bits below the top of the 16
/// bits that follow the control byte: `((hi << 8) | lo) >> 3` rebuilds it (`touch-power.md`,
/// "The reply").
const REPLY_SHIFT_12: u32 = 3;
/// An 8-bit result is sent right after the dummy bit, so it sits seven bits below the top.
const REPLY_SHIFT_8: u32 = 7;
/// The 12-bit result drops its low four bits in 8-bit mode.
const EIGHT_BIT_DROP: u32 = 4;

// TODO(hw-unverified): GBATEK gives no values for Z1 and Z2 (`touch-power.md`, "Unverified").
const Z1_TOUCHED: u16 = 0x0600;
const Z2_TOUCHED: u16 = 0x0A00;
const Z1_RELEASED: u16 = 0x0000;
const Z2_RELEASED: u16 = FULL_SCALE;
// TODO(hw-unverified): the temperature readings are ours; GBATEK's difference formula turns the
// 142 counts between them into about 297 K (`touch-power.md`, "Unverified").
const TEMP0: u16 = 0x0600;
const TEMP1: u16 = 0x068E;
// TODO(#44): the microphone brings real samples with the blow key; until then it is silent.
// TODO(hw-unverified): the silence value, the middle of the range (`touch-power.md`, "Unverified").
const AUX_SILENCE: u16 = 0x0800;

/// The touch-screen controller. Plain data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tsc {
    /// The power-down mode of the last control byte, 0-3.
    // TODO(hw-unverified): GBATEK states no power-on mode; mode 0 here (`touch-power.md`,
    // "Unverified").
    mode: u8,
    /// The last control byte, for the debugger. 0 before any command.
    command: u8,
    /// The 16 bits of reply that follow the last control byte, those not yet clocked out.
    reply: u16,
    /// How many of the two reply bytes are still on the wire: 2, 1 or 0.
    left: u8,
}

impl Tsc {
    /// The chip at power-on.
    pub const fn new() -> Self {
        Self {
            // TODO(hw-unverified): the power-on mode is 0 (`touch-power.md`, "Unverified").
            mode: 0,
            command: 0,
            reply: 0,
            left: 0,
        }
    }

    /// A chip in power-down mode `mode`, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) const fn with_mode_for_test(mode: u8) -> Self {
        Self {
            mode,
            ..Self::new()
        }
    }

    /// The power-down mode of the last control byte.
    pub const fn mode(&self) -> u8 {
        self.mode
    }

    /// The last control byte (0 before any).
    pub const fn command(&self) -> u8 {
        self.command
    }

    /// Whether the chip's `/PENIRQ` output is enabled: modes 0 and 2 (`touch-power.md`, "Power-down
    /// mode and the pen line"). EXTKEYIN bit 6 reads "pressed" only while it is.
    pub const fn pen_irq_enabled(&self) -> bool {
        self.mode & 1 == 0
    }

    /// One byte exchanged with the chip while the panel is `panel`: the byte that comes back. A
    /// control byte starts a conversion; `hold` false ends the session and with it a reply still
    /// on the wire.
    // TODO(hw-unverified): deselect drops the rest of the reply but keeps the mode; a control byte
    // before the reply is done replaces it; the byte that carries a control byte gets the old
    // reply's tail or zero (`touch-power.md`, "Unverified").
    pub fn transfer(&mut self, byte: u8, hold: bool, panel: Option<Touch>) -> u8 {
        let out = match self.left {
            2 => (self.reply >> 8) as u8,
            1 => self.reply as u8,
            _ => 0,
        };
        self.left = self.left.saturating_sub(1);
        if self.left == 0 {
            self.reply = 0;
        } else {
            // The high byte is out; only the low byte is left.
            self.reply &= 0x00FF;
        }
        // TODO(hw-unverified): a byte without the start bit is padding and starts nothing
        // (`touch-power.md`, "Unverified").
        if byte & START != 0 {
            self.command = byte;
            self.mode = byte & MODE_MASK;
            // TODO(hw-unverified): a conversion is instant (`touch-power.md`, "Unverified").
            let result = convert(byte, panel);
            // TODO(hw-unverified): the 8-bit result is the top eight bits of the 12-bit one
            // (`touch-power.md`, "Unverified").
            self.reply = if byte & EIGHT_BIT != 0 {
                (result >> EIGHT_BIT_DROP) << REPLY_SHIFT_8
            } else {
                result << REPLY_SHIFT_12
            };
            self.left = 2;
        }
        if !hold {
            self.reply = 0;
            self.left = 0;
        }
        out
    }

    /// Whether the state is one the chip can be in: a mode that is the last command's, and a reply that holds only
    /// what is still on the wire.
    pub fn well_formed(&self) -> bool {
        // The mode is the low two bits of the last control byte, and 0 before any.
        self.mode == self.command & MODE_MASK
            // The dummy bit on top and the three bits under a 12-bit result are zero.
            && self.reply & 0x8007 == 0
            && match self.left {
                0 => self.reply == 0,
                1 => self.reply >> 8 == 0,
                2 => true,
                _ => false,
            }
    }
}

impl Default for Tsc {
    fn default() -> Self {
        Self::new()
    }
}

/// The 12-bit result of the conversion `command` asks for, with the panel at `panel`.
// TODO(hw-unverified): a differential conversion of a channel that exists only single-ended reads
// zero, and X, Y, Z1 and Z2 read the same in both reference modes (`touch-power.md`,
// "Unverified").
fn convert(command: u8, panel: Option<Touch>) -> u16 {
    let channel = (command >> CHANNEL_SHIFT) & CHANNEL_MASK;
    let single = command & SINGLE_ENDED != 0;
    let pixel = |p: u8| u16::from(p) * ADC_PER_PIXEL;
    match channel {
        CH_Y => panel.map_or(FULL_SCALE, |t| pixel(t.y)),
        CH_X => panel.map_or(0, |t| pixel(t.x)),
        CH_Z1 => panel.map_or(Z1_RELEASED, |_| Z1_TOUCHED),
        CH_Z2 => panel.map_or(Z2_RELEASED, |_| Z2_TOUCHED),
        _ if !single => 0,
        CH_TEMP0 => TEMP0,
        CH_BATTERY => 0,
        CH_AUX => AUX_SILENCE,
        // Channel 7, temperature 1: the only one left of the three-bit channel.
        _ => TEMP1,
    }
}

#[cfg(test)]
mod tests;
