// SPDX-License-Identifier: Apache-2.0
//! The ARMv5TE instructions the ARM9 adds to ARM and Thumb state (#14). Note:
//! `docs/hw/armv5te.md`. The switch to Thumb on a load into R15 is in `arm_transfer.rs`.

use super::Cpu;
use super::arm::{ARM_PC_AHEAD, ARM_WORD, LR, THUMB_HALFWORD, branch_offset};
use super::psr::FLAG_Q;

/// BLX (immediate) adds H, bit 24, as a halfword to the word-scaled offset.
const BLX_H_SHIFT: u32 = 1;
/// Rd's position in CLZ and the saturating adds (bits 15–12).
const RD_SHIFT: u32 = 12;
/// Rn's position in the saturating adds (bits 19–16).
const RN_SHIFT: u32 = 16;
/// R15's index.
const PC: u32 = 15;
/// Bit 0 of a link value written from Thumb state, so that a return by BX stays in Thumb.
const THUMB_LINK_BIT: u32 = 1;

impl<const ARM9: bool> Cpu<ARM9> {
    /// BLX with an immediate (`docs/hw/armv5te.md`, "BLX"): LR = the next instruction, then enter
    /// Thumb state at R15 + the word offset + H × 2. Always writes the PC.
    pub(super) fn branch_link_exchange_immediate(&mut self, word: u32) -> bool {
        let r15 = self.regs.get(15);
        let half = (word >> 24 & 1) << BLX_H_SHIFT;
        self.regs.set(LR, r15.wrapping_sub(ARM_PC_AHEAD - ARM_WORD));
        let target = r15.wrapping_add(branch_offset(word)).wrapping_add(half);
        // Bit 0 of the target is 0 (H only sets bit 1), so it is set here to select Thumb.
        self.exchange_to(target | THUMB_LINK_BIT)
    }

    /// BLX with a register in ARM state (`docs/hw/armv5te.md`, "BLX"): the target is read before
    /// LR is written, so `BLX LR` jumps to the old LR.
    // TODO(hw-unverified): Rm = R15 reads the executing address + 8 (`docs/hw/armv5te.md`,
    // "Unverified").
    pub(super) fn branch_link_exchange(&mut self, word: u32) -> bool {
        let target = self.operand(word & 0xF, 0);
        self.regs
            .set(LR, self.regs.get(15).wrapping_sub(ARM_PC_AHEAD - ARM_WORD));
        self.exchange_to(target)
    }

    /// BLX with a register in Thumb state: the link is the next instruction with bit 0 set.
    // TODO(hw-unverified): Rm = R15 reads the executing address + 4 (`docs/hw/armv5te.md`,
    // "Unverified").
    pub(super) fn branch_link_exchange_thumb(&mut self, word: u32) -> bool {
        let target = self.operand(word >> 3 & 0xF, 0);
        // R15 is this halfword's address + 4; the next instruction is two bytes past the halfword.
        let next = self.regs.get(15).wrapping_sub(THUMB_HALFWORD);
        self.regs.set(LR, next | THUMB_LINK_BIT);
        self.exchange_to(target)
    }

    /// CLZ (`docs/hw/armv5te.md`, "CLZ"): Rd = the leading zero bits of Rm, 32 for 0. No flags.
    pub(super) fn count_leading_zeros(&mut self, word: u32) -> bool {
        let value = self.operand(word & 0xF, 0);
        self.write_destination(word >> RD_SHIFT & 0xF, value.leading_zeros())
    }

    /// QADD, QSUB, QDADD and QDSUB (`docs/hw/armv5te.md`, "Saturating arithmetic"): Rd = Rm ± Rn,
    /// with Rn doubled first by the D forms, each step clamped to the signed 32-bit range. Q is
    /// set if any step clamped and is never cleared; N, Z, C and V stay.
    pub(super) fn saturating_arithmetic(&mut self, word: u32) -> bool {
        // The casts are the signed reading of the registers.
        let rm = i64::from(self.operand(word & 0xF, 0) as i32);
        let rn = i64::from(self.operand(word >> RN_SHIFT & 0xF, 0) as i32);
        // Bit 22 doubles Rn (QDADD, QDSUB); bit 21 subtracts (QSUB, QDSUB).
        let (rn, doubling_clamped) = match word >> 22 & 1 {
            1 => saturate(rn * 2),
            _ => (rn, false),
        };
        let (result, clamped) = match word >> 21 & 1 {
            1 => saturate(rm - rn),
            _ => saturate(rm + rn),
        };
        self.set_saturation(clamped || doubling_clamped);
        self.write_destination(word >> RD_SHIFT & 0xF, result as u32) // truncation: the clamped value fits 32 bits
    }

    /// Sets Q when `saturated`; otherwise leaves the CPSR alone.
    fn set_saturation(&mut self, saturated: bool) {
        let cpsr = self.regs.cpsr();
        self.regs.set_cpsr(match saturated {
            true => cpsr | FLAG_Q,
            false => cpsr,
        });
    }

    /// Writes a result to Rd; R15 takes the value and the step refills from it. True if R15.
    // TODO(hw-unverified): R15 is not allowed as the destination of these instructions; it is
    // written like a data-processing result (`docs/hw/armv5te.md`, "Unverified").
    fn write_destination(&mut self, rd: u32, value: u32) -> bool {
        match rd {
            PC => self.write_pc(value),
            _ => self.regs.set(rd as usize, value),
        }
        rd == PC
    }
}

/// `value` clamped to the signed 32-bit range, and whether it had to be.
fn saturate(value: i64) -> (i64, bool) {
    let clamped = value.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    (clamped, clamped != value)
}
