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
