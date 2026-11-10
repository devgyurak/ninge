// SPDX-License-Identifier: Apache-2.0
//! Thumb instructions through `Cpu::step`. Expected values come from `docs/hw/thumb.md` and are
//! worked out by hand, never from running the implementation: R15 reads the instruction's
//! address + 4, a step that does not write R15 advances it by 2, and a write refills the
//! pipeline with two halfword fetches from the target.
//!
//! The recording bus answers a fetch with the address it was asked for, so the pipeline
//! contents show which address each entry came from.

use crate::cpu::*;
use proptest::prelude::*;

/// Address of the instruction under test.
const ADDR: u32 = 0x1000;
/// R15 while it executes (address + 4).
const PC_READ: u32 = ADDR + 4;
/// The opcode already sitting in `pipeline[1]` before the step.
const NEXT: u32 = 0xAAAA;
/// CPSR of SVC mode, flags clear.
const SVC: u32 = 0x13;
/// CPSR of User mode, flags clear.
const USER: u32 = 0x10;
const T_BIT: u32 = 0x20;
const I_BIT: u32 = 0x80;
const N: u32 = 0x8000_0000;
const Z: u32 = 0x4000_0000;
const C: u32 = 0x2000_0000;
const V: u32 = 0x1000_0000;
const NZCV: u32 = N | Z | C | V;
/// What a data read returns when a test does not choose.
const DATA: u32 = 0x1122_3344;

/// One code fetch or data read: width in bits and address.
type Access = (u8, u32);
/// One data write: width in bits, address and value.
type Write = (u8, u32, u32);

/// A bus that logs every access. Fetches answer with the fetched address; data reads all return
/// `data`.
struct RecBus {
    fetches: Vec<Access>,
    reads: Vec<Access>,
    writes: Vec<Write>,
    data: u32,
}

impl RecBus {
    fn new(data: u32) -> Self {
        Self {
            fetches: Vec::new(),
            reads: Vec::new(),
            writes: Vec::new(),
            data,
        }
    }
}

impl Bus for RecBus {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.fetches.push((32, addr));
        addr
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.fetches.push((16, addr));
        addr as u16
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.reads.push((8, addr));
        self.data as u8
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.reads.push((16, addr));
        self.data as u16
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.reads.push((32, addr));
        self.data
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.writes.push((8, addr, u32::from(value)));
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.writes.push((16, addr, u32::from(value)));
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.writes.push((32, addr, value));
    }
}

/// The CPU after the steps and what the bus saw.
struct Stepped {
    cpu: Cpu<false>,
    bus: RecBus,
}

impl Stepped {
    fn reg(&self, index: usize) -> u32 {
        self.cpu.regs().get(index)
    }
    fn r15(&self) -> u32 {
        self.reg(15)
    }
    fn flags(&self) -> u32 {
        self.cpu.regs().cpsr() & NZCV
    }
    fn thumb(&self) -> bool {
        self.cpu.regs().cpsr() & T_BIT != 0
    }
}

/// Runs `opcodes` in order, the first at `addr`, in Thumb state with `cpsr`.
fn run(addr: u32, cpsr: u32, setup: &[(usize, u32)], data: u32, opcodes: &[u16]) -> Stepped {
    let mut cpu = Cpu::<false>::new();
    cpu.regs_mut().set_cpsr(cpsr | T_BIT);
    setup.iter().for_each(|&(i, v)| cpu.regs_mut().set(i, v));
    cpu.regs_mut().set(15, addr + 4);
    let following = opcodes.get(1).map_or(NEXT, |&o| u32::from(o));
    cpu.set_pipeline([u32::from(opcodes[0]), following]);
    let mut bus = RecBus::new(data);
    opcodes.iter().for_each(|_| cpu.step(&mut bus));
    Stepped { cpu, bus }
}

fn exec_at(addr: u32, cpsr: u32, setup: &[(usize, u32)], data: u32, opcode: u16) -> Stepped {
    run(addr, cpsr, setup, data, &[opcode])
}

fn exec_with(cpsr: u32, setup: &[(usize, u32)], data: u32, opcode: u16) -> Stepped {
    exec_at(ADDR, cpsr, setup, data, opcode)
}

fn exec(cpsr: u32, setup: &[(usize, u32)], opcode: u16) -> Stepped {
    exec_with(cpsr, setup, DATA, opcode)
}

/// The halfword fetch pair of a Thumb refill at `target`, after the step fetch at `ADDR + 4`.
fn thumb_fetches(target: u32) -> Vec<Access> {
    vec![(16, PC_READ), (16, target & !1), (16, (target & !1) + 2)]
}

// Opcode assemblers, one per format of `docs/hw/thumb.md`, "Formats". They only place fields.

/// Format 1: `op` 0 LSL, 1 LSR, 2 ASR.
const fn shift(op: u16, off5: u16, rs: u16, rd: u16) -> u16 {
    op << 11 | off5 << 6 | rs << 3 | rd
}
/// Format 2: `imm` 1 takes `field` as a 3-bit immediate; `sub` 1 subtracts.
const fn add_sub(imm: u16, sub: u16, field: u16, rs: u16, rd: u16) -> u16 {
    0x1800 | imm << 10 | sub << 9 | field << 6 | rs << 3 | rd
}
/// Format 3: `op` 0 MOV, 1 CMP, 2 ADD, 3 SUB.
const fn imm8(op: u16, rd: u16, value: u16) -> u16 {
    0x2000 | op << 11 | rd << 8 | value
}
/// Format 4.
const fn alu(op: u16, rs: u16, rd: u16) -> u16 {
    0x4000 | op << 6 | rs << 3 | rd
}
/// Format 5 with full register numbers: `op` 0 ADD, 1 CMP, 2 MOV, 3 BX (rd ignored).
const fn hi(op: u16, rs: u16, rd: u16) -> u16 {
    0x4400 | op << 8 | (rd >> 3) << 7 | (rs >> 3) << 6 | (rs & 7) << 3 | (rd & 7)
}
/// Format 6.
const fn ldr_pc(rd: u16, word_offset: u16) -> u16 {
    0x4800 | rd << 8 | word_offset
}
/// Format 7.
const fn mem_reg(load: u16, byte: u16, ro: u16, rb: u16, rd: u16) -> u16 {
    0x5000 | load << 11 | byte << 10 | ro << 6 | rb << 3 | rd
}
/// Format 8: `h s` 00 STRH, 01 LDSB, 10 LDRH, 11 LDSH.
const fn mem_signed(h: u16, s: u16, ro: u16, rb: u16, rd: u16) -> u16 {
    0x5200 | h << 11 | s << 10 | ro << 6 | rb << 3 | rd
}
/// Format 9.
const fn mem_imm(byte: u16, load: u16, off5: u16, rb: u16, rd: u16) -> u16 {
    0x6000 | byte << 12 | load << 11 | off5 << 6 | rb << 3 | rd
}
/// Format 10.
const fn mem_half(load: u16, off5: u16, rb: u16, rd: u16) -> u16 {
    0x8000 | load << 11 | off5 << 6 | rb << 3 | rd
}
/// Format 11.
const fn mem_sp(load: u16, rd: u16, word_offset: u16) -> u16 {
    0x9000 | load << 11 | rd << 8 | word_offset
}
/// Format 12: `sp` 1 adds to SP, 0 to the PC.
const fn load_address(sp: u16, rd: u16, word_offset: u16) -> u16 {
    0xA000 | sp << 11 | rd << 8 | word_offset
}
/// Format 13: `sub` 1 subtracts.
const fn sp_adjust(sub: u16, word_offset: u16) -> u16 {
    0xB000 | sub << 7 | word_offset
}
/// Format 14: `extra` adds LR (PUSH) or PC (POP).
const fn push_pop(load: u16, extra: u16, list: u16) -> u16 {
    0xB400 | load << 11 | extra << 8 | list
}
/// Format 15.
const fn block(load: u16, rb: u16, list: u16) -> u16 {
    0xC000 | load << 11 | rb << 8 | list
}
/// Format 16.
const fn cond_branch(cond: u16, off8: u16) -> u16 {
    0xD000 | cond << 8 | off8
}
/// Format 17.
const fn swi(value: u16) -> u16 {
    0xDF00 | value
}
/// Format 18.
const fn branch(off11: u16) -> u16 {
    0xE000 | off11
}
/// Format 19, first halfword.
const fn bl_prefix(off11: u16) -> u16 {
    0xF000 | off11
}
/// Format 19, second halfword.
const fn bl_suffix(off11: u16) -> u16 {
    0xF800 | off11
}

// ------------------------------------------------------------------ the step and the pipeline

#[test]
fn given_thumb_state_when_a_plain_instruction_runs_then_r15_advances_by_2_and_one_halfword_is_fetched()
 {
    let s = exec(SVC, &[], imm8(0, 0, 1));
    assert_eq!(s.reg(0), 1);
    assert_eq!(s.r15(), ADDR + 6);
    assert_eq!(s.bus.fetches, vec![(16, PC_READ)]);
    assert_eq!(s.cpu.pipeline(), [NEXT, PC_READ]);
}

#[test]
fn given_mov_rd_pc_when_executed_then_rd_is_the_address_plus_4() {
    let s = exec(SVC, &[], hi(2, 15, 0));
    assert_eq!(s.reg(0), ADDR + 4);
    assert_eq!(s.r15(), ADDR + 6);
}

#[test]
fn given_mov_pc_rs_when_executed_then_the_pipeline_refills_in_thumb_state() {
    let s = exec(SVC, &[(1, 0x2000)], hi(2, 1, 15));
    assert_eq!(s.bus.fetches, thumb_fetches(0x2000));
    assert_eq!(s.cpu.pipeline(), [0x2000, 0x2002]);
    assert_eq!(s.r15(), 0x2004);
    assert!(s.thumb());
}

#[test]
fn given_mov_pc_rs_with_bit_0_set_when_executed_then_the_fetch_drops_it_and_the_state_stays_thumb()
{
    let s = exec(SVC, &[(1, 0x2001)], hi(2, 1, 15));
    assert_eq!(s.bus.fetches, thumb_fetches(0x2000));
    assert_eq!(s.r15(), 0x2004);
    assert!(s.thumb());
}

#[test]
fn given_add_pc_rs_when_executed_then_the_target_is_the_address_plus_4_plus_rs() {
    let s = exec(SVC, &[(1, 0x10)], hi(0, 1, 15));
    assert_eq!(s.bus.fetches, thumb_fetches(PC_READ + 0x10));
    assert_eq!(s.r15(), PC_READ + 0x10 + 4);
}

// ------------------------------------------------------------------ BX

#[test]
fn given_bx_with_an_even_target_when_executed_then_t_clears_and_the_refill_is_two_words() {
    let s = exec(SVC, &[(1, 0x2000)], hi(3, 1, 0));
    assert!(!s.thumb());
    assert_eq!(
        s.bus.fetches,
        vec![(16, PC_READ), (32, 0x2000), (32, 0x2004)]
    );
    assert_eq!(s.r15(), 0x2008);
}

#[test]
fn given_bx_with_an_odd_target_when_executed_then_the_state_stays_thumb_at_the_even_address() {
    let s = exec(SVC, &[(1, 0x3001)], hi(3, 1, 0));
    assert!(s.thumb());
    assert_eq!(s.bus.fetches, thumb_fetches(0x3000));
    assert_eq!(s.r15(), 0x3004);
}

#[test]
fn given_bx_pc_when_executed_then_the_target_is_the_address_plus_4_in_arm_state() {
    let s = exec(SVC, &[], hi(3, 15, 0));
    assert!(!s.thumb());
    assert_eq!(s.r15(), PC_READ + 8);
}

#[test]
fn given_bx_when_executed_then_the_flags_and_mode_are_untouched() {
    let s = exec(SVC | NZCV, &[(1, 0x2000)], hi(3, 1, 0));
    assert_eq!(s.cpu.regs().cpsr(), SVC | NZCV);
}

// ------------------------------------------------------------------ formats 1 to 3

#[test]
fn given_lsl_by_an_immediate_when_executed_then_the_last_bit_out_is_c() {
    let s = exec(SVC, &[(0, 0x1000_0001)], shift(0, 4, 0, 1));
    assert_eq!(s.reg(1), 0x10);
    assert_eq!(s.flags(), C);
}

#[test]
fn given_lsl_by_zero_when_executed_then_the_value_and_c_are_kept() {
    let s = exec(SVC | C, &[(0, 7)], shift(0, 0, 0, 1));
    assert_eq!(s.reg(1), 7);
    assert_eq!(s.flags(), C);
}

#[test]
fn given_lsr_by_zero_when_executed_then_it_shifts_by_32() {
    let s = exec(SVC, &[(0, 0x8000_0000)], shift(1, 0, 0, 1));
    assert_eq!(s.reg(1), 0);
    assert_eq!(s.flags(), Z | C);
}

#[test]
fn given_asr_by_zero_when_executed_then_it_shifts_by_32_and_fills_with_the_sign() {
    let s = exec(SVC, &[(0, 0x8000_0000)], shift(2, 0, 0, 1));
    assert_eq!(s.reg(1), 0xFFFF_FFFF);
    assert_eq!(s.flags(), N | C);
}

#[test]
fn given_a_shift_when_executed_then_v_is_unchanged() {
    let s = exec(SVC | V, &[(0, 1)], shift(0, 1, 0, 1));
    assert_eq!(s.flags(), V);
}

#[test]
fn given_add_of_two_registers_when_it_wraps_to_zero_then_z_and_c_are_set() {
    let s = exec(SVC, &[(1, 0xFFFF_FFFF), (2, 1)], add_sub(0, 0, 2, 1, 0));
    assert_eq!(s.reg(0), 0);
    assert_eq!(s.flags(), Z | C);
}

#[test]
fn given_add_that_overflows_the_signed_range_when_executed_then_v_and_n_are_set() {
    let s = exec(SVC, &[(1, 0x7FFF_FFFF), (2, 1)], add_sub(0, 0, 2, 1, 0));
    assert_eq!(s.reg(0), 0x8000_0000);
    assert_eq!(s.flags(), N | V);
}

#[test]
fn given_sub_of_a_3_bit_immediate_when_it_borrows_then_c_is_clear_and_n_is_set() {
    let s = exec(SVC, &[(1, 0)], add_sub(1, 1, 1, 1, 0));
    assert_eq!(s.reg(0), 0xFFFF_FFFF);
    assert_eq!(s.flags(), N);
}

#[test]
fn given_add_of_a_3_bit_immediate_when_executed_then_rd_is_rs_plus_the_immediate() {
    let s = exec(SVC, &[(1, 10)], add_sub(1, 0, 7, 1, 0));
    assert_eq!(s.reg(0), 17);
}

#[test]
fn given_mov_immediate_when_executed_then_only_n_and_z_change() {
    let s = exec(SVC | C | V, &[(0, 9)], imm8(0, 0, 0));
    assert_eq!(s.reg(0), 0);
    assert_eq!(s.flags(), Z | C | V);
}

#[test]
fn given_cmp_immediate_of_equal_values_when_executed_then_z_and_c_are_set_and_rd_is_kept() {
    let s = exec(SVC, &[(0, 5)], imm8(1, 0, 5));
    assert_eq!(s.reg(0), 5);
    assert_eq!(s.flags(), Z | C);
}

#[test]
fn given_add_immediate_when_executed_then_rd_gains_the_8_bit_value() {
    let s = exec(SVC, &[(2, 0x100)], imm8(2, 2, 0xFF));
    assert_eq!(s.reg(2), 0x1FF);
}

#[test]
fn given_sub_immediate_when_executed_then_rd_loses_the_8_bit_value_and_c_means_no_borrow() {
    let s = exec(SVC, &[(2, 0x100)], imm8(3, 2, 1));
    assert_eq!(s.reg(2), 0xFF);
    assert_eq!(s.flags(), C);
}

// ------------------------------------------------------------------ format 4

/// (name, op, Rd before, Rs before, flags before, Rd after, flags after). Rd is R0, Rs is R1.
type AluCase = (&'static str, u16, u32, u32, u32, u32, u32);

const ALU_CASES: [AluCase; 21] = [
    ("AND", 0, 0xF0F0_F0F0, 0xFFFF_0000, 0, 0xF0F0_0000, N),
    ("EOR keeps C and V", 1, 0xFF, 0xFF, C | V, 0, Z | C | V),
    ("LSL", 2, 1, 4, 0, 0x10, 0),
    ("LSL carries out bit 31", 2, 0x8000_0000, 1, 0, 0, Z | C),
    ("LSL by 0 keeps C", 2, 5, 0, C, 5, C),
    ("LSR", 3, 0x18, 4, 0, 1, C),
    ("ASR", 4, 0x8000_0000, 4, 0, 0xF800_0000, N),
    ("ADC", 5, 1, 1, C, 3, 0),
    ("SBC", 6, 5, 3, 0, 1, C),
    ("ROR", 7, 3, 1, 0, 0x8000_0001, N | C),
    ("ROR by 32", 7, 0x8000_0000, 32, 0, 0x8000_0000, N | C),
    ("TST", 8, 0x0F, 0xF0, 0, 0x0F, Z),
    ("NEG", 9, 7, 1, 0, 0xFFFF_FFFF, N),
    ("NEG of 0", 9, 7, 0, 0, 0, Z | C),
    ("CMP", 10, 3, 5, 0, 3, N),
    ("CMN", 11, 0xFFFF_FFFF, 1, 0, 0xFFFF_FFFF, Z | C),
    ("ORR", 12, 0xF0, 0x0F, 0, 0xFF, 0),
    ("MUL", 13, 3, 5, 0, 15, 0),
    (
        "MUL to zero, C from the carry model",
        13,
        0x8000_0000,
        2,
        0,
        0,
        Z | C,
    ),
    ("BIC", 14, 0xFF, 0x0F, 0, 0xF0, 0),
    ("MVN", 15, 9, 0, 0, 0xFFFF_FFFF, N),
];

/// The cases of `ALU_CASES` whose Rd or flags differ from the table.
fn alu_failures() -> Vec<(&'static str, u32, u32)> {
    ALU_CASES
        .iter()
        .map(|&(name, op, rd, rs, flags, want_rd, want_flags)| {
            let s = exec(SVC | flags, &[(0, rd), (1, rs)], alu(op, 1, 0));
            (name, s.reg(0), s.flags(), want_rd, want_flags)
        })
        .filter(|&(_, rd, flags, want_rd, want_flags)| (rd, flags) != (want_rd, want_flags))
        .map(|(name, rd, flags, _, _)| (name, rd, flags))
        .collect()
}

#[test]
fn given_each_alu_operation_when_executed_then_rd_and_the_flags_match_the_hand_worked_table() {
    assert_eq!(alu_failures(), vec![]);
}

// ------------------------------------------------------------------ format 5

#[test]
fn given_add_on_a_high_register_when_executed_then_the_flags_are_untouched() {
    let s = exec(SVC | N | C, &[(8, 5), (1, 3)], hi(0, 1, 8));
    assert_eq!(s.reg(8), 8);
    assert_eq!(s.flags(), N | C);
    assert_eq!(s.bus.fetches.len(), 1);
}

#[test]
fn given_cmp_on_a_high_register_when_executed_then_it_sets_the_flags_and_writes_nothing() {
    let s = exec(SVC, &[(8, 3), (1, 3)], hi(1, 1, 8));
    assert_eq!(s.reg(8), 3);
    assert_eq!(s.flags(), Z | C);
}

#[test]
fn given_mov_between_high_registers_when_executed_then_rd_copies_rs_without_flags() {
    let s = exec(SVC | V, &[(9, 0xDEAD)], hi(2, 9, 8));
    assert_eq!(s.reg(8), 0xDEAD);
    assert_eq!(s.flags(), V);
}

// ------------------------------------------------------------------ loads and stores

#[test]
fn given_ldr_pc_relative_when_the_instruction_is_word_aligned_then_the_address_is_pc_plus_the_offset()
 {
    let s = exec_at(0x1000, SVC, &[], DATA, ldr_pc(0, 2));
    assert_eq!(s.bus.reads, vec![(32, 0x100C)]);
    assert_eq!(s.reg(0), DATA);
}

#[test]
fn given_ldr_pc_relative_when_the_instruction_is_halfword_aligned_then_bit_1_of_the_pc_is_ignored()
{
    let s = exec_at(0x1002, SVC, &[], DATA, ldr_pc(0, 2));
    assert_eq!(s.bus.reads, vec![(32, 0x100C)]);
}

#[test]
fn given_add_rd_pc_when_the_instruction_is_halfword_aligned_then_bit_1_of_the_pc_is_ignored() {
    let s = exec_at(0x1002, SVC, &[], DATA, load_address(0, 3, 2));
    assert_eq!(s.reg(3), 0x100C);
}

#[test]
fn given_add_rd_sp_when_executed_then_rd_is_sp_plus_the_word_offset() {
    let s = exec(SVC, &[(13, 0x500)], load_address(1, 3, 2));
    assert_eq!(s.reg(3), 0x508);
}

#[test]
fn given_str_with_a_register_offset_when_executed_then_the_word_goes_to_rb_plus_ro() {
    let s = exec(
        SVC,
        &[(0, 0xAABB_CCDD), (1, 0x2000), (2, 4)],
        mem_reg(0, 0, 2, 1, 0),
    );
    assert_eq!(s.bus.writes, vec![(32, 0x2004, 0xAABB_CCDD)]);
}

#[test]
fn given_strb_with_a_register_offset_when_executed_then_the_low_byte_is_written() {
    let s = exec(
        SVC,
        &[(0, 0xAABB_CCDD), (1, 0x2000), (2, 4)],
        mem_reg(0, 1, 2, 1, 0),
    );
    assert_eq!(s.bus.writes, vec![(8, 0x2004, 0xDD)]);
}

#[test]
fn given_ldr_at_an_unaligned_address_when_executed_then_the_word_is_rotated_right_by_8_times_the_low_bits()
 {
    let s = exec(SVC, &[(1, 0x2000), (2, 1)], mem_reg(1, 0, 2, 1, 0));
    assert_eq!(s.bus.reads, vec![(32, 0x2001)]);
    assert_eq!(s.reg(0), 0x4411_2233);
}

#[test]
fn given_ldrb_when_executed_then_the_byte_is_zero_extended() {
    let s = exec_with(
        SVC,
        &[(1, 0x2000), (2, 0)],
        0x1122_33FF,
        mem_reg(1, 1, 2, 1, 0),
    );
    assert_eq!(s.reg(0), 0xFF);
}

#[test]
fn given_ldsb_when_executed_then_the_byte_is_sign_extended() {
    let s = exec_with(
        SVC,
        &[(1, 0x2000), (2, 0)],
        0x1122_33FF,
        mem_signed(0, 1, 2, 1, 0),
    );
    assert_eq!(s.reg(0), 0xFFFF_FFFF);
}

#[test]
fn given_ldsh_at_an_even_address_when_executed_then_the_halfword_is_sign_extended() {
    let s = exec_with(
        SVC,
        &[(1, 0x2000), (2, 0)],
        0x8000,
        mem_signed(1, 1, 2, 1, 0),
    );
    assert_eq!(s.reg(0), 0xFFFF_8000);
}

#[test]
fn given_ldsh_at_an_odd_address_on_the_arm7_when_executed_then_the_high_byte_is_sign_extended() {
    let s = exec_with(
        SVC,
        &[(1, 0x2000), (2, 1)],
        0x8000,
        mem_signed(1, 1, 2, 1, 0),
    );
    assert_eq!(s.reg(0), 0xFFFF_FF80);
}

#[test]
fn given_ldrh_at_an_odd_address_on_the_arm7_when_executed_then_the_halfword_is_rotated_by_8() {
    let s = exec_with(
        SVC,
        &[(1, 0x2000), (2, 1)],
        0x1234,
        mem_signed(1, 0, 2, 1, 0),
    );
    assert_eq!(s.reg(0), 0x3400_0012);
}

#[test]
fn given_strh_with_a_register_offset_when_executed_then_the_low_halfword_is_written() {
    let s = exec(
        SVC,
        &[(0, 0xAABB_CCDD), (1, 0x2000), (2, 2)],
        mem_signed(0, 0, 2, 1, 0),
    );
    assert_eq!(s.bus.writes, vec![(16, 0x2002, 0xCCDD)]);
}

#[test]
fn given_ldr_with_an_immediate_offset_when_executed_then_the_offset_is_scaled_by_4() {
    let s = exec(SVC, &[(1, 0x2000)], mem_imm(0, 1, 3, 1, 0));
    assert_eq!(s.bus.reads, vec![(32, 0x200C)]);
}

#[test]
fn given_strb_with_an_immediate_offset_when_executed_then_the_offset_is_not_scaled() {
    let s = exec(SVC, &[(0, 0x55), (1, 0x2000)], mem_imm(1, 0, 3, 1, 0));
    assert_eq!(s.bus.writes, vec![(8, 0x2003, 0x55)]);
}

#[test]
fn given_ldrh_with_an_immediate_offset_when_executed_then_the_offset_is_scaled_by_2() {
    let s = exec(SVC, &[(1, 0x2000)], mem_half(1, 3, 1, 0));
    assert_eq!(s.bus.reads, vec![(16, 0x2006)]);
}

#[test]
fn given_strh_with_an_immediate_offset_when_executed_then_the_halfword_goes_to_rb_plus_twice_the_offset()
 {
    let s = exec(SVC, &[(0, 0x1234_5678), (1, 0x2000)], mem_half(0, 3, 1, 0));
    assert_eq!(s.bus.writes, vec![(16, 0x2006, 0x5678)]);
}

#[test]
fn given_a_load_when_executed_then_no_base_register_is_written_back() {
    let s = exec(SVC, &[(1, 0x2000)], mem_imm(0, 1, 3, 1, 0));
    assert_eq!(s.reg(1), 0x2000);
}

#[test]
fn given_sp_relative_load_when_executed_then_the_address_is_sp_plus_the_word_offset() {
    let s = exec(SVC, &[(13, 0x3000)], mem_sp(1, 2, 4));
    assert_eq!(s.bus.reads, vec![(32, 0x3010)]);
    assert_eq!(s.reg(2), DATA);
}

#[test]
fn given_sp_relative_store_when_executed_then_the_word_goes_to_sp_plus_the_word_offset() {
    let s = exec(SVC, &[(13, 0x3000), (2, 0x77)], mem_sp(0, 2, 4));
    assert_eq!(s.bus.writes, vec![(32, 0x3010, 0x77)]);
}

// ------------------------------------------------------------------ SP adjust, PUSH, POP

#[test]
fn given_add_sp_when_executed_then_sp_gains_4_times_the_offset_and_the_flags_stay() {
    let s = exec(SVC | Z, &[(13, 0x100)], sp_adjust(0, 3));
    assert_eq!(s.reg(13), 0x10C);
    assert_eq!(s.flags(), Z);
}

#[test]
fn given_sub_sp_when_executed_then_sp_loses_4_times_the_offset() {
    let s = exec(SVC, &[(13, 0x100)], sp_adjust(1, 0x7F));
    assert_eq!(s.reg(13), 0x100u32.wrapping_sub(0x1FC));
}

#[test]
fn given_push_with_lr_when_executed_then_registers_are_stored_ascending_from_the_new_sp() {
    let s = exec(
        SVC,
        &[(13, 0x100), (0, 0xA0), (1, 0xA1), (14, 0xEE)],
        push_pop(0, 1, 0b11),
    );
    assert_eq!(
        s.bus.writes,
        vec![(32, 0xF4, 0xA0), (32, 0xF8, 0xA1), (32, 0xFC, 0xEE)]
    );
    assert_eq!(s.reg(13), 0xF4);
}

#[test]
fn given_pop_with_pc_when_executed_then_sp_advances_and_the_pipeline_refills_in_thumb_state() {
    let s = exec_with(SVC, &[(13, 0x100)], 0x2000, push_pop(1, 1, 0b1));
    assert_eq!(s.bus.reads, vec![(32, 0x100), (32, 0x104)]);
    assert_eq!(s.reg(0), 0x2000);
    assert_eq!(s.reg(13), 0x108);
    assert_eq!(s.bus.fetches, thumb_fetches(0x2000));
    assert_eq!(s.r15(), 0x2004);
}

#[test]
fn given_pop_pc_of_an_odd_value_on_the_arm7_when_executed_then_the_state_stays_thumb() {
    let s = exec_with(SVC, &[(13, 0x100)], 0x2001, push_pop(1, 1, 0));
    assert!(s.thumb());
    assert_eq!(s.bus.fetches, thumb_fetches(0x2000));
}

#[test]
fn given_pop_without_pc_when_executed_then_there_is_no_refill() {
    let s = exec(SVC, &[(13, 0x100)], push_pop(1, 0, 0b110));
    assert_eq!(s.reg(13), 0x108);
    assert_eq!(s.bus.fetches.len(), 1);
}

// ------------------------------------------------------------------ LDMIA and STMIA

#[test]
fn given_stmia_when_executed_then_registers_go_to_ascending_addresses_and_rb_is_written_back() {
    let s = exec(SVC, &[(0, 0x100), (1, 0xA1), (2, 0xA2)], block(0, 0, 0b110));
    assert_eq!(s.bus.writes, vec![(32, 0x100, 0xA1), (32, 0x104, 0xA2)]);
    assert_eq!(s.reg(0), 0x108);
}

#[test]
fn given_stmia_with_rb_first_in_the_list_when_executed_on_the_arm7_then_the_old_base_is_stored() {
    let s = exec(SVC, &[(0, 0x100), (1, 0xA1)], block(0, 0, 0b11));
    assert_eq!(s.bus.writes, vec![(32, 0x100, 0x100), (32, 0x104, 0xA1)]);
}

#[test]
fn given_stmia_with_rb_not_first_in_the_list_when_executed_on_the_arm7_then_the_new_base_is_stored()
{
    let s = exec(SVC, &[(0, 0xA0), (1, 0x100)], block(0, 1, 0b11));
    assert_eq!(s.bus.writes, vec![(32, 0x100, 0xA0), (32, 0x104, 0x108)]);
}

#[test]
fn given_ldmia_with_rb_in_the_list_when_executed_on_the_arm7_then_the_loaded_value_wins_over_writeback()
 {
    let s = exec_with(SVC, &[(0, 0x100)], 0x55, block(1, 0, 0b11));
    assert_eq!(s.reg(0), 0x55);
    assert_eq!(s.reg(1), 0x55);
}

#[test]
fn given_ldmia_without_rb_in_the_list_when_executed_then_rb_is_written_back() {
    let s = exec(SVC, &[(0, 0x100)], block(1, 0, 0b110));
    assert_eq!(s.reg(0), 0x108);
    assert_eq!(s.bus.reads, vec![(32, 0x100), (32, 0x104)]);
}

#[test]
fn given_stmia_with_an_empty_list_when_executed_on_the_arm7_then_one_word_is_stored_and_rb_moves_by_0x40()
 {
    let s = exec(SVC, &[(0, 0x100)], block(0, 0, 0));
    assert_eq!(s.reg(0), 0x140);
    assert_eq!(s.bus.writes, vec![(32, 0x100, ADDR + 6)]);
}

// ------------------------------------------------------------------ branches

#[test]
fn given_a_taken_conditional_branch_when_executed_then_the_target_is_pc_plus_twice_the_offset() {
    let s = exec(SVC | Z, &[], cond_branch(0, 2));
    assert_eq!(s.bus.fetches, thumb_fetches(PC_READ + 4));
    assert_eq!(s.r15(), PC_READ + 8);
}

#[test]
fn given_a_conditional_branch_with_a_negative_offset_when_taken_then_it_goes_backwards() {
    let s = exec(SVC | Z, &[], cond_branch(0, 0xFE));
    assert_eq!(s.bus.fetches, thumb_fetches(ADDR));
}

#[test]
fn given_a_conditional_branch_whose_condition_fails_when_executed_then_it_is_a_plain_step() {
    let s = exec(SVC | Z, &[], cond_branch(1, 2));
    assert_eq!(s.r15(), ADDR + 6);
    assert_eq!(s.bus.fetches, vec![(16, PC_READ)]);
}

#[test]
fn given_an_unconditional_branch_with_offset_minus_1_when_executed_then_the_target_is_pc_minus_2() {
    let s = exec(SVC, &[], branch(0x7FF));
    assert_eq!(s.bus.fetches, thumb_fetches(PC_READ - 2));
}

#[test]
fn given_an_unconditional_branch_with_the_largest_offset_when_executed_then_it_reaches_pc_plus_0x7fe()
 {
    let s = exec(SVC, &[], branch(0x3FF));
    assert_eq!(s.bus.fetches, thumb_fetches(PC_READ + 0x7FE));
}

#[test]
fn given_the_bl_prefix_when_executed_then_lr_holds_pc_plus_the_offset_shifted_by_12_and_nothing_flushes()
 {
    let s = exec(SVC, &[], bl_prefix(1));
    assert_eq!(s.reg(14), PC_READ + 0x1000);
    assert_eq!(s.r15(), ADDR + 6);
    assert_eq!(s.bus.fetches.len(), 1);
}

#[test]
fn given_the_bl_prefix_with_a_negative_offset_when_executed_then_lr_is_pc_minus_0x1000() {
    let s = exec(SVC, &[], bl_prefix(0x7FF));
    assert_eq!(s.reg(14), PC_READ - 0x1000);
}

#[test]
fn given_a_bl_pair_when_executed_then_pc_is_the_summed_target_and_lr_is_the_return_address_with_bit_0_set()
 {
    let s = run(ADDR, SVC, &[], DATA, &[bl_prefix(1), bl_suffix(3)]);
    let target = PC_READ + 0x1000 + 6;
    assert_eq!(s.reg(14), PC_READ | 1);
    assert_eq!(
        s.bus.fetches,
        vec![
            (16, PC_READ),
            (16, ADDR + 6),
            (16, target),
            (16, target + 2)
        ]
    );
    assert_eq!(s.r15(), target + 4);
    assert!(s.thumb());
}

#[test]
fn given_a_bl_suffix_alone_when_executed_then_it_jumps_from_lr_and_links_the_next_instruction() {
    let s = exec(SVC, &[(14, 0x3000)], bl_suffix(3));
    assert_eq!(s.bus.fetches, thumb_fetches(0x3006));
    assert_eq!(s.reg(14), (ADDR + 2) | 1);
}

// ------------------------------------------------------------------ SWI and undefined

#[test]
fn given_swi_in_thumb_state_when_executed_then_svc_is_entered_with_lr_at_the_next_instruction() {
    let s = exec(USER, &[], swi(5));
    assert_eq!(s.cpu.regs().cpsr() & 0x1F, 0x13);
    assert!(!s.thumb());
    assert_eq!(s.cpu.regs().cpsr() & I_BIT, I_BIT);
    assert_eq!(s.reg(14), ADDR + 2);
    assert_eq!(s.cpu.regs().spsr(Bank::Supervisor), Some(USER | T_BIT));
    assert_eq!(s.bus.fetches, vec![(16, PC_READ), (32, 0x08), (32, 0x0C)]);
    assert_eq!(s.r15(), 0x10);
}

#[test]
fn given_a_conditional_branch_with_condition_1110_when_executed_then_it_is_taken_like_an_always_branch()
 {
    let s = exec(USER, &[], cond_branch(14, 2));
    assert_eq!(s.bus.fetches, thumb_fetches(PC_READ + 4));
    assert_eq!(s.cpu.regs().cpsr(), USER | T_BIT);
}

#[test]
fn given_bx_with_h1_set_when_executed_on_the_arm7_then_it_branches_to_the_register_h2_names() {
    let s = exec(SVC, &[(10, 0x2000)], 0x4700 | 0x80 | 0x40 | 2 << 3);
    assert!(!s.thumb());
    assert_eq!(s.bus.fetches[1], (32, 0x2000));
    assert_eq!(s.r15(), 0x2008);
}

#[test]
fn given_pop_with_an_empty_list_when_the_loaded_value_is_odd_then_r15_keeps_bit_0_and_the_fetch_drops_it()
 {
    let s = exec_with(SVC, &[(13, 0x100)], 0x2001, push_pop(1, 0, 0));
    assert_eq!(s.bus.fetches, thumb_fetches(0x2000));
    assert_eq!(s.r15(), 0x2005);
    assert_eq!(s.reg(13), 0x140);
}

#[test]
fn given_push_with_an_empty_list_when_executed_then_it_stores_the_address_plus_6_and_moves_sp_by_0x40()
 {
    let s = exec(SVC, &[(13, 0x100)], push_pop(0, 0, 0));
    assert_eq!(s.bus.writes, vec![(32, 0xC0, ADDR + 6)]);
    assert_eq!(s.reg(13), 0xC0);
}

#[test]
fn given_ldmia_with_an_empty_list_when_executed_then_pc_is_loaded_and_rb_moves_by_0x40() {
    let s = exec_with(SVC, &[(0, 0x100)], 0x3000, block(1, 0, 0));
    assert_eq!(s.bus.fetches, thumb_fetches(0x3000));
    assert_eq!(s.reg(0), 0x140);
}

#[test]
fn given_the_blx_suffix_space_when_executed_on_armv4t_then_it_is_a_plain_step() {
    let s = exec(SVC, &[], 0xE800);
    assert_eq!(s.r15(), ADDR + 6);
    assert_eq!(s.bus.fetches, vec![(16, PC_READ)]);
}

proptest! {
    #[test]
    fn given_any_operands_when_add_runs_then_the_result_and_flags_follow_the_wide_sum(
        a: u32,
        b: u32,
    ) {
        let s = exec(SVC, &[(1, a), (2, b)], add_sub(0, 0, 2, 1, 0));
        let wide = u64::from(a) + u64::from(b);
        let signed = i64::from(a as i32) + i64::from(b as i32);
        let flags = [
            ((wide as u32) >> 31 == 1, N),
            (wide as u32 == 0, Z),
            (wide >> 32 == 1, C),
            (signed != i64::from(wide as u32 as i32), V),
        ]
        .iter()
        .filter(|(set, _)| *set)
        .fold(0, |acc, (_, bit)| acc | bit);
        prop_assert_eq!(s.reg(0), wide as u32);
        prop_assert_eq!(s.flags(), flags);
    }

    #[test]
    fn given_any_operands_when_sub_runs_then_c_means_no_borrow_and_v_means_signed_overflow(
        a: u32,
        b: u32,
    ) {
        let s = exec(SVC, &[(1, a), (2, b)], add_sub(0, 1, 2, 1, 0));
        let signed = i64::from(a as i32) - i64::from(b as i32);
        let result = a.wrapping_sub(b);
        prop_assert_eq!(s.reg(0), result);
        prop_assert_eq!(s.flags() & C != 0, a >= b);
        prop_assert_eq!(s.flags() & V != 0, signed != i64::from(result as i32));
    }

    #[test]
    fn given_any_conditional_branch_offset_when_taken_then_the_target_is_pc_plus_twice_the_signed_offset(
        off8 in 0u16..0x100,
    ) {
        let target = PC_READ.wrapping_add_signed(i32::from(off8 as u8 as i8) * 2);
        let s = exec(SVC | Z, &[], cond_branch(0, off8));
        prop_assert_eq!(s.bus.fetches, thumb_fetches(target));
    }
}

/// Runs one Thumb instruction on the ARM9 with `setup` registers and `data` for every read.
fn exec_arm9(setup: &[(usize, u32)], data: u32, opcode: u16) -> (Cpu<true>, RecBus) {
    let mut cpu = Cpu::<true>::new();
    cpu.regs_mut().set_cpsr(SVC | T_BIT);
    setup.iter().for_each(|&(i, v)| cpu.regs_mut().set(i, v));
    cpu.regs_mut().set(15, PC_READ);
    cpu.set_pipeline([u32::from(opcode), NEXT]);
    let mut bus = RecBus::new(data);
    cpu.step(&mut bus);
    (cpu, bus)
}

#[test]
fn given_ldmia_with_rb_in_the_list_when_executed_on_the_arm9_then_the_loaded_value_wins_and_there_is_no_writeback()
 {
    let (cpu, _) = exec_arm9(&[(0, 0x100)], 0x55, block(1, 0, 0b11));
    assert_eq!(cpu.regs().get(0), 0x55);
}

#[test]
fn given_ldmia_with_rb_as_the_only_register_when_executed_on_the_arm9_then_the_loaded_value_wins() {
    let (cpu, _) = exec_arm9(&[(0, 0x100)], 0x55, block(1, 0, 0b1));
    assert_eq!(cpu.regs().get(0), 0x55);
}

#[test]
fn given_ldmia_without_rb_in_the_list_when_executed_on_the_arm9_then_rb_is_written_back() {
    let (cpu, _) = exec_arm9(&[(0, 0x100)], 0x55, block(1, 0, 0b110));
    assert_eq!(cpu.regs().get(0), 0x108);
}

#[test]
fn given_stmia_with_rb_not_first_when_executed_on_the_arm9_then_the_old_base_is_stored() {
    let (_, bus) = exec_arm9(&[(0, 0xA0), (1, 0x100)], 0, block(0, 1, 0b11));
    assert_eq!(bus.writes, vec![(32, 0x100, 0xA0), (32, 0x104, 0x100)]);
}

#[test]
fn given_mul_that_the_arm7_gives_c_when_executed_on_the_arm9_then_c_is_kept() {
    // The same operands as the "MUL to zero" row of the ALU table: the ARM7 sets C, the ARM9
    // leaves it as it was (docs/hw/arm-multiply.md).
    let (cpu, _) = exec_arm9(&[(0, 0x8000_0000), (1, 2)], 0, alu(13, 1, 0));
    assert_eq!(cpu.regs().cpsr() & C, 0);
}
