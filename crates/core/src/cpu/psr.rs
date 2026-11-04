// SPDX-License-Identifier: Apache-2.0
//! Condition codes and PSR-transfer helpers (`docs/hw/arm-data-processing.md`).

/// N, Z, C, V in the CPSR (`docs/hw/cpu-registers.md`, "CPSR bits").
pub(crate) const FLAG_N: u32 = 1 << 31;
pub(crate) const FLAG_Z: u32 = 1 << 30;
pub(crate) const FLAG_C: u32 = 1 << 29;
pub(crate) const FLAG_V: u32 = 1 << 28;
/// CPSR bit 27, Q: the ARM9's sticky saturation flag (`docs/hw/armv5te.md`, "The Q flag").
/// TODO(hw-unverified): the ARM7 stores an MSR write of this bit (`docs/hw/armv5te.md`).
pub(crate) const FLAG_Q: u32 = 1 << 27;
/// CPSR bits 7 and 6, I and F: 1 = IRQ or FIQ disabled (`docs/hw/cpu-registers.md`).
pub(crate) const FLAG_I: u32 = 1 << 7;
pub(crate) const FLAG_F: u32 = 1 << 6;
/// CPSR bit 5, T: 1 = Thumb state (`docs/hw/cpu-registers.md`).
pub(crate) const FLAG_T: u32 = 1 << 5;
/// The flags byte of a PSR, the `f` field of an MSR mask.
pub(crate) const FLAGS_BYTE: u32 = 0xFF00_0000;
/// Mode bits M4–M0 of User mode.
const USER_MODE: u32 = 0x10;
const MODE_MASK: u32 = 0x1F;

/// Whether the 4-bit condition holds for `cpsr` (`docs/hw/cpu-registers.md`, "Condition codes").
pub(crate) fn condition_holds(cond: u32, cpsr: u32) -> bool {
    let n = cpsr & FLAG_N != 0;
    let z = cpsr & FLAG_Z != 0;
    let c = cpsr & FLAG_C != 0;
    let v = cpsr & FLAG_V != 0;
    match cond & 0xF {
        0 => z,
        1 => !z,
        2 => c,
        3 => !c,
        4 => n,
        5 => !n,
        6 => v,
        7 => !v,
        8 => c && !z,
        9 => !c || z,
        10 => n == v,
        11 => n != v,
        12 => !z && n == v,
        13 => z || n != v,
        14 => true,
        // Condition 0xF never holds. The ARM9 does not ask: it decodes such a word in the
        // unconditional space (`decode.rs`; `docs/hw/armv5te.md`).
        _ => false,
    }
}

/// `cpsr` with N, Z, C, V replaced.
pub(crate) fn with_flags(cpsr: u32, n: bool, z: bool, c: bool, v: bool) -> u32 {
    let flags = [(n, FLAG_N), (z, FLAG_Z), (c, FLAG_C), (v, FLAG_V)]
        .iter()
        .filter(|&&(set, _)| set)
        .fold(0, |acc, &(_, bit)| acc | bit);
    (cpsr & !(FLAG_N | FLAG_Z | FLAG_C | FLAG_V)) | flags
}

/// The bytes an MSR field mask (bits 19–16: `f`, `s`, `x`, `c`) selects.
pub(crate) fn field_mask(mask: u32) -> u32 {
    (0..4)
        .filter(|field| mask >> field & 1 == 1)
        .fold(0, |acc, field| acc | 0xFF << (8 * field))
}

/// Whether the CPSR is in User mode, where only the flags byte can be written by MSR.
// TODO(hw-unverified): mode bits that name no mode count as privileged here. No suite case
// starts in such a mode, so this is inference (`docs/hw/arm-data-processing.md`, "Unverified").
pub(crate) fn is_user(cpsr: u32) -> bool {
    cpsr & MODE_MASK == USER_MODE
}
