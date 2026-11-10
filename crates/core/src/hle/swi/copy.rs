// SPDX-License-Identifier: Apache-2.0
//! CpuSet and CpuFastSet (`docs/hw/hle-swi.md`, "The copy, math and decompression SWIs").
//! Memory is reached through the calling CPU's bus view, so a write to an I/O register or VRAM is
//! what an instruction's write would be.

use crate::cpu::Bus;

/// `R2` bits 0-20: the unit count.
const COUNT_MASK: u32 = 0x1F_FFFF;
/// `R2` bit 24: fill instead of copy. Bit 26: 32-bit units instead of 16-bit.
const FILL: u32 = 1 << 24;
const WIDE: u32 = 1 << 26;
/// CpuFastSet moves whole blocks of this many words.
const FAST_BLOCK_WORDS: u32 = 8;

/// CpuSet: `count` halfwords or words copied from `src` to `dst`, or one unit read at `src` and
/// written `count` times. A count of 0 does nothing.
// TODO(hw-unverified): the addresses are aligned down to the unit; the registers are left as they
// were (`hle-swi.md`, "Unverified").
pub(super) fn cpu_set<B: Bus>(bus: &mut B, src: u32, dst: u32, control: u32) {
    let count = control & COUNT_MASK;
    match (control & FILL != 0, control & WIDE != 0) {
        (false, false) => copy16(bus, src & !1, dst & !1, count),
        (false, true) => copy32(bus, src & !3, dst & !3, count),
        (true, false) => fill16(bus, src & !1, dst & !1, count),
        (true, true) => fill32(bus, src & !3, dst & !3, count),
    }
}

/// CpuFastSet: words only, and the count rounded up to a whole number of 8-word blocks.
pub(super) fn cpu_fast_set<B: Bus>(bus: &mut B, src: u32, dst: u32, control: u32) {
    let count = control & COUNT_MASK;
    let words = count.div_ceil(FAST_BLOCK_WORDS) * FAST_BLOCK_WORDS;
    match control & FILL != 0 {
        false => copy32(bus, src & !3, dst & !3, words),
        true => fill32(bus, src & !3, dst & !3, words),
    }
}

fn copy16<B: Bus>(bus: &mut B, src: u32, dst: u32, count: u32) {
    (0..count).for_each(|i| {
        let unit = bus.read16(src.wrapping_add(2 * i));
        bus.write16(dst.wrapping_add(2 * i), unit);
    });
}

fn copy32<B: Bus>(bus: &mut B, src: u32, dst: u32, count: u32) {
    (0..count).for_each(|i| {
        let unit = bus.read32(src.wrapping_add(4 * i));
        bus.write32(dst.wrapping_add(4 * i), unit);
    });
}

fn fill16<B: Bus>(bus: &mut B, src: u32, dst: u32, count: u32) {
    if count != 0 {
        let unit = bus.read16(src);
        (0..count).for_each(|i| bus.write16(dst.wrapping_add(2 * i), unit));
    }
}

fn fill32<B: Bus>(bus: &mut B, src: u32, dst: u32, count: u32) {
    if count != 0 {
        let unit = bus.read32(src);
        (0..count).for_each(|i| bus.write32(dst.wrapping_add(4 * i), unit));
    }
}
