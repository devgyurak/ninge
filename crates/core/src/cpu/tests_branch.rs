// SPDX-License-Identifier: Apache-2.0
//! B, BL, BX and the prefetch pipeline through `Cpu::step`. Expected values come from
//! `docs/hw/arm-branch-pipeline.md`: a step takes `pipeline[0]`, fetches at R15 (address + 8),
//! shifts the pipeline, executes with R15 = address + 8, then either advances R15 by 4 or
//! refills the pipeline from the written R15.
//!
//! The recording bus answers a fetch with the address it was asked for, so the pipeline
//! contents show which address each entry came from.

use super::*;
use proptest::prelude::*;

/// Address of the instruction under test.
const ADDR: u32 = 0x1000;
/// R15 while it executes (address + 8).
const PC_READ: u32 = ADDR + 8;
/// The opcode already sitting in `pipeline[1]` before the step.
const NEXT: u32 = 0xAAAA_0001;
/// CPSR of SVC mode, ARM state, flags clear.
const SVC: u32 = 0x13;
/// The T (Thumb) bit of the CPSR.
const T_BIT: u32 = 0x20;
const Z: u32 = 0x4000_0000;
const LR_MARK: u32 = 0xDEAD_BEEF;

/// One code fetch: width in bits and address.
type Fetch = (u8, u32);

/// A bus that logs every code fetch and answers it with the fetched address. Data reads all
/// return `data`.
struct RecBus {
    fetches: Vec<Fetch>,
    data: u32,
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
    fn read8(&mut self, _addr: u32) -> u8 {
        self.data as u8
    }
    fn read16(&mut self, _addr: u32) -> u16 {
        self.data as u16
    }
    fn read32(&mut self, _addr: u32) -> u32 {
        self.data
    }
    fn write8(&mut self, _addr: u32, _value: u8) {}
    fn write16(&mut self, _addr: u32, _value: u16) {}
    fn write32(&mut self, _addr: u32, _value: u32) {}
}

/// The CPU after one step and the fetches the step made.
struct Stepped {
    cpu: Cpu<false>,
    fetches: Vec<Fetch>,
}

impl Stepped {
    fn r15(&self) -> u32 {
        self.cpu.regs().get(15)
    }
    fn lr(&self) -> u32 {
        self.cpu.regs().get(14)
    }
    fn thumb(&self) -> bool {
        self.cpu.regs().cpsr() & T_BIT != 0
    }
}

/// Runs `opcode` as the instruction at `ADDR`; `data` is what data reads return.
fn exec_with(cpsr: u32, setup: &[(usize, u32)], data: u32, opcode: u32) -> Stepped {
    let mut cpu = Cpu::<false>::new();
    cpu.regs_mut().set_cpsr(cpsr);
    setup.iter().for_each(|&(i, v)| cpu.regs_mut().set(i, v));
    cpu.regs_mut().set(15, PC_READ);
    cpu.set_pipeline([opcode, NEXT]);
    let mut bus = RecBus {
        fetches: Vec::new(),
        data,
    };
    cpu.step(&mut bus);
    Stepped {
        cpu,
        fetches: bus.fetches,
    }
}

fn exec(cpsr: u32, setup: &[(usize, u32)], opcode: u32) -> Stepped {
    exec_with(cpsr, setup, 0, opcode)
}

/// `B`/`BL` with condition AL and a 24-bit word offset.
const fn branch(link: u32, offset24: u32) -> u32 {
    0xEA00_0000 | link << 24 | offset24
}

/// The word fetch pair of an ARM refill at `target`, after the step fetch at `ADDR + 8`.
fn arm_fetches(target: u32) -> Vec<Fetch> {
    vec![(32, PC_READ), (32, target & !3), (32, (target & !3) + 4)]
}

/// R15 after `BX` to `target`: Thumb refill gives `(t & !1) + 4`, ARM refill `t + 8`.
fn bx_expected_r15(target: u32) -> u32 {
    if target & 1 == 1 {
        (target & !1).wrapping_add(4)
    } else {
        target.wrapping_add(8)
    }
}

// ---------------------------------------------------------------- B and BL

#[test]
fn given_b_forward_when_executed_then_r15_is_target_plus_8_and_pipeline_holds_target_words() {
    // Offset 4 words: target = 0x1008 + 16 = 0x1018.
    let s = exec(SVC, &[], branch(0, 4));
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (0x1020, [0x1018, 0x101C], arm_fetches(0x1018))
    );
}

#[test]
fn given_b_backward_when_executed_then_it_lands_below_the_instruction() {
    // Offset -4 words: target = 0x1008 - 16 = 0xFF8.
    let s = exec(SVC, &[], branch(0, 0xFF_FFFC));
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (0x1000, [0xFF8, 0xFFC], arm_fetches(0xFF8))
    );
}

#[test]
fn given_b_with_offset_minus_2_when_executed_then_it_branches_to_itself() {
    // `B .` encodes -2 words: target = 0x1008 - 8 = 0x1000.
    let s = exec(SVC, &[], branch(0, 0xFF_FFFE));
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (0x1008, [0x1000, 0x1004], arm_fetches(0x1000))
    );
}

#[test]
fn given_b_does_not_link_when_executed_then_lr_is_untouched() {
    let s = exec(SVC, &[(14, LR_MARK)], branch(0, 4));
    assert_eq!(s.lr(), LR_MARK);
}

#[test]
fn given_bl_forward_when_executed_then_lr_is_next_instruction_and_it_branches() {
    let s = exec(SVC, &[], branch(1, 4));
    assert_eq!(
        (s.lr(), s.r15(), s.cpu.pipeline(), s.fetches),
        (0x1004, 0x1020, [0x1018, 0x101C], arm_fetches(0x1018))
    );
}

#[test]
fn given_bl_backward_when_executed_then_lr_is_next_instruction_and_it_lands_below() {
    let s = exec(SVC, &[], branch(1, 0xFF_FFFC));
    assert_eq!(
        (s.lr(), s.r15(), s.cpu.pipeline(), s.fetches),
        (0x1004, 0x1000, [0xFF8, 0xFFC], arm_fetches(0xFF8))
    );
}

#[test]
fn given_b_with_largest_positive_offset_when_executed_then_it_reaches_32_mib_ahead() {
    // 0x7FFFFF words << 2 = 0x1FF_FFFC; target = 0x1008 + 0x1FF_FFFC = 0x200_1004.
    let s = exec(SVC, &[], branch(0, 0x7F_FFFF));
    assert_eq!((s.r15(), s.fetches), (0x200_100C, arm_fetches(0x200_1004)));
}

#[test]
fn given_b_with_most_negative_offset_when_executed_then_it_sign_extends_32_mib_back() {
    // 0x800000 sign-extends to -0x80_0000 words = -0x200_0000 bytes;
    // target = 0x1008 - 0x200_0000 = 0xFE00_1008 (wrapping).
    let s = exec(SVC, &[], branch(0, 0x80_0000));
    assert_eq!(
        (s.r15(), s.fetches),
        (0xFE00_1010, arm_fetches(0xFE00_1008))
    );
}

// ---------------------------------------------------------------- BX

#[test]
fn given_bx_to_even_address_when_executed_then_it_stays_in_arm_state_and_refills_words() {
    let s = exec(SVC, &[(1, 0x2000)], 0xE12F_FF11);
    assert_eq!(
        (s.thumb(), s.r15(), s.cpu.pipeline(), s.fetches),
        (false, 0x2008, [0x2000, 0x2004], arm_fetches(0x2000))
    );
}

#[test]
fn given_bx_to_odd_address_when_executed_then_t_is_set_and_it_refills_two_halfwords() {
    let s = exec(SVC, &[(1, 0x2001)], 0xE12F_FF11);
    assert_eq!(
        (s.thumb(), s.r15(), s.cpu.pipeline(), s.fetches),
        (
            true,
            0x2004,
            [0x2000, 0x2002],
            vec![(32, PC_READ), (16, 0x2000), (16, 0x2002)]
        )
    );
}

#[test]
fn given_bx_to_odd_address_when_executed_then_flags_and_mode_are_untouched() {
    let s = exec(SVC | Z, &[(1, 0x2001)], 0xE12F_FF11);
    assert_eq!(s.cpu.regs().cpsr(), SVC | Z | T_BIT);
}

#[test]
fn given_bx_to_address_with_bit1_set_when_executed_then_fetches_align_but_r15_keeps_the_bits() {
    // Unverified (TODO(hw-unverified) in the note): R15 = target + 8 stays unaligned.
    let s = exec(SVC, &[(1, 0x2002)], 0xE12F_FF11);
    assert_eq!(
        (s.thumb(), s.r15(), s.fetches),
        (false, 0x200A, arm_fetches(0x2002))
    );
}

#[test]
fn given_bx_r15_when_executed_then_the_target_is_the_instruction_address_plus_8() {
    // Rn = R15 reads address + 8 = 0x1008 (even), so it refills in ARM state from there.
    let s = exec(SVC, &[], 0xE12F_FF1F);
    assert_eq!(
        (s.thumb(), s.r15(), s.fetches),
        (false, 0x1010, arm_fetches(0x1008))
    );
}

#[test]
fn given_bx_when_executed_then_lr_is_untouched() {
    let s = exec(SVC, &[(1, 0x2001), (14, LR_MARK)], 0xE12F_FF11);
    assert_eq!(s.lr(), LR_MARK);
}

// ---------------------------------------------------------------- failed conditions

/// The failed-condition cases: B, BL and BX with a condition that does not hold (EQ, Z clear).
/// Each must leave LR and the CPSR alone, shift the pipeline once and advance R15 by 4.
fn failed_condition_failures() -> Vec<String> {
    let cases = [
        ("beq", 0x0A00_0004_u32),
        ("bleq", 0x0B00_0004),
        ("bxeq", 0x012F_FF11),
    ];
    cases
        .iter()
        .filter_map(|&(name, opcode)| {
            let s = exec(SVC, &[(1, 0x2001), (14, LR_MARK)], opcode);
            let got = (
                s.r15(),
                s.cpu.pipeline(),
                s.lr(),
                s.cpu.regs().cpsr(),
                s.fetches,
            );
            let want = (
                PC_READ + 4,
                [NEXT, PC_READ],
                LR_MARK,
                SVC,
                vec![(32, PC_READ)],
            );
            (got != want).then(|| format!("{name}: got {got:x?}, want {want:x?}"))
        })
        .collect()
}

#[test]
fn given_branches_with_failed_condition_when_executed_then_only_the_normal_shift_and_advance_happen()
 {
    assert_eq!(failed_condition_failures(), Vec::<String>::new());
}

// ---------------------------------------------------------------- the pipeline of a plain step

#[test]
fn given_plain_instruction_when_executed_then_pipeline_shifts_in_the_word_fetched_at_r15() {
    // MOV R0, #1 does not write R15: one fetch at address + 8, R15 advances by 4.
    let s = exec(SVC, &[], 0xE3A0_0001);
    assert_eq!(
        (s.cpu.pipeline(), s.r15(), s.fetches),
        ([NEXT, PC_READ], PC_READ + 4, vec![(32, PC_READ)])
    );
}

// ---------------------------------------------------------------- refill after other R15 writes

#[test]
fn given_mov_pc_immediate_when_executed_then_it_refills_from_the_immediate() {
    // MOV PC, #0x2000 (0x20 ror 24).
    let s = exec(SVC, &[], 0xE3A0_FC20);
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (0x2008, [0x2000, 0x2004], arm_fetches(0x2000))
    );
}

#[test]
fn given_add_pc_pc_8_when_executed_then_it_refills_from_address_plus_16() {
    // ADD PC, PC, #8: R15 reads 0x1008, result 0x1010.
    let s = exec(SVC, &[], 0xE28F_F008);
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (0x1018, [0x1010, 0x1014], arm_fetches(0x1010))
    );
}

#[test]
fn given_mov_pc_unaligned_register_when_executed_then_fetches_align_and_r15_keeps_the_bits() {
    // MOV PC, R1 with R1 = 0x2002; the note keeps the low bits (TODO(hw-unverified)).
    let s = exec(SVC, &[(1, 0x2002)], 0xE1A0_F001);
    assert_eq!((s.r15(), s.fetches), (0x200A, arm_fetches(0x2002)));
}

#[test]
fn given_ldr_pc_when_executed_then_it_refills_from_the_loaded_word() {
    // LDR PC, [R0] loading 0x2000.
    let s = exec_with(SVC, &[(0, 0x3000)], 0x2000, 0xE590_F000);
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (0x2008, [0x2000, 0x2004], arm_fetches(0x2000))
    );
}

#[test]
fn given_ldm_with_pc_in_list_when_executed_then_it_refills_from_the_loaded_word() {
    // LDMIA R0, {PC} loading 0x2000.
    let s = exec_with(SVC, &[(0, 0x3000)], 0x2000, 0xE890_8000);
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (0x2008, [0x2000, 0x2004], arm_fetches(0x2000))
    );
}

#[test]
fn given_mrs_into_r15_when_executed_then_r15_is_value_plus_4_and_there_is_no_refill() {
    // Unverified (TODO(hw-unverified) in the note): MRS PC, CPSR is a plain write followed by
    // the normal advance, so R15 = CPSR + 4 and only the step fetch happens.
    let s = exec(SVC, &[], 0xE10F_F000);
    assert_eq!(
        (s.r15(), s.cpu.pipeline(), s.fetches),
        (SVC + 4, [NEXT, PC_READ], vec![(32, PC_READ)])
    );
}

#[test]
fn given_ldr_writeback_to_r15_when_executed_then_it_flushes_to_the_moved_address_plus_4() {
    // Unverified (suite): LDR R1,[R15,#4]! moves R15 (0x1008) to 0x100C and flushes to + 4.
    let s = exec(SVC, &[], 0xE5BF_1004);
    assert_eq!((s.r15(), s.fetches), (0x1018, arm_fetches(0x1010)));
}

#[test]
fn given_ldm_writeback_to_r15_when_executed_then_it_flushes_to_the_moved_address() {
    // Unverified (suite): LDMIA R15!,{R1} moves R15 (0x1008) to 0x100C and flushes there.
    let s = exec(SVC, &[], 0xE8BF_0002);
    assert_eq!((s.r15(), s.fetches), (0x1014, arm_fetches(0x100C)));
}

#[test]
fn given_mul_into_r15_when_executed_then_it_refills_from_the_product() {
    // MUL PC, R1, R2 = 3 * 4 = 12.
    let s = exec(SVC, &[(1, 3), (2, 4)], 0xE00F_0291);
    assert_eq!((s.r15(), s.fetches), (20, arm_fetches(12)));
}

#[test]
fn given_umull_with_rdhi_r15_when_executed_then_it_refills_from_the_high_word() {
    // UMULL R0, PC, R1, R2 = 0x1000_0000 * 0x10 = 0x1_0000_0000, so RdHi = 1; the low bit is
    // kept in R15 and dropped for the fetches.
    let s = exec(SVC, &[(1, 0x1000_0000), (2, 0x10)], 0xE08F_0291);
    assert_eq!((s.r15(), s.fetches), (9, arm_fetches(1)));
}

#[test]
fn given_swp_into_r15_when_executed_then_it_refills_from_the_old_memory_word() {
    // SWP PC, R1, [R0] with the memory word 0x2000.
    let s = exec_with(SVC, &[(0, 0x3000), (1, 0x77)], 0x2000, 0xE100_F091);
    assert_eq!((s.r15(), s.fetches), (0x2008, arm_fetches(0x2000)));
}

#[test]
fn given_ldm_pc_with_s_and_a_thumb_spsr_when_executed_then_it_refills_as_thumb() {
    // LDMIA R0,{PC}^ restores the CPSR from SPSR_svc (User mode, T set), then refills in
    // Thumb state from 0x2001: halfwords at 0x2000 and 0x2002, R15 = 0x2004.
    let mut cpu = Cpu::<false>::new();
    cpu.regs_mut().set_cpsr(SVC);
    cpu.regs_mut().set_spsr(Bank::Supervisor, 0x10 | T_BIT);
    cpu.regs_mut().set(0, 0x3000);
    cpu.regs_mut().set(15, PC_READ);
    cpu.set_pipeline([0xE8D0_8000, NEXT]);
    let mut bus = RecBus {
        fetches: Vec::new(),
        data: 0x2001,
    };
    cpu.step(&mut bus);
    assert_eq!(
        (cpu.regs().get(15), cpu.pipeline(), bus.fetches),
        (
            0x2004,
            [0x2000, 0x2002],
            vec![(32, PC_READ), (16, 0x2000), (16, 0x2002)]
        )
    );
}

proptest! {
    #[test]
    fn given_any_b_offset_when_executed_then_r15_is_address_plus_8_plus_offset_plus_8(
        offset in 0u32..0x100_0000,
    ) {
        // Sign-extend the 24-bit field, shift by 2, add to R15 (address + 8), refill adds 8.
        let delta = ((offset << 8) as i32 >> 6) as u32;
        let target = PC_READ.wrapping_add(delta);
        let s = exec(SVC, &[], branch(0, offset));
        prop_assert_eq!(s.r15(), target.wrapping_add(8));
        prop_assert_eq!(s.fetches, arm_fetches(target));
    }

    #[test]
    fn given_any_bl_offset_when_executed_then_lr_is_address_plus_4(offset in 0u32..0x100_0000) {
        let s = exec(SVC, &[], branch(1, offset));
        prop_assert_eq!(s.lr(), ADDR + 4);
    }

    #[test]
    fn given_any_bx_target_when_executed_then_t_is_bit0_and_r15_follows_the_state(
        target in any::<u32>(),
    ) {
        let s = exec(SVC, &[(1, target)], 0xE12F_FF11);
        prop_assert_eq!(s.thumb(), target & 1 == 1);
        prop_assert_eq!(s.r15(), bx_expected_r15(target));
    }
}
