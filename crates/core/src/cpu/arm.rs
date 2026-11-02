// SPDX-License-Identifier: Apache-2.0
//! ARM-state execution: data processing and PSR transfer (#8). Notes:
//! `docs/hw/arm-data-processing.md`, `docs/hw/arm-barrel-shifter.md`.

use super::alu;
use super::decode::{Kind, decode};
use super::psr::{FLAG_C, FLAG_V, FLAGS_BYTE, condition_holds, field_mask, is_user, with_flags};
use super::shifter::{ShiftKind, rotated_immediate, shift_immediate, shift_register};
use super::{Bus, Cpu};

/// How far R15 runs ahead of the executing instruction while it runs, and so the distance
/// between R15 and the address `step` fetches from. Interim until #11.
const PC_READ_OFFSET: u32 = 8;
/// A register-specified shift costs an extra cycle, so R15 reads that far ahead again.
const REGISTER_SHIFT_EXTRA: u32 = 4;
/// Size of an ARM instruction.
const ARM_WORD: u32 = 4;
/// CPSR bit 4 (M4), which the 32-bit modes all have set.
const MODE_BIT_4: u32 = 0x10;

impl<const ARM9: bool> Cpu<ARM9> {
    /// Executes the ARM instruction at R15 − 8.
    // TODO(phase1): Thumb state (T = 1) arrives in #13; the T bit is ignored until then.
    pub(super) fn step_arm<B: Bus>(&mut self, bus: &mut B) {
        let word = bus.fetch32(self.regs.get(15).wrapping_sub(PC_READ_OFFSET));
        let wrote_pc = condition_holds(word >> 28, self.regs.cpsr())
            && match decode(word) {
                Kind::DataImmediate | Kind::DataImmediateShift | Kind::DataRegisterShift => {
                    self.data_processing(word)
                }
                Kind::Mrs => self.mrs(word),
                Kind::MsrImmediate | Kind::MsrRegister => self.msr(word),
                Kind::Unhandled => false,
            };
        // A write to the PC already moved it; everything else, including a failed condition,
        // steps to the next instruction.
        let pc = self.regs.get(15);
        self.regs
            .set(15, [pc.wrapping_add(ARM_WORD), pc][usize::from(wrote_pc)]);
    }

    /// Stores a value written to the PC.
    ///
    /// Interim until #11: the suite's convention, kept here and nowhere else. R15 becomes the
    /// value + 8 and the low bits stay as written. #11 replaces this with the real pipeline.
    // TODO(hw-unverified): the low two bits of an ALU result in R15 are kept unaligned because
    // the suite does (`docs/hw/arm-data-processing.md`, "Unverified").
    fn write_pc(&mut self, value: u32) {
        self.regs.set(15, value.wrapping_add(PC_READ_OFFSET));
    }

    /// A register as an instruction reads it; R15 reads `extra` further ahead.
    fn operand(&self, index: u32, extra: u32) -> u32 {
        let value = self.regs.get(index as usize);
        [value, value.wrapping_add(extra)][usize::from(index == 15)]
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
    // TODO(hw-unverified): Rd = R15 is a plain write plus the normal advance, as in the suite.
    fn mrs(&mut self, word: u32) -> bool {
        let rd = (word >> 12) & 0xF;
        let cpsr = self.regs.cpsr();
        // TODO(hw-unverified): User/System have no SPSR; the suite reads the CPSR instead.
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
            _ => self.operand(word & 0xF, 0),
        };
        let selected = field_mask((word >> 16) & 0xF);
        match word >> 22 & 1 {
            1 => {
                // TODO(hw-unverified): an SPSR write in User/System is ignored.
                let bank = self.regs.bank();
                let old = self.regs.spsr(bank).unwrap_or(0);
                self.regs
                    .set_spsr(bank, (old & !selected) | (value & selected));
            }
            _ => {
                // User mode may change the flags byte only. Privileged modes pass the control
                // byte through, the T bit included.
                // TODO(hw-unverified): MSR changing T (`docs/hw/arm-data-processing.md`).
                let allowed = [selected, selected & FLAGS_BYTE][usize::from(is_user(cpsr))];
                // TODO(hw-unverified): M4 reads back as 1 after any control-byte write; the
                // suite shows it on every written value (`docs/hw/arm-data-processing.md`).
                let forced = [0, MODE_BIT_4][usize::from(allowed & 0xFF != 0)];
                self.regs
                    .set_cpsr((cpsr & !allowed) | (value & allowed) | forced);
            }
        }
        false
    }
}
