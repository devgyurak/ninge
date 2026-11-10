// SPDX-License-Identifier: Apache-2.0
//! The ARMv5TE instructions the ARM9 adds to ARM and Thumb state. Note:
//! `docs/hw/armv5te.md`. The switch to Thumb on a load into R15 is in `arm_transfer.rs`.

use super::Cpu;
use super::arm::{ARM_PC_AHEAD, ARM_WORD, THUMB_HALFWORD, branch_offset};
use super::psr::FLAG_Q;
use super::regs::{LR, PC};

/// BLX (immediate) adds H, bit 24, as a halfword to the word-scaled offset.
const BLX_H_SHIFT: u32 = 1;
/// Rd's position in CLZ and the saturating adds (bits 15–12).
const RD_SHIFT: u32 = 12;
/// Rn's position in the saturating adds (bits 19–16).
const RN_SHIFT: u32 = 16;
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

    /// SMLAxy, SMLAWy, SMULWy, SMLALxy and SMULxy (`docs/hw/armv5te.md`, "Signed halfword
    /// multiplies"). Bits 22–21 pick the operation; `x` (bit 5) picks Rm's halfword and `y`
    /// (bit 6) Rs's. Only the 32-bit accumulates touch Q; no form touches N, Z, C or V.
    pub(super) fn signed_halfword_multiply(&mut self, word: u32) -> bool {
        let rm = self.operand(word & 0xF, 0);
        let rs = self.operand(word >> 8 & 0xF, 0);
        let rd = word >> 16 & 0xF;
        let rn = word >> RD_SHIFT & 0xF;
        let y_half = halfword(rs, word >> 6 & 1 == 1);
        match (word >> 21 & 3, word >> 5 & 1) {
            // SMLAxy
            (0, _) => self.accumulate(rd, halfword(rm, word >> 5 & 1 == 1) * y_half, rn),
            // SMULWy (bit 5 set): the top 32 bits of the 48-bit product
            (1, 1) => self.write_destination(rd, wide_product(rm, y_half) as u32), // truncation: fits 32 bits
            // SMLAWy
            (1, _) => self.accumulate(rd, wide_product(rm, y_half), rn),
            // SMLALxy: RdHi is the Rd field, RdLo the Rn field.
            (2, _) => {
                let product = i64::from(halfword(rm, word >> 5 & 1 == 1) * y_half);
                let high = u64::from(self.operand(rd, 0));
                let low = u64::from(self.operand(rn, 0));
                // The casts are the signed reading of the 64-bit pair.
                let sum = ((high << 32 | low) as i64).wrapping_add(product) as u64;
                let wrote_lo = self.write_destination(rn, sum as u32); // truncation: the low half
                let wrote_hi = self.write_destination(rd, (sum >> 32) as u32);
                wrote_lo || wrote_hi
            }
            // SMULxy
            _ => self.write_destination(rd, (halfword(rm, word >> 5 & 1 == 1) * y_half) as u32), // two's complement bits
        }
    }

    /// Rd = `product` + Rn, wrapping; Q is set if the signed add overflowed.
    fn accumulate(&mut self, rd: u32, product: i32, rn: u32) -> bool {
        let (sum, overflowed) = product.overflowing_add(self.operand(rn, 0) as i32); // signed reading
        self.set_saturation(overflowed);
        self.write_destination(rd, sum as u32) // two's complement bits
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
        match rd as usize {
            PC => self.write_pc(value),
            other => self.regs.set(other, value),
        }
        rd as usize == PC
    }
}

/// `value` clamped to the signed 32-bit range, and whether it had to be.
fn saturate(value: i64) -> (i64, bool) {
    let clamped = value.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    (clamped, clamped != value)
}

/// The signed bottom (`top` false) or top halfword of `value`.
fn halfword(value: u32, top: bool) -> i32 {
    // The casts are the signed reading of the 16 bits.
    match top {
        true => i32::from((value >> 16) as u16 as i16),
        false => i32::from(value as u16 as i16),
    }
}

/// The top 32 bits of the 48-bit product of the signed word `rm` and halfword `half`.
fn wide_product(rm: u32, half: i32) -> i32 {
    // The 48-bit product shifted right by 16 always fits 32 bits.
    ((i64::from(rm as i32) * i64::from(half)) >> 16) as i32
}
