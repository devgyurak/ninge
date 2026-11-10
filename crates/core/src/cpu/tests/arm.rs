// SPDX-License-Identifier: Apache-2.0
//! ARM data processing, the barrel shifter and PSR transfer, through `Cpu::step` only.
//! Expected values come from `docs/hw/arm-data-processing.md`,
//! `docs/hw/arm-barrel-shifter.md` and `docs/hw/cpu-registers.md`.
//!
//! Pipeline model (`docs/hw/arm-branch-pipeline.md`): the instruction under test is
//! `pipeline[0]` and R15 is its address + 8 while it executes; afterwards R15 is +4 more if
//! the instruction did not write it, or the written value + 8 if it did. The bus is only asked
//! for the prefetch, whose value these tests do not look at.

use crate::cpu::*;
use proptest::prelude::*;

/// Address of the instruction under test.
const ADDR: u32 = 0x1000;
const N: u32 = 0x8000_0000;
const Z: u32 = 0x4000_0000;
const C: u32 = 0x2000_0000;
const V: u32 = 0x1000_0000;
const FLAGS: u32 = N | Z | C | V;
/// CPSR mode bits used below.
const USR: u32 = 0x10;
const IRQ: u32 = 0x12;
const SVC: u32 = 0x13;
const SYS: u32 = 0x1F;

// Shift types (bits 6-5).
const LSL: u32 = 0;
const LSR: u32 = 1;
const ASR: u32 = 2;
const ROR: u32 = 3;

/// A bus that serves one value for every code fetch (the prefetch).
struct FixedBus(u32);

impl Bus for FixedBus {
    fn fetch32(&mut self, _addr: u32) -> u32 {
        self.0
    }
    fn fetch16(&mut self, _addr: u32) -> u16 {
        0
    }
    fn read8(&mut self, _addr: u32) -> u8 {
        0
    }
    fn read16(&mut self, _addr: u32) -> u16 {
        0
    }
    fn read32(&mut self, _addr: u32) -> u32 {
        0
    }
    fn write8(&mut self, _addr: u32, _value: u8) {}
    fn write16(&mut self, _addr: u32, _value: u16) {}
    fn write32(&mut self, _addr: u32, _value: u32) {}
}

/// `cond=AL` data-processing word: `op` is the opcode field, `s` the S bit.
const fn dp(op: u32, s: u32, rn: u32, rd: u32, op2: u32) -> u32 {
    0xE000_0000 | op << 21 | s << 20 | rn << 16 | rd << 12 | op2
}

/// Operand 2 = R1 shifted by an immediate amount.
const fn ish(ty: u32, amount: u32) -> u32 {
    amount << 7 | ty << 5 | 1
}

/// Operand 2 = R1 shifted by the low byte of R2.
const fn rsh(ty: u32) -> u32 {
    2 << 8 | ty << 5 | 0x10 | 1
}

/// Operand 2 = rotated immediate (sets the I bit).
const fn imm(rot: u32, value: u32) -> u32 {
    1 << 25 | rot << 8 | value
}

/// Replaces the condition field of `opcode`.
const fn with_cond(opcode: u32, cond: u32) -> u32 {
    opcode & 0x0FFF_FFFF | cond << 28
}

/// `MSR <CPSR|SPSR>_<mask>, Rm` with Rm = R1.
const fn msr_reg(spsr: u32, mask: u32) -> u32 {
    0xE120_F001 | spsr << 22 | mask << 16
}

/// `MSR <CPSR|SPSR>_<mask>, #imm12`.
const fn msr_imm(spsr: u32, mask: u32, imm12: u32) -> u32 {
    0xE320_F000 | spsr << 22 | mask << 16 | imm12
}

/// `MRS R0, <CPSR|SPSR>`.
const fn mrs(spsr: u32) -> u32 {
    0xE10F_0000 | spsr << 22
}

/// A CPU whose CPSR is `cpsr` (so its register bank is switched).
fn cpu_in(cpsr: u32) -> Cpu<false> {
    let mut cpu = Cpu::<false>::new();
    cpu.regs_mut().set_cpsr(cpsr);
    cpu
}

/// Runs `opcode` as the instruction at `ADDR`.
fn step_op(cpu: &mut Cpu<false>, opcode: u32) {
    cpu.regs_mut().set(15, ADDR + 8);
    cpu.set_pipeline([opcode, 0]);
    cpu.step(&mut FixedBus(0));
}

/// Sets the listed visible registers, then runs `opcode`.
fn run(cpsr: u32, setup: &[(usize, u32)], opcode: u32) -> Cpu<false> {
    let mut cpu = cpu_in(cpsr);
    setup.iter().for_each(|&(i, v)| cpu.regs_mut().set(i, v));
    step_op(&mut cpu, opcode);
    cpu
}

fn reg(cpu: &Cpu<false>, i: usize) -> u32 {
    cpu.regs().get(i)
}

// ---------------------------------------------------------------- shifter tables

/// One `MOVS R0, R1 <op2>` case: R1 = `rm`, R2 = `rs`, the C flag before = `carry_in`.
struct ShiftCase {
    op2: u32,
    rm: u32,
    rs: u32,
    carry_in: u32,
    result: u32,
    flags: u32,
}

const fn sc(op2: u32, rm: u32, rs: u32, carry_in: u32, result: u32, flags: u32) -> ShiftCase {
    ShiftCase {
        op2,
        rm,
        rs,
        carry_in,
        result,
        flags,
    }
}

/// Cases whose (R0, CPSR) after `MOVS R0, R1 <op2>` differs from the table.
fn shift_failures(cases: &[ShiftCase]) -> Vec<String> {
    cases
        .iter()
        .filter_map(|c| {
            let cpu = run(
                SVC | c.carry_in,
                &[(1, c.rm), (2, c.rs)],
                dp(0xD, 1, 0, 0, c.op2),
            );
            let got = (reg(&cpu, 0), cpu.regs().cpsr());
            let want = (c.result, SVC | c.flags);
            (got != want).then(|| {
                format!(
                    "op2={:#x} rm={:#x} rs={:#x} cin={:#x}: got {got:x?}, want {want:x?}",
                    c.op2, c.rm, c.rs, c.carry_in
                )
            })
        })
        .collect()
}

#[test]
fn given_lsl_immediate_zero_when_movs_executes_then_value_and_carry_are_unchanged() {
    let cases = [
        sc(ish(LSL, 0), 0x8000_0001, 0, C, 0x8000_0001, N | C),
        sc(ish(LSL, 0), 0x8000_0001, 0, 0, 0x8000_0001, N),
        sc(ish(LSL, 0), 0, 0, C, 0, Z | C),
        sc(ish(LSL, 0), 0, 0, 0, 0, Z),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_lsr_immediate_zero_when_movs_executes_then_it_shifts_by_32_with_bit31_as_carry() {
    let cases = [
        sc(ish(LSR, 0), 0x8000_0000, 0, 0, 0, Z | C),
        sc(ish(LSR, 0), 0x7FFF_FFFF, 0, C, 0, Z),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_asr_immediate_zero_when_movs_executes_then_it_shifts_by_32_filling_with_the_sign() {
    let cases = [
        sc(ish(ASR, 0), 0x8000_0000, 0, 0, 0xFFFF_FFFF, N | C),
        sc(ish(ASR, 0), 0x7FFF_FFFF, 0, C, 0, Z),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_ror_immediate_zero_when_movs_executes_then_it_is_rrx_with_bit0_as_carry() {
    let cases = [
        sc(ish(ROR, 0), 3, 0, C, 0x8000_0001, N | C),
        sc(ish(ROR, 0), 3, 0, 0, 1, C),
        sc(ish(ROR, 0), 2, 0, C, 0x8000_0001, N),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_nonzero_immediate_shift_amounts_when_movs_executes_then_result_and_last_bit_out_are_exact()
{
    let cases = [
        sc(ish(LSL, 4), 0x1000_0000, 0, 0, 0, Z | C),
        sc(ish(LSR, 4), 0x18, 0, 0, 1, C),
        sc(ish(ASR, 4), 0x8000_0008, 0, 0, 0xF800_0000, N | C),
        sc(ish(ROR, 4), 1, 0, C, 0x1000_0000, 0),
        sc(ish(ROR, 1), 1, 0, 0, 0x8000_0000, N | C),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_register_shift_amount_zero_when_movs_executes_then_value_and_carry_are_unchanged() {
    let cases = [
        sc(rsh(LSL), 0x8000_0001, 0, C, 0x8000_0001, N | C),
        sc(rsh(LSL), 0x8000_0001, 0, 0, 0x8000_0001, N),
        sc(rsh(LSR), 0x8000_0001, 0, C, 0x8000_0001, N | C),
        sc(rsh(LSR), 0x8000_0001, 0, 0, 0x8000_0001, N),
        sc(rsh(ASR), 0x8000_0001, 0, C, 0x8000_0001, N | C),
        sc(rsh(ASR), 0x8000_0001, 0, 0, 0x8000_0001, N),
        sc(rsh(ROR), 0x8000_0001, 0, C, 0x8000_0001, N | C),
        sc(rsh(ROR), 0x8000_0001, 0, 0, 0x8000_0001, N),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_register_shift_amount_32_when_movs_executes_then_result_and_carry_follow_the_32_rules() {
    let cases = [
        sc(rsh(LSL), 3, 32, 0, 0, Z | C),
        sc(rsh(LSL), 2, 32, C, 0, Z),
        sc(rsh(LSR), 0x8000_0000, 32, 0, 0, Z | C),
        sc(rsh(ASR), 0x8000_0000, 32, 0, 0xFFFF_FFFF, N | C),
        sc(rsh(ASR), 0x7FFF_FFFF, 32, C, 0, Z),
        sc(rsh(ROR), 0x8000_0001, 32, 0, 0x8000_0001, N | C),
        sc(rsh(ROR), 1, 32, C, 1, 0),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_register_shift_amount_above_32_when_movs_executes_then_result_and_carry_follow_the_rules()
{
    let cases = [
        sc(rsh(LSL), 1, 33, C, 0, Z),
        sc(rsh(LSR), 0x8000_0000, 33, C, 0, Z),
        sc(rsh(ASR), 0x8000_0000, 33, 0, 0xFFFF_FFFF, N | C),
        sc(rsh(ASR), 0x7FFF_FFFF, 100, C, 0, Z),
        sc(rsh(ROR), 1, 33, 0, 0x8000_0000, N | C),
        sc(rsh(ROR), 0x8000_0000, 64, 0, 0x8000_0000, N | C),
        sc(rsh(ROR), 1, 64, C, 1, 0),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_register_shift_amount_with_high_bits_when_movs_executes_then_only_the_low_byte_counts() {
    let cases = [
        sc(rsh(LSL), 1, 0x100, C, 1, C),
        sc(rsh(LSL), 1, 0xFFFF_FF01, 0, 2, 0),
        sc(rsh(LSL), 3, 0x1_0020, 0, 0, Z | C),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_rotated_immediates_when_movs_executes_then_carry_is_kept_for_rotation_zero_else_bit31() {
    let cases = [
        sc(imm(0, 0xFF), 0, 0, C, 0xFF, C),
        sc(imm(0, 0xFF), 0, 0, 0, 0xFF, 0),
        sc(imm(4, 0xFF), 0, 0, 0, 0xFF00_0000, N | C),
        sc(imm(1, 0x03), 0, 0, 0, 0xC000_0000, N | C),
        sc(imm(1, 0x04), 0, 0, C, 1, 0),
    ];
    assert_eq!(shift_failures(&cases), Vec::<String>::new());
}

// ---------------------------------------------------------------- ALU

/// (R0, CPSR flag bits) after `<op>S R0, R1, R2` with the given inputs and flags.
fn alu_s(op: u32, r1: u32, r2: u32, flags_in: u32) -> (u32, u32) {
    let cpu = run(SVC | flags_in, &[(1, r1), (2, r2)], dp(op, 1, 1, 0, 2));
    (reg(&cpu, 0), cpu.regs().cpsr() & FLAGS)
}

#[test]
fn given_carry_set_when_adc_r0_r1_r2_executes_then_r0_includes_carry() {
    let cpu = run(SVC | C, &[(1, 1), (2, 1)], dp(5, 0, 1, 0, 2));
    assert_eq!(reg(&cpu, 0), 3);
}

#[test]
fn given_adcs_inputs_when_executed_then_c_and_v_come_from_the_three_operand_sum() {
    assert_eq!(
        [
            alu_s(5, 0xFFFF_FFFF, 0, C),
            alu_s(5, 0x7FFF_FFFF, 0, C),
            alu_s(5, 0xFFFF_FFFF, 0, 0),
        ],
        [(0, Z | C), (0x8000_0000, N | V), (0xFFFF_FFFF, N)]
    );
}

#[test]
fn given_sbcs_inputs_when_executed_then_result_is_a_minus_b_minus_1_plus_c_with_not_borrow() {
    assert_eq!(
        [
            alu_s(6, 5, 5, 0),
            alu_s(6, 5, 5, C),
            alu_s(6, 0x8000_0000, 0, 0),
        ],
        [(0xFFFF_FFFF, N), (0, Z | C), (0x7FFF_FFFF, C | V),]
    );
}

#[test]
fn given_rscs_inputs_when_executed_then_result_is_b_minus_a_minus_1_plus_c_with_not_borrow() {
    assert_eq!(
        [alu_s(7, 1, 5, 0), alu_s(7, 5, 5, 0), alu_s(7, 2, 1, C),],
        [(3, C), (0xFFFF_FFFF, N), (0xFFFF_FFFF, N)]
    );
}

#[test]
fn given_cmp_inputs_when_executed_then_flags_are_set_and_rd_is_left_alone() {
    let run_cmp = |a: u32, b: u32| {
        let cpu = run(SVC, &[(0, 0x1234), (1, a), (2, b)], dp(0xA, 1, 1, 0, 2));
        (reg(&cpu, 0), cpu.regs().cpsr() & FLAGS)
    };
    assert_eq!(
        [
            run_cmp(5, 5),
            run_cmp(0, 1),
            run_cmp(0x8000_0000, 1),
            run_cmp(1, 0)
        ],
        [(0x1234, Z | C), (0x1234, N), (0x1234, C | V), (0x1234, C)]
    );
}

#[test]
fn given_cmn_inputs_when_executed_then_flags_come_from_the_sum_and_rd_is_left_alone() {
    let run_cmn = |a: u32, b: u32| {
        let cpu = run(SVC, &[(0, 0x1234), (1, a), (2, b)], dp(0xB, 1, 1, 0, 2));
        (reg(&cpu, 0), cpu.regs().cpsr() & FLAGS)
    };
    assert_eq!(
        [run_cmn(0xFFFF_FFFF, 1), run_cmn(0x7FFF_FFFF, 1)],
        [(0x1234, Z | C), (0x1234, N | V)]
    );
}

#[test]
fn given_v_set_when_ands_with_shifter_carry_executes_then_c_is_the_shifter_carry_and_v_is_kept() {
    // ANDS R0, R1, R2 LSR #1 with R2 = 1: operand 2 = 0, shifter carry-out = 1.
    let cpu = run(
        SVC | V,
        &[(1, 0xF), (2, 1)],
        dp(0, 1, 1, 0, 2 | LSR << 5 | 1 << 7),
    );
    assert_eq!((reg(&cpu, 0), cpu.regs().cpsr() & FLAGS), (0, Z | C | V));
}

#[test]
fn given_shifter_carry_out_when_adds_executes_then_c_ignores_it() {
    // ADDS R0, R1, R2 LSR #1 with R1 = 1, R2 = 1: shifter carry-out 1, sum 1 has no carry.
    let cpu = run(
        SVC,
        &[(1, 1), (2, 1)],
        dp(4, 1, 1, 0, 2 | LSR << 5 | 1 << 7),
    );
    assert_eq!((reg(&cpu, 0), cpu.regs().cpsr() & FLAGS), (1, 0));
}

#[test]
fn given_s_bit_clear_when_add_overflows_then_flags_are_untouched() {
    // 0x7FFF_FFFF + 1 would set N and V with S; without S the result lands and CPSR stays.
    let cpu = run(SVC | C, &[(1, 0x7FFF_FFFF), (2, 1)], dp(4, 0, 1, 0, 2));
    assert_eq!((reg(&cpu, 0), cpu.regs().cpsr()), (0x8000_0000, SVC | C));
}

// ---------------------------------------------------------------- R15

#[test]
fn given_plain_instruction_when_executed_then_r15_advances_by_4_from_the_execute_view() {
    let cpu = run(SVC, &[], dp(0xD, 0, 0, 0, imm(0, 1)));
    assert_eq!(reg(&cpu, 15), ADDR + 12);
}

#[test]
fn given_mov_r0_pc_with_immediate_shift_when_executed_then_r0_is_address_plus_8() {
    let cpu = run(SVC, &[], dp(0xD, 0, 0, 0, 0xF));
    assert_eq!(reg(&cpu, 0), ADDR + 8);
}

#[test]
fn given_mov_r0_pc_with_register_shift_when_executed_then_r0_is_address_plus_12() {
    // MOV R0, R15 LSL R2 with R2 = 0.
    let cpu = run(SVC, &[(2, 0)], dp(0xD, 0, 0, 0, 2 << 8 | 0x10 | 0xF));
    assert_eq!(reg(&cpu, 0), ADDR + 12);
}

#[test]
fn given_add_rn_pc_with_register_shift_when_executed_then_rn_reads_address_plus_12() {
    // ADD R0, R15, R1 LSL R2 with R1 = 1, R2 = 0.
    let cpu = run(SVC, &[(1, 1), (2, 0)], dp(4, 0, 15, 0, rsh(LSL)));
    assert_eq!(reg(&cpu, 0), ADDR + 12 + 1);
}

#[test]
fn given_pc_as_shifted_register_with_register_amount_when_executed_then_it_shifts_address_plus_12()
{
    // MOV R0, R15 LSL R2 with R2 = 1.
    let cpu = run(SVC, &[(2, 1)], dp(0xD, 0, 0, 0, 2 << 8 | 0x10 | 0xF));
    assert_eq!(reg(&cpu, 0), (ADDR + 12) << 1);
}

#[test]
fn given_mov_pc_without_s_when_executed_then_r15_is_value_plus_8_and_cpsr_is_unchanged() {
    // MOV R15, R1
    let cpu = run(SVC | C, &[(1, 0x4000)], dp(0xD, 0, 0, 15, 1));
    assert_eq!((reg(&cpu, 15), cpu.regs().cpsr()), (0x4008, SVC | C));
}

#[test]
fn given_svc_with_spsr_when_movs_pc_lr_executes_then_cpsr_is_spsr_and_the_bank_switches() {
    let mut cpu = cpu_in(SVC);
    cpu.regs_mut().set(14, 0x2000);
    cpu.regs_mut().set_banked(Bank::UserSystem, 14, 0xAAAA);
    cpu.regs_mut().set_spsr(Bank::Supervisor, Z | C | USR);
    step_op(&mut cpu, dp(0xD, 1, 0, 15, 14)); // MOVS PC, LR
    assert_eq!(
        (
            reg(&cpu, 15),
            cpu.regs().cpsr(),
            cpu.regs().bank(),
            reg(&cpu, 14),
            cpu.regs().banked(Bank::Supervisor, 14)
        ),
        (0x2008, Z | C | USR, Bank::UserSystem, 0xAAAA, 0x2000)
    );
}

#[test]
fn given_irq_mode_when_subs_pc_lr_4_executes_then_pc_is_lr_minus_4_plus_8_and_cpsr_is_spsr() {
    let mut cpu = cpu_in(IRQ);
    cpu.regs_mut().set(14, 0x3000);
    cpu.regs_mut().set_spsr(Bank::Irq, N | C | SYS);
    step_op(&mut cpu, dp(2, 1, 14, 15, imm(0, 4))); // SUBS PC, LR, #4
    assert_eq!(
        (reg(&cpu, 15), cpu.regs().cpsr(), cpu.regs().bank()),
        (0x3004, N | C | SYS, Bank::UserSystem)
    );
}

#[test]
fn given_user_mode_when_movs_pc_executes_then_only_flags_change_and_control_bits_stay() {
    let cpu = run(USR | C, &[(1, 0x4000)], dp(0xD, 1, 0, 15, 1));
    assert_eq!(
        (reg(&cpu, 15), cpu.regs().cpsr(), cpu.regs().bank()),
        (0x4008, USR | C, Bank::UserSystem)
    );
}

#[test]
fn given_svc_with_spsr_when_cmp_with_rd_pc_executes_then_r15_advances_and_cpsr_is_restored_from_spsr()
 {
    let mut cpu = cpu_in(SVC);
    cpu.regs_mut().set_spsr(Bank::Supervisor, SYS);
    cpu.regs_mut().set(1, 7);
    cpu.regs_mut().set(2, 7);
    step_op(&mut cpu, dp(0xA, 1, 1, 15, 2)); // CMP R1, R2 with Rd field = 15
    assert_eq!((reg(&cpu, 15), cpu.regs().cpsr()), (ADDR + 12, SYS));
}

// ---------------------------------------------------------------- condition

#[test]
fn given_condition_ne_and_z_set_when_adds_executes_then_state_is_kept_and_r15_advances_by_4() {
    let opcode = with_cond(dp(4, 1, 1, 0, 2), 1); // ADDNES R0, R1, R2
    let cpu = run(SVC | Z, &[(0, 9), (1, 1), (2, 2)], opcode);
    assert_eq!(
        (reg(&cpu, 0), cpu.regs().cpsr(), reg(&cpu, 15)),
        (9, SVC | Z, ADDR + 12)
    );
}

#[test]
fn given_condition_eq_and_z_set_when_add_executes_then_it_runs() {
    let opcode = with_cond(dp(4, 0, 1, 0, 2), 0); // ADDEQ R0, R1, R2
    let cpu = run(SVC | Z, &[(0, 9), (1, 1), (2, 2)], opcode);
    assert_eq!(reg(&cpu, 0), 3);
}

// ---------------------------------------------------------------- PSR transfer

#[test]
fn given_svc_when_msr_cpsr_f_executes_then_only_the_flag_byte_changes() {
    let cpu = run(SVC | 0x80, &[(1, 0xF000_00FF)], msr_reg(0, 0b1000));
    assert_eq!(cpu.regs().cpsr(), 0xF000_0000 | SVC | 0x80);
}

#[test]
fn given_svc_when_msr_cpsr_c_to_system_executes_then_mode_and_bank_change_and_flags_stay() {
    let cpu = run(SVC | C, &[(1, 0xF000_001F)], msr_reg(0, 0b0001));
    assert_eq!(
        (cpu.regs().cpsr(), cpu.regs().bank()),
        (C | SYS, Bank::UserSystem)
    );
}

#[test]
fn given_svc_when_msr_cpsr_fc_executes_then_both_bytes_are_written() {
    let cpu = run(SVC, &[(1, 0x9000_00D3)], msr_reg(0, 0b1001));
    assert_eq!(cpu.regs().cpsr(), 0x9000_00D3);
}

#[test]
fn given_user_mode_when_msr_cpsr_fsxc_executes_then_only_the_flag_byte_changes() {
    let cpu = run(USR, &[(1, 0xF000_00DF)], msr_reg(0, 0b1111));
    assert_eq!(
        (cpu.regs().cpsr(), cpu.regs().bank()),
        (0xF000_0010, Bank::UserSystem)
    );
}

#[test]
fn given_user_mode_when_msr_cpsr_c_executes_then_cpsr_and_bank_are_unchanged() {
    let cpu = run(USR, &[(1, 0x13)], msr_reg(0, 0b0001));
    assert_eq!(
        (cpu.regs().cpsr(), cpu.regs().bank(), reg(&cpu, 15)),
        (USR, Bank::UserSystem, ADDR + 12)
    );
}

#[test]
fn given_svc_when_msr_cpsr_f_immediate_executes_then_the_rotated_immediate_fills_the_flag_byte() {
    // #0xF000_0000 = 0x0F rotated right by 2 * 2.
    let cpu = run(SVC, &[], msr_imm(0, 0b1000, 2 << 8 | 0x0F));
    assert_eq!(cpu.regs().cpsr(), 0xF000_0000 | SVC);
}

#[test]
fn given_svc_when_msr_spsr_f_executes_then_only_spsr_flag_byte_changes_and_cpsr_is_kept() {
    let mut cpu = cpu_in(SVC);
    cpu.regs_mut().set_spsr(Bank::Supervisor, SYS);
    cpu.regs_mut().set(1, 0xA000_0000);
    step_op(&mut cpu, msr_reg(1, 0b1000));
    assert_eq!(
        (cpu.regs().spsr(Bank::Supervisor), cpu.regs().cpsr()),
        (Some(0xA000_0000 | SYS), SVC)
    );
}

#[test]
fn given_irq_mode_when_msr_spsr_executes_then_other_modes_spsr_is_untouched() {
    let mut cpu = cpu_in(IRQ);
    cpu.regs_mut()
        .set_spsr(Bank::Supervisor, 0x1234_5678 & !0x60);
    cpu.regs_mut().set(1, 0xB000_0000);
    step_op(&mut cpu, msr_reg(1, 0b1000));
    assert_eq!(
        (
            cpu.regs().spsr(Bank::Irq),
            cpu.regs().spsr(Bank::Supervisor)
        ),
        (Some(0xB000_0000), Some(0x1234_5678 & !0x60))
    );
}

#[test]
fn given_svc_when_mrs_cpsr_executes_then_r0_is_the_whole_cpsr_and_flags_stay() {
    let cpsr = N | C | 0x80 | SVC;
    let cpu = run(cpsr, &[], mrs(0));
    assert_eq!((reg(&cpu, 0), cpu.regs().cpsr()), (cpsr, cpsr));
}

#[test]
fn given_svc_and_irq_spsrs_when_mrs_spsr_executes_in_irq_then_r0_is_the_irq_spsr() {
    let mut cpu = cpu_in(IRQ);
    cpu.regs_mut().set_spsr(Bank::Irq, Z | SYS);
    cpu.regs_mut().set_spsr(Bank::Supervisor, N | USR);
    step_op(&mut cpu, mrs(1));
    assert_eq!(reg(&cpu, 0), Z | SYS);
}

// ---------------------------------------------------------------- properties

/// (R0, CPSR) after `<op>S R0, R1, R2` in SVC with `flags` set before.
fn run_alu_s(op: u32, a: u32, b: u32, flags: u32) -> (u32, u32) {
    let cpu = run(SVC | flags, &[(1, a), (2, b)], dp(op, 1, 1, 0, 2));
    (reg(&cpu, 0), cpu.regs().cpsr())
}

/// Packs a result and the four flag booleans into the (R0, CPSR) shape.
fn pack(res: u32, carry: bool, overflow: bool) -> (u32, u32) {
    let flags =
        res & N | u32::from(res == 0) << 30 | u32::from(carry) << 29 | u32::from(overflow) << 28;
    (res, SVC | flags)
}

fn adc_reference(a: u32, b: u32, c: u32) -> (u32, u32) {
    let wide = u64::from(a) + u64::from(b) + u64::from(c);
    let res = wide as u32; // truncation is the 32-bit result
    let signed = i64::from(a as i32) + i64::from(b as i32) + i64::from(c);
    pack(res, wide >> 32 != 0, signed != i64::from(res as i32))
}

fn sbc_reference(a: u32, b: u32, c: u32) -> (u32, u32) {
    let res = a.wrapping_sub(b).wrapping_sub(1).wrapping_add(c);
    let signed = i64::from(a as i32) - i64::from(b as i32) - 1 + i64::from(c);
    pack(
        res,
        u64::from(a) + u64::from(c) > u64::from(b),
        signed != i64::from(res as i32),
    )
}

/// R0 after `MOV R0, R1 <shift> R2`.
fn mov_reg_shift(ty: u32, value: u32, amount: u32) -> u32 {
    reg(
        &run(SVC, &[(1, value), (2, amount)], dp(0xD, 0, 0, 0, rsh(ty))),
        0,
    )
}

proptest! {
    #[test]
    fn given_any_adds_operands_when_executed_then_nzcv_match_wide_arithmetic(
        a in any::<u32>(), b in any::<u32>(), f in 0u32..16,
    ) {
        prop_assert_eq!(run_alu_s(4, a, b, f << 28), adc_reference(a, b, 0));
    }

    #[test]
    fn given_any_adcs_operands_when_executed_then_nzcv_match_wide_arithmetic(
        a in any::<u32>(), b in any::<u32>(), c in 0u32..2,
    ) {
        prop_assert_eq!(run_alu_s(5, a, b, c << 29), adc_reference(a, b, c));
    }

    #[test]
    fn given_any_subs_operands_when_executed_then_c_is_not_borrow_and_v_is_signed_overflow(
        a in any::<u32>(), b in any::<u32>(), f in 0u32..16,
    ) {
        prop_assert_eq!(run_alu_s(2, a, b, f << 28), sbc_reference(a, b, 1));
    }

    #[test]
    fn given_any_sbcs_operands_when_executed_then_nzcv_match_wide_arithmetic(
        a in any::<u32>(), b in any::<u32>(), c in 0u32..2,
    ) {
        prop_assert_eq!(run_alu_s(6, a, b, c << 29), sbc_reference(a, b, c));
    }

    #[test]
    fn given_any_ands_operands_when_executed_then_c_and_v_are_kept_and_nz_follow_the_result(
        a in any::<u32>(), b in any::<u32>(), f in 0u32..16,
    ) {
        let (res, cpsr) = run_alu_s(0, a, b, f << 28);
        let want = a & b;
        prop_assert_eq!(
            (res, cpsr),
            (want, SVC | (f << 28 & (C | V)) | want & N | u32::from(want == 0) << 30)
        );
    }

    #[test]
    fn given_any_value_and_immediate_amount_when_lsl_movs_executes_then_result_and_carry_match(
        v in any::<u32>(), n in 1u32..32,
    ) {
        let cpu = run(SVC, &[(1, v)], dp(0xD, 1, 0, 0, ish(LSL, n)));
        let carry = (v >> (32 - n) & 1) << 29;
        prop_assert_eq!(
            (reg(&cpu, 0), cpu.regs().cpsr() & C),
            (v << n, carry)
        );
    }

    #[test]
    fn given_any_value_and_register_amount_when_lsl_executes_then_result_is_a_checked_shift(
        v in any::<u32>(), s in any::<u32>(),
    ) {
        prop_assert_eq!(mov_reg_shift(LSL, v, s), v.checked_shl(s & 0xFF).unwrap_or(0));
    }

    #[test]
    fn given_any_value_and_register_amount_when_lsr_executes_then_result_is_a_checked_shift(
        v in any::<u32>(), s in any::<u32>(),
    ) {
        prop_assert_eq!(mov_reg_shift(LSR, v, s), v.checked_shr(s & 0xFF).unwrap_or(0));
    }

    #[test]
    fn given_any_value_and_register_amount_when_asr_executes_then_result_is_a_clamped_signed_shift(
        v in any::<u32>(), s in any::<u32>(),
    ) {
        prop_assert_eq!(
            mov_reg_shift(ASR, v, s),
            ((v as i32) >> (s & 0xFF).min(31)) as u32
        );
    }

    #[test]
    fn given_any_value_and_register_amount_when_ror_executes_then_result_is_a_rotate_by_the_low_byte(
        v in any::<u32>(), s in any::<u32>(),
    ) {
        prop_assert_eq!(mov_reg_shift(ROR, v, s), v.rotate_right(s & 0xFF));
    }
}

// ---------------------------------------------------------------- multiply (docs/hw/arm-multiply.md)

/// `cond=AL` multiply word: `bits` is (long, signed, accumulate, flags).
/// Fields: `hi` = Rd/RdHi (19-16), `lo` = Rn/RdLo (15-12), Rs, Rm.
const fn mul_op(bits: (u32, u32, u32, u32), hi: u32, lo: u32, rs: u32, rm: u32) -> u32 {
    let (l, u, a, s) = bits;
    0xE000_0090 | l << 23 | u << 22 | a << 21 | s << 20 | hi << 16 | lo << 12 | rs << 8 | rm
}

/// Rm = R1, Rs = R2, Rn/RdLo = R3, Rd/RdHi = R4 throughout.
const MUL: u32 = mul_op((0, 0, 0, 0), 4, 0, 2, 1);
const MLA: u32 = mul_op((0, 0, 1, 0), 4, 3, 2, 1);
const UMULL: u32 = mul_op((1, 0, 0, 0), 4, 3, 2, 1);
const UMLAL: u32 = mul_op((1, 0, 1, 0), 4, 3, 2, 1);
const SMULL: u32 = mul_op((1, 1, 0, 0), 4, 3, 2, 1);
const SMLAL: u32 = mul_op((1, 1, 1, 0), 4, 3, 2, 1);
const S_BIT: u32 = 1 << 20;

/// One multiply: registers set before, registers expected after.
struct MulCase {
    name: &'static str,
    opcode: u32,
    setup: &'static [(usize, u32)],
    want: &'static [(usize, u32)],
}

const fn mc(
    name: &'static str,
    opcode: u32,
    setup: &'static [(usize, u32)],
    want: &'static [(usize, u32)],
) -> MulCase {
    MulCase {
        name,
        opcode,
        setup,
        want,
    }
}

/// Names of the cases whose registers differ from `want`, with the registers' actual values.
fn mul_failures(cases: &[MulCase]) -> Vec<String> {
    cases
        .iter()
        .filter_map(|c| {
            let cpu = run(SVC, c.setup, c.opcode);
            let got: Vec<u32> = c.want.iter().map(|&(i, _)| reg(&cpu, i)).collect();
            let want: Vec<u32> = c.want.iter().map(|&(_, v)| v).collect();
            (got != want).then(|| format!("{}: got {got:x?}, want {want:x?}", c.name))
        })
        .collect()
}

#[test]
fn given_mul_and_mla_operands_when_executed_then_rd_is_the_low_32_bits() {
    let cases = [
        mc("mul", MUL, &[(1, 6), (2, 7)], &[(4, 42)]),
        mc(
            "mul wraps",
            MUL,
            &[(1, 0x1_0000), (2, 0x1_0001)],
            &[(4, 0x0001_0000)],
        ),
        mc(
            "mul negative",
            MUL,
            &[(1, 0xFFFF_FFFF), (2, 3)],
            &[(4, 0xFFFF_FFFD)],
        ),
        mc("mla", MLA, &[(1, 3), (2, 4), (3, 5)], &[(4, 17)]),
        mc(
            "mla wraps",
            MLA,
            &[(1, 0xFFFF_FFFF), (2, 2), (3, 3)],
            &[(4, 1)],
        ),
        mc(
            "mul Rd = Rm",
            mul_op((0, 0, 0, 0), 1, 0, 2, 1),
            &[(1, 5), (2, 6)],
            &[(1, 30)],
        ),
        mc(
            "mla Rd = Rm",
            mul_op((0, 0, 1, 0), 1, 3, 2, 1),
            &[(1, 5), (2, 6), (3, 1)],
            &[(1, 31)],
        ),
        mc(
            "mul ignores Rn",
            mul_op((0, 0, 0, 0), 4, 3, 2, 1),
            &[(1, 6), (2, 7), (3, 100)],
            &[(4, 42)],
        ),
    ];
    assert_eq!(mul_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_long_multiply_operands_when_executed_then_hi_and_lo_hold_the_64_bit_result() {
    let cases = [
        mc(
            "umull max",
            UMULL,
            &[(1, u32::MAX), (2, u32::MAX)],
            &[(4, 0xFFFF_FFFE), (3, 1)],
        ),
        mc(
            "smull minus one squared",
            SMULL,
            &[(1, u32::MAX), (2, u32::MAX)],
            &[(4, 0), (3, 1)],
        ),
        mc(
            "smull negative",
            SMULL,
            &[(1, u32::MAX), (2, 2)],
            &[(4, u32::MAX), (3, 0xFFFF_FFFE)],
        ),
        mc(
            "umull same bits unsigned",
            UMULL,
            &[(1, u32::MAX), (2, 2)],
            &[(4, 1), (3, 0xFFFF_FFFE)],
        ),
        mc(
            "umlal carries into hi",
            UMLAL,
            &[(1, 2), (2, 3), (4, 1), (3, u32::MAX)],
            &[(4, 2), (3, 5)],
        ),
        mc(
            "umlal wraps at 64",
            UMLAL,
            &[(1, 1), (2, 1), (4, u32::MAX), (3, u32::MAX)],
            &[(4, 0), (3, 0)],
        ),
        mc(
            "smlal",
            SMLAL,
            &[(1, 0xFFFF_FFFD), (2, 4), (4, 0), (3, 10)],
            &[(4, u32::MAX), (3, 0xFFFF_FFFE)],
        ),
        mc(
            "umull RdLo = Rm",
            mul_op((1, 0, 0, 0), 4, 1, 2, 1),
            &[(1, 0x10), (2, 0x10)],
            &[(1, 0x100), (4, 0)],
        ),
        mc(
            "umull RdHi = Rm",
            mul_op((1, 0, 0, 0), 1, 3, 2, 1),
            &[(1, 0x1_0000), (2, 0x1_0000)],
            &[(1, 1), (3, 0)],
        ),
    ];
    assert_eq!(mul_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_rdhi_equal_rdlo_when_umull_executes_then_the_hi_half_remains() {
    let both = mul_op((1, 0, 0, 0), 4, 4, 2, 1);
    let cpu = run(SVC, &[(1, u32::MAX), (2, u32::MAX)], both);
    assert_eq!(reg(&cpu, 4), 0xFFFF_FFFE);
}

#[test]
fn given_rdhi_equal_rdlo_when_umlal_executes_then_the_register_is_the_accumulator_of_both_halves_and_hi_remains()
 {
    let both = mul_op((1, 0, 1, 0), 4, 4, 2, 1);
    let cpu = run(SVC, &[(1, 1), (2, 1), (4, 1)], both);
    assert_eq!(reg(&cpu, 4), 1);
}

#[test]
fn given_r15_operands_when_multiply_executes_then_each_reads_the_address_plus_12() {
    let cases = [
        mc(
            "Rm",
            mul_op((0, 0, 0, 0), 4, 0, 2, 15),
            &[(2, 1)],
            &[(4, ADDR + 12)],
        ),
        mc(
            "Rs",
            mul_op((0, 0, 0, 0), 4, 0, 15, 1),
            &[(1, 1)],
            &[(4, ADDR + 12)],
        ),
        mc(
            "Rn",
            mul_op((0, 0, 1, 0), 4, 15, 2, 1),
            &[(1, 0), (2, 0)],
            &[(4, ADDR + 12)],
        ),
        mc(
            "RdLo accumulator",
            mul_op((1, 0, 1, 0), 4, 15, 2, 1),
            &[(1, 0), (2, 0), (4, 7)],
            &[(4, 7)],
        ),
    ];
    assert_eq!(mul_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_rd_r15_when_mul_executes_then_r15_is_the_value_plus_8() {
    let cpu = run(SVC, &[(1, 3), (2, 5)], mul_op((0, 0, 0, 0), 15, 0, 2, 1));
    assert_eq!(reg(&cpu, 15), 15 + 8);
}

#[test]
fn given_rdhi_r15_when_umull_executes_then_r15_is_hi_plus_8_and_lo_is_written() {
    let cpu = run(
        SVC,
        &[(1, 0x1_0000), (2, 0x1_0000), (3, 9)],
        mul_op((1, 0, 0, 0), 15, 3, 2, 1),
    );
    assert_eq!((reg(&cpu, 15), reg(&cpu, 3)), (1 + 8, 0));
}

#[test]
fn given_rdlo_r15_when_umull_executes_then_r15_is_lo_plus_8_and_hi_is_written() {
    let cpu = run(
        SVC,
        &[(1, 2), (2, 2), (4, 99)],
        mul_op((1, 0, 0, 0), 4, 15, 2, 1),
    );
    assert_eq!((reg(&cpu, 15), reg(&cpu, 4)), (4 + 8, 0));
}

#[test]
fn given_rdlo_r15_accumulator_when_umlal_executes_then_lo_is_the_address_plus_12_plus_the_product()
{
    let cpu = run(SVC, &[(1, 0), (2, 0)], mul_op((1, 0, 1, 0), 4, 15, 2, 1));
    assert_eq!(reg(&cpu, 15), ADDR + 12 + 8);
}

#[test]
fn given_plain_multiply_when_executed_then_r15_advances_by_4_from_the_execute_view() {
    let cpu = run(SVC, &[], MUL);
    assert_eq!(reg(&cpu, 15), ADDR + 12);
}

#[test]
fn given_failed_condition_when_mul_executes_then_registers_and_cpsr_are_kept_and_r15_advances() {
    let ne = with_cond(MUL, 1);
    let cpu = run(SVC | Z, &[(1, 6), (2, 7), (4, 55)], ne);
    assert_eq!(
        (reg(&cpu, 4), cpu.regs().cpsr(), reg(&cpu, 15)),
        (55, SVC | Z, ADDR + 12)
    );
}

/// CPSR after an instruction run from `SVC | flags` with the listed registers.
fn cpsr_after(flags: u32, setup: &[(usize, u32)], opcode: u32) -> u32 {
    run(SVC | flags, setup, opcode).regs().cpsr()
}

/// Cases of (name, opcode, setup, flags before, flags after) checked on CPSR.
struct FlagCase {
    name: &'static str,
    opcode: u32,
    setup: &'static [(usize, u32)],
    before: u32,
    after: u32,
}

const fn fc(
    name: &'static str,
    opcode: u32,
    setup: &'static [(usize, u32)],
    before: u32,
    after: u32,
) -> FlagCase {
    FlagCase {
        name,
        opcode,
        setup,
        before,
        after,
    }
}

fn flag_failures(cases: &[FlagCase]) -> Vec<String> {
    cases
        .iter()
        .filter_map(|c| {
            let got = cpsr_after(c.before, c.setup, c.opcode);
            (got != SVC | c.after)
                .then(|| format!("{}: got {got:#x}, want {:#x}", c.name, SVC | c.after))
        })
        .collect()
}

#[test]
fn given_s_multiplies_when_executed_then_n_and_z_follow_the_result_c_the_carry_model_and_v_is_kept()
{
    let cases = [
        fc("muls zero", MUL | S_BIT, &[(1, 0), (2, 5)], C | V, Z | V),
        fc(
            "muls negative",
            MUL | S_BIT,
            &[(1, 0x8000_0000), (2, 1)],
            0,
            N,
        ),
        fc(
            "muls clears n and z",
            MUL | S_BIT,
            &[(1, 1), (2, 1)],
            FLAGS,
            V,
        ),
        fc(
            "mlas zero",
            MLA | S_BIT,
            &[(1, 0), (2, 5), (3, 0)],
            V,
            Z | V,
        ),
        fc(
            "mlas negative",
            MLA | S_BIT,
            &[(1, 1), (2, 1), (3, 0x8000_0000)],
            C,
            N,
        ),
        fc(
            "umulls n from bit 63",
            UMULL | S_BIT,
            &[(1, u32::MAX), (2, u32::MAX)],
            0,
            N | C,
        ),
        fc(
            "umulls bit 31 is not n",
            UMULL | S_BIT,
            &[(1, 1), (2, 0x8000_0000)],
            0,
            0,
        ),
        fc(
            "umulls lo zero hi nonzero is not z",
            UMULL | S_BIT,
            &[(1, 0x1_0000), (2, 0x1_0000)],
            Z,
            0,
        ),
        fc(
            "umulls whole zero",
            UMULL | S_BIT,
            &[(1, 0), (2, 9)],
            C | V,
            Z | V,
        ),
        fc(
            "smulls negative",
            SMULL | S_BIT,
            &[(1, u32::MAX), (2, 1)],
            V,
            N | V,
        ),
        fc(
            "umlals",
            UMLAL | S_BIT,
            &[(1, 1), (2, 1), (4, 0), (3, 0)],
            N | Z,
            0,
        ),
        fc(
            "smlals zero",
            SMLAL | S_BIT,
            &[(1, 1), (2, 1), (4, u32::MAX), (3, u32::MAX)],
            C,
            Z,
        ),
    ];
    assert_eq!(flag_failures(&cases), Vec::<String>::new());
}

#[test]
fn given_multiplies_without_s_when_executed_then_the_cpsr_is_unchanged() {
    let cases = [
        fc("mul", MUL, &[(1, 0), (2, 5)], FLAGS, FLAGS),
        fc("mla", MLA, &[(1, 0x8000_0000), (2, 1), (3, 0)], 0, 0),
        fc("umull", UMULL, &[(1, 0), (2, 5)], N | C, N | C),
        fc("smlal", SMLAL, &[(1, u32::MAX), (2, 1)], Z, Z),
    ];
    assert_eq!(flag_failures(&cases), Vec::<String>::new());
}

/// CPSR flags and result of an S multiply run on the ARM9.
fn arm9_after(setup: &[(usize, u32)], opcode: u32, flags: u32) -> (u32, u32) {
    let mut cpu = Cpu::<true>::new();
    cpu.regs_mut().set_cpsr(SVC | flags);
    setup.iter().for_each(|&(i, v)| cpu.regs_mut().set(i, v));
    cpu.regs_mut().set(15, ADDR + 8);
    cpu.set_pipeline([opcode, 0]);
    cpu.step(&mut FixedBus(0));
    (cpu.regs().get(4), cpu.regs().cpsr())
}

#[test]
fn given_arm9_when_umulls_would_give_c_on_the_arm7_then_c_is_kept() {
    // 0xFFFF_FFFF squared is the "umulls n from bit 63" row above: the ARM7 sets C there.
    assert_eq!(
        arm9_after(&[(1, u32::MAX), (2, u32::MAX)], UMULL | S_BIT, 0).1,
        SVC | N
    );
}

#[test]
fn given_arm9_and_c_set_when_muls_gives_zero_then_z_is_set_and_c_is_kept() {
    assert_eq!(
        arm9_after(&[(1, 0), (2, 5)], MUL | S_BIT, C),
        (0, SVC | Z | C)
    );
}

#[test]
fn given_arm9_and_c_clear_when_mlas_gives_a_negative_then_n_is_set_and_c_stays_clear() {
    assert_eq!(
        arm9_after(&[(1, 1), (2, 1), (3, 0x8000_0000)], MLA | S_BIT, V),
        (0x8000_0001, SVC | N | V)
    );
}

#[test]
fn given_arm9_when_umulls_executes_then_n_comes_from_bit_63_and_c_is_kept() {
    assert_eq!(
        arm9_after(&[(1, u32::MAX), (2, u32::MAX)], UMULL | S_BIT, C),
        (0xFFFF_FFFE, SVC | N | C)
    );
}

#[test]
fn given_arm9_when_smlal_executes_then_the_result_matches_the_arm7() {
    assert_eq!(
        arm9_after(&[(1, 0xFFFF_FFFD), (2, 4), (3, 10)], SMLAL, 0),
        (u32::MAX, SVC)
    );
}

/// R4:R3 after a long multiply of `a` and `b` with accumulator `acc`.
fn long_result(opcode: u32, a: u32, b: u32, acc: u64) -> u64 {
    let cpu = run(
        SVC,
        &[(1, a), (2, b), (4, (acc >> 32) as u32), (3, acc as u32)],
        opcode,
    );
    u64::from(reg(&cpu, 4)) << 32 | u64::from(reg(&cpu, 3))
}

proptest! {
    #[test]
    fn given_any_operands_when_umull_executes_then_the_result_is_the_u64_product(
        a in any::<u32>(), b in any::<u32>(),
    ) {
        prop_assert_eq!(long_result(UMULL, a, b, 0), u64::from(a) * u64::from(b));
    }

    #[test]
    fn given_any_operands_when_smull_executes_then_the_result_is_the_i64_product(
        a in any::<u32>(), b in any::<u32>(),
    ) {
        prop_assert_eq!(
            long_result(SMULL, a, b, 0),
            (i64::from(a as i32) * i64::from(b as i32)) as u64
        );
    }

    #[test]
    fn given_any_operands_when_umlal_executes_then_the_result_wraps_at_64_bits(
        a in any::<u32>(), b in any::<u32>(), acc in any::<u64>(),
    ) {
        prop_assert_eq!(
            long_result(UMLAL, a, b, acc),
            (u64::from(a) * u64::from(b)).wrapping_add(acc)
        );
    }

    #[test]
    fn given_any_operands_when_smlal_executes_then_the_result_wraps_at_64_bits(
        a in any::<u32>(), b in any::<u32>(), acc in any::<u64>(),
    ) {
        prop_assert_eq!(
            long_result(SMLAL, a, b, acc),
            ((i64::from(a as i32) * i64::from(b as i32)) as u64).wrapping_add(acc)
        );
    }

    #[test]
    fn given_any_operands_when_mul_and_mla_execute_then_rd_wraps_at_32_bits(
        a in any::<u32>(), b in any::<u32>(), n in any::<u32>(),
    ) {
        let mul = run(SVC, &[(1, a), (2, b)], MUL);
        let mla = run(SVC, &[(1, a), (2, b), (3, n)], MLA);
        prop_assert_eq!(
            (reg(&mul, 4), reg(&mla, 4)),
            (a.wrapping_mul(b), a.wrapping_mul(b).wrapping_add(n))
        );
    }

    #[test]
    fn given_any_operands_when_muls_executes_then_n_and_z_follow_the_32_bit_result_and_v_is_kept(
        a in any::<u32>(), b in any::<u32>(), f in 0u32..16,
    ) {
        let want = a.wrapping_mul(b);
        // C comes from the carry model, which `tests/multiply_carry.rs` pins.
        prop_assert_eq!(
            cpsr_after(f << 28, &[(1, a), (2, b)], MUL | S_BIT) & !C,
            SVC | (f << 28 & V) | want & N | u32::from(want == 0) << 30
        );
    }
}
