// SPDX-License-Identifier: Apache-2.0
//! The ARM7's power-down register, HALTCNT. Note: `docs/hw/interrupts.md` ("HALTCNT", "Halt").
//!
//! A plain device for one CPU (`AGENTS.md` §4, "Per-CPU devices"): the ARM9 halts with CP15
//! wait-for-interrupt instead. The register itself holds nothing: a write whose mode is "halt"
//! leaves a request that the machine turns into the ARM7's halted state after the access.

use crate::io::device::{Device, Effect, Width};
use serde::{Deserialize, Serialize};

/// POSTFLG, then HALTCNT, at `0400_0300h` (ARM7). The device's register space starts at POSTFLG.
pub(crate) const POSTFLG_ADDR: u32 = 0x0400_0300;
/// Bytes of the block the device owns: POSTFLG, HALTCNT and two unused bytes.
pub(crate) const BLOCK_BYTES: u32 = 4;
/// HALTCNT's offset from POSTFLG.
const HALTCNT_OFFSET: u32 = 1;
/// HALTCNT bits 6-7: the mode (`interrupts.md`, "HALTCNT bits").
const MODE_SHIFT: u32 = 6;
const MODE_MASK: u32 = 0x3;
/// Mode 2: halt until an interrupt is pending.
const MODE_HALT: u32 = 2;

/// HALTCNT of the ARM7.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    halt_requested: bool,
}

impl Power {
    /// A device with no request pending.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a write asked for a halt that the machine has not taken yet.
    pub fn halt_requested(&self) -> bool {
        self.halt_requested
    }

    /// Takes the pending halt request: true once per halting write.
    pub(crate) fn take_halt(&mut self) -> bool {
        std::mem::take(&mut self.halt_requested)
    }
}

impl Device<false> for Power {
    /// HALTCNT reads 0.
    // TODO(hw-unverified): GBATEK lists HALTCNT as readable; it reads 0 here (`interrupts.md`,
    // "Unverified").
    fn peek(&self, _offset: u32, _width: Width, _now: u64) -> u32 {
        0
    }

    /// Each byte of the write goes to the register at its own offset; only HALTCNT does anything,
    /// and only mode 2 (halt). The GBA-mode and sleep modes do nothing.
    // TODO(phase5): mode 3 (sleep) powers down most of the hardware.
    fn write(&mut self, offset: u32, width: Width, value: u32, _now: u64) -> Effect {
        let halting = (0..width.bytes()).any(|i| {
            let byte = (value >> (8 * i)) & 0xFF;
            offset.wrapping_add(i) == HALTCNT_OFFSET
                && (byte >> MODE_SHIFT) & MODE_MASK == MODE_HALT
        });
        self.halt_requested |= halting;
        Effect::default()
    }
}

#[cfg(test)]
mod tests;
