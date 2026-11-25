// SPDX-License-Identifier: Apache-2.0
//! The ARM7's power-down register, HALTCNT (`docs/hw/interrupts.md`, "HALTCNT", "Halt"), and
//! POWCNT2, the sound and Wi-Fi power register (`docs/hw/sound-registers.md`, "POWCNT2").
//!
//! A plain device for one CPU (`AGENTS.md` §4, "Per-CPU devices"): the ARM9 halts with CP15
//! wait-for-interrupt instead. The register itself holds nothing: a write whose mode is "halt"
//! leaves a request that the machine turns into the ARM7's halted state after the access.

use crate::io::device::{Checked, Cx, Device, Effect, StateTimes, Width, read_bytes};
use serde::{Deserialize, Serialize};

/// POSTFLG, then HALTCNT, at `0400_0300h` (ARM7). The device's register space starts at POSTFLG.
pub(crate) const POSTFLG_ADDR: u32 = 0x0400_0300;
/// Bytes of the block the device owns: POSTFLG, HALTCNT, two unused bytes and POWCNT2 (`0400_0304h`,
/// two bytes) with its two unused bytes.
pub(crate) const BLOCK_BYTES: u32 = 8;
/// POWCNT2's offset from POSTFLG.
const POWCNT2_OFFSET: u32 = 4;
/// POWCNT2 bits 0 (speakers) and 1 (Wi-Fi); bits 2-31 are not used.
pub const POWCNT2_MASK: u16 = 0b11;
/// POWCNT2 bit 0: the internal speakers (`spu.md`, "The gates").
const POWCNT2_SPEAKERS: u16 = 1 << 0;
/// POWCNT2 at power-on: the speakers on, Wi-Fi off (GBATEK's initial setting).
// TODO(hw-unverified): the value after direct boot (`sound-registers.md`, "Unverified").
const POWCNT2_START: u16 = 0b01;
/// HALTCNT's offset from POSTFLG.
const HALTCNT_OFFSET: u32 = 1;
/// HALTCNT bits 6-7: the mode (`interrupts.md`, "HALTCNT bits").
const MODE_SHIFT: u32 = 6;
const MODE_MASK: u32 = 0x3;
/// Mode 2: halt until an interrupt is pending.
const MODE_HALT: u32 = 2;

/// HALTCNT of the ARM7.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    /// POWCNT2: stored and read back. The speaker bit gates the sound output (`spu.md`, "The
    /// gates"); Wi-Fi is not modeled.
    powcnt2: u16,
}

impl Default for Power {
    fn default() -> Self {
        Self::new()
    }
}

impl Power {
    /// A device with POWCNT2 at its initial setting.
    pub fn new() -> Self {
        Self {
            powcnt2: POWCNT2_START,
        }
    }

    /// POWCNT2 bit 0: the internal speakers are on (`spu.md`, "The gates").
    pub fn speakers_on(&self) -> bool {
        self.powcnt2 & POWCNT2_SPEAKERS != 0
    }

    /// POWCNT2 as stored.
    pub fn powcnt2(&self) -> u16 {
        self.powcnt2
    }
}

impl Checked for Power {
    /// POWCNT2 holds only its two bits.
    fn well_formed(&self, _times: StateTimes) -> bool {
        self.powcnt2 & !POWCNT2_MASK == 0
    }
}

impl Device<false> for Power {
    /// HALTCNT reads 0, and so do the unused bytes; POWCNT2 reads back.
    // TODO(hw-unverified): GBATEK lists HALTCNT as readable; it reads 0 here, and POSTFLG (offset
    // 0) is not modeled: it reads 0 and drops writes (`interrupts.md`, "Unverified").
    fn peek(&self, offset: u32, width: Width, _cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| match at {
            POWCNT2_OFFSET => self.powcnt2 as u8,
            _ => 0,
        })
    }

    /// Each byte of the write goes to the register at its own offset; only HALTCNT does anything,
    /// and only mode 2 (halt). The GBA-mode and sleep modes do nothing.
    // TODO(phase5): mode 3 (sleep) powers down most of the hardware.
    fn write(&mut self, offset: u32, width: Width, value: u32, _cx: Cx<'_>) -> Effect {
        let halting = (0..width.bytes()).any(|i| {
            let byte = (value >> (8 * i)) & 0xFF;
            offset.wrapping_add(i) == HALTCNT_OFFSET
                && (byte >> MODE_SHIFT) & MODE_MASK == MODE_HALT
        });
        // Only POWCNT2's low byte holds bits; its high byte is unused.
        (0..width.bytes())
            .filter(|&i| offset.wrapping_add(i) == POWCNT2_OFFSET)
            .for_each(|i| self.powcnt2 = ((value >> (8 * i)) as u16) & POWCNT2_MASK);
        Effect {
            halt: halting,
            ..Effect::default()
        }
    }
}

#[cfg(test)]
mod tests;
