// SPDX-License-Identifier: Apache-2.0
//! The ARM7's RTCCNT, the register whose pins the clock chip hangs on. Note: `docs/hw/rtc.md`; the
//! chip is [`crate::rtc::Chip`].
//!
//! A plain device for one CPU (`AGENTS.md` §4, "Per-CPU devices"): only the ARM7 reaches it. The
//! register is the program's hands on three wires, so the device only turns a write into the edges
//! the wires make (chip select rising or falling, the serial clock rising or falling) and hands each
//! to the chip. It has no timing and no event: the program does the waiting, and the clock the chip
//! shows is the host's time (`Cx::host`) plus the program's own offset.

use crate::io::device::{Checked, Cx, Device, Effect, StateTimes, Width, read_bytes, write_bytes};
use crate::rtc::Chip;
use serde::{Deserialize, Serialize};

/// RTCCNT, at `0400_0138h`; the device's register space starts here.
pub(crate) const RTCCNT_ADDR: u32 = 0x0400_0138;
/// Bytes of the block the device owns: RTCCNT's two bytes and the two that keep the block on the
/// 4-byte grid.
pub(crate) const BLOCK_BYTES: u32 = 4;

/// RTCCNT bit 0: the data line (SIO) level.
const SIO: u16 = 1 << 0;
/// RTCCNT bit 1: the serial clock (SCK) level.
const SCK: u16 = 1 << 1;
/// RTCCNT bit 2: the chip select (CS) level.
const CS: u16 = 1 << 2;
/// RTCCNT bit 4: 1 when the program drives SIO.
const SIO_OUT: u16 = 1 << 4;
/// RTCCNT bit 5: 1 when the program drives SCK.
const SCK_OUT: u16 = 1 << 5;
/// RTCCNT bit 6: 1 when the program drives CS.
const CS_OUT: u16 = 1 << 6;
/// The level bits of every line, the three pins and the five unused ones (bits 3 and 8-11).
const LEVELS: u16 = 0x0F0F;
/// The direction bits, each four above its level (bits 4-7 and 12-15) (`rtc.md`, "RTCCNT bits").
const DIRECTIONS: u16 = 0xF0F0;
/// How far a direction bit sits above its level bit.
const DIRECTION_SHIFT: u32 = 4;

/// RTCCNT and the clock chip behind it. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rtc {
    /// The bits the program wrote: every level and direction bit is kept.
    // TODO(hw-unverified): RTCCNT's reset value and what an undriven line reads (`rtc.md`,
    // "Unverified").
    control: u16,
    /// The clock chip.
    chip: Chip,
}

impl Rtc {
    /// The register at power-on, every line an input, with a chip at power-on.
    pub fn new() -> Self {
        Self {
            control: 0,
            chip: Chip::new(),
        }
    }

    /// A device holding `chip`, whatever it is, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn with_chip_for_test(chip: Chip) -> Self {
        Self {
            chip,
            ..Self::new()
        }
    }

    /// The clock chip, for the debugger.
    pub fn chip(&self) -> &Chip {
        &self.chip
    }

    /// RTCCNT as the ARM7 reads it: a line the program drives reads back what it wrote, and a line
    /// it reads shows the pin. The chip is the only thing that drives a pin, and only SIO.
    // TODO(hw-unverified): the unused lines (bits 3 and 8-11), their directions, and a line nothing
    // drives, which reads zero here (`rtc.md`, "Unverified").
    pub fn control(&self) -> u16 {
        let driven = (self.control & DIRECTIONS) >> DIRECTION_SHIFT;
        let pins = u16::from(self.chip.data_out());
        (self.control & DIRECTIONS) | (self.control & driven) | (pins & !driven & LEVELS)
    }

    /// Whether the program drives the line whose level bit is `level` and direction bit is
    /// `direction`, and holds it high: the level the chip sees on that pin.
    fn pin(control: u16, level: u16, direction: u16) -> bool {
        control & level != 0 && control & direction != 0
    }

    /// The byte of the register space at `offset`.
    fn byte(&self, offset: u32) -> u8 {
        match offset {
            0 | 1 => self.control().to_le_bytes()[offset as usize],
            _ => 0,
        }
    }

    /// A write of one byte at `offset`: RTCCNT changes, and the edges the pins made go to the chip.
    // TODO(phase5): RCNT's SI input, where the chip's /INT line arrives (`rtc.md`, "The interrupt").
    // TODO(hw-unverified): RCNT is not checked; GBATEK says it should be in general-purpose mode
    // before the RTC is used (`rtc.md`, "Unverified").
    fn write_byte(&mut self, offset: u32, byte: u8, host_time: u32) {
        let before = self.control;
        self.control = match offset {
            0 => (before & 0xFF00) | u16::from(byte),
            1 => (before & 0x00FF) | (u16::from(byte) << 8),
            _ => return,
        };
        let (cs_before, cs_after) = (
            Self::pin(before, CS, CS_OUT),
            Self::pin(self.control, CS, CS_OUT),
        );
        let (sck_before, sck_after) = (
            Self::pin(before, SCK, SCK_OUT),
            Self::pin(self.control, SCK, SCK_OUT),
        );
        // TODO(hw-unverified): a change of a direction bit alone moves the pin's level to or from
        // zero, so it can make an edge (`rtc.md`, "Unverified").
        // The chip select goes first: a clock edge in the same write as a deselect finds the chip
        // idle, and a rising one in the same write as a select is not taken (below).
        let selecting = !cs_before && cs_after;
        if selecting {
            self.chip.select();
        } else if cs_before && !cs_after {
            self.chip.deselect();
        }
        if sck_before && !sck_after {
            self.chip.clock_falling();
        } else if !sck_before && sck_after && !selecting {
            // TODO(hw-unverified): a clock edge in the same write as the select is not seen, so the
            // data line at that moment is not a command bit (`rtc.md`, "Unverified").
            // A rising clock in any other write:
            // The chip takes the bit the program drives as the clock rises.
            let sio = Self::pin(self.control, SIO, SIO_OUT);
            self.chip.clock_rising(sio, host_time);
        }
    }
}

impl Default for Rtc {
    fn default() -> Self {
        Self::new()
    }
}

impl Checked for Rtc {
    fn well_formed(&self, _times: StateTimes) -> bool {
        self.chip.well_formed()
    }
}

impl Device<false> for Rtc {
    // TODO(hw-unverified): the ARM9 never reaches this device: its map has no entry here, so it reads
    // 0 and drops writes (`io/map.rs`; `rtc.md`, "Unverified").
    fn peek(&self, offset: u32, width: Width, _cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.byte(at))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        // The host's time is read once, here, and not kept: the chip adds its own offset to it.
        // TODO(hw-unverified): the clock moves only when the host's time does, at frame boundaries
        // (`rtc.md`, "Unverified").
        let host_time = cx.host.wall_time;
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte(at, byte, host_time);
            Effect::default()
        })
    }
}

#[cfg(test)]
mod tests;
