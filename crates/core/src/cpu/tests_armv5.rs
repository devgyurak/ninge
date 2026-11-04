// SPDX-License-Identifier: Apache-2.0
//! The ARMv5TE extensions through `Cpu::step`. Expected values come from `docs/hw/armv5te.md`,
//! never from running the implementation. The ARM7 tests pin that the same encodings keep their
//! ARMv4T behavior.
//!
//! The bus answers a code fetch with the address it was asked for, so the pipeline shows which
//! address each entry came from; a data read returns the value placed at that address, or a
//! marker for unset memory. R15 reads the executing address + 8 (ARM) or + 4 (Thumb).

use super::*;
use std::collections::BTreeMap;

/// Address of the instruction under test.
const ADDR: u32 = 0x1000;
/// The opcode already sitting in `pipeline[1]`.
const NEXT: u32 = 0xAAAA_0001;
/// What unset memory reads as.
const UNSET: u32 = 0x0BAD_0BAD;
const T_BIT: u32 = 0x20;
const Q: u32 = 0x0800_0000;
const N: u32 = 0x8000_0000;
const Z: u32 = 0x4000_0000;
const C: u32 = 0x2000_0000;
const V: u32 = 0x1000_0000;
const NZCV: u32 = N | Z | C | V;
const SVC: u32 = 0x13;
const ABT: u32 = 0x17;
const SYS: u32 = 0x1F;
const IRQ_OFF: u32 = 0x80;

/// A memory-backed bus that logs data writes and answers fetches with their address.
struct Mem {
    cells: BTreeMap<u32, u32>,
    writes: Vec<(u32, u32)>,
    reads: Vec<u32>,
}

impl Mem {
    fn new(cells: &[(u32, u32)]) -> Self {
        Self {
            cells: cells.iter().copied().collect(),
            writes: Vec::new(),
            reads: Vec::new(),
        }
    }

    fn cell(&self, addr: u32) -> u32 {
        self.cells.get(&addr).copied().unwrap_or(UNSET)
    }
}

impl Bus for Mem {
    fn fetch32(&mut self, addr: u32) -> u32 {
        addr
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        addr as u16
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.reads.push(addr);
        self.cell(addr) as u8
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.reads.push(addr);
        self.cell(addr) as u16
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.reads.push(addr);
        self.cell(addr)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.writes.push((addr, u32::from(value)));
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.writes.push((addr, u32::from(value)));
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.writes.push((addr, value));
    }
}

/// A core after one step and the bus it ran against.
struct Run<const ARM9: bool> {
    cpu: Cpu<ARM9>,
    bus: Mem,
}

impl<const ARM9: bool> Run<ARM9> {
    fn r(&self, index: usize) -> u32 {
        self.cpu.regs().get(index)
    }
    fn cpsr(&self) -> u32 {
        self.cpu.regs().cpsr()
    }
    fn thumb(&self) -> bool {
        self.cpsr() & T_BIT != 0
    }
}

/// The R15 value an instruction reads: the executing address + 8 in ARM state, + 4 in Thumb.
fn pc_read(cpsr: u32) -> u32 {
    match cpsr & T_BIT != 0 {
        true => ADDR + 4,
        false => ADDR + 8,
    }
}

/// Steps `word` as the instruction at `ADDR` with the given CPSR, registers and memory.
fn run<const ARM9: bool>(
    cpsr: u32,
    regs: &[(usize, u32)],
    mem: &[(u32, u32)],
    word: u32,
) -> Run<ARM9> {
    let mut cpu = Cpu::<ARM9>::new();
    cpu.regs_mut().set_cpsr(cpsr);
    cpu.regs_mut().set(15, pc_read(cpsr));
    regs.iter().for_each(|&(i, v)| cpu.regs_mut().set(i, v));
    cpu.set_pipeline([word, NEXT]);
    let mut bus = Mem::new(mem);
    cpu.step(&mut bus);
    Run { cpu, bus }
}

/// ARM state, supervisor mode, flags clear.
fn arm<const ARM9: bool>(regs: &[(usize, u32)], mem: &[(u32, u32)], word: u32) -> Run<ARM9> {
    run::<ARM9>(SVC, regs, mem, word)
}

/// Thumb state, supervisor mode, flags clear.
fn thumb<const ARM9: bool>(regs: &[(usize, u32)], mem: &[(u32, u32)], halfword: u16) -> Run<ARM9> {
    run::<ARM9>(SVC | T_BIT, regs, mem, u32::from(halfword))
}

// ---------------------------------------------------------------- the switch to Thumb

/// LDR PC, [R0].
const LDR_PC: u32 = 0xE590_F000;
/// LDMIA R0, {PC} and LDMIA R0, {PC}^.
const LDM_PC: u32 = 0xE890_8000;
const LDM_PC_S: u32 = 0xE8D0_8000;
/// Thumb POP {PC}.
const POP_PC: u16 = 0xBD00;

#[test]
fn given_an_arm9_ldr_pc_when_the_loaded_bit_0_is_set_then_the_core_enters_thumb_at_the_even_target()
{
    let r = arm::<true>(&[(0, 0x100)], &[(0x100, 0x2001)], LDR_PC);
    assert_eq!(
        (r.thumb(), r.r(15), r.cpu.pipeline()),
        (true, 0x2004, [0x2000, 0x2002])
    );
}

#[test]
fn given_an_arm9_ldr_pc_when_the_loaded_bit_0_is_clear_then_the_core_stays_in_arm_state() {
    let r = arm::<true>(&[(0, 0x100)], &[(0x100, 0x2000)], LDR_PC);
    assert_eq!(
        (r.thumb(), r.r(15), r.cpu.pipeline()),
        (false, 0x2008, [0x2000, 0x2004])
    );
}

#[test]
fn given_an_arm7_ldr_pc_when_the_loaded_bit_0_is_set_then_the_state_stays_arm() {
    let r = arm::<false>(&[(0, 0x100)], &[(0x100, 0x2001)], LDR_PC);
    assert_eq!((r.thumb(), r.r(15)), (false, 0x2009));
}

#[test]
fn given_an_arm9_ldrb_into_pc_when_executed_then_the_state_does_not_change() {
    // LDRB PC, [R0]: only a word load interworks.
    let r = arm::<true>(&[(0, 0x100)], &[(0x100, 0x01)], 0xE5D0_F000);
    assert!(!r.thumb());
}

#[test]
fn given_an_arm9_ldm_with_pc_when_the_loaded_bit_0_is_set_then_the_core_enters_thumb() {
    let r = arm::<true>(&[(0, 0x100)], &[(0x100, 0x3001)], LDM_PC);
    assert_eq!(
        (r.thumb(), r.r(15), r.cpu.pipeline()),
        (true, 0x3004, [0x3000, 0x3002])
    );
}

#[test]
fn given_an_arm9_ldm_with_pc_when_the_loaded_bit_0_is_clear_then_the_core_stays_in_arm_state() {
    let r = arm::<true>(&[(0, 0x100)], &[(0x100, 0x3000)], LDM_PC);
    assert_eq!((r.thumb(), r.r(15)), (false, 0x3008));
}

#[test]
fn given_an_arm7_ldm_with_pc_when_the_loaded_bit_0_is_set_then_the_state_stays_arm() {
    let r = arm::<false>(&[(0, 0x100)], &[(0x100, 0x3001)], LDM_PC);
    assert_eq!((r.thumb(), r.r(15)), (false, 0x3009));
}

#[test]
fn given_an_arm9_ldm_pc_with_s_when_the_loaded_bit_0_is_set_then_t_comes_from_the_spsr() {
    let mut cpu = Cpu::<true>::new();
    cpu.regs_mut().set_cpsr(SVC);
    cpu.regs_mut().set_spsr(Bank::Supervisor, SYS);
    cpu.regs_mut().set(0, 0x100);
    cpu.regs_mut().set(15, ADDR + 8);
    cpu.set_pipeline([LDM_PC_S, NEXT]);
    let mut bus = Mem::new(&[(0x100, 0x3001)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs().cpsr(), SYS);
}

#[test]
fn given_an_arm9_ldm_pc_with_s_when_the_spsr_has_t_set_then_the_core_enters_thumb_whatever_bit_0_is()
 {
    let mut cpu = Cpu::<true>::new();
    cpu.regs_mut().set_cpsr(SVC);
    cpu.regs_mut().set_spsr(Bank::Supervisor, SYS | T_BIT);
    cpu.regs_mut().set(0, 0x100);
    cpu.regs_mut().set(15, ADDR + 8);
    cpu.set_pipeline([LDM_PC_S, NEXT]);
    let mut bus = Mem::new(&[(0x100, 0x3000)]);
    cpu.step(&mut bus);
    assert_eq!(
        (cpu.regs().cpsr(), cpu.regs().get(15)),
        (SYS | T_BIT, 0x3004)
    );
}

#[test]
fn given_an_arm9_thumb_pop_pc_when_the_popped_bit_0_is_clear_then_the_core_enters_arm_state() {
    let r = thumb::<true>(&[(13, 0x100)], &[(0x100, 0x4000)], POP_PC);
    assert_eq!(
        (r.thumb(), r.r(13), r.r(15), r.cpu.pipeline()),
        (false, 0x104, 0x4008, [0x4000, 0x4004])
    );
}

#[test]
fn given_an_arm9_thumb_pop_pc_when_the_popped_bit_0_is_set_then_the_core_stays_in_thumb() {
    let r = thumb::<true>(&[(13, 0x100)], &[(0x100, 0x4001)], POP_PC);
    assert_eq!(
        (r.thumb(), r.r(15), r.cpu.pipeline()),
        (true, 0x4004, [0x4000, 0x4002])
    );
}

#[test]
fn given_an_arm7_thumb_pop_pc_when_the_popped_bit_0_is_clear_then_the_core_stays_in_thumb() {
    let r = thumb::<false>(&[(13, 0x100)], &[(0x100, 0x4000)], POP_PC);
    assert!(r.thumb());
}

// ---------------------------------------------------------------- BLX

/// R14.
const LR: usize = 14;
/// BLX R1 in ARM state, with condition AL and with condition NE.
const BLX_R1: u32 = 0xE12F_FF31;
const BLX_R1_NE: u32 = 0x112F_FF31;
const BLX_LR: u32 = 0xE12F_FF3E;
/// Thumb BLX R1 (`0100 0111 1 Rm 000`).
const THUMB_BLX_R1: u16 = 0x4788;
/// BLX immediate: condition field 1111, bits 27–25 = 101, H in bit 24.
const BLX_PLUS_0: u32 = 0xFA00_0000;
const BLX_PLUS_0_H: u32 = 0xFB00_0000;
const BLX_MINUS_8: u32 = 0xFAFF_FFFE;
/// Thumb BLX suffix (`11101 offset11`) with offset 4 and with offset 2.
const THUMB_BLX_SUFFIX_4: u16 = 0xE804;
const THUMB_BLX_SUFFIX_2: u16 = 0xE802;
/// Thumb BL/BLX prefix with offset 1 (LR = R15 + 0x1000).
const THUMB_PREFIX_1: u16 = 0xF001;

#[test]
fn given_an_arm9_blx_immediate_when_executed_then_it_links_enters_thumb_and_jumps_to_r15_plus_the_offset()
 {
    let r = arm::<true>(&[], &[], BLX_PLUS_0);
    assert_eq!(
        (r.thumb(), r.r(LR), r.r(15), r.cpu.pipeline()),
        (true, ADDR + 4, 0x100C, [0x1008, 0x100A])
    );
}

#[test]
fn given_an_arm9_blx_immediate_when_h_is_set_then_the_target_is_a_halfword_further() {
    let r = arm::<true>(&[], &[], BLX_PLUS_0_H);
    assert_eq!((r.r(15), r.cpu.pipeline()), (0x100E, [0x100A, 0x100C]));
}

#[test]
fn given_an_arm9_blx_immediate_when_the_offset_is_negative_then_the_target_is_before_r15() {
    let r = arm::<true>(&[], &[], BLX_MINUS_8);
    assert_eq!((r.thumb(), r.cpu.pipeline()), (true, [0x1000, 0x1002]));
}

#[test]
fn given_an_arm9_blx_immediate_when_every_flag_is_set_then_it_still_executes() {
    let r = run::<true>(SVC | NZCV, &[], &[], BLX_PLUS_0);
    assert_eq!((r.thumb(), r.r(LR)), (true, ADDR + 4));
}

#[test]
fn given_an_arm7_word_with_condition_1111_when_executed_then_nothing_happens_but_the_advance() {
    let r = arm::<false>(&[(LR, 0x77)], &[], BLX_PLUS_0);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (false, 0x77, 0x100C));
}

#[test]
fn given_an_arm9_blx_register_when_bit_0_is_set_then_it_links_and_enters_thumb() {
    let r = arm::<true>(&[(1, 0x2001)], &[], BLX_R1);
    assert_eq!(
        (r.thumb(), r.r(LR), r.r(15), r.cpu.pipeline()),
        (true, ADDR + 4, 0x2004, [0x2000, 0x2002])
    );
}

#[test]
fn given_an_arm9_blx_register_when_bit_0_is_clear_then_it_links_and_stays_in_arm_state() {
    let r = arm::<true>(&[(1, 0x2000)], &[], BLX_R1);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (false, ADDR + 4, 0x2008));
}

#[test]
fn given_an_arm9_blx_lr_when_executed_then_the_old_lr_is_the_target() {
    let r = arm::<true>(&[(LR, 0x2001)], &[], BLX_LR);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (true, ADDR + 4, 0x2004));
}

#[test]
fn given_an_arm9_blx_register_when_its_condition_fails_then_nothing_happens_but_the_advance() {
    let r = run::<true>(SVC | Z, &[(1, 0x2001), (LR, 0x77)], &[], BLX_R1_NE);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (false, 0x77, 0x100C));
}

#[test]
fn given_an_arm7_blx_register_word_when_executed_then_nothing_happens_but_the_advance() {
    let r = arm::<false>(&[(1, 0x2001), (LR, 0x77)], &[], BLX_R1);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (false, 0x77, 0x100C));
}

#[test]
fn given_an_arm9_thumb_blx_register_when_bit_0_is_clear_then_it_links_with_bit_0_set_and_enters_arm_state()
 {
    let r = thumb::<true>(&[(1, 0x2000)], &[], THUMB_BLX_R1);
    assert_eq!(
        (r.thumb(), r.r(LR), r.r(15), r.cpu.pipeline()),
        (false, ADDR + 3, 0x2008, [0x2000, 0x2004])
    );
}

#[test]
fn given_an_arm9_thumb_blx_register_when_bit_0_is_set_then_it_links_and_stays_in_thumb() {
    let r = thumb::<true>(&[(1, 0x2001)], &[], THUMB_BLX_R1);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (true, ADDR + 3, 0x2004));
}

#[test]
fn given_an_arm7_thumb_bx_with_h1_set_when_executed_then_it_branches_without_linking() {
    let r = thumb::<false>(&[(1, 0x2001), (LR, 0x77)], &[], THUMB_BLX_R1);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (true, 0x77, 0x2004));
}

#[test]
fn given_an_arm9_thumb_blx_suffix_when_executed_then_it_jumps_word_aligned_in_arm_state() {
    let r = thumb::<true>(&[(LR, 0x2002)], &[], THUMB_BLX_SUFFIX_4);
    assert_eq!(
        (r.thumb(), r.r(LR), r.r(15), r.cpu.pipeline()),
        (false, ADDR + 3, 0x2010, [0x2008, 0x200C])
    );
}

#[test]
fn given_an_arm9_thumb_prefix_and_blx_suffix_when_both_execute_then_lr_returns_past_the_pair() {
    let mut cpu = Cpu::<true>::new();
    cpu.regs_mut().set_cpsr(SVC | T_BIT);
    cpu.regs_mut().set(15, ADDR + 4);
    cpu.set_pipeline([u32::from(THUMB_PREFIX_1), u32::from(THUMB_BLX_SUFFIX_2)]);
    let mut bus = Mem::new(&[]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(
        (
            cpu.regs().cpsr() & T_BIT,
            cpu.regs().get(LR),
            cpu.regs().get(15)
        ),
        (0, ADDR + 5, 0x2010)
    );
}

#[test]
fn given_an_arm7_thumb_blx_suffix_when_executed_then_nothing_happens_but_the_advance() {
    let r = thumb::<false>(&[(LR, 0x2002)], &[], THUMB_BLX_SUFFIX_4);
    assert_eq!((r.thumb(), r.r(LR), r.r(15)), (true, 0x2002, ADDR + 6));
}

// ---------------------------------------------------------------- CLZ

/// CLZ R2, R1.
const CLZ_R2_R1: u32 = 0xE16F_2F11;

/// Inputs and their leading-zero counts, from `docs/hw/armv5te.md`, "CLZ".
const CLZ_CASES: [(u32, u32); 8] = [
    (0, 32),
    (1, 31),
    (0x8000_0000, 0),
    (0xFFFF_FFFF, 0),
    (0x0001_0000, 15),
    (0x0000_FFFF, 16),
    (0x0000_0100, 23),
    (0x4000_0000, 1),
];

/// The cases where the ARM9's R2 differs from the count.
fn clz_failures() -> Vec<(u32, u32, u32)> {
    CLZ_CASES
        .iter()
        .map(|&(x, want)| {
            (
                x,
                want,
                arm::<true>(&[(1, x), (2, 0x77)], &[], CLZ_R2_R1).r(2),
            )
        })
        .filter(|(_, want, got)| want != got)
        .collect()
}

/// Counts leading zeros bit by bit from the top, independently of `leading_zeros`.
fn model_clz(x: u32) -> u32 {
    (0..32).take_while(|bit| x >> (31 - bit) & 1 == 0).count() as u32
}

#[test]
fn given_an_arm9_clz_when_executed_on_the_edge_values_then_rd_is_the_leading_zero_count() {
    assert_eq!(clz_failures(), vec![]);
}

#[test]
fn given_an_arm9_clz_when_executed_with_every_flag_set_then_the_cpsr_is_unchanged() {
    let r = run::<true>(SVC | NZCV, &[(1, 0)], &[], CLZ_R2_R1);
    assert_eq!(r.cpsr(), SVC | NZCV);
}

#[test]
fn given_an_arm7_clz_word_when_executed_then_rd_is_kept_and_only_r15_advances() {
    let r = arm::<false>(&[(1, 0), (2, 0x77)], &[], CLZ_R2_R1);
    assert_eq!((r.r(2), r.r(15)), (0x77, 0x100C));
}

proptest::proptest! {
    #[test]
    fn given_any_rm_when_an_arm9_clz_executes_then_rd_matches_a_bitwise_count(x in proptest::prelude::any::<u32>()) {
        let r = arm::<true>(&[(1, x)], &[], CLZ_R2_R1);
        proptest::prop_assert_eq!(r.r(2), model_clz(x));
    }
}

// ---------------------------------------------------------------- QADD, QSUB, QDADD, QDSUB

const OP_QADD: u32 = 0;
const OP_QSUB: u32 = 1;
const OP_QDADD: u32 = 2;
const OP_QDSUB: u32 = 3;
const MAX: u32 = 0x7FFF_FFFF;
const MIN: u32 = 0x8000_0000;

/// The word `Q<op> R2, R1, R0`: Rd = R2, Rm = R1, Rn = R0 (`cond 0001 0 op 0 Rn Rd 0000 0101 Rm`).
fn q_word(op: u32) -> u32 {
    0xE100_0050 | op << 21 | 2 << 12 | 1
}

/// One case: the operation, Rm, Rn, whether Q starts set, and the expected Rd and Q.
type QCase = (u32, u32, u32, bool, u32, bool);

/// Cases worked out from `docs/hw/armv5te.md`, "Saturating arithmetic".
const Q_CASES: [QCase; 17] = [
    (OP_QADD, 1, 2, false, 3, false),
    (OP_QADD, MAX - 1, 1, false, MAX, false),
    (OP_QADD, MAX, 1, false, MAX, true),
    (OP_QADD, MIN, 0xFFFF_FFFF, false, MIN, true),
    (OP_QADD, 1, 2, true, 3, true),
    (OP_QSUB, 5, 3, false, 2, false),
    (OP_QSUB, MIN, 1, false, MIN, true),
    (OP_QSUB, MAX, 0xFFFF_FFFF, false, MAX, true),
    (OP_QSUB, 0, MIN, false, MAX, true),
    (OP_QSUB, MIN + 1, 1, false, MIN, false),
    (OP_QDADD, 1, 3, false, 7, false),
    (OP_QDADD, 0, 0x4000_0000, false, MAX, true),
    // The doubling saturates, the add does not: -2^31 + (2^31 - 1) = -1.
    (OP_QDADD, MIN, 0x4000_0000, false, 0xFFFF_FFFF, true),
    (OP_QDADD, 0, 0x3FFF_FFFF, false, 0x7FFF_FFFE, false),
    (OP_QDSUB, 10, 3, false, 4, false),
    (OP_QDSUB, 0, MIN, false, MAX, true),
    (OP_QDSUB, MAX, 0xC000_0000, false, MAX, true),
];

/// The expected Rd and Q for any inputs, computed in 64-bit arithmetic.
fn model_saturating(op: u32, rm: u32, rn: u32) -> (u32, bool) {
    let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    let rn = i64::from(rn as i32);
    let doubled = clamp(rn * 2);
    let rn_used = [rn, rn, doubled, doubled][op as usize];
    let sum = i64::from(rm as i32) + [rn_used, -rn_used][op as usize & 1];
    let result = clamp(sum);
    let saturated = result != sum || (op >= 2 && doubled != rn * 2);
    (result as i32 as u32, saturated)
}

/// The cases whose Rd or Q differ from the table.
fn q_failures() -> Vec<(QCase, u32, bool)> {
    Q_CASES
        .iter()
        .map(|&case| {
            let (op, rm, rn, q_in, _, _) = case;
            let cpsr = [SVC, SVC | Q][usize::from(q_in)];
            let r = run::<true>(cpsr, &[(1, rm), (0, rn)], &[], q_word(op));
            (case, r.r(2), r.cpsr() & Q != 0)
        })
        .filter(|&((_, _, _, _, want_rd, want_q), rd, q)| (want_rd, want_q) != (rd, q))
        .collect()
}

#[test]
fn given_the_saturating_adds_when_executed_at_the_edges_then_rd_and_q_follow_the_definitions() {
    assert_eq!(q_failures(), vec![]);
}

#[test]
fn given_a_saturating_add_when_it_saturates_with_every_flag_set_then_only_q_changes_in_the_cpsr() {
    let r = run::<true>(SVC | NZCV, &[(1, MAX), (0, 1)], &[], q_word(OP_QADD));
    assert_eq!(r.cpsr(), SVC | NZCV | Q);
}

#[test]
fn given_a_saturating_add_when_it_does_not_saturate_with_q_set_then_q_stays_set() {
    let r = run::<true>(SVC | Q, &[(1, 1), (0, 1)], &[], q_word(OP_QDSUB));
    assert_eq!(r.cpsr(), SVC | Q);
}

#[test]
fn given_an_arm7_qadd_word_when_executed_then_rd_and_the_cpsr_are_kept() {
    let r = arm::<false>(&[(1, MAX), (0, 1), (2, 0x77)], &[], q_word(OP_QADD));
    assert_eq!((r.r(2), r.cpsr(), r.r(15)), (0x77, SVC, 0x100C));
}

proptest::proptest! {
    #[test]
    fn given_any_operands_when_an_arm9_saturating_add_executes_then_rd_and_q_match_the_i64_model(
        op in 0u32..4,
        rm in proptest::prelude::any::<u32>(),
        rn in proptest::prelude::any::<u32>(),
    ) {
        let r = arm::<true>(&[(1, rm), (0, rn)], &[], q_word(op));
        proptest::prop_assert_eq!((r.r(2), r.cpsr() & Q != 0), model_saturating(op, rm, rn));
    }
}
