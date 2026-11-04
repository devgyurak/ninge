// SPDX-License-Identifier: Apache-2.0
//! ARM-state execution: data processing and PSR transfer (#8), multiply (#9), branches and the
//! prefetch pipeline (#11); loads and stores are in `arm_transfer.rs` (#10). Notes:
//! `docs/hw/arm-data-processing.md`, `docs/hw/arm-barrel-shifter.md`,
//! `docs/hw/arm-multiply.md`, `docs/hw/arm-branch-pipeline.md`.

use super::alu;
use super::decode::{Kind, decode};
use super::exception::Exception;
use super::psr::{
    FLAG_C, FLAG_T, FLAG_V, FLAGS_BYTE, condition_holds, field_mask, is_user, with_flags,
};
use super::shifter::{ShiftKind, rotated_immediate, shift_immediate, shift_register};
use super::{Bus, Cpu};

/// CPSR bit 4 (M4), which the 32-bit modes all have set.
const MODE_BIT_4: u32 = 0x10;
/// How far R15 runs ahead of the executing ARM instruction (`docs/hw/arm-branch-pipeline.md`,
/// "The prefetch pipeline"): the two words the pipeline holds.
pub(super) const ARM_PC_AHEAD: u32 = 8;
/// A Thumb instruction's R15 runs this far ahead of it.
pub(super) const THUMB_PC_AHEAD: u32 = 4;
/// Size of an ARM instruction, and of the step from one refill fetch to the next.
pub(super) const ARM_WORD: u32 = 4;
/// Size of a Thumb instruction.
pub(super) const THUMB_HALFWORD: u32 = 2;
/// An R15 read after the prefetch has advanced the PC once more reads this far ahead again
/// (`docs/hw/arm-branch-pipeline.md`, "Late R15 reads"): a register-specified shift's Rn and Rm,
/// a multiply's operands, the stored register of STR, STRH and STM, and SWP's registers.
// TODO(hw-unverified): the multiply and SWP late reads are the suite's, oracle evidence only
// (`docs/hw/arm-branch-pipeline.md`, "Unverified").
pub(super) const LATE_PC_EXTRA: u32 = 4;
/// B and BL: the offset is a word offset, so the field is shifted left by 2.
const BRANCH_OFFSET_SHIFT: u32 = 2;
/// The branch offset field is 24 bits; moving it to the top and back sign-extends it.
const BRANCH_FIELD_TO_TOP: u32 = 8;
/// R14, which BL writes.
pub(super) const LR: usize = 14;

/// The branch offset field of B, BL and BLX (immediate), sign-extended and multiplied by 4.
pub(super) fn branch_offset(word: u32) -> u32 {
    // Moving the field to the top and shifting back arithmetically sign-extends it; the casts
    // are the signed reading.
    (((word << BRANCH_FIELD_TO_TOP) as i32) >> (BRANCH_FIELD_TO_TOP - BRANCH_OFFSET_SHIFT)) as u32
}

impl<const ARM9: bool> Cpu<ARM9> {
    /// Executes the ARM instruction at the head of the pipeline
    /// (`docs/hw/arm-branch-pipeline.md`, "The prefetch pipeline").
    // TODO(hw-unverified): the fetch sequence (one fetch at R15 before execution, two on a
    // refill, 32/16/16 bits for BX to Thumb) is the suite's, oracle evidence only
    // (`docs/hw/arm-branch-pipeline.md`, "Unverified"). Decided in #NN.
    pub(super) fn step_arm<B: Bus>(&mut self, bus: &mut B) {
        let word = self.pipeline[0];
        // The fetch comes before the instruction does anything, so a data access follows it.
        let fetched = bus.fetch32(self.regs.get(15));
        self.pipeline = [self.pipeline[1], fetched];
        // On the ARM9 condition 1111 selects the unconditional space, whose words always run.
        let runs = (ARM9 && word >> 28 == 0xF) || condition_holds(word >> 28, self.regs.cpsr());
        let wrote_pc = runs
            && match decode::<ARM9>(word) {
                Kind::DataImmediate | Kind::DataImmediateShift | Kind::DataRegisterShift => {
                    self.data_processing(word)
                }
                Kind::Mrs => self.mrs(word),
                Kind::MsrImmediate | Kind::MsrRegister => self.msr(word),
                Kind::Multiply => self.multiply(word),
                Kind::MultiplyLong => self.multiply_long(word),
                Kind::SingleTransferImmediate | Kind::SingleTransferRegister => {
                    self.single_transfer(bus, word)
                }
                Kind::HalfwordTransfer => self.halfword_transfer(bus, word),
                // PLD is a cache hint: it changes no register, flag or memory, and takes no abort.
                Kind::Preload => false,
                Kind::DoubleTransfer => self.double_transfer(bus, word),
                Kind::Swap => self.swap(bus, word),
                Kind::BlockTransfer => self.block_transfer(bus, word),
                Kind::Branch => self.branch(word),
                Kind::BranchExchange => self.branch_exchange(word),
                Kind::BranchLinkExchange => self.branch_link_exchange(word),
                Kind::CountLeadingZeros => self.count_leading_zeros(word),
                Kind::SaturatingArithmetic => self.saturating_arithmetic(word),
                Kind::SignedHalfwordMultiply => self.signed_halfword_multiply(word),
                Kind::BranchLinkExchangeImmediate => self.branch_link_exchange_immediate(word),
                Kind::Swi => self.enter(Exception::SoftwareInterrupt),
                Kind::Breakpoint => self.enter(Exception::PrefetchAbort),
                Kind::Undefined => self.enter(Exception::Undefined),
                Kind::Coprocessor => self.coprocessor(word),
                Kind::Unhandled => false,
            };
        // A write to the PC flushes the pipeline; everything else, including a failed
        // condition, steps to the next instruction.
        match wrote_pc {
            true => self.refill(bus),
            false => self.regs.set(15, self.regs.get(15).wrapping_add(ARM_WORD)),
        }
    }

    /// Refills the pipeline from R15 after a write to it, in the state T selects: R15 becomes
    /// the pipeline's read-ahead past the target.
    // TODO(hw-unverified): the refill's fetches (see `step_arm`), and the low two bits of an ARM
    // target, dropped for the fetches but kept in R15 (`docs/hw/arm-branch-pipeline.md`, "Unverified").
    pub(super) fn refill<B: Bus>(&mut self, bus: &mut B) {
        let target = self.regs.get(15);
        match self.regs.cpsr() & FLAG_T != 0 {
            true => {
                let first = target & !1;
                let second = first.wrapping_add(THUMB_HALFWORD);
                self.pipeline = [
                    u32::from(bus.fetch16(first)),
                    u32::from(bus.fetch16(second)),
                ];
                self.regs.set(15, first.wrapping_add(THUMB_PC_AHEAD));
            }
            false => {
                let first = target & !3;
                let second = first.wrapping_add(ARM_WORD);
                self.pipeline = [bus.fetch32(first), bus.fetch32(second)];
                self.regs.set(15, target.wrapping_add(ARM_PC_AHEAD));
            }
        }
    }

    /// Writes a value to R15: the instruction has written the PC, and the step refills the
    /// pipeline from it.
    pub(super) fn write_pc(&mut self, value: u32) {
        self.regs.set(15, value);
    }

    /// A register as an instruction reads it. R15 holds the executing address + 8 while an
    /// instruction runs, and reads `extra` further ahead for a late read ([`LATE_PC_EXTRA`]).
    pub(super) fn operand(&self, index: u32, extra: u32) -> u32 {
        let value = self.regs.get(index as usize);
        match index {
            15 => value.wrapping_add(extra),
            _ => value,
        }
    }

    /// A register as a store writes it: a late R15 read, one instruction width further than
    /// the usual read. Only a Thumb store of R15 (an empty register list) can tell the widths
    /// apart.
    // TODO(hw-unverified): the Thumb width is the suite's, oracle evidence only
    // (`docs/hw/thumb.md`, "Unverified").
    pub(super) fn store_operand(&self, index: u32) -> u32 {
        let extra = match self.regs.cpsr() & FLAG_T != 0 {
            true => THUMB_HALFWORD,
            false => LATE_PC_EXTRA,
        };
        self.operand(index, extra)
    }

    /// B and BL (`docs/hw/arm-branch-pipeline.md`, "B and BL"): R15 + the sign-extended word
    /// offset, with BL first writing the next instruction's address to R14. Always writes the
    /// PC.
    fn branch(&mut self, word: u32) -> bool {
        let r15 = self.regs.get(15);
        let offset = branch_offset(word);
        // Bit 24 is L.
        if word >> 24 & 1 == 1 {
            self.regs.set(LR, r15.wrapping_sub(ARM_PC_AHEAD - ARM_WORD));
        }
        self.write_pc(r15.wrapping_add(offset));
        true
    }

    /// BX (`docs/hw/arm-branch-pipeline.md`, "BX"): jumps to Rn and copies its bit 0 to T.
    // TODO(hw-unverified): ARM BX with bit 1 set, and should-be-one fields that are not all ones
    // (`docs/hw/arm-branch-pipeline.md`, "Unverified").
    fn branch_exchange(&mut self, word: u32) -> bool {
        let target = self.operand(word & 0xF, 0);
        self.exchange_to(target)
    }

    /// Jumps to `target` and copies its bit 0 to T; shared by ARM and Thumb BX
    /// (`docs/hw/arm-branch-pipeline.md`, "BX"; `docs/hw/thumb.md`, "Format 5 and BX").
    pub(super) fn exchange_to(&mut self, target: u32) -> bool {
        let cpsr = self.regs.cpsr();
        self.regs.set_cpsr(match target & 1 {
            1 => cpsr | FLAG_T,
            _ => cpsr & !FLAG_T,
        });
        self.write_pc(target);
        true
    }

    /// Runs a data-processing instruction; true if it wrote the PC.
    fn data_processing(&mut self, word: u32) -> bool {
        let opcode = (word >> 21) & 0xF;
        let rn = (word >> 16) & 0xF;
        let rd = (word >> 12) & 0xF;
        let cpsr = self.regs.cpsr();
        let carry = cpsr & FLAG_C != 0;
        let overflow = cpsr & FLAG_V != 0;
        let kind = ShiftKind::from_bits(word >> 5);
        // Bit 25 selects the immediate; bit 4 the register-specified amount.
        let (extra, (operand2, shifter_carry)) = match (word >> 25 & 1, word >> 4 & 1) {
            (1, _) => (0, rotated_immediate(word & 0xFF, (word >> 8) & 0xF, carry)),
            (_, 0) => (
                0,
                shift_immediate(kind, self.operand(word & 0xF, 0), (word >> 7) & 0x1F, carry),
            ),
            _ => {
                // TODO(hw-unverified): Rs = R15 reads the +8 view, not the +12 one Rn and Rm get; the
                // suite agrees (`docs/hw/arm-barrel-shifter.md`, "Unverified").
                let amount = self.operand((word >> 8) & 0xF, 0) & 0xFF;
                (
                    LATE_PC_EXTRA,
                    shift_register(kind, self.operand(word & 0xF, LATE_PC_EXTRA), amount, carry),
                )
            }
        };
        let result = alu::execute(
            opcode,
            self.operand(rn, extra),
            operand2,
            shifter_carry,
            carry,
            overflow,
        );
        let compare = alu::is_compare(opcode);
        let set_flags = word >> 20 & 1 == 1 || compare;
        let writes_pc = rd == 15 && !compare;
        // The PSR-restore form: S with Rd = R15, compare opcodes included.
        let restores_psr = rd == 15 && word >> 20 & 1 == 1;
        // TODO(hw-unverified): in a mode whose bits name no mode the SPSR follows the kept bank;
        // no suite case starts there (`docs/hw/arm-data-processing.md`, "Unverified").
        let spsr = self.regs.spsr(self.regs.bank());
        if set_flags {
            let flags = with_flags(
                cpsr,
                result.result >> 31 == 1,
                result.result == 0,
                result.carry,
                result.overflow,
            );
            // CPSR <- SPSR when S is set and the PC is the destination, in a mode with an
            // SPSR. Everywhere else the flags update as usual.
            // TODO(hw-unverified): in User/System (no SPSR) the suite updates the flags and
            // keeps the control bits; TST/TEQ/CMP/CMN with Rd = R15 restore too
            // (`docs/hw/arm-data-processing.md`, "Unverified").
            self.regs
                .set_cpsr(spsr.filter(|_| restores_psr).unwrap_or(flags));
        }
        match compare {
            true => {}
            false if rd == 15 => self.write_pc(result.result),
            false => self.regs.set(rd as usize, result.result),
        }
        writes_pc
    }

    /// MRS: copies the CPSR or SPSR to Rd. Rd = R15 is a plain register write: the PC is not
    /// marked as written, so R15 ends as the value + 4.
    // TODO(hw-unverified): the suite shows no flush for MRS Rd = R15 (1,580 cases); the ARM ARM
    // leaves it unpredictable (`docs/hw/arm-branch-pipeline.md`, "Unverified").
    fn mrs(&mut self, word: u32) -> bool {
        let rd = (word >> 12) & 0xF;
        let cpsr = self.regs.cpsr();
        // TODO(hw-unverified): User/System have no SPSR; the suite reads the CPSR instead. In a
        // mode whose bits name no mode the SPSR follows the kept bank, which no suite case covers.
        let value = match word >> 22 & 1 {
            1 => self.regs.spsr(self.regs.bank()).unwrap_or(cpsr),
            _ => cpsr,
        };
        self.regs.set(rd as usize, value);
        false
    }

    /// MSR: writes the bytes of the CPSR or SPSR the field mask selects.
    fn msr(&mut self, word: u32) -> bool {
        let cpsr = self.regs.cpsr();
        let value = match word >> 25 & 1 {
            1 => rotated_immediate(word & 0xFF, (word >> 8) & 0xF, false).0,
            // TODO(hw-unverified): Rm = R15 reads the +8 view; the ARM ARM leaves it
            // unpredictable (`docs/hw/arm-data-processing.md`, "Unverified").
            _ => self.operand(word & 0xF, 0),
        };
        let selected = field_mask((word >> 16) & 0xF);
        match word >> 22 & 1 {
            1 => {
                // TODO(hw-unverified): an SPSR write in User/System is ignored; in a mode whose
                // bits name no mode it goes to the kept bank, which no suite case covers.
                let bank = self.regs.bank();
                let old = self.regs.spsr(bank).unwrap_or(0);
                self.regs
                    .set_spsr(bank, (old & !selected) | (value & selected));
            }
            _ => {
                // User mode may change the flags byte only. Privileged modes pass the control
                // byte through, the T bit included.
                // TODO(hw-unverified): MSR changing T (`docs/hw/arm-data-processing.md`).
                let allowed = match is_user(cpsr) {
                    true => selected & FLAGS_BYTE,
                    false => selected,
                };
                // TODO(hw-unverified): M4 reads back as 1 after any control-byte write; the
                // suite shows it on every written value (`docs/hw/arm-data-processing.md`).
                let forced = match allowed & 0xFF {
                    0 => 0,
                    _ => MODE_BIT_4,
                };
                self.regs
                    .set_cpsr((cpsr & !allowed) | (value & allowed) | forced);
            }
        }
        false
    }

    /// MUL and MLA (`docs/hw/arm-multiply.md`): Rd = Rm × Rs (+ Rn), low 32 bits. True if it
    /// wrote the PC.
    // TODO(hw-unverified): ARM7 C after S is left unchanged, which the suite contradicts in about
    // half the S cases (#81); no carry model is implemented (its origin is the idea recorded in
    // `docs/hw/arm-multiply.md`, "ARM7TDMI multiply carry", and is itself unverified). On ARM9 C
    // is unaffected per GBATEK, no suite covers it. An R15 operand reads the address + 12 and an
    // R15 Rd flushes the pipeline (suite; `docs/hw/arm-branch-pipeline.md`). Rd = Rm gives the plain
    // result because every operand is read before the write (suite). MUL does not read its Rn
    // field (inferred).
    fn multiply(&mut self, word: u32) -> bool {
        let rd = (word >> 16) & 0xF;
        let rn = (word >> 12) & 0xF;
        let rs = (word >> 8) & 0xF;
        let rm = word & 0xF;
        let product = self
            .operand(rm, LATE_PC_EXTRA)
            .wrapping_mul(self.operand(rs, LATE_PC_EXTRA));
        // Bit 21 is A: only MLA reads Rn.
        let result = match word >> 21 & 1 {
            1 => product.wrapping_add(self.operand(rn, LATE_PC_EXTRA)),
            _ => product,
        };
        self.set_multiply_flags(word, result >> 31 == 1, result == 0);
        match rd {
            15 => self.write_pc(result),
            _ => self.regs.set(rd as usize, result),
        }
        rd == 15
    }

    /// UMULL, UMLAL, SMULL and SMLAL (`docs/hw/arm-multiply.md`): RdHi:RdLo = Rm × Rs (+ the old
    /// pair), wrapping at 64 bits. True if it wrote the PC.
    // TODO(hw-unverified): ARM7 C after S is left unchanged (#81; no carry model, see `multiply`)
    // and on ARM9 it is unaffected per GBATEK; V is left unchanged although GBATEK marks it
    // "destroyed???" for ARMv4; RdHi = RdLo leaves the Hi half (suite); R15 operands and
    // destinations follow the late-read rule and the flush (suite, `docs/hw/arm-branch-pipeline.md`); RdHi/RdLo = Rm gives the plain
    // result (suite).
    fn multiply_long(&mut self, word: u32) -> bool {
        let rd_hi = (word >> 16) & 0xF;
        let rd_lo = (word >> 12) & 0xF;
        let rs = (word >> 8) & 0xF;
        let rm = word & 0xF;
        let multiplicand = self.operand(rm, LATE_PC_EXTRA);
        let multiplier = self.operand(rs, LATE_PC_EXTRA);
        // Bit 22 is U: set for the signed forms. The sign-extending casts are the signed
        // reading of the 32-bit operands.
        let product = match word >> 22 & 1 {
            1 => (i64::from(multiplicand as i32).wrapping_mul(i64::from(multiplier as i32))) as u64,
            _ => u64::from(multiplicand).wrapping_mul(u64::from(multiplier)),
        };
        // Bit 21 is A: the accumulate forms add the old pair, read before either half is written.
        let result = match word >> 21 & 1 {
            1 => {
                let high = u64::from(self.operand(rd_hi, LATE_PC_EXTRA));
                let low = u64::from(self.operand(rd_lo, LATE_PC_EXTRA));
                product.wrapping_add(high << 32 | low)
            }
            _ => product,
        };
        self.set_multiply_flags(word, result >> 63 == 1, result == 0);
        // Lo first, so that with RdHi = RdLo the Hi half remains.
        let lo = result as u32; // truncation: the low half
        let hi = (result >> 32) as u32;
        self.write_multiply_result(rd_lo, lo);
        self.write_multiply_result(rd_hi, hi);
        rd_lo == 15 || rd_hi == 15
    }

    /// Writes one half of a multiply result; R15 takes the value and the step refills from it.
    fn write_multiply_result(&mut self, index: u32, value: u32) {
        match index {
            15 => self.write_pc(value),
            _ => self.regs.set(index as usize, value),
        }
    }

    /// With S (bit 20): N and Z from the result. V and C stay.
    // TODO(hw-unverified): see `multiply` and `multiply_long` for C and V.
    fn set_multiply_flags(&mut self, word: u32, negative: bool, zero: bool) {
        let cpsr = self.regs.cpsr();
        let flags = with_flags(cpsr, negative, zero, cpsr & FLAG_C != 0, cpsr & FLAG_V != 0);
        self.regs.set_cpsr(match word >> 20 & 1 {
            1 => flags,
            _ => cpsr,
        });
    }
}
