// SPDX-License-Identifier: Apache-2.0
//! The power-management device on SPI device select 0 (`docs/hw/touch-power.md`): an index byte,
//! then a data byte, over registers 0-3. The original console's chip, so register 4 and above
//! mirror 0-3.
//!
//! The device stores what a program writes and reads it back. The shutdown bit and the two
//! backlight bits are reported to the host through accessors (`touch-power.md`, "What the power
//! register does"); the amplifier bits are stored only. The bits are visible to the debugger.

use serde::{Deserialize, Serialize};

/// Index byte bit 7: 1 = read, 0 = write.
const READ: u8 = 1 << 7;
/// Index byte bits 0-6 select the register; on the original console only bits 0-1 matter, since
/// 4-7Fh mirror 0-3.
const REGISTER_MASK: u8 = 0b11;

/// Register 0, power-management control: bits 0-6; bit 7 is not used and reads zero.
// TODO(hw-unverified): which bits of register 0 a real console stores; GBATEK lists 0-6
// (`touch-power.md`, "Unverified").
const CONTROL_MASK: u8 = 0x7F;
/// Register 0 as a program finds it after direct boot: amplifier, lower and upper backlight on.
const START_CONTROL: u8 = CONTROL_AMP | CONTROL_LOWER_BACKLIGHT | CONTROL_UPPER_BACKLIGHT;
/// Register 2, microphone amplifier: bit 0.
const AMP_MASK: u8 = 0b1;
/// Register 3, microphone gain: bits 0-1.
const GAIN_MASK: u8 = 0b11;

/// Register 0 bit 6: the program asks the console to shut down. Once set it stays set, and the
/// machine runs no more frames (`touch-power.md`, "What the power register does").
// TODO(hw-unverified): the bit is sticky and the rest of its frame runs (`touch-power.md`,
// "Unverified").
pub const CONTROL_SHUTDOWN: u8 = 1 << 6;
/// Register 0 bit 0: sound amplifier enable. With it clear the sound is silent (`spu.md`, "The
/// gates").
// TODO(hw-unverified): the original DS is "very silent, but still audible" with it clear; here it
// is silent (`spu.md`, "Unverified").
pub const CONTROL_AMP: u8 = 1 << 0;
/// Register 0 bit 1: sound amplifier mute (original DS); with the amplifier on, muted is silent.
pub const CONTROL_AMP_MUTE: u8 = 1 << 1;
/// Register 0 bit 2: lower backlight; 0 = off, 1 = on. The host draws an unlit screen black.
pub const CONTROL_LOWER_BACKLIGHT: u8 = 1 << 2;
/// Register 0 bit 3: upper backlight; 0 = off, 1 = on.
pub const CONTROL_UPPER_BACKLIGHT: u8 = 1 << 3;

/// The power-management device. Plain data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerManagement {
    /// Register 0: the power-management control bits.
    control: u8,
    /// Register 2: the microphone amplifier enable.
    amp: u8,
    /// Register 3: the microphone gain.
    gain: u8,
    /// The index byte of the session in progress: the data byte is next. `None` between sessions.
    index: Option<u8>,
}

impl PowerManagement {
    /// The device as a program finds it after direct boot: the sound amplifier and both backlights
    /// on in register 0, the microphone amplifier off, gain 0 (`direct-boot.md`, "Devices at
    /// entry").
    // TODO(hw-unverified): register 0's start value is ours, and GBATEK gives none for registers 2
    // and 3 beyond "the microphone amplifier is off after power-up" (`touch-power.md`,
    // "Unverified").
    pub const fn new() -> Self {
        Self {
            control: START_CONTROL,
            amp: 0,
            gain: 0,
            index: None,
        }
    }

    /// A device whose register 0 holds `control`, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) const fn with_control_for_test(control: u8) -> Self {
        Self {
            control,
            ..Self::new()
        }
    }

    /// Register 0 as stored.
    pub const fn control(&self) -> u8 {
        self.control
    }

    /// Whether the sound amplifier lets the sound through: enabled and not muted (`spu.md`, "The
    /// gates").
    pub const fn amplifier_audible(&self) -> bool {
        self.control & CONTROL_AMP != 0 && self.control & CONTROL_AMP_MUTE == 0
    }

    /// Whether the program asked the console to shut down: register 0 bit 6, which no later write
    /// clears.
    pub const fn is_shut_down(&self) -> bool {
        self.control & CONTROL_SHUTDOWN != 0
    }

    /// Whether the lower screen's backlight is on (register 0 bit 2).
    pub const fn lower_backlight(&self) -> bool {
        self.control & CONTROL_LOWER_BACKLIGHT != 0
    }

    /// Whether the upper screen's backlight is on (register 0 bit 3).
    pub const fn upper_backlight(&self) -> bool {
        self.control & CONTROL_UPPER_BACKLIGHT != 0
    }

    /// Register 2 as stored.
    pub const fn amp(&self) -> u8 {
        self.amp
    }

    /// Register 3 as stored.
    pub const fn gain(&self) -> u8 {
        self.gain
    }

    /// What a read of register `index` gives, mirrors included.
    pub const fn register(&self, index: u8) -> u8 {
        match index & REGISTER_MASK {
            0 => self.control,
            // TODO(hw-unverified): the battery reads "power good", bit 0 clear, always
            // (`touch-power.md`, "Unverified").
            1 => 0,
            2 => self.amp,
            _ => self.gain,
        }
    }

    /// One byte exchanged with the device: the byte that comes back. The first byte of a session is
    /// the index; the next is the data, written or read as the index says. `hold` false ends the
    /// session.
    // TODO(hw-unverified): the index byte's own reply and a write's reply are zero, and every
    // further byte of a session repeats the access (`touch-power.md`, "Unverified").
    pub fn transfer(&mut self, byte: u8, hold: bool) -> u8 {
        let out = match self.index {
            None => {
                self.index = Some(byte);
                0
            }
            Some(index) if index & READ != 0 => self.register(index),
            Some(index) => {
                self.write(index, byte);
                0
            }
        };
        if !hold {
            self.index = None;
        }
        out
    }

    /// A write of `value` to the register `index` selects. The battery register is read-only.
    // TODO(hw-unverified): a write to the read-only battery register is dropped (`touch-power.md`,
    // "Unverified").
    fn write(&mut self, index: u8, value: u8) {
        match index & REGISTER_MASK {
            // TODO(hw-unverified): the bit is sticky (`touch-power.md`, "Unverified").
            // Nothing runs after the power is gone to clear the shutdown bit, so a write keeps it.
            0 => self.control = (value & CONTROL_MASK) | (self.control & CONTROL_SHUTDOWN),
            2 => self.amp = value & AMP_MASK,
            3 => self.gain = value & GAIN_MASK,
            _ => {}
        }
    }

    /// Whether the state is one the device can be in: only the bits each register keeps are set.
    pub const fn well_formed(&self) -> bool {
        self.control & !CONTROL_MASK == 0
            && self.amp & !AMP_MASK == 0
            && self.gain & !GAIN_MASK == 0
    }
}

impl Default for PowerManagement {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
