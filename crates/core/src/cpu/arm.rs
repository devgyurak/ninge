// SPDX-License-Identifier: Apache-2.0
//! ARM-state execution: data processing and PSR transfer (#8), multiply (#9). Notes:
//! `docs/hw/arm-data-processing.md`, `docs/hw/arm-barrel-shifter.md`,
//! `docs/hw/arm-multiply.md`.

use super::alu;
use super::decode::{Kind, decode};
use super::psr::{FLAG_C, FLAG_V, FLAGS_BYTE, condition_holds, field_mask, is_user, with_flags};
use super::shifter::{ShiftKind, rotated_immediate, shift_immediate, shift_register};
use super::{Bus, Cpu};

/// CPSR bit 4 (M4), which the 32-bit modes all have set.
const MODE_BIT_4: u32 = 0x10;

// Interim until #11: the PC convention below and the section in `impl Cpu` that uses it.
/// How far R15 runs ahead of the executing instruction, and so the distance between R15 and
/// the fetch address.
const PC_READ_OFFSET: u32 = 8;
/// A register-specified shift costs an extra cycle, so R15 reads that far ahead again.
const REGISTER_SHIFT_EXTRA: u32 = 4;
/// A multiply's R15 operand reads this far ahead again (the suite: address + 12).
const MULTIPLY_OPERAND_EXTRA: u32 = 4;
/// Size of an ARM instruction.
const ARM_WORD: u32 = 4;

impl<const ARM9: bool> Cpu<ARM9> {
    /// Executes the ARM instruction at the fetch address.
    // TODO(phase1): Thumb state (T = 1) arrives in #13; the T bit is ignored until then.
    pub(super) fn step_arm<B: Bus>(&mut self, bus: &mut B) {
        let word = bus.fetch32(self.fetch_address());
        let wrote_pc = condition_holds(word >> 28, self.regs.cpsr())
            && match decode(word) {
                Kind::DataImmediate | Kind::DataImmediateShift | Kind::DataRegisterShift => {
                    self.data_processing(word)
                }
                Kind::Mrs => self.mrs(word),
                Kind::MsrImmediate | Kind::MsrRegister => self.msr(word),
                Kind::Multiply => self.multiply(word),
                Kind::MultiplyLong => self.multiply_long(word),
                Kind::Unhandled => false,
            };
        // A write to the PC already moved it; everything else, including a failed condition,
        // steps to the next instruction.
        if !wrote_pc {
            self.advance_pc();
        }
    }

    // ---- Interim PC convention, until #11 ----------------------------------------------
    //
    // The JSON suite's convention: while an instruction executes R15 is its address + 8, a
    // step that does not write the PC ends with R15 + 4, and an ALU result written to R15 ends
    // as the value + 8. Everything that depends on it is in this section and in the four
    // constants above the `impl` (`PC_READ_OFFSET`, `REGISTER_SHIFT_EXTRA`,
    // `MULTIPLY_OPERAND_EXTRA`, `ARM_WORD`);
    // #11 replaces the section and `operand` with the real pipeline.

    /// The address `step_arm` fetches from: R15 less the interim read-ahead.
    fn fetch_address(&self) -> u32 {
        self.regs.get(15).wrapping_sub(PC_READ_OFFSET)
    }

    /// Steps R15 to the next instruction.
    fn advance_pc(&mut self) {
        self.regs.set(15, self.regs.get(15).wrapping_add(ARM_WORD));
    }

    /// Stores a value written to the PC by an ALU result: R15 becomes the value + 8 and the
    /// low bits stay as written.
    // TODO(hw-unverified): the low two bits of an ALU result in R15 are kept unaligned because
    // the suite does (`docs/hw/arm-data-processing.md`, "Unverified").
    fn write_pc(&mut self, value: u32) {
        self.regs.set(15, value.wrapping_add(PC_READ_OFFSET));
    }

    /// Stores a value in R15 as it is. MRS with Rd = R15 uses this: the normal advance follows
    /// instead of the +8 of [`Self::write_pc`], so R15 ends as the value + 4.
    // TODO(hw-unverified): the suite shows this for MRS Rd = R15 (1,580 cases); the ARM ARM
    // leaves it unpredictable (`docs/hw/arm-data-processing.md`, "Unverified").
    fn write_pc_unadjusted(&mut self, value: u32) {
        self.regs.set(15, value);
    }

    /// A register as an instruction reads it. R15 holds the executing address + 8 under the
    /// interim convention, and reads `extra` further ahead (the register-shift form).
    fn operand(&self, index: u32, extra: u32) -> u32 {
        let value = self.regs.get(index as usize);
        match index {
            15 => value.wrapping_add(extra),
            _ => value,
        }
    }

    // ---- End of the interim PC convention ----------------------------------------------

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
                    REGISTER_SHIFT_EXTRA,
                    shift_register(
                        kind,
                        self.operand(word & 0xF, REGISTER_SHIFT_EXTRA),
                        amount,
                        carry,
                    ),
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

    /// MRS: copies the CPSR or SPSR to Rd. The PC is not marked as written, even for Rd = R15.
    fn mrs(&mut self, word: u32) -> bool {
        let rd = (word >> 12) & 0xF;
        let cpsr = self.regs.cpsr();
        // TODO(hw-unverified): User/System have no SPSR; the suite reads the CPSR instead. In a
        // mode whose bits name no mode the SPSR follows the kept bank, which no suite case covers.
        let value = match word >> 22 & 1 {
            1 => self.regs.spsr(self.regs.bank()).unwrap_or(cpsr),
            _ => cpsr,
        };
        match rd {
            15 => self.write_pc_unadjusted(value),
            _ => self.regs.set(rd as usize, value),
        }
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
    // R15 Rd ends as the value + 8 (suite; the real pipeline is #11). Rd = Rm gives the plain
    // result because every operand is read before the write (suite). MUL does not read its Rn
    // field (inferred).
    fn multiply(&mut self, word: u32) -> bool {
        let rd = (word >> 16) & 0xF;
        let rn = (word >> 12) & 0xF;
        let rs = (word >> 8) & 0xF;
        let rm = word & 0xF;
        let product = self
            .operand(rm, MULTIPLY_OPERAND_EXTRA)
            .wrapping_mul(self.operand(rs, MULTIPLY_OPERAND_EXTRA));
        // Bit 21 is A: only MLA reads Rn.
        let result = match word >> 21 & 1 {
            1 => product.wrapping_add(self.operand(rn, MULTIPLY_OPERAND_EXTRA)),
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
    // destinations follow the interim convention (suite, #11); RdHi/RdLo = Rm gives the plain
    // result (suite).
    fn multiply_long(&mut self, word: u32) -> bool {
        let rd_hi = (word >> 16) & 0xF;
        let rd_lo = (word >> 12) & 0xF;
        let rs = (word >> 8) & 0xF;
        let rm = word & 0xF;
        let multiplicand = self.operand(rm, MULTIPLY_OPERAND_EXTRA);
        let multiplier = self.operand(rs, MULTIPLY_OPERAND_EXTRA);
        // Bit 22 is U: set for the signed forms. The sign-extending casts are the signed
        // reading of the 32-bit operands.
        let product = match word >> 22 & 1 {
            1 => (i64::from(multiplicand as i32).wrapping_mul(i64::from(multiplier as i32))) as u64,
            _ => u64::from(multiplicand).wrapping_mul(u64::from(multiplier)),
        };
        // Bit 21 is A: the accumulate forms add the old pair, read before either half is written.
        let result = match word >> 21 & 1 {
            1 => {
                let high = u64::from(self.operand(rd_hi, MULTIPLY_OPERAND_EXTRA));
                let low = u64::from(self.operand(rd_lo, MULTIPLY_OPERAND_EXTRA));
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

    /// Writes one half of a multiply result; R15 gets the value + 8.
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
