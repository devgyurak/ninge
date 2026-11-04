// SPDX-License-Identifier: Apache-2.0
//! ARM loads and stores through `Cpu::step`. Expected values come from `docs/hw/arm-load-store.md`.
//!
//! Interim R15 convention (`docs/hw/arm-data-processing.md`): while an instruction executes R15
//! is its address + 8; afterwards R15 is +4 more if the instruction did not write it, or the
//! written value + 8 if it did. The bus receives the raw effective address, low bits included.

use super::arm_transfer::{block_start, halfword_value, rotate_word};
use super::*;
use proptest::prelude::*;
use std::collections::BTreeMap;

/// Address of the instruction under test.
const ADDR: u32 = 0x1000;
/// R15 as an instruction reads it (address + 8).
const PC_READ: u32 = ADDR + 8;
/// R15 as an instruction stores it (address + 12).
const PC_STORE: u32 = ADDR + 12;
/// R15 after an instruction that did not write it.
const PC_NEXT: u32 = ADDR + 12;
const N: u32 = 0x8000_0000;
const Z: u32 = 0x4000_0000;
const C: u32 = 0x2000_0000;
const V: u32 = 0x1000_0000;
const FLAGS: u32 = N | Z | C | V;
const FIQ: u32 = 0x11;
const IRQ: u32 = 0x12;
const SVC: u32 = 0x13;
/// Condition field NE: fails when Z is set, and `FLAGS` has Z set.
const COND_NE: u32 = 1;

// ---------------------------------------------------------------- the bus

/// One recorded data access: ('R' or 'W', size in bytes, raw address, value).
type Access = (char, u8, u32, u32);

fn rd(size: u8, addr: u32, value: u32) -> Access {
    ('R', size, addr, value)
}

fn wr(size: u8, addr: u32, value: u32) -> Access {
    ('W', size, addr, value)
}

/// What unset memory reads as: a value derived from the address, so a wrong address shows.
fn pattern(addr: u32) -> u32 {
    addr.wrapping_mul(0x9E37_79B1) ^ 0x5A5A_C3C3
}

/// A memory-backed bus keyed by raw address; it logs every data access and never rotates.
struct LogBus {
    opcode: u32,
    mem: BTreeMap<u32, u32>,
    log: Vec<Access>,
}

impl LogBus {
    fn new(opcode: u32, mem: &[(u32, u32)]) -> Self {
        Self {
            opcode,
            mem: mem.iter().copied().collect(),
            log: Vec::new(),
        }
    }

    fn value(&self, addr: u32) -> u32 {
        self.mem
            .get(&addr)
            .copied()
            .unwrap_or_else(|| pattern(addr))
    }
}

impl Bus for LogBus {
    fn fetch32(&mut self, _addr: u32) -> u32 {
        self.opcode
    }
    fn fetch16(&mut self, _addr: u32) -> u16 {
        0
    }
    fn read8(&mut self, addr: u32) -> u8 {
        let v = self.value(addr) as u8;
        self.log.push(rd(1, addr, u32::from(v)));
        v
    }
    fn read16(&mut self, addr: u32) -> u16 {
        let v = self.value(addr) as u16;
        self.log.push(rd(2, addr, u32::from(v)));
        v
    }
    fn read32(&mut self, addr: u32) -> u32 {
        let v = self.value(addr);
        self.log.push(rd(4, addr, v));
        v
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.log.push(wr(1, addr, u32::from(value)));
        self.mem.insert(addr, u32::from(value));
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.log.push(wr(2, addr, u32::from(value)));
        self.mem.insert(addr, u32::from(value));
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.log.push(wr(4, addr, value));
        self.mem.insert(addr, value);
    }
}

// ---------------------------------------------------------------- opcode builders

/// LDR/STR word: `i` register offset, `p` pre, `u` up, `b` byte, `w` writeback/T, `l` load.
#[expect(clippy::too_many_arguments, reason = "mirrors the encoding fields")]
const fn sdt(i: u32, p: u32, u: u32, b: u32, w: u32, l: u32, rn: u32, rd: u32, off: u32) -> u32 {
    0xE400_0000
        | i << 25
        | p << 24
        | u << 23
        | b << 22
        | w << 21
        | l << 20
        | rn << 16
        | rd << 12
        | off
}

/// Halfword/signed transfer: `i` immediate offset (`off` is 8 bits, split into nibbles) or a
/// register offset (`off` = Rm); `sh` is bits 6-5.
#[expect(clippy::too_many_arguments, reason = "mirrors the encoding fields")]
const fn hw(p: u32, u: u32, i: u32, w: u32, l: u32, rn: u32, rd: u32, sh: u32, off: u32) -> u32 {
    0xE000_0090
        | p << 24
        | u << 23
        | i << 22
        | w << 21
        | l << 20
        | rn << 16
        | rd << 12
        | (off & 0xF0) << 4
        | sh << 5
        | (off & 0xF)
}

/// LDM/STM: `p` pre, `u` up, `s` the S bit, `w` writeback, `l` load.
const fn blk(p: u32, u: u32, s: u32, w: u32, l: u32, rn: u32, list: u32) -> u32 {
    0xE800_0000 | p << 24 | u << 23 | s << 22 | w << 21 | l << 20 | rn << 16 | list
}

/// SWP (`b` = 0) or SWPB (`b` = 1).
const fn swp(b: u32, rn: u32, rd: u32, rm: u32) -> u32 {
    0xE100_0090 | b << 22 | rn << 16 | rd << 12 | rm
}

/// Replaces the condition field of `opcode`.
const fn with_cond(opcode: u32, cond: u32) -> u32 {
    opcode & 0x0FFF_FFFF | cond << 28
}

// ---------------------------------------------------------------- harness

/// One instruction run: the state before and the state expected after.
struct Case {
    name: &'static str,
    cpsr: u32,
    regs: Vec<(usize, u32)>,
    mem: Vec<(u32, u32)>,
    opcode: u32,
    want_regs: Vec<(usize, u32)>,
    want_log: Vec<Access>,
    want_cpsr: u32,
    want_pc: u32,
}

fn case(
    name: &'static str,
    opcode: u32,
    regs: &[(usize, u32)],
    mem: &[(u32, u32)],
    want_regs: &[(usize, u32)],
    want_log: &[Access],
) -> Case {
    Case {
        name,
        cpsr: SVC | FLAGS,
        regs: regs.to_vec(),
        mem: mem.to_vec(),
        opcode,
        want_regs: want_regs.to_vec(),
        want_log: want_log.to_vec(),
        want_cpsr: SVC | FLAGS,
        want_pc: PC_NEXT,
    }
}

impl Case {
    /// Expects R15 to end as `pc` instead of the plain advance.
    fn pc(mut self, pc: u32) -> Self {
        self.want_pc = pc;
        self
    }
}

/// Runs `opcode` as the instruction at `ADDR`, returning the CPU and the data accesses.
fn run<const A: bool>(
    cpsr: u32,
    setup: &[(usize, u32)],
    mem: &[(u32, u32)],
    opcode: u32,
) -> (Cpu<A>, Vec<Access>) {
    let mut cpu = Cpu::<A>::new();
    cpu.regs_mut().set_cpsr(cpsr);
    setup.iter().for_each(|&(i, v)| cpu.regs_mut().set(i, v));
    cpu.regs_mut().set(15, PC_READ);
    let mut bus = LogBus::new(opcode, mem);
    cpu.step(&mut bus);
    (cpu, bus.log)
}

/// The cases whose registers, R15, CPSR or bus log differ from the table, on one CPU.
fn table<const A: bool>(cases: &[Case]) -> Vec<String> {
    let who = if A { "arm9" } else { "arm7" };
    cases
        .iter()
        .filter_map(|c| {
            let (cpu, log) = run::<A>(c.cpsr, &c.regs, &c.mem, c.opcode);
            let regs: Vec<(usize, u32)> = c
                .want_regs
                .iter()
                .map(|&(i, _)| (i, cpu.regs().get(i)))
                .collect();
            let got = (regs, log, cpu.regs().cpsr(), cpu.regs().get(15));
            let want = (
                c.want_regs.clone(),
                c.want_log.clone(),
                c.want_cpsr,
                c.want_pc,
            );
            (got != want).then(|| format!("{who} {}: got {got:x?}, want {want:x?}", c.name))
        })
        .collect()
}

/// The failures of a table that holds for both CPUs.
fn both(cases: &[Case]) -> Vec<String> {
    let mut failures = table::<false>(cases);
    failures.extend(table::<true>(cases));
    failures
}

/// Selects the ARM7 or ARM9 expectation.
fn pick(arm9: bool, arm7_value: u32, arm9_value: u32) -> u32 {
    if arm9 { arm9_value } else { arm7_value }
}

// ---------------------------------------------------------------- LDR / STR

fn single_addressing_cases() -> Vec<Case> {
    let b = [(0, 0x2000)];
    vec![
        case(
            "LDR pre up",
            sdt(0, 1, 1, 0, 0, 1, 0, 1, 4),
            &b,
            &[(0x2004, 0xDEAD_BEEF)],
            &[(1, 0xDEAD_BEEF), (0, 0x2000)],
            &[rd(4, 0x2004, 0xDEAD_BEEF)],
        ),
        case(
            "LDR post up writes back",
            sdt(0, 0, 1, 0, 0, 1, 0, 1, 4),
            &b,
            &[(0x2000, 0x0102_0304)],
            &[(1, 0x0102_0304), (0, 0x2004)],
            &[rd(4, 0x2000, 0x0102_0304)],
        ),
        case(
            "LDR pre up writeback",
            sdt(0, 1, 1, 0, 1, 1, 0, 1, 8),
            &b,
            &[(0x2008, 0x1111_2222)],
            &[(1, 0x1111_2222), (0, 0x2008)],
            &[rd(4, 0x2008, 0x1111_2222)],
        ),
        case(
            "LDR pre down no writeback",
            sdt(0, 1, 0, 0, 0, 1, 0, 1, 4),
            &b,
            &[(0x1FFC, 0x3333_4444)],
            &[(1, 0x3333_4444), (0, 0x2000)],
            &[rd(4, 0x1FFC, 0x3333_4444)],
        ),
        case(
            "LDR post down writes back",
            sdt(0, 0, 0, 0, 0, 1, 0, 1, 4),
            &b,
            &[(0x2000, 0x5555_6666)],
            &[(1, 0x5555_6666), (0, 0x1FFC)],
            &[rd(4, 0x2000, 0x5555_6666)],
        ),
        case(
            "LDR pre down writeback",
            sdt(0, 1, 0, 0, 1, 1, 0, 1, 4),
            &b,
            &[(0x1FFC, 0x7777_8888)],
            &[(1, 0x7777_8888), (0, 0x1FFC)],
            &[rd(4, 0x1FFC, 0x7777_8888)],
        ),
        case(
            "STR pre up",
            sdt(0, 1, 1, 0, 0, 0, 0, 1, 4),
            &[(0, 0x2000), (1, 0xCAFE_F00D)],
            &[],
            &[(0, 0x2000)],
            &[wr(4, 0x2004, 0xCAFE_F00D)],
        ),
        case(
            "STR post up writes back",
            sdt(0, 0, 1, 0, 0, 0, 0, 1, 4),
            &[(0, 0x2000), (1, 0xCAFE_F00D)],
            &[],
            &[(0, 0x2004)],
            &[wr(4, 0x2000, 0xCAFE_F00D)],
        ),
        case(
            "STR pre down writeback",
            sdt(0, 1, 0, 0, 1, 0, 0, 1, 4),
            &[(0, 0x2000), (1, 0xCAFE_F00D)],
            &[],
            &[(0, 0x1FFC)],
            &[wr(4, 0x1FFC, 0xCAFE_F00D)],
        ),
    ]
}

#[test]
fn given_word_loads_and_stores_when_executed_then_pre_post_up_down_and_writeback_follow_the_note() {
    assert_eq!(both(&single_addressing_cases()), Vec::<String>::new());
}

fn byte_cases() -> Vec<Case> {
    vec![
        case(
            "LDRB zero-extends",
            sdt(0, 1, 1, 1, 0, 1, 0, 1, 1),
            &[(0, 0x2000), (1, 0xFFFF_FF00)],
            &[(0x2001, 0xFF)],
            &[(1, 0x0000_00FF)],
            &[rd(1, 0x2001, 0xFF)],
        ),
        case(
            "LDRB post down writes back",
            sdt(0, 0, 0, 1, 0, 1, 0, 1, 1),
            &[(0, 0x2000)],
            &[(0x2000, 0x5A)],
            &[(1, 0x5A), (0, 0x1FFF)],
            &[rd(1, 0x2000, 0x5A)],
        ),
        case(
            "STRB stores the low byte",
            sdt(0, 1, 1, 1, 0, 0, 0, 1, 1),
            &[(0, 0x2000), (1, 0xCAFE_F00D)],
            &[],
            &[(0, 0x2000)],
            &[wr(1, 0x2001, 0x0D)],
        ),
        case(
            "STRB pre writeback",
            sdt(0, 1, 1, 1, 1, 0, 0, 1, 2),
            &[(0, 0x2000), (1, 0x1234_56AB)],
            &[],
            &[(0, 0x2002)],
            &[wr(1, 0x2002, 0xAB)],
        ),
    ]
}

#[test]
fn given_byte_loads_and_stores_when_executed_then_load_zero_extends_and_store_writes_the_low_byte()
{
    assert_eq!(both(&byte_cases()), Vec::<String>::new());
}

fn register_offset_cases() -> Vec<Case> {
    vec![
        case(
            "LDR [R0, R2, LSL #2]",
            sdt(1, 1, 1, 0, 0, 1, 0, 1, 2 << 7 | 2),
            &[(0, 0x2000), (2, 3)],
            &[(0x200C, 0x0A0B_0C0D)],
            &[(1, 0x0A0B_0C0D), (0, 0x2000)],
            &[rd(4, 0x200C, 0x0A0B_0C0D)],
        ),
        case(
            "LDR [R0, R2, LSR #1]",
            sdt(1, 1, 1, 0, 0, 1, 0, 1, 1 << 7 | 1 << 5 | 2),
            &[(0, 0x2000), (2, 8)],
            &[(0x2004, 0x0102_0304)],
            &[(1, 0x0102_0304)],
            &[rd(4, 0x2004, 0x0102_0304)],
        ),
        case(
            "LDR with LSR #0 shifts by 32 so the offset is 0",
            sdt(1, 1, 1, 0, 0, 1, 0, 1, 1 << 5 | 2),
            &[(0, 0x2000), (2, 0xFFFF_FFFF)],
            &[(0x2000, 0x1357_9BDF)],
            &[(1, 0x1357_9BDF)],
            &[rd(4, 0x2000, 0x1357_9BDF)],
        ),
        case(
            "LDR with ASR #0 shifts by 32 so a negative index fills with ones",
            sdt(1, 1, 1, 0, 0, 1, 0, 1, 2 << 5 | 2),
            &[(0, 0x2000), (2, 0x8000_0000)],
            &[(0x1FFF, 0x1122_3344)],
            // The raw address is 0x2000 + 0xFFFF_FFFF = 0x1FFF; rotate right by 24.
            &[(1, 0x2233_4411)],
            &[rd(4, 0x1FFF, 0x1122_3344)],
        ),
        case(
            "LDR post down with a register offset writes back",
            sdt(1, 0, 0, 0, 0, 1, 0, 1, 2),
            &[(0, 0x2000), (2, 8)],
            &[(0x2000, 0x2468_ACE0)],
            &[(1, 0x2468_ACE0), (0, 0x1FF8)],
            &[rd(4, 0x2000, 0x2468_ACE0)],
        ),
        case(
            "STR [R0, R2, LSL #2]!",
            sdt(1, 1, 1, 0, 1, 0, 0, 1, 2 << 7 | 2),
            &[(0, 0x2000), (1, 0xCAFE_F00D), (2, 1)],
            &[],
            &[(0, 0x2004)],
            &[wr(4, 0x2004, 0xCAFE_F00D)],
        ),
        case(
            "STRB register offset",
            sdt(1, 1, 1, 1, 0, 0, 0, 1, 2),
            &[(0, 0x2000), (1, 0xFFFF_FF77), (2, 5)],
            &[],
            &[(0, 0x2000)],
            &[wr(1, 0x2005, 0x77)],
        ),
    ]
}

#[test]
fn given_register_offsets_when_ldr_or_str_executes_then_the_shifted_offset_forms_the_address() {
    assert_eq!(both(&register_offset_cases()), Vec::<String>::new());
}

fn unaligned_word_cases() -> Vec<Case> {
    vec![
        case(
            "LDR at +1 rotates right by 8",
            sdt(0, 1, 1, 0, 0, 1, 0, 1, 0),
            &[(0, 0x2001)],
            &[(0x2001, 0x1122_3344)],
            &[(1, 0x4411_2233)],
            &[rd(4, 0x2001, 0x1122_3344)],
        ),
        case(
            "LDR at +2 rotates right by 16",
            sdt(0, 1, 1, 0, 0, 1, 0, 1, 0),
            &[(0, 0x2002)],
            &[(0x2002, 0x1122_3344)],
            &[(1, 0x3344_1122)],
            &[rd(4, 0x2002, 0x1122_3344)],
        ),
        case(
            "LDR at +3 rotates right by 24",
            sdt(0, 1, 1, 0, 0, 1, 0, 1, 0),
            &[(0, 0x2003)],
            &[(0x2003, 0x1122_3344)],
            &[(1, 0x2233_4411)],
            &[rd(4, 0x2003, 0x1122_3344)],
        ),
        case(
            "STR at +3 hands the bus the raw address and the unrotated word",
            sdt(0, 1, 1, 0, 0, 0, 0, 1, 0),
            &[(0, 0x2003), (1, 0xCAFE_F00D)],
            &[],
            &[(0, 0x2003)],
            &[wr(4, 0x2003, 0xCAFE_F00D)],
        ),
        case(
            "STR at +1 post-indexed keeps the raw base for the access and the write-back",
            sdt(0, 0, 1, 0, 0, 0, 0, 1, 4),
            &[(0, 0x2001), (1, 0x0BAD_CAFE)],
            &[],
            &[(0, 0x2005)],
            &[wr(4, 0x2001, 0x0BAD_CAFE)],
        ),
    ]
}

#[test]
fn given_unaligned_addresses_when_ldr_and_str_execute_then_ldr_rotates_and_str_is_unrotated() {
    assert_eq!(both(&unaligned_word_cases()), Vec::<String>::new());
}

fn translate_cases() -> Vec<Case> {
    vec![
        case(
            "LDRT behaves as a post-indexed LDR",
            sdt(0, 0, 1, 0, 1, 1, 0, 1, 4),
            &[(0, 0x2000)],
            &[(0x2000, 0x1234_5678)],
            &[(1, 0x1234_5678), (0, 0x2004)],
            &[rd(4, 0x2000, 0x1234_5678)],
        ),
        case(
            "STRT behaves as a post-indexed STR",
            sdt(0, 0, 1, 0, 1, 0, 0, 1, 4),
            &[(0, 0x2000), (1, 0x8765_4321)],
            &[],
            &[(0, 0x2004)],
            &[wr(4, 0x2000, 0x8765_4321)],
        ),
        case(
            "LDRBT behaves as a post-indexed LDRB",
            sdt(0, 0, 1, 1, 1, 1, 0, 1, 1),
            &[(0, 0x2000)],
            &[(0x2000, 0x9C)],
            &[(1, 0x9C), (0, 0x2001)],
            &[rd(1, 0x2000, 0x9C)],
        ),
        case(
            "STRBT behaves as a post-indexed STRB",
            sdt(0, 0, 0, 1, 1, 0, 0, 1, 1),
            &[(0, 0x2000), (1, 0xFFFF_FF5D)],
            &[],
            &[(0, 0x1FFF)],
            &[wr(1, 0x2000, 0x5D)],
        ),
    ]
}

#[test]
fn given_translated_transfers_when_executed_then_they_behave_like_the_post_indexed_forms() {
    assert_eq!(both(&translate_cases()), Vec::<String>::new());
}

fn load_base_cases() -> Vec<Case> {
    vec![
        case(
            "LDR R0,[R0,#4]! keeps the loaded value",
            sdt(0, 1, 1, 0, 1, 1, 0, 0, 4),
            &[(0, 0x2000)],
            &[(0x2004, 0xABCD_0123)],
            &[(0, 0xABCD_0123)],
            &[rd(4, 0x2004, 0xABCD_0123)],
        ),
        case(
            "LDR R0,[R0],#4 keeps the loaded value",
            sdt(0, 0, 1, 0, 0, 1, 0, 0, 4),
            &[(0, 0x2000)],
            &[(0x2000, 0x0123_ABCD)],
            &[(0, 0x0123_ABCD)],
            &[rd(4, 0x2000, 0x0123_ABCD)],
        ),
        case(
            "LDRB R0,[R0],#1 keeps the loaded byte",
            sdt(0, 0, 1, 1, 0, 1, 0, 0, 1),
            &[(0, 0x2000)],
            &[(0x2000, 0x7E)],
            &[(0, 0x7E)],
            &[rd(1, 0x2000, 0x7E)],
        ),
        case(
            "LDRH R0,[R0,#2]! keeps the loaded halfword",
            hw(1, 1, 1, 1, 1, 0, 0, 1, 2),
            &[(0, 0x2000)],
            &[(0x2002, 0x1234)],
            &[(0, 0x1234)],
            &[rd(2, 0x2002, 0x1234)],
        ),
    ]
}

#[test]
fn given_rd_equal_to_rn_with_writeback_when_a_load_executes_then_the_loaded_value_stays() {
    assert_eq!(both(&load_base_cases()), Vec::<String>::new());
}

fn single_r15_cases() -> Vec<Case> {
    vec![
        case(
            "LDR R1,[R15,#4] uses address + 8 as the base",
            sdt(0, 1, 1, 0, 0, 1, 15, 1, 4),
            &[],
            &[(0x100C, 0x55AA_55AA)],
            &[(1, 0x55AA_55AA)],
            &[rd(4, 0x100C, 0x55AA_55AA)],
        ),
        case(
            "LDR R1,[R15] reads at address + 8",
            sdt(0, 1, 1, 0, 0, 1, 15, 1, 0),
            &[],
            &[(0x1008, 0x0F0F_F0F0)],
            &[(1, 0x0F0F_F0F0)],
            &[rd(4, 0x1008, 0x0F0F_F0F0)],
        ),
        case(
            "STR R15 stores address + 12",
            sdt(0, 1, 1, 0, 0, 0, 0, 15, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x2000)],
            &[wr(4, 0x2000, PC_STORE)],
        ),
        case(
            "STRB R15 stores the low byte of address + 12",
            sdt(0, 1, 1, 1, 0, 0, 0, 15, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x2000)],
            &[wr(1, 0x2000, PC_STORE & 0xFF)],
        ),
        case(
            "LDR R15 ends as the loaded value + 8",
            sdt(0, 1, 1, 0, 0, 1, 0, 15, 0),
            &[(0, 0x2000)],
            &[(0x2000, 0x0000_3000)],
            &[],
            &[rd(4, 0x2000, 0x3000)],
        )
        .pc(0x3008),
        case(
            "LDR R15 keeps the low bits of the loaded value",
            sdt(0, 1, 1, 0, 0, 1, 0, 15, 0),
            &[(0, 0x2000)],
            &[(0x2000, 0x0000_3001)],
            &[],
            &[rd(4, 0x2000, 0x3001)],
        )
        .pc(0x3009),
    ]
}

#[test]
fn given_r15_operands_when_single_transfers_execute_then_pipeline_offsets_apply() {
    assert_eq!(both(&single_r15_cases()), Vec::<String>::new());
}

fn failed_condition_cases() -> Vec<Case> {
    let regs = [(0, 0x2000), (1, 0x77), (2, 0x88)];
    let want = [(0, 0x2000), (1, 0x77), (2, 0x88)];
    vec![
        case(
            "LDR with NE while Z is set",
            with_cond(sdt(0, 1, 1, 0, 1, 1, 0, 1, 4), COND_NE),
            &regs,
            &[],
            &want,
            &[],
        ),
        case(
            "STR with NE while Z is set",
            with_cond(sdt(0, 0, 1, 0, 0, 0, 0, 1, 4), COND_NE),
            &regs,
            &[],
            &want,
            &[],
        ),
        case(
            "LDRH with NE while Z is set",
            with_cond(hw(0, 1, 1, 0, 1, 0, 1, 1, 2), COND_NE),
            &regs,
            &[],
            &want,
            &[],
        ),
        case(
            "SWP with NE while Z is set",
            with_cond(swp(0, 0, 1, 2), COND_NE),
            &regs,
            &[],
            &want,
            &[],
        ),
        case(
            "LDMIA with writeback and NE while Z is set",
            with_cond(blk(0, 1, 0, 1, 1, 0, 0b110), COND_NE),
            &regs,
            &[],
            &want,
            &[],
        ),
        case(
            "STMDB with writeback and NE while Z is set",
            with_cond(blk(1, 0, 0, 1, 0, 0, 0b110), COND_NE),
            &regs,
            &[],
            &want,
            &[],
        ),
    ]
}

#[test]
fn given_a_failed_condition_when_a_transfer_executes_then_nothing_changes_but_the_pc_advances() {
    assert_eq!(both(&failed_condition_cases()), Vec::<String>::new());
}

// ---------------------------------------------------------------- halfword and signed

fn halfword_cases() -> Vec<Case> {
    let b = [(0, 0x2000)];
    vec![
        case(
            "STRH pre up stores the low halfword",
            hw(1, 1, 1, 0, 0, 0, 1, 1, 2),
            &[(0, 0x2000), (1, 0xCAFE_BEEF)],
            &[],
            &[(0, 0x2000)],
            &[wr(2, 0x2002, 0xBEEF)],
        ),
        case(
            "STRH post up writes back",
            hw(0, 1, 1, 0, 0, 0, 1, 1, 2),
            &[(0, 0x2000), (1, 0xCAFE_BEEF)],
            &[],
            &[(0, 0x2002)],
            &[wr(2, 0x2000, 0xBEEF)],
        ),
        case(
            "STRH register offset down writeback",
            hw(1, 0, 0, 1, 0, 0, 1, 1, 2),
            &[(0, 0x2000), (1, 0xCAFE_BEEF), (2, 4)],
            &[],
            &[(0, 0x1FFC)],
            &[wr(2, 0x1FFC, 0xBEEF)],
        ),
        case(
            "LDRH zero-extends",
            hw(1, 1, 1, 0, 1, 0, 1, 1, 2),
            &[(0, 0x2000), (1, 0xFFFF_0000)],
            &[(0x2002, 0xBEEF)],
            &[(1, 0x0000_BEEF)],
            &[rd(2, 0x2002, 0xBEEF)],
        ),
        case(
            "LDRH immediate split into nibbles (0x2A)",
            hw(1, 1, 1, 0, 1, 0, 1, 1, 0x2A),
            &b,
            &[(0x202A, 0x1234)],
            &[(1, 0x1234)],
            &[rd(2, 0x202A, 0x1234)],
        ),
        case(
            "LDRH register offset",
            hw(1, 1, 0, 0, 1, 0, 1, 1, 2),
            &[(0, 0x2000), (2, 6)],
            &[(0x2006, 0x4321)],
            &[(1, 0x4321)],
            &[rd(2, 0x2006, 0x4321)],
        ),
        case(
            "LDRH post down register offset writes back",
            hw(0, 0, 0, 0, 1, 0, 1, 1, 2),
            &[(0, 0x2000), (2, 4)],
            &[(0x2000, 0x1357)],
            &[(1, 0x1357), (0, 0x1FFC)],
            &[rd(2, 0x2000, 0x1357)],
        ),
        case(
            "LDRH pre up writeback",
            hw(1, 1, 1, 1, 1, 0, 1, 1, 2),
            &b,
            &[(0x2002, 0x2468)],
            &[(1, 0x2468), (0, 0x2002)],
            &[rd(2, 0x2002, 0x2468)],
        ),
        case(
            "LDRSB sign-extends a negative byte",
            hw(1, 1, 1, 0, 1, 0, 1, 2, 1),
            &b,
            &[(0x2001, 0x80)],
            &[(1, 0xFFFF_FF80)],
            &[rd(1, 0x2001, 0x80)],
        ),
        case(
            "LDRSB keeps a positive byte",
            hw(1, 1, 1, 0, 1, 0, 1, 2, 1),
            &[(0, 0x2000), (1, 0xFFFF_FFFF)],
            &[(0x2001, 0x7F)],
            &[(1, 0x7F)],
            &[rd(1, 0x2001, 0x7F)],
        ),
        case(
            "LDRSB at an odd base reads that byte",
            hw(1, 1, 1, 0, 1, 0, 1, 2, 0),
            &[(0, 0x2003)],
            &[(0x2003, 0xF0)],
            &[(1, 0xFFFF_FFF0)],
            &[rd(1, 0x2003, 0xF0)],
        ),
        case(
            "LDRSH sign-extends a negative halfword",
            hw(1, 1, 1, 0, 1, 0, 1, 3, 2),
            &b,
            &[(0x2002, 0x8001)],
            &[(1, 0xFFFF_8001)],
            &[rd(2, 0x2002, 0x8001)],
        ),
        case(
            "LDRSH keeps a positive halfword",
            hw(1, 1, 1, 0, 1, 0, 1, 3, 2),
            &[(0, 0x2000), (1, 0xFFFF_FFFF)],
            &[(0x2002, 0x7001)],
            &[(1, 0x7001)],
            &[rd(2, 0x2002, 0x7001)],
        ),
        case(
            "STRH at an odd address passes the raw address",
            hw(1, 1, 1, 0, 0, 0, 1, 1, 0),
            &[(0, 0x2001), (1, 0xCAFE_BEEF)],
            &[],
            &[(0, 0x2001)],
            &[wr(2, 0x2001, 0xBEEF)],
        ),
        case(
            "LDRH R1,[R15,#4] uses address + 8 as the base",
            hw(1, 1, 1, 0, 1, 15, 1, 1, 4),
            &[],
            &[(0x100C, 0x6789)],
            &[(1, 0x6789)],
            &[rd(2, 0x100C, 0x6789)],
        ),
    ]
}

#[test]
fn given_halfword_and_signed_forms_when_executed_then_extension_and_addressing_follow_the_note() {
    assert_eq!(both(&halfword_cases()), Vec::<String>::new());
}

/// The four odd-address cases; `want` holds the expected R1 of each.
fn odd_halfword_cases(want: [u32; 4]) -> Vec<Case> {
    vec![
        case(
            "LDRH at 0x2001",
            hw(1, 1, 1, 0, 1, 0, 1, 1, 0),
            &[(0, 0x2001)],
            &[(0x2001, 0xBEEF)],
            &[(1, want[0])],
            &[rd(2, 0x2001, 0xBEEF)],
        ),
        case(
            "LDRH at 0x2003",
            hw(1, 1, 1, 0, 1, 0, 1, 1, 0),
            &[(0, 0x2003)],
            &[(0x2003, 0x1234)],
            &[(1, want[1])],
            &[rd(2, 0x2003, 0x1234)],
        ),
        case(
            "LDRSH at 0x2001 with a negative high byte",
            hw(1, 1, 1, 0, 1, 0, 1, 3, 0),
            &[(0, 0x2001)],
            &[(0x2001, 0x80EF)],
            &[(1, want[2])],
            &[rd(2, 0x2001, 0x80EF)],
        ),
        case(
            "LDRSH at 0x2001 with a positive high byte",
            hw(1, 1, 1, 0, 1, 0, 1, 3, 0),
            &[(0, 0x2001)],
            &[(0x2001, 0x7F10)],
            &[(1, want[3])],
            &[rd(2, 0x2001, 0x7F10)],
        ),
    ]
}

#[test]
fn given_an_arm7_when_ldrh_and_ldrsh_read_an_odd_address_then_the_halfword_rotates_and_ldrsh_is_a_byte()
 {
    let cases = odd_halfword_cases([0xEF00_00BE, 0x3400_0012, 0xFFFF_FF80, 0x0000_007F]);
    assert_eq!(table::<false>(&cases), Vec::<String>::new());
}

#[test]
fn given_an_arm9_when_ldrh_and_ldrsh_read_an_odd_address_then_the_aligned_halfword_is_not_rotated()
{
    let cases = odd_halfword_cases([0x0000_BEEF, 0x0000_1234, 0xFFFF_80EF, 0x0000_7F10]);
    assert_eq!(table::<true>(&cases), Vec::<String>::new());
}

// ---------------------------------------------------------------- SWP

fn swap_cases() -> Vec<Case> {
    let mem = [(0x2000, 0x1234_5678)];
    vec![
        case(
            "SWP R1,R2,[R0]",
            swp(0, 0, 1, 2),
            &[(0, 0x2000), (1, 0x1111), (2, 0xAAAA_5555)],
            &mem,
            &[(1, 0x1234_5678), (2, 0xAAAA_5555), (0, 0x2000)],
            &[rd(4, 0x2000, 0x1234_5678), wr(4, 0x2000, 0xAAAA_5555)],
        ),
        case(
            "SWP R1,R1,[R0] with Rm = Rd",
            swp(0, 0, 1, 1),
            &[(0, 0x2000), (1, 0xAAAA_5555)],
            &mem,
            &[(1, 0x1234_5678)],
            &[rd(4, 0x2000, 0x1234_5678), wr(4, 0x2000, 0xAAAA_5555)],
        ),
        case(
            "SWP R1,R0,[R0] with Rm = Rn",
            swp(0, 0, 1, 0),
            &[(0, 0x2000)],
            &mem,
            &[(1, 0x1234_5678), (0, 0x2000)],
            &[rd(4, 0x2000, 0x1234_5678), wr(4, 0x2000, 0x2000)],
        ),
        case(
            "SWPB swaps the low byte and zero-extends the load",
            swp(1, 0, 1, 2),
            &[(0, 0x2000), (1, 0xFFFF_FF00), (2, 0x1234_56CD)],
            &[(0x2000, 0xAB)],
            &[(1, 0xAB)],
            &[rd(1, 0x2000, 0xAB), wr(1, 0x2000, 0xCD)],
        ),
        case(
            "SWP at +1 reads rotated and writes unrotated at the raw address",
            swp(0, 0, 1, 2),
            &[(0, 0x2001), (2, 0xAAAA_5555)],
            &[(0x2001, 0x1122_3344)],
            &[(1, 0x4411_2233)],
            &[rd(4, 0x2001, 0x1122_3344), wr(4, 0x2001, 0xAAAA_5555)],
        ),
        case(
            "SWP at +2 rotates the read by 16",
            swp(0, 0, 1, 2),
            &[(0, 0x2002), (2, 0x0000_0001)],
            &[(0x2002, 0x1122_3344)],
            &[(1, 0x3344_1122)],
            &[rd(4, 0x2002, 0x1122_3344), wr(4, 0x2002, 1)],
        ),
    ]
}

#[test]
fn given_swp_and_swpb_when_executed_then_the_old_value_is_loaded_and_rm_is_stored_unrotated() {
    assert_eq!(both(&swap_cases()), Vec::<String>::new());
}

// ---------------------------------------------------------------- LDM / STM

/// One LDM or STM of R1-R3 on base R0 = 0x2000 in the given mode.
fn mode_case(name: &'static str, p: u32, u: u32, w: u32, l: u32, start: u32, last: u32) -> Case {
    let base_after = if w == 1 { last } else { 0x2000 };
    let list = 0b1110;
    let loads = [0xA1A1_A1A1, 0xA2A2_A2A2, 0xA3A3_A3A3];
    let stores = [0xB1B1_B1B1, 0xB2B2_B2B2, 0xB3B3_B3B3];
    if l == 1 {
        case(
            name,
            blk(p, u, 0, w, 1, 0, list),
            &[(0, 0x2000)],
            &[
                (start, loads[0]),
                (start + 4, loads[1]),
                (start + 8, loads[2]),
            ],
            &[(1, loads[0]), (2, loads[1]), (3, loads[2]), (0, base_after)],
            &[
                rd(4, start, loads[0]),
                rd(4, start + 4, loads[1]),
                rd(4, start + 8, loads[2]),
            ],
        )
    } else {
        case(
            name,
            blk(p, u, 0, w, 0, 0, list),
            &[(0, 0x2000), (1, stores[0]), (2, stores[1]), (3, stores[2])],
            &[],
            &[(0, base_after)],
            &[
                wr(4, start, stores[0]),
                wr(4, start + 4, stores[1]),
                wr(4, start + 8, stores[2]),
            ],
        )
    }
}

fn four_mode_cases(l: u32) -> Vec<Case> {
    vec![
        mode_case("IA", 0, 1, 0, l, 0x2000, 0x200C),
        mode_case("IA!", 0, 1, 1, l, 0x2000, 0x200C),
        mode_case("IB", 1, 1, 0, l, 0x2004, 0x200C),
        mode_case("IB!", 1, 1, 1, l, 0x2004, 0x200C),
        mode_case("DA", 0, 0, 0, l, 0x1FF8, 0x1FF4),
        mode_case("DA!", 0, 0, 1, l, 0x1FF8, 0x1FF4),
        mode_case("DB", 1, 0, 0, l, 0x1FF4, 0x1FF4),
        mode_case("DB!", 1, 0, 1, l, 0x1FF4, 0x1FF4),
    ]
}

#[test]
fn given_ldm_in_all_four_modes_when_executed_then_registers_load_ascending_and_base_writes_back() {
    assert_eq!(both(&four_mode_cases(1)), Vec::<String>::new());
}

#[test]
fn given_stm_in_all_four_modes_when_executed_then_registers_store_ascending_and_base_writes_back() {
    assert_eq!(both(&four_mode_cases(0)), Vec::<String>::new());
}

fn block_edge_cases() -> Vec<Case> {
    vec![
        case(
            "LDMIA with an unaligned base keeps the raw address and does not rotate",
            blk(0, 1, 0, 1, 1, 0, 0b110),
            &[(0, 0x2002)],
            &[(0x2002, 0x1122_3344), (0x2006, 0x5566_7788)],
            &[(1, 0x1122_3344), (2, 0x5566_7788), (0, 0x200A)],
            &[rd(4, 0x2002, 0x1122_3344), rd(4, 0x2006, 0x5566_7788)],
        ),
        case(
            "STMIA with an unaligned base keeps the raw address",
            blk(0, 1, 0, 1, 0, 0, 0b110),
            &[(0, 0x2002), (1, 0xB1B1_B1B1), (2, 0xB2B2_B2B2)],
            &[],
            &[(0, 0x200A)],
            &[wr(4, 0x2002, 0xB1B1_B1B1), wr(4, 0x2006, 0xB2B2_B2B2)],
        ),
        case(
            "STMIA {R1, R15} stores address + 12 for R15",
            blk(0, 1, 0, 0, 0, 0, 0x8002),
            &[(0, 0x2000), (1, 0xB1B1_B1B1)],
            &[],
            &[(0, 0x2000)],
            &[wr(4, 0x2000, 0xB1B1_B1B1), wr(4, 0x2004, PC_STORE)],
        ),
        case(
            "LDMIA R15,{R1} uses address + 8 as the base",
            blk(0, 1, 0, 0, 1, 15, 0b10),
            &[],
            &[(0x1008, 0xC0DE_C0DE)],
            &[(1, 0xC0DE_C0DE)],
            &[rd(4, 0x1008, 0xC0DE_C0DE)],
        ),
        case(
            "LDMIA {R1, R15} ends R15 as the loaded value + 8 with the low bit kept",
            blk(0, 1, 0, 0, 1, 0, 0x8002),
            &[(0, 0x2000)],
            &[(0x2000, 0xA1A1_A1A1), (0x2004, 0x0000_6001)],
            &[(1, 0xA1A1_A1A1)],
            &[rd(4, 0x2000, 0xA1A1_A1A1), rd(4, 0x2004, 0x6001)],
        )
        .pc(0x6009),
        case(
            "LDMIA R0,{R15} with a single register",
            blk(0, 1, 0, 0, 1, 0, 0x8000),
            &[(0, 0x2000)],
            &[(0x2000, 0x0000_5000)],
            &[],
            &[rd(4, 0x2000, 0x5000)],
        )
        .pc(0x5008),
    ]
}

#[test]
fn given_block_transfers_at_the_edges_when_executed_then_raw_addresses_and_r15_rules_apply() {
    assert_eq!(both(&block_edge_cases()), Vec::<String>::new());
}

// ---------------------------------------------------------------- block: S bit

/// STM^ of the user R13 while in IRQ mode: (bus log, IRQ R13 afterwards).
fn stm_user_r13<const A: bool>() -> (Vec<Access>, u32) {
    let mut cpu = Cpu::<A>::new();
    cpu.regs_mut().set_cpsr(IRQ);
    cpu.regs_mut().set_banked(Bank::UserSystem, 13, 0x1111_0000);
    cpu.regs_mut().set(13, 0x2222_0000);
    cpu.regs_mut().set(0, 0x2000);
    cpu.regs_mut().set(15, PC_READ);
    let mut bus = LogBus::new(blk(0, 1, 1, 0, 0, 0, 1 << 13), &[]);
    cpu.step(&mut bus);
    (bus.log, cpu.regs().get(13))
}

#[test]
fn given_stm_with_s_and_no_r15_when_executed_in_irq_mode_then_the_user_bank_r13_is_stored() {
    let want = (vec![wr(4, 0x2000, 0x1111_0000)], 0x2222_0000);
    assert_eq!(
        (stm_user_r13::<false>(), stm_user_r13::<true>()),
        (want.clone(), want)
    );
}

/// LDM^ into the user R13 while in IRQ mode: (user R13, IRQ R13) afterwards.
fn ldm_user_r13<const A: bool>() -> (u32, u32) {
    let mut cpu = Cpu::<A>::new();
    cpu.regs_mut().set_cpsr(IRQ);
    cpu.regs_mut().set_banked(Bank::UserSystem, 13, 0x1111_0000);
    cpu.regs_mut().set(13, 0x2222_0000);
    cpu.regs_mut().set(0, 0x2000);
    cpu.regs_mut().set(15, PC_READ);
    let mut bus = LogBus::new(blk(0, 1, 1, 0, 1, 0, 1 << 13), &[(0x2000, 0xAAAA_0001)]);
    cpu.step(&mut bus);
    (cpu.regs().banked(Bank::UserSystem, 13), cpu.regs().get(13))
}

#[test]
fn given_ldm_with_s_and_no_r15_when_executed_in_irq_mode_then_the_user_bank_r13_is_loaded() {
    let want = (0xAAAA_0001, 0x2222_0000);
    assert_eq!(
        (ldm_user_r13::<false>(), ldm_user_r13::<true>()),
        (want, want)
    );
}

/// STM^ of R8 while in FIQ mode: the log (the user R8 is not the FIQ copy).
fn stm_user_r8<const A: bool>() -> Vec<Access> {
    let mut cpu = Cpu::<A>::new();
    cpu.regs_mut().set_cpsr(FIQ);
    cpu.regs_mut().set_banked(Bank::UserSystem, 8, 0x0000_0088);
    cpu.regs_mut().set(8, 0x0000_0099);
    cpu.regs_mut().set(0, 0x2000);
    cpu.regs_mut().set(15, PC_READ);
    let mut bus = LogBus::new(blk(0, 1, 1, 0, 0, 0, 1 << 8), &[]);
    cpu.step(&mut bus);
    bus.log
}

#[test]
fn given_stm_with_s_when_executed_in_fiq_mode_then_the_user_r8_is_stored_not_the_fiq_copy() {
    let want = vec![wr(4, 0x2000, 0x88)];
    assert_eq!(
        (stm_user_r8::<false>(), stm_user_r8::<true>()),
        (want.clone(), want)
    );
}

/// LDM^ {R15} in IRQ mode with an SPSR: (CPSR, R15, bank) afterwards.
fn ldm_pc_with_s<const A: bool>() -> (u32, u32, Bank) {
    let mut cpu = Cpu::<A>::new();
    cpu.regs_mut().set_cpsr(IRQ);
    cpu.regs_mut().set_spsr(Bank::Irq, N | SVC);
    cpu.regs_mut().set(0, 0x2000);
    cpu.regs_mut().set(15, PC_READ);
    let mut bus = LogBus::new(blk(0, 1, 1, 0, 1, 0, 0x8000), &[(0x2000, 0x0000_4000)]);
    cpu.step(&mut bus);
    (cpu.regs().cpsr(), cpu.regs().get(15), cpu.regs().bank())
}

#[test]
fn given_ldm_with_s_and_r15_when_executed_in_irq_mode_then_the_spsr_is_copied_to_the_cpsr() {
    let want = (N | SVC, 0x4008, Bank::Supervisor);
    assert_eq!(
        (ldm_pc_with_s::<false>(), ldm_pc_with_s::<true>()),
        (want, want)
    );
}

// ---------------------------------------------------------------- block: base in list, empty list

/// Base R1 = 0x2000 in the list, with writeback unless stated.
fn base_in_list_cases(arm9: bool) -> Vec<Case> {
    let mem = [(0x2000, 0x1111), (0x2004, 0x2222)];
    vec![
        case(
            "STMIA R1!,{R1,R2}: base first stores the old base",
            blk(0, 1, 0, 1, 0, 1, 0b110),
            &[(1, 0x2000), (2, 0xBB)],
            &[],
            &[(1, 0x2008)],
            &[wr(4, 0x2000, 0x2000), wr(4, 0x2004, 0xBB)],
        ),
        case(
            "STMIA R1!,{R0,R1}: base not first stores the new base on ARM7, the old on ARM9",
            blk(0, 1, 0, 1, 0, 1, 0b11),
            &[(0, 0xAA), (1, 0x2000)],
            &[],
            &[(1, 0x2008)],
            &[
                wr(4, 0x2000, 0xAA),
                wr(4, 0x2004, pick(arm9, 0x2008, 0x2000)),
            ],
        ),
        case(
            "LDMIA R1,{R1,R2} without writeback keeps the loaded value",
            blk(0, 1, 0, 0, 1, 1, 0b110),
            &[(1, 0x2000)],
            &mem,
            &[(1, 0x1111), (2, 0x2222)],
            &[rd(4, 0x2000, 0x1111), rd(4, 0x2004, 0x2222)],
        ),
        case(
            "LDMIA R1!,{R1,R2}: base first, ARM7 has no writeback, ARM9 writes back",
            blk(0, 1, 0, 1, 1, 1, 0b110),
            &[(1, 0x2000)],
            &mem,
            &[(1, pick(arm9, 0x1111, 0x2008)), (2, 0x2222)],
            &[rd(4, 0x2000, 0x1111), rd(4, 0x2004, 0x2222)],
        ),
        case(
            "LDMIA R1!,{R0,R1}: base last in a longer list keeps the loaded value",
            blk(0, 1, 0, 1, 1, 1, 0b11),
            &[(1, 0x2000)],
            &mem,
            &[(0, 0x1111), (1, 0x2222)],
            &[rd(4, 0x2000, 0x1111), rd(4, 0x2004, 0x2222)],
        ),
        case(
            "LDMIA R1!,{R1}: a single base register, ARM7 has no writeback, ARM9 writes back",
            blk(0, 1, 0, 1, 1, 1, 0b10),
            &[(1, 0x2000)],
            &mem,
            &[(1, pick(arm9, 0x1111, 0x2004))],
            &[rd(4, 0x2000, 0x1111)],
        ),
    ]
}

#[test]
fn given_an_arm7_when_the_base_is_in_the_list_then_stm_and_ldm_follow_the_arm7_column() {
    assert_eq!(
        table::<false>(&base_in_list_cases(false)),
        Vec::<String>::new()
    );
}

#[test]
fn given_an_arm9_when_the_base_is_in_the_list_then_stm_and_ldm_follow_the_arm9_column() {
    assert_eq!(
        table::<true>(&base_in_list_cases(true)),
        Vec::<String>::new()
    );
}

#[test]
fn given_an_arm7_when_the_register_list_is_empty_then_r15_transfers_and_the_base_moves_0x40() {
    let cases = [
        case(
            "STMDB R0!,{} stores R15 at the lowest address",
            blk(1, 0, 0, 1, 0, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x1FC0)],
            &[wr(4, 0x1FC0, PC_STORE)],
        ),
        case(
            "LDMDB R0!,{} loads R15",
            blk(1, 0, 0, 1, 1, 0, 0),
            &[(0, 0x2000)],
            &[(0x1FC0, 0x0000_7000)],
            &[(0, 0x1FC0)],
            &[rd(4, 0x1FC0, 0x7000)],
        )
        .pc(0x7008),
        // The transfer address of the other modes and the W = 0 form are not measured on the
        // suite (`docs/hw/arm-load-store.md`, "Unverified"); these pin the chosen behavior.
        case(
            "STMIA R0!,{} stores R15 at the base",
            blk(0, 1, 0, 1, 0, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x2040)],
            &[wr(4, 0x2000, PC_STORE)],
        ),
        case(
            "STMIB R0!,{} stores R15 above the base",
            blk(1, 1, 0, 1, 0, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x2040)],
            &[wr(4, 0x2004, PC_STORE)],
        ),
        case(
            "STMDA R0!,{} stores R15 at the lowest address",
            blk(0, 0, 0, 1, 0, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x1FC0)],
            &[wr(4, 0x1FC4, PC_STORE)],
        ),
        case(
            "STMIA R0,{} without writeback leaves the base",
            blk(0, 1, 0, 0, 0, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x2000)],
            &[wr(4, 0x2000, PC_STORE)],
        ),
    ];
    assert_eq!(table::<false>(&cases), Vec::<String>::new());
}

#[test]
fn given_an_arm9_when_the_register_list_is_empty_then_the_base_moves_0x40_with_no_transfer() {
    let cases = [
        case(
            "STMDB R0!,{}",
            blk(1, 0, 0, 1, 0, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x1FC0)],
            &[],
        ),
        case(
            "STMIA R0!,{}",
            blk(0, 1, 0, 1, 0, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x2040)],
            &[],
        ),
        case(
            "LDMDB R0!,{}",
            blk(1, 0, 0, 1, 1, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x1FC0)],
            &[],
        ),
        case(
            "LDMIA R0!,{}",
            blk(0, 1, 0, 1, 1, 0, 0),
            &[(0, 0x2000)],
            &[],
            &[(0, 0x2040)],
            &[],
        ),
    ];
    assert_eq!(table::<true>(&cases), Vec::<String>::new());
}

// ---------------------------------------------------------------- properties

proptest! {
    #[test]
    fn given_any_word_and_address_when_rotated_then_byte0_is_the_byte_at_the_low_address_bits(
        value in any::<u32>(), address in any::<u32>(),
    ) {
        let shift = 8 * (address & 3);
        prop_assert_eq!(rotate_word(value, address) & 0xFF, (value >> shift) & 0xFF);
    }

    #[test]
    fn given_any_word_when_the_address_is_aligned_then_rotate_word_is_the_identity(
        value in any::<u32>(), address in any::<u32>(),
    ) {
        prop_assert_eq!(rotate_word(value, address & !3), value);
    }

    #[test]
    fn given_any_word_when_rotated_and_rotated_back_then_the_word_is_unchanged(
        value in any::<u32>(), address in any::<u32>(),
    ) {
        let back = rotate_word(rotate_word(value, address), 4u32.wrapping_sub(address & 3));
        prop_assert_eq!(back, value);
    }

    #[test]
    fn given_any_address_when_rotated_then_only_the_low_two_bits_matter(
        value in any::<u32>(), address in any::<u32>(), high in any::<u32>(),
    ) {
        prop_assert_eq!(
            rotate_word(value, address),
            rotate_word(value, (high & !3) | (address & 3))
        );
    }

    #[test]
    fn given_an_arm9_when_ldrh_reads_any_address_then_the_halfword_is_zero_extended(
        address in any::<u32>(), data in any::<u16>(),
    ) {
        prop_assert_eq!(halfword_value(true, false, address, data), u32::from(data));
    }

    #[test]
    fn given_an_arm9_when_ldrsh_reads_any_address_then_the_halfword_is_sign_extended(
        address in any::<u32>(), data in any::<u16>(),
    ) {
        prop_assert_eq!(halfword_value(true, true, address, data), i32::from(data as i16) as u32);
    }

    #[test]
    fn given_an_arm7_when_ldrh_reads_an_even_address_then_the_halfword_is_zero_extended(
        address in any::<u32>(), data in any::<u16>(),
    ) {
        prop_assert_eq!(halfword_value(false, false, address & !1, data), u32::from(data));
    }

    #[test]
    fn given_an_arm7_when_ldrh_reads_an_odd_address_then_the_halfword_rotates_right_by_8(
        address in any::<u32>(), data in any::<u16>(),
    ) {
        let want = (u32::from(data) >> 8) | (u32::from(data & 0xFF) << 24);
        prop_assert_eq!(halfword_value(false, false, address | 1, data), want);
    }

    #[test]
    fn given_an_arm7_when_ldrsh_reads_an_even_address_then_the_halfword_is_sign_extended(
        address in any::<u32>(), data in any::<u16>(),
    ) {
        prop_assert_eq!(
            halfword_value(false, true, address & !1, data),
            i32::from(data as i16) as u32
        );
    }

    #[test]
    fn given_an_arm7_when_ldrsh_reads_an_odd_address_then_the_high_byte_is_sign_extended(
        address in any::<u32>(), data in any::<u16>(),
    ) {
        let want = i32::from((data >> 8) as u8 as i8) as u32;
        prop_assert_eq!(halfword_value(false, true, address | 1, data), want);
    }

    #[test]
    fn given_any_base_and_count_when_block_start_is_asked_then_ia_ib_da_db_follow_the_note(
        base in any::<u32>(), count in 1u32..=16,
    ) {
        let span = 4 * count;
        prop_assert_eq!(block_start(false, true, count, base), base);
        prop_assert_eq!(block_start(true, true, count, base), base.wrapping_add(4));
        prop_assert_eq!(
            block_start(false, false, count, base),
            base.wrapping_sub(span).wrapping_add(4)
        );
        prop_assert_eq!(block_start(true, false, count, base), base.wrapping_sub(span));
    }
}
