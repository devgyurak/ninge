// SPDX-License-Identifier: Apache-2.0
//! Thumb-state execution (#13). Note: `docs/hw/thumb.md`.
//!
//! The loads, stores and block transfers run the ARM handlers on the equivalent ARM word: the
//! hardware semantics (alignment, rotation, the base in the register list, the empty list) are
//! the same, so there is one implementation of each (`arm_transfer.rs`).

use super::alu::{self, Alu};
use super::arm::{THUMB_HALFWORD, THUMB_PC_AHEAD};
use super::exception::Exception;
use super::psr::{FLAG_C, FLAG_V, condition_holds, with_flags};
use super::shifter::{ShiftKind, shift_immediate, shift_register};
use super::thumb_decode::{ThumbKind, decode};
use super::{Bus, Cpu};

/// R13 and R14.
const SP: usize = 13;
const LR: usize = 14;
/// R15's index.
const PC: u32 = 15;
/// A word offset is scaled by 4; a halfword offset by 2.
const WORD_SHIFT: u32 = 2;
const HALFWORD_SHIFT: u32 = 1;
/// BL's prefix offset is shifted left by 12, its suffix offset by 1.
const PREFIX_SHIFT: u32 = 12;
/// An 11-bit offset moved to the top of a word and shifted back arithmetically is sign-extended
/// and, by shifting one bit less, doubled (or shifted by 12 for the prefix).
const OFFSET11_TO_TOP: u32 = 21;
/// The ARM data-processing opcodes the Thumb operations map onto (`alu.rs`).
const ARM_SUB: u32 = 2;
const ARM_RSB: u32 = 3;
const ARM_ADD: u32 = 4;
const ARM_CMP: u32 = 10;
const ARM_MOV: u32 = 13;
/// Format 3's `op` (bits 12–11) as an ARM opcode: MOV, CMP, ADD, SUB.
const IMMEDIATE_OPCODES: [u32; 4] = [ARM_MOV, ARM_CMP, ARM_ADD, ARM_SUB];
/// Format 4's `op` values that are not an ARM opcode of the same number.
const OP_LSL: u32 = 2;
const OP_LSR: u32 = 3;
const OP_ASR: u32 = 4;
const OP_ROR: u32 = 7;
const OP_TST: u32 = 8;
const OP_NEG: u32 = 9;
const OP_CMP: u32 = 10;
const OP_CMN: u32 = 11;
const OP_MUL: u32 = 13;
/// Format 5's `op`.
const HI_ADD: u32 = 0;
const HI_CMP: u32 = 1;
/// Format 8 as the ARM halfword-transfer fields, indexed by bits 11–10 (H, S): the L bit and the
/// two SH bits.
const SIGNED_AS_ARM: [(u32, u32); 4] = [(0, 0b01), (1, 0b10), (1, 0b01), (1, 0b11)];
/// The ARM words the Thumb transfers stand for, with every field but Rn, Rd, the offset and
/// L/B cleared: LDR and STR with a register offset (`cond 011 PUBWL`, P and U set), the same
/// with a 12-bit immediate, STRH and LDRH with a register offset and with an 8-bit immediate,
/// STMIA, LDMIA, PUSH (STMDB SP!) and POP (LDMIA SP!), all with condition AL.
const ARM_SINGLE_REGISTER: u32 = 0xE780_0000;
const ARM_SINGLE_IMMEDIATE: u32 = 0xE580_0000;
const ARM_HALFWORD_REGISTER: u32 = 0xE180_0090;
const ARM_HALFWORD_IMMEDIATE: u32 = 0xE1C0_00B0;
const ARM_STMIA: u32 = 0xE8A0_0000;
const ARM_LDMIA: u32 = 0xE8B0_0000;
const ARM_PUSH: u32 = 0xE92D_0000;
const ARM_POP: u32 = 0xE8BD_0000;
/// Bits of an ARM word: B at 22, W at 21, L at 20, Rn at 16, Rd at 12, and the halfword form's
/// SH at 5.
const ARM_B: u32 = 22;
const ARM_W: u32 = 21;
const ARM_L: u32 = 20;
const ARM_RN: u32 = 16;
const ARM_RD: u32 = 12;
const ARM_SH: u32 = 5;
/// The register-list bits of LR (PUSH) and PC (POP).
const LIST_LR: u32 = 14;
const LIST_PC: u32 = 15;
/// The BLX suffix clears the low two bits of its target.
const BLX_WORD_ALIGN: u32 = !3;
/// Format 6 and 12 use the PC with bit 1 cleared.
const PC_WORD_ALIGN: u32 = !2;

impl<const ARM9: bool> Cpu<ARM9> {
    /// Executes the Thumb instruction at the head of the pipeline
    /// (`docs/hw/thumb.md`, "State and registers"; the pipeline is
    /// `docs/hw/arm-branch-pipeline.md`).
    // TODO(hw-unverified): the fetch sequence (one halfword fetch at R15 before execution, two
    // on a refill) is the suite's, oracle evidence only (`docs/hw/thumb.md`, "Unverified").
    pub(super) fn step_thumb<B: Bus>(&mut self, bus: &mut B) {
        // The pipeline holds a Thumb opcode in its low halfword.
        let halfword = self.pipeline[0] as u16; // truncation: a Thumb opcode is 16 bits
        let word = u32::from(halfword);
        let fetched = bus.fetch16(self.regs.get(15));
        self.pipeline = [self.pipeline[1], u32::from(fetched)];
        let wrote_pc = match decode::<ARM9>(halfword) {
            ThumbKind::ShiftImmediate => self.shift_by_immediate(word),
            ThumbKind::AddSub => self.add_sub(word),
            ThumbKind::Immediate => self.immediate(word),
            ThumbKind::Alu => self.alu_register(word),
            ThumbKind::HiRegister => self.hi_register(word),
            ThumbKind::BranchExchange => self.branch_exchange_thumb(word),
            ThumbKind::BranchLinkExchange => self.branch_link_exchange_thumb(word),
            ThumbKind::LoadPc => self.load_pc(bus, word),
            ThumbKind::TransferRegister => {
                let arm = ARM_SINGLE_REGISTER
                    | (word >> 10 & 1) << ARM_B
                    | (word >> 11 & 1) << ARM_L
                    | transfer_registers(word)
                    | (word >> 6 & 7);
                self.single_transfer(bus, arm)
            }
            ThumbKind::TransferSigned => {
                let (load, sh) = SIGNED_AS_ARM[(word >> 10 & 3) as usize];
                let arm = ARM_HALFWORD_REGISTER
                    | load << ARM_L
                    | sh << ARM_SH
                    | transfer_registers(word)
                    | (word >> 6 & 7);
                self.halfword_transfer(bus, arm)
            }
            ThumbKind::TransferImmediate => {
                // Bit 12 is B: a byte transfer is unscaled, a word transfer is scaled by 4.
                let byte = word >> 12 & 1;
                let offset = (word >> 6 & 0x1F) << (WORD_SHIFT * (1 - byte));
                let arm = ARM_SINGLE_IMMEDIATE
                    | byte << ARM_B
                    | (word >> 11 & 1) << ARM_L
                    | transfer_registers(word)
                    | offset;
                self.single_transfer(bus, arm)
            }
            ThumbKind::TransferHalfword => {
                let offset = (word >> 6 & 0x1F) << HALFWORD_SHIFT;
                // The ARM immediate is split in two nibbles, bits 11–8 and 3–0.
                let arm = ARM_HALFWORD_IMMEDIATE
                    | (word >> 11 & 1) << ARM_L
                    | transfer_registers(word)
                    | (offset >> 4) << 8
                    | (offset & 0xF);
                self.halfword_transfer(bus, arm)
            }
            ThumbKind::TransferSp => {
                let arm = ARM_SINGLE_IMMEDIATE
                    | (word >> 11 & 1) << ARM_L
                    | (SP as u32) << ARM_RN
                    | (word >> 8 & 7) << ARM_RD
                    | (word & 0xFF) << WORD_SHIFT;
                self.single_transfer(bus, arm)
            }
            ThumbKind::LoadAddress => self.load_address(word),
            ThumbKind::AdjustSp => self.adjust_sp(word),
            ThumbKind::PushPop => {
                // Bit 8 adds LR to a PUSH and PC to a POP.
                let extra = word >> 8 & 1;
                let arm = match word >> 11 & 1 {
                    1 => ARM_POP | extra << LIST_PC | (word & 0xFF),
                    _ => ARM_PUSH | extra << LIST_LR | (word & 0xFF),
                };
                self.block_transfer(bus, arm)
            }
            ThumbKind::BlockTransfer => {
                let rb = word >> 8 & 7;
                let in_list = word >> rb & 1 == 1;
                // Thumb LDMIA with Rb in the list never writes back, on the ARM9 too, unlike
                // the ARM form (`docs/hw/thumb.md`, "LDMIA and STMIA"), so W is left out.
                let base = match (word >> 11 & 1, in_list) {
                    (1, true) => ARM_LDMIA & !(1 << ARM_W),
                    (1, false) => ARM_LDMIA,
                    _ => ARM_STMIA,
                };
                self.block_transfer(bus, base | rb << ARM_RN | (word & 0xFF))
            }
            ThumbKind::ConditionalBranch => self.conditional_branch(word),
            ThumbKind::Swi => self.enter(Exception::SoftwareInterrupt),
            ThumbKind::Breakpoint => self.enter(Exception::PrefetchAbort),
            ThumbKind::Branch => self.branch_thumb(word),
            ThumbKind::LongBranchPrefix => self.long_branch_prefix(word),
            ThumbKind::LongBranchSuffix => self.long_branch_suffix(word),
            ThumbKind::LongBranchExchangeSuffix => self.long_branch_exchange_suffix(word),
            ThumbKind::Unhandled => false,
        };
        // A write to the PC flushes the pipeline; everything else steps to the next instruction.
        match wrote_pc {
            true => {
                let target = self.regs.get(15);
                self.refill(bus);
                // TODO(hw-unverified): a PC load from an empty register list leaves R15 as the
                // loaded value + 4 with bit 0 kept, unlike every other Thumb refill (suite;
                // `docs/hw/thumb.md`, "Unverified").
                if loads_pc_from_empty_list(halfword) {
                    self.regs.set(15, target.wrapping_add(THUMB_PC_AHEAD));
                }
            }
            false => self
                .regs
                .set(15, self.regs.get(15).wrapping_add(THUMB_HALFWORD)),
        }
    }

    /// The flags a Thumb data-processing instruction leaves: N and Z from the result, C and V as
    /// the operation computed them.
    fn set_flags(&mut self, result: Alu) {
        let cpsr = self.regs.cpsr();
        self.regs.set_cpsr(with_flags(
            cpsr,
            result.result >> 31 == 1,
            result.result == 0,
            result.carry,
            result.overflow,
        ));
    }

    /// The current C and V, for an operation that computes neither.
    fn carry_and_overflow(&self) -> (bool, bool) {
        let cpsr = self.regs.cpsr();
        (cpsr & FLAG_C != 0, cpsr & FLAG_V != 0)
    }

    /// Format 1: LSL, LSR and ASR by an immediate; N, Z, C update.
    fn shift_by_immediate(&mut self, word: u32) -> bool {
        let (carry, overflow) = self.carry_and_overflow();
        let (value, shifter_carry) = shift_immediate(
            ShiftKind::from_bits(word >> 11),
            self.regs.get((word >> 3 & 7) as usize),
            word >> 6 & 0x1F,
            carry,
        );
        let result = alu::execute(ARM_MOV, 0, value, shifter_carry, carry, overflow);
        self.regs.set((word & 7) as usize, result.result);
        self.set_flags(result);
        false
    }

    /// Format 2: ADD and SUB of a register or a 3-bit immediate; N, Z, C, V update.
    fn add_sub(&mut self, word: u32) -> bool {
        let (carry, overflow) = self.carry_and_overflow();
        let field = word >> 6 & 7;
        // Bit 10 is I: the field is an immediate rather than a register.
        let operand = match word >> 10 & 1 {
            1 => field,
            _ => self.regs.get(field as usize),
        };
        // Bit 9 is the subtract bit.
        let opcode = match word >> 9 & 1 {
            1 => ARM_SUB,
            _ => ARM_ADD,
        };
        let result = alu::execute(
            opcode,
            self.regs.get((word >> 3 & 7) as usize),
            operand,
            carry,
            carry,
            overflow,
        );
        self.regs.set((word & 7) as usize, result.result);
        self.set_flags(result);
        false
    }

    /// Format 3: MOV, CMP, ADD, SUB with an 8-bit immediate; N, Z update, and C, V too except
    /// for MOV.
    fn immediate(&mut self, word: u32) -> bool {
        let (carry, overflow) = self.carry_and_overflow();
        let opcode = IMMEDIATE_OPCODES[(word >> 11 & 3) as usize];
        let rd = (word >> 8 & 7) as usize;
        let result = alu::execute(
            opcode,
            self.regs.get(rd),
            word & 0xFF,
            carry,
            carry,
            overflow,
        );
        match opcode {
            ARM_CMP => {}
            _ => self.regs.set(rd, result.result),
        }
        self.set_flags(result);
        false
    }

    /// Format 4: the sixteen register ALU operations; N and Z always update.
    // TODO(hw-unverified): MUL leaves C and V unchanged. C after MUL is meaningless on ARMv4T
    // per GBATEK and the suite shows it changing (#81, the same gap as the ARM multiply); V is
    // unchanged (`docs/hw/thumb.md`, "Unverified").
    fn alu_register(&mut self, word: u32) -> bool {
        let (carry, overflow) = self.carry_and_overflow();
        let op = word >> 6 & 0xF;
        let rd = (word & 7) as usize;
        let a = self.regs.get(rd);
        let b = self.regs.get((word >> 3 & 7) as usize);
        let shift = |kind| shift_register(kind, a, b & 0xFF, carry);
        let result = match op {
            OP_LSL | OP_LSR | OP_ASR | OP_ROR => {
                let (value, shifter_carry) = shift(match op {
                    OP_LSL => ShiftKind::Lsl,
                    OP_LSR => ShiftKind::Lsr,
                    OP_ASR => ShiftKind::Asr,
                    _ => ShiftKind::Ror,
                });
                alu::execute(ARM_MOV, 0, value, shifter_carry, carry, overflow)
            }
            // NEG is 0 - Rs: a reverse subtract with Rs as the left operand.
            OP_NEG => alu::execute(ARM_RSB, b, 0, carry, carry, overflow),
            OP_MUL => Alu {
                result: a.wrapping_mul(b),
                carry,
                overflow,
            },
            // The rest are the ARM data-processing opcode of the same number.
            _ => alu::execute(op, a, b, carry, carry, overflow),
        };
        match op {
            OP_TST | OP_CMP | OP_CMN => {}
            _ => self.regs.set(rd, result.result),
        }
        self.set_flags(result);
        false
    }

    /// Format 5: ADD, CMP and MOV on any of R0–R15. Only CMP sets flags. True if it wrote the PC.
    // TODO(hw-unverified): H1 = H2 = 0 is undefined on ARMv4T per GBATEK; it runs here as the
    // same operation on low registers, which is what the suite shows
    // (`docs/hw/thumb.md`, "Unverified").
    fn hi_register(&mut self, word: u32) -> bool {
        // H1 (bit 7) and H2 (bit 6) are the high bits of Rd and Rs.
        let rd = (word & 7) | (word >> 4 & 8);
        let rs = word >> 3 & 0xF;
        let a = self.operand(rd, 0);
        let b = self.operand(rs, 0);
        match word >> 8 & 3 {
            HI_CMP => {
                let (carry, overflow) = self.carry_and_overflow();
                self.set_flags(alu::execute(ARM_CMP, a, b, carry, carry, overflow));
                false
            }
            op => {
                let value = match op {
                    HI_ADD => a.wrapping_add(b),
                    _ => b,
                };
                self.write_register(rd, value)
            }
        }
    }

    /// Writes a register; R15 takes the value and the step refills from it. True if R15.
    fn write_register(&mut self, index: u32, value: u32) -> bool {
        match index {
            PC => self.write_pc(value),
            _ => self.regs.set(index as usize, value),
        }
        index == PC
    }

    /// BX Rs (`docs/hw/thumb.md`, "Format 5 and BX").
    // TODO(hw-unverified): GBATEK says BX R15 aligns the PC as ((address + 4) AND NOT 2) and
    // wants Rd bits zero, but the suite shows R15 keeping bit 1 and non-zero Rd bits running as
    // BX (1,607 cases with the alignment tried). We follow the suite
    // (`docs/hw/thumb.md`, "Unverified").
    // TODO(hw-unverified): H1 = 1 runs as BX here on the ARM7; the suite shows it on ARMv4T
    // (`docs/hw/thumb.md`, "Unverified"). The ARM9 decodes it as BLX.
    fn branch_exchange_thumb(&mut self, word: u32) -> bool {
        let target = self.operand(word >> 3 & 0xF, 0);
        self.exchange_to(target)
    }

    /// Format 6: LDR Rd, [PC, #imm * 4] from the word-aligned PC (`docs/hw/thumb.md`,
    /// "PC-relative forms").
    fn load_pc<B: Bus>(&mut self, bus: &mut B, word: u32) -> bool {
        let address =
            (self.operand(PC, 0) & PC_WORD_ALIGN).wrapping_add((word & 0xFF) << WORD_SHIFT);
        let value = bus.read32(address);
        self.regs.set((word >> 8 & 7) as usize, value);
        false
    }

    /// Format 12: ADD Rd, PC or SP, #imm * 4; no flags.
    fn load_address(&mut self, word: u32) -> bool {
        // Bit 11 selects SP; the PC form uses the word-aligned PC.
        let base = match word >> 11 & 1 {
            1 => self.regs.get(SP),
            _ => self.operand(PC, 0) & PC_WORD_ALIGN,
        };
        self.regs.set(
            (word >> 8 & 7) as usize,
            base.wrapping_add((word & 0xFF) << WORD_SHIFT),
        );
        false
    }

    /// Format 13: ADD SP, #imm * 4 or SUB SP, #imm * 4; no flags.
    fn adjust_sp(&mut self, word: u32) -> bool {
        let offset = (word & 0x7F) << WORD_SHIFT;
        let sp = self.regs.get(SP);
        // Bit 7 is the subtract bit.
        self.regs.set(
            SP,
            match word >> 7 & 1 {
                1 => sp.wrapping_sub(offset),
                _ => sp.wrapping_add(offset),
            },
        );
        false
    }

    /// Format 16: B with a condition; taken, R15 += the signed offset * 2.
    fn conditional_branch(&mut self, word: u32) -> bool {
        match condition_holds(word >> 8 & 0xF, self.regs.cpsr()) {
            true => {
                // The casts are the signed reading of the 8-bit offset.
                let offset = i32::from(word as u8 as i8) << HALFWORD_SHIFT;
                self.write_pc(self.regs.get(15).wrapping_add_signed(offset));
                true
            }
            false => false,
        }
    }

    /// Format 18: B; R15 += the signed 11-bit offset * 2.
    fn branch_thumb(&mut self, word: u32) -> bool {
        let offset = offset11(word, HALFWORD_SHIFT);
        self.write_pc(self.regs.get(15).wrapping_add(offset));
        true
    }

    /// Format 19, first halfword: LR = R15 + the signed offset << 12. No branch.
    fn long_branch_prefix(&mut self, word: u32) -> bool {
        let offset = offset11(word, PREFIX_SHIFT);
        self.regs.set(LR, self.regs.get(15).wrapping_add(offset));
        false
    }

    /// Format 19, second halfword: jump to LR + the offset << 1, and leave the return address
    /// with bit 0 set in LR.
    fn long_branch_suffix(&mut self, word: u32) -> bool {
        // R15 is this halfword's address + 4; the next instruction is two bytes past it.
        let next = self.regs.get(15).wrapping_sub(THUMB_HALFWORD);
        let target = self
            .regs
            .get(LR)
            .wrapping_add((word & 0x7FF) << HALFWORD_SHIFT);
        self.regs.set(LR, next | 1);
        self.write_pc(target);
        true
    }

    /// Format 19, second halfword of BLX (ARM9; `docs/hw/armv5te.md`, "BLX"): like the BL suffix,
    /// but the target is word aligned and the core enters ARM state.
    // TODO(hw-unverified): offset bit 0 set is dropped by the alignment
    // (`docs/hw/armv5te.md`, "Unverified").
    fn long_branch_exchange_suffix(&mut self, word: u32) -> bool {
        let next = self.regs.get(15).wrapping_sub(THUMB_HALFWORD);
        let target = self
            .regs
            .get(LR)
            .wrapping_add((word & 0x7FF) << HALFWORD_SHIFT)
            & BLX_WORD_ALIGN;
        self.regs.set(LR, next | 1);
        // Bit 0 of the aligned target is 0, which selects ARM state.
        self.exchange_to(target)
    }
}

/// Whether the halfword is a POP or LDMIA with an empty register list, which loads R15 (on the
/// ARM7; the ARM9 loads nothing, so the caller only asks after a PC write).
fn loads_pc_from_empty_list(halfword: u16) -> bool {
    let word = u32::from(halfword);
    // POP: bits 15–9 = 1011110 and R (bit 8) clear; LDMIA: bits 15–11 = 11001.
    let pop = word >> 8 == 0b1011_1100;
    let ldmia = word >> 11 == 0b11001;
    (pop || ldmia) && word & 0xFF == 0
}

/// The Rb and Rd fields of formats 7–10 (bits 5–3 and 2–0) as the ARM Rn and Rd fields.
fn transfer_registers(word: u32) -> u32 {
    (word >> 3 & 7) << ARM_RN | (word & 7) << ARM_RD
}

/// The 11-bit offset field sign-extended and shifted left by `shift` (at most 12).
fn offset11(word: u32, shift: u32) -> u32 {
    // Moving the field to the top and shifting back arithmetically sign-extends it; the casts
    // are the signed reading.
    (((word & 0x7FF) << OFFSET11_TO_TOP) as i32 >> (OFFSET11_TO_TOP - shift)) as u32
}
