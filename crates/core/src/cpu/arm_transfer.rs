// SPDX-License-Identifier: Apache-2.0
//! ARM-state loads and stores (#10). Note: `docs/hw/arm-load-store.md`.
//!
//! Every access passes the effective address to the bus unchanged, low bits included; a load
//! rotates or sign-extends from the low bits of that address and a store writes its register
//! unrotated (`docs/hw/arm-load-store.md`, "Alignment"; measured on the suite).

use super::psr::FLAG_C;
use super::regs::Bank;
use super::shifter::{ShiftKind, shift_immediate};
use super::{Bus, Cpu};

/// Bytes in a word, the step between the registers of a block transfer.
const WORD: u32 = 4;
/// What an empty register list moves the base by, in words (0x40 bytes).
const EMPTY_LIST_WORDS: u32 = 16;
/// R15's index and its bit in a register list.
const PC: u32 = 15;
/// R14, which LDRD and STRD cannot use as Rd (its pair would include R15).
const LR: u32 = 14;

impl<const ARM9: bool> Cpu<ARM9> {
    /// LDR, STR, LDRB, STRB and the T forms; true if it wrote the PC.
    // On the ARM9 a word load into R15 switches state on bit 0 (`docs/hw/armv5te.md`, "Switching
    // to Thumb on a load into R15"). TODO(hw-unverified): the T forms (LDRT, STRT) run as the
    // plain forms because the bus has no privilege (revisit with CP15 protection); Rn = R15
    // with writeback is not allowed by GBATEK and follows the suite
    // (`docs/hw/arm-load-store.md`, "Unverified").
    pub(super) fn single_transfer<B: Bus>(&mut self, bus: &mut B, word: u32) -> bool {
        let (rn, rd) = ((word >> 16) & 0xF, (word >> 12) & 0xF);
        // Bit 25 is I: a register offset shifted by an immediate; otherwise a 12-bit immediate.
        let offset = match word >> 25 & 1 {
            1 => {
                let carry = self.regs.cpsr() & FLAG_C != 0;
                let shifted = shift_immediate(
                    ShiftKind::from_bits(word >> 5),
                    self.operand(word & 0xF, 0),
                    (word >> 7) & 0x1F,
                    carry,
                );
                shifted.0
            }
            _ => word & 0xFFF,
        };
        let (address, moved, writeback) = self.transfer_address(word, rn, offset);
        let byte = word >> 22 & 1 == 1;
        match word >> 20 & 1 {
            1 => {
                let value = match byte {
                    true => u32::from(bus.read8(address)),
                    false => rotate_word(bus.read32(address), address),
                };
                let flushed = self.write_back_single(writeback, rn, moved);
                // Only a word load interworks.
                // TODO(hw-unverified): a loaded target with low bits `10` keeps bit 1 in R15, and
                // an LDR into R15 from an address that is not word aligned (the ARM ARM calls it
                // unpredictable) switches state on the rotated word
                // (`docs/hw/armv5te.md`, "Unverified").
                match (ARM9, byte, rd) {
                    (true, false, PC) => self.exchange_to(value) || flushed,
                    _ => self.write_loaded(rd, value) || flushed,
                }
            }
            _ => {
                let value = self.store_operand(rd);
                match byte {
                    true => bus.write8(address, value as u8), // truncation: a byte store
                    false => bus.write32(address, value),
                }
                self.write_back_single(writeback, rn, moved)
            }
        }
    }

    /// LDRH, STRH, LDRSB and LDRSH; true if it wrote the PC.
    // TODO(hw-unverified): the ARM9 odd-address LDRH and LDRSH (see `halfword_value`); a P = 0
    // form with bit 21 set writes back as any post-indexed form does, which the suite shows
    // although GBATEK says the bit must be zero (`docs/hw/arm-load-store.md`, "Unverified").
    pub(super) fn halfword_transfer<B: Bus>(&mut self, bus: &mut B, word: u32) -> bool {
        let (rn, rd) = ((word >> 16) & 0xF, (word >> 12) & 0xF);
        // Bit 22 is I: the immediate is split in two nibbles; otherwise Rm is the offset.
        let offset = match word >> 22 & 1 {
            1 => ((word >> 4) & 0xF0) | (word & 0xF),
            _ => self.operand(word & 0xF, 0),
        };
        let (address, moved, writeback) = self.transfer_address(word, rn, offset);
        match word >> 20 & 1 {
            1 => {
                // Bits 6-5: 01 halfword, 10 signed byte, 11 signed halfword.
                let value = match (word >> 5) & 3 {
                    2 => i32::from(bus.read8(address) as i8) as u32, // sign extension
                    kind => halfword_value(ARM9, kind == 3, address, bus.read16(address)),
                };
                let flushed = self.write_back_single(writeback, rn, moved);
                self.write_loaded(rd, value) || flushed
            }
            _ => {
                bus.write16(address, self.store_operand(rd) as u16); // truncation: a halfword store
                self.write_back_single(writeback, rn, moved)
            }
        }
    }

    /// LDRD and STRD (`docs/hw/armv5te.md`, "LDRD and STRD"): the halfword form's addressing, two
    /// words at the address and the address + 4 to or from Rd and Rd + 1. Bit 5 distinguishes
    /// the store (SH = 11) from the load (SH = 10). True if it wrote the PC.
    // TODO(hw-unverified): R15 as Rm is used like any register (Rn = R15 reads the +8 view and
    // is allowed); an odd Rd or R14 is not executed, a doubleword address that is not 8-byte
    // aligned reaches the bus as given, and with writeback to Rd or Rd + 1 the loaded value
    // wins (`docs/hw/armv5te.md`, "Unverified").
    pub(super) fn double_transfer<B: Bus>(&mut self, bus: &mut B, word: u32) -> bool {
        let (rn, rd) = ((word >> 16) & 0xF, (word >> 12) & 0xF);
        // Bit 22 is I: the split immediate, or Rm.
        let offset = match word >> 22 & 1 {
            1 => ((word >> 4) & 0xF0) | (word & 0xF),
            _ => self.operand(word & 0xF, 0),
        };
        match (rd & 1, rd) {
            (1, _) | (_, LR) => false,
            _ => {
                let (address, moved, writeback) = self.transfer_address(word, rn, offset);
                let second = rd as usize + 1;
                match word >> 5 & 1 {
                    0 => {
                        let first_value = bus.read32(address);
                        let second_value = bus.read32(address.wrapping_add(WORD));
                        let flushed = self.write_back_single(writeback, rn, moved);
                        self.regs.set(rd as usize, first_value);
                        self.regs.set(second, second_value);
                        flushed
                    }
                    _ => {
                        bus.write32(address, self.regs.get(rd as usize));
                        bus.write32(address.wrapping_add(WORD), self.regs.get(second));
                        self.write_back_single(writeback, rn, moved)
                    }
                }
            }
        }
    }

    /// SWP and SWPB; true if it wrote the PC.
    // TODO(hw-unverified): R15 is not allowed in any field; Rd = R15 is written like a load.
    pub(super) fn swap<B: Bus>(&mut self, bus: &mut B, word: u32) -> bool {
        let (rn, rd, rm) = ((word >> 16) & 0xF, (word >> 12) & 0xF, word & 0xF);
        // TODO(hw-unverified): the suite reads an R15 Rn or Rm as address + 12, though GBATEK
        // does not allow R15 here.
        let address = self.store_operand(rn);
        let source = self.store_operand(rm);
        // Bit 22 is B. The read comes first and the write goes to the same raw address.
        let value = match word >> 22 & 1 {
            1 => {
                let old = u32::from(bus.read8(address));
                bus.write8(address, source as u8); // truncation: the low byte
                old
            }
            _ => {
                let old = rotate_word(bus.read32(address), address);
                bus.write32(address, source);
                old
            }
        };
        self.write_loaded(rd, value)
    }

    /// LDM and STM; true if it wrote the PC.
    // On the ARM9 an LDM of R15 without S switches state on bit 0 (`docs/hw/armv5te.md`,
    // "Switching to Thumb on a load into R15"). TODO(hw-unverified): the ARM9 rules for the base
    // in the list and for the empty list come from GBATEK's prose and no suite case checks
    // them; the empty list's transfer address is measured only for STMDB, and its base
    // adjustment without writeback is taken as none;
    // S together with writeback and an LDM with R15 and S in User/System (no SPSR, so the CPSR
    // stays) follow the suite (`docs/hw/arm-load-store.md`, "Unverified").
    pub(super) fn block_transfer<B: Bus>(&mut self, bus: &mut B, word: u32) -> bool {
        let rn = (word >> 16) & 0xF;
        let (pre, up) = (word >> 24 & 1 == 1, word >> 23 & 1 == 1);
        let (set_s, writeback, load) = (
            word >> 22 & 1 == 1,
            word >> 21 & 1 == 1,
            word >> 20 & 1 == 1,
        );
        let mut list = word & 0xFFFF;
        // An empty list moves the base by 0x40. ARM7 also transfers R15; ARM9 transfers nothing.
        let span = match list {
            0 => EMPTY_LIST_WORDS,
            _ => list.count_ones(),
        };
        list = match (list == 0, ARM9) {
            (true, false) => 1 << PC,
            _ => list,
        };
        let base = self.regs.get(rn as usize);
        let start = block_start(pre, up, span, base);
        let moved = match up {
            true => base.wrapping_add(span * WORD),
            false => base.wrapping_sub(span * WORD),
        };
        // Bit 22 without a load of R15 reads or writes the User bank (a load of R15 restores
        // the CPSR instead).
        let loads_pc = load && list >> PC & 1 == 1;
        let user_bank = set_s && !loads_pc;
        let in_list = list >> rn & 1 == 1;
        let first = list.trailing_zeros() == rn;
        let last = list.checked_ilog2() == Some(rn);
        let mut address = start;
        let mut pc_value = None;
        for index in 0..=PC {
            if list >> index & 1 == 0 {
                continue;
            }
            match (load, index) {
                (true, PC) => pc_value = Some(bus.read32(address)),
                (true, _) => {
                    let value = bus.read32(address);
                    match user_bank {
                        true => self
                            .regs
                            .set_banked(Bank::UserSystem, index as usize, value),
                        false => self.regs.set(index as usize, value),
                    }
                }
                (false, _) => {
                    let value = self.block_store_value(index, user_bank);
                    // STM with the base in the list: ARM7 stores the written-back base unless
                    // it is the first register; ARM9 always stores the old base.
                    let value = match writeback && index == rn && !first && !ARM9 {
                        true => moved,
                        false => value,
                    };
                    bus.write32(address, value);
                }
            }
            address = address.wrapping_add(WORD);
        }
        // LDM with the base in the list: ARM7 never writes back; ARM9 does unless the base is
        // the last register of a list longer than one.
        let skip = load && in_list && (!ARM9 || (last && list.count_ones() > 1));
        self.write_back(writeback && !skip, rn, moved, user_bank);
        // TODO(hw-unverified): a block transfer whose written-back Rn is R15 flushes to the moved
        // address, so R15 ends as the value + 8 (suite; `docs/hw/arm-branch-pipeline.md`).
        let wrote_base_pc = writeback && !skip && rn == PC;
        match pc_value {
            Some(value) => {
                // The PSR-restore form: S with R15 in an LDM list, in a mode with an SPSR; T then
                // comes from the SPSR. Otherwise the ARM9 takes T from the loaded bit 0.
                // TODO(hw-unverified): S in User/System (no SPSR) switches on bit 0 on the ARM9
                // (`docs/hw/armv5te.md`, "Unverified").
                match self.regs.spsr(self.regs.bank()).filter(|_| set_s) {
                    Some(spsr) => {
                        self.regs.set_cpsr(spsr);
                        self.write_pc(value);
                    }
                    // TODO(hw-unverified): a loaded target with low bits `10` keeps bit 1 in R15
                    // (`docs/hw/armv5te.md`, "Unverified").
                    None if ARM9 => {
                        self.exchange_to(value);
                    }
                    None => self.write_pc(value),
                }
                true
            }
            None => wrote_base_pc,
        }
    }

    /// The access address, the moved base and whether the base is written back, for a single
    /// or halfword transfer. P (bit 24) selects pre-indexing; U (bit 23) the direction; bit 21
    /// is W for pre-indexed forms, and post-indexed forms always write back.
    fn transfer_address(&self, word: u32, rn: u32, offset: u32) -> (u32, u32, bool) {
        let base = self.operand(rn, 0);
        let moved = match word >> 23 & 1 {
            1 => base.wrapping_add(offset),
            _ => base.wrapping_sub(offset),
        };
        match word >> 24 & 1 {
            1 => (moved, moved, word >> 21 & 1 == 1),
            _ => (base, moved, true),
        }
    }

    /// Writes the moved base back to Rn when the form asks for it. `user_bank` is the S bit of a
    /// block transfer: the suite writes the User-bank Rn then, not the current mode's.
    // TODO(hw-unverified): a block writeback to R15 flushes to the moved address (suite; GBATEK
    // does not allow R15 as a base with writeback), and the S-bit writeback into the User bank
    // is the suite's behavior where GBATEK says not to use it (`docs/hw/arm-load-store.md`,
    // "Unverified"; `docs/hw/arm-branch-pipeline.md`).
    fn write_back(&mut self, writeback: bool, rn: u32, moved: u32, user_bank: bool) {
        match (writeback, rn, user_bank) {
            (false, _, _) => {}
            (true, PC, _) => self.write_pc(moved),
            (true, _, true) => self.regs.set_banked(Bank::UserSystem, rn as usize, moved),
            (true, _, false) => self.regs.set(rn as usize, moved),
        }
    }

    /// The writeback of a single or halfword transfer. Unlike a block transfer, a written-back R15
    /// flushes to the moved address + 4, so R15 ends as the moved address + 12. True if it wrote
    /// the PC.
    // TODO(hw-unverified): the suite's flush target for this form; GBATEK does not allow R15 as a
    // base with writeback (`docs/hw/arm-branch-pipeline.md`, "Unverified"). Decided in #NN.
    fn write_back_single(&mut self, writeback: bool, rn: u32, moved: u32) -> bool {
        match (writeback, rn) {
            (false, _) => false,
            (true, PC) => {
                self.write_pc(moved.wrapping_add(WORD));
                true
            }
            (true, _) => {
                self.regs.set(rn as usize, moved);
                false
            }
        }
    }

    /// Writes a loaded value to Rd; R15 takes the value and the step refills from it. True if
    /// R15.
    fn write_loaded(&mut self, rd: u32, value: u32) -> bool {
        match rd {
            PC => self.write_pc(value),
            _ => self.regs.set(rd as usize, value),
        }
        rd == PC
    }

    /// The value STM writes for register `index`: R15 reads address + 12, and with the S bit the
    /// other registers come from the User bank.
    // TODO(hw-unverified): STM with S = 1 and R15 in the list stores address + 12 as a plain STM
    // does (`docs/hw/arm-load-store.md`, "Unverified").
    fn block_store_value(&self, index: u32, user_bank: bool) -> u32 {
        match (index, user_bank) {
            (PC, _) => self.store_operand(PC),
            (_, true) => self.regs.banked(Bank::UserSystem, index as usize),
            _ => self.regs.get(index as usize),
        }
    }
}

/// An LDR or SWP word read at `address`: the bus value rotated right by 8 × (address & 3)
/// (`docs/hw/arm-load-store.md`, "Alignment").
pub(super) fn rotate_word(value: u32, address: u32) -> u32 {
    value.rotate_right(8 * (address & 3))
}

/// The register value of LDRH (`signed` = false) or LDRSH (`signed` = true) for the halfword
/// the bus returned at `address`; ARM7 and ARM9 differ at an odd address
/// (`docs/hw/arm-load-store.md`, "Alignment").
// TODO(hw-unverified): the ARM9 column is GBATEK's prose; the suite is ARMv4T only, so a
// self-written unit test is its only check (`docs/hw/arm-load-store.md`, "Unverified").
pub(super) fn halfword_value(arm9: bool, signed: bool, address: u32, data: u16) -> u32 {
    match (signed, arm9, address & 1 == 1) {
        (false, false, true) => u32::from(data).rotate_right(8),
        (false, _, _) => u32::from(data),
        // ARM7 LDRSH at an odd address is an LDRSB of the byte at that address: the high byte.
        (true, false, true) => i32::from((data >> 8) as u8 as i8) as u32,
        (true, _, _) => i32::from(data as i16) as u32,
    }
}

/// The lowest address an LDM or STM touches, from P, U, the register count and the base.
pub(super) fn block_start(pre: bool, up: bool, count: u32, base: u32) -> u32 {
    match (pre, up) {
        (false, true) => base,
        (true, true) => base.wrapping_add(WORD),
        (false, false) => base.wrapping_sub(count * WORD).wrapping_add(WORD),
        (true, false) => base.wrapping_sub(count * WORD),
    }
}
