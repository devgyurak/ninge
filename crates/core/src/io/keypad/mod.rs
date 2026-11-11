// SPDX-License-Identifier: Apache-2.0
//! The keypad registers: KEYINPUT and KEYCNT on both CPUs, EXTKEYIN on the ARM7. Note:
//! `docs/hw/keypad.md`.
//!
//! An independent copy for each CPU (`AGENTS.md` §4, "Per-CPU devices"): each CPU has its own
//! KEYCNT. This is the "nothing pressed" stub: no key is ever down, so the key registers read
//! their released values and the keypad interrupt condition is never met.
// TODO(phase5): real input (buttons, the pen, the hinge) and the keypad IRQ arrive with the
// frontend's input handling (#44); until then nothing is pressed. The empty-mask AND
// condition, the hinge IRQ and the pen bit's link to the touch controller come with it
// (`keypad.md`, "Unverified").

use crate::io::device::{Device, Effect, Width, read_bytes, write_bytes};
use serde::{Deserialize, Serialize};

/// KEYINPUT, at `0400_0130h`; the device's register space starts here. Both CPUs.
pub(crate) const KEYINPUT_ADDR: u32 = 0x0400_0130;
/// Bytes of the ARM9's block: KEYINPUT and KEYCNT.
pub(crate) const ARM9_BYTES: u32 = 4;
/// Bytes of the ARM7's block: KEYINPUT, KEYCNT, RCNT (not modeled) and EXTKEYIN.
pub(crate) const ARM7_BYTES: u32 = 8;

/// KEYCNT's offset from KEYINPUT.
const KEYCNT_OFFSET: u32 = 2;
/// EXTKEYIN's offset from KEYINPUT (ARM7 only).
const EXTKEYIN_OFFSET: u32 = 6;

/// KEYINPUT with nothing pressed: the ten key bits set, the rest read 0 (`keypad.md`, "KEYINPUT
/// bits").
// TODO(hw-unverified): bits 10-15 read 0 here; GBATEK only says "not used" (`keypad.md`,
// "Unverified").
const KEYINPUT_RELEASED: u16 = 0x03FF;
/// EXTKEYIN with nothing pressed and the hinge open: X, Y, the debug button and the pen
/// released (bits 0, 1, 3, 6), the unknown bits 2, 4 and 5 set, the hinge (bit 7) clear
/// (`keypad.md`, "EXTKEYIN bits").
// TODO(hw-unverified): the unknown bits and the debug button (`keypad.md`, "Unverified").
const EXTKEYIN_RELEASED: u16 = 0x007F;
/// The KEYCNT bits that are kept: the ten key selects, the IRQ enable (14) and the condition
/// (15) (`keypad.md`, "KEYCNT bits").
// TODO(hw-unverified): whether bits 10-13 are writable (`keypad.md`, "Unverified").
const KEYCNT_MASK: u16 = 0xC3FF;

/// One CPU's keypad registers. The only state is KEYCNT: the key bits are constants until input
/// exists (Phase 5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keypad<const ARM9: bool> {
    keycnt: u16,
}

impl<const ARM9: bool> Keypad<ARM9> {
    /// The registers at reset: KEYCNT clear.
    // TODO(hw-unverified): GBATEK states no reset value for KEYCNT (`keypad.md`, "Unverified").
    pub fn new() -> Self {
        Self::default()
    }

    /// KEYCNT as last written, for the debugger.
    pub fn keycnt(&self) -> u16 {
        self.keycnt
    }

    /// The byte of the register space at `offset`: a hole, and on the ARM9 EXTKEYIN, read 0.
    fn byte(&self, offset: u32) -> u8 {
        let keycnt = self.keycnt.to_le_bytes();
        let released = KEYINPUT_RELEASED.to_le_bytes();
        let ext = EXTKEYIN_RELEASED.to_le_bytes();
        match offset {
            0 | 1 => released[offset as usize],
            KEYCNT_OFFSET | 3 => keycnt[(offset - KEYCNT_OFFSET) as usize],
            EXTKEYIN_OFFSET | 7 if !ARM9 => ext[(offset - EXTKEYIN_OFFSET) as usize],
            _ => 0,
        }
    }
}

impl<const ARM9: bool> Device<ARM9> for Keypad<ARM9> {
    fn peek(&self, offset: u32, width: Width, _now: u64) -> u32 {
        read_bytes(offset, width, |at| self.byte(at))
    }

    /// Each byte of the write goes to the register at its own offset; only KEYCNT keeps anything,
    /// and only its defined bits. KEYINPUT and EXTKEYIN are read-only.
    fn write(&mut self, offset: u32, width: Width, value: u32, _now: u64) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            let shift = match at {
                KEYCNT_OFFSET => Some(0),
                3 => Some(8),
                _ => None,
            };
            if let Some(shift) = shift {
                let kept = self.keycnt & !(0xFF_u16 << shift);
                self.keycnt = kept | ((u16::from(byte) << shift) & KEYCNT_MASK);
            }
            Effect::default()
        })
    }
}

#[cfg(test)]
mod tests;
