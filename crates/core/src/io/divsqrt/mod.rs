// SPDX-License-Identifier: Apache-2.0
//! The ARM9's hardware divider and square-root unit. Note: `docs/hw/divsqrt.md`.
//!
//! A plain device for one CPU (`AGENTS.md` §4, "Per-CPU devices"): the ARM7 has no such registers.
//! An operation's result is computed when it starts and is visible at once; only the busy bit
//! follows the duration, and it is a function of the time of the read, so the unit has no
//! scheduler event (`divsqrt.md`, "Modeling choice").

use crate::io::device::{Device, Effect, Width, read_bytes};
use crate::scheduler::SLICE;
use serde::{Deserialize, Serialize};

/// DIVCNT, at `0400_0280h`; the device's register space starts here and runs to SQRT_PARAM's last
/// byte.
pub(crate) const DIVCNT_ADDR: u32 = 0x0400_0280;
/// Bytes of the block: DIVCNT to SQRT_PARAM, with the holes between them.
pub(crate) const BLOCK_BYTES: u32 = 0x40;

// Offsets from DIVCNT: each register's first byte, and where the next one starts.
const NUMER: u32 = 0x10;
const DENOM: u32 = 0x18;
const RESULT: u32 = 0x20;
const REMAINDER: u32 = 0x28;
const SQRTCNT: u32 = 0x30;
const SQRT_RESULT: u32 = 0x34;
const SQRT_PARAM: u32 = 0x38;
const END: u32 = 0x40;

/// DIVCNT bits 0-1: the mode (`divsqrt.md`, "DIVCNT bits").
const DIV_MODE_MASK: u8 = 0x3;
/// DIVCNT bit 14: the zero-denominator flag.
const DIV0_BIT: u32 = 1 << 14;
/// DIVCNT and SQRTCNT bit 15: busy.
const BUSY_BIT: u32 = 1 << 15;
/// SQRTCNT bit 0: 64-bit input.
const SQRT_64_BIT: u8 = 1;

/// One bus cycle in the master clock's unit, ARM9 cycles (`scheduler.md`, "One master clock"):
/// GBATEK counts the units' durations in 33.51 MHz units.
const BUS_CYCLE: u64 = 2;
/// Bus cycles a division takes in mode 0, and in modes 1 and 2 (`divsqrt.md`, "Duration").
const DIV_32_CYCLES: u64 = 18;
const DIV_64_CYCLES: u64 = 34;
/// Bus cycles a square root takes in either mode.
const SQRT_CYCLES: u64 = 13;

/// The most master cycles an operation can take: the longest of the two units.
const LONGEST: u64 = DIV_64_CYCLES * BUS_CYCLE;

/// The divider and the square-root unit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivSqrt {
    div_mode: u8,
    numer: u64,
    denom: u64,
    quotient: u64,
    remainder: u64,
    div0: bool,
    div_busy_until: u64,
    sqrt_mode: u8,
    sqrt_param: u64,
    sqrt_result: u32,
    sqrt_busy_until: u64,
}

// TODO(hw-unverified): 0 / 0 takes the non-negative numerator's quotient, -1 (`divsqrt.md`,
// "Unverified").
/// Signed division with the hardware's two overflows: by zero the remainder is the numerator and
/// the quotient is +1 for a negative numerator and -1 otherwise; the most negative number over -1
/// wraps to itself with remainder 0 (`divsqrt.md`, "Overflows").
fn signed_divide(numer: i64, denom: i64) -> (i64, i64) {
    if denom == 0 {
        (if numer < 0 { 1 } else { -1 }, numer)
    } else {
        (numer.wrapping_div(denom), numer.wrapping_rem(denom))
    }
}

/// A division's quotient and remainder, as the registers hold them (`divsqrt.md`, "Behavior").
/// `mode` is DIVCNT bits 0-1; mode 3 divides as mode 1.
// TODO(hw-unverified): a denominator with a zero low half and a non-zero upper half divides by
// zero here, in modes 0 and 1 (`divsqrt.md`, "Unverified").
fn divide(mode: u8, numer: u64, denom: u64) -> (u64, u64) {
    match mode & DIV_MODE_MASK {
        0 => {
            let (n, d) = (
                i64::from(numer as u32 as i32),
                i64::from(denom as u32 as i32),
            );
            let (quotient, remainder) = signed_divide(n, d);
            // The result is 32 bits wide: `-MAX / -1` does not fit and wraps to `-MAX`.
            let quotient = i64::from(quotient as i32) as u64;
            // On either overflow the upper half of the sign-extended quotient is inverted.
            let overflow = d == 0 || (n == i64::from(i32::MIN) && d == -1);
            let flip = if overflow { 0xFFFF_FFFF_0000_0000 } else { 0 };
            (quotient ^ flip, remainder as u64)
        }
        2 => {
            let (quotient, remainder) = signed_divide(numer as i64, denom as i64);
            (quotient as u64, remainder as u64)
        }
        _ => {
            let (quotient, remainder) = signed_divide(numer as i64, i64::from(denom as u32 as i32));
            // The remainder is 32 bits wide, sign-extended.
            // TODO(hw-unverified): for a zero denominator it is the numerator's low 32 bits
            // (`divsqrt.md`, "Unverified").
            (quotient as u64, i64::from(remainder as i32) as u64)
        }
    }
}

/// The integer square root of the parameter: its low 32 bits in mode 0, all 64 in mode 1.
// TODO(hw-unverified): the root is rounded down; GBATEK states only the types (`divsqrt.md`,
// "Unverified").
fn square_root(mode: u8, param: u64) -> u32 {
    let value = if mode & SQRT_64_BIT == 0 {
        u64::from(param as u32)
    } else {
        param
    };
    // The root of a 64-bit value fits in 32 bits.
    value.isqrt() as u32
}

impl DivSqrt {
    /// The units in their reset state: every register zero and nothing busy.
    // TODO(hw-unverified): the reset values are all zero (`divsqrt.md`, "Unverified").
    pub fn new() -> Self {
        Self::default()
    }

    /// DIVCNT as a read at `now` sees it: the mode bits, the zero flag, and the busy bit.
    // TODO(hw-unverified): the mode bits read back as written, mode 3 included, and the unused
    // bits read 0 (`divsqrt.md`, "Unverified").
    pub fn divcnt(&self, now: u64) -> u32 {
        u32::from(self.div_mode)
            | if self.div0 { DIV0_BIT } else { 0 }
            | if now < self.div_busy_until {
                BUSY_BIT
            } else {
                0
            }
    }

    /// SQRTCNT as a read at `now` sees it.
    // TODO(hw-unverified): the unused bits read 0 (`divsqrt.md`, "Unverified").
    pub fn sqrtcnt(&self, now: u64) -> u32 {
        u32::from(self.sqrt_mode)
            | if now < self.sqrt_busy_until {
                BUSY_BIT
            } else {
                0
            }
    }

    /// The last division's quotient and remainder, for the debugger.
    pub fn division_result(&self) -> (u64, u64) {
        (self.quotient, self.remainder)
    }

    /// The last square root, for the debugger.
    pub fn sqrt_result(&self) -> u32 {
        self.sqrt_result
    }

    /// Whether the registers hold values the hardware can: an end time further ahead than the
    /// longest operation (and one slice, the ARM9's lead over the clock) is corrupt
    /// (`savestate`).
    pub(crate) fn is_well_formed(&self, now: u64) -> bool {
        let limit = now.saturating_add(LONGEST + SLICE);
        self.div_mode <= DIV_MODE_MASK
            && self.sqrt_mode <= SQRT_64_BIT
            && self.div_busy_until <= limit
            && self.sqrt_busy_until <= limit
    }

    /// Starts a division on the registers as they are, ending after the mode's duration.
    // TODO(hw-unverified): a start while busy, or one that changes nothing, restarts the count
    // (`divsqrt.md`, "Unverified").
    fn start_division(&mut self, now: u64) {
        (self.quotient, self.remainder) = divide(self.div_mode, self.numer, self.denom);
        // The flag looks at the full 64-bit denominator, even in mode 0.
        self.div0 = self.denom == 0;
        let cycles = if self.div_mode & DIV_MODE_MASK == 0 {
            DIV_32_CYCLES
        } else {
            DIV_64_CYCLES
        };
        self.div_busy_until = now.saturating_add(cycles * BUS_CYCLE);
    }

    // TODO(hw-unverified): a start while busy restarts the count (`divsqrt.md`, "Unverified").
    fn start_sqrt(&mut self, now: u64) {
        self.sqrt_result = square_root(self.sqrt_mode, self.sqrt_param);
        self.sqrt_busy_until = now.saturating_add(SQRT_CYCLES * BUS_CYCLE);
    }

    /// The byte of the register space at `offset`; a hole reads 0.
    // TODO(hw-unverified): a result register read while busy returns the finished value
    // (`divsqrt.md`, "Unverified").
    fn byte(&self, offset: u32, now: u64) -> u8 {
        let le = |value: u64, base: u32| value.to_le_bytes()[(offset - base) as usize];
        match offset {
            0..=1 => le(u64::from(self.divcnt(now)), 0),
            NUMER..DENOM => le(self.numer, NUMER),
            DENOM..RESULT => le(self.denom, DENOM),
            RESULT..REMAINDER => le(self.quotient, RESULT),
            REMAINDER..SQRTCNT => le(self.remainder, REMAINDER),
            SQRTCNT..=0x31 => le(u64::from(self.sqrtcnt(now)), SQRTCNT),
            SQRT_RESULT..SQRT_PARAM => le(u64::from(self.sqrt_result), SQRT_RESULT),
            SQRT_PARAM..END => le(self.sqrt_param, SQRT_PARAM),
            _ => 0,
        }
    }
}

/// Replaces the byte of `value` at `index` with `byte`.
fn with_byte(value: u64, index: u32, byte: u8) -> u64 {
    let shift = 8 * index;
    (value & !(0xFF << shift)) | (u64::from(byte) << shift)
}

impl Device<true> for DivSqrt {
    fn peek(&self, offset: u32, width: Width, now: u64) -> u32 {
        read_bytes(offset, width, |at| self.byte(at, now))
    }

    /// Each byte of the write goes to the register at its own offset. A write to DIVCNT,
    /// DIV_NUMER or DIV_DENOM starts a division and one to SQRTCNT or SQRT_PARAM a square root,
    /// on the registers as the whole access leaves them; the result registers and the busy and
    /// flag bits are read-only. It does not use `device::write_bytes`: a start happens once,
    /// after the whole access, not after each byte (a word write of a numerator half would
    /// otherwise start four operations, the last three on half-written operands).
    // TODO(hw-unverified): narrow accesses, and which bytes of a register start an operation; a
    // write to the unused upper half of a DIVCNT or SQRTCNT word starts nothing here
    // (`divsqrt.md`, "Unverified").
    fn write(&mut self, offset: u32, width: Width, value: u32, now: u64) -> Effect {
        let (mut divide_now, mut sqrt_now) = (false, false);
        (0..width.bytes()).for_each(|i| {
            let (at, byte) = (offset.wrapping_add(i), (value >> (8 * i)) as u8);
            match at {
                0 => {
                    self.div_mode = byte & DIV_MODE_MASK;
                    divide_now = true;
                }
                1 => divide_now = true,
                NUMER..DENOM => {
                    self.numer = with_byte(self.numer, at - NUMER, byte);
                    divide_now = true;
                }
                DENOM..RESULT => {
                    self.denom = with_byte(self.denom, at - DENOM, byte);
                    divide_now = true;
                }
                SQRTCNT => {
                    self.sqrt_mode = byte & SQRT_64_BIT;
                    sqrt_now = true;
                }
                0x31 => sqrt_now = true,
                SQRT_PARAM..END => {
                    self.sqrt_param = with_byte(self.sqrt_param, at - SQRT_PARAM, byte);
                    sqrt_now = true;
                }
                _ => {}
            }
        });
        if divide_now {
            self.start_division(now);
        }
        if sqrt_now {
            self.start_sqrt(now);
        }
        Effect::default()
    }
}

#[cfg(test)]
mod tests;
