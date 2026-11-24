// SPDX-License-Identifier: Apache-2.0
//! The HLE BIOS: the traps at the BIOS vectors, the SWI entry and the IRQ dispatcher. Note:
//! `docs/hw/hle-swi.md`.
//!
//! The CPU enters an exception as it always does and lands in the synthetic BIOS area; before it
//! executes there, the machine compares R15 with the trap addresses and does in Rust what the
//! BIOS code would do. Nothing is encoded in the area itself.

use super::swi::{self, Next};
use crate::bus::{BIOS7_BASE, BIOS9_BASE};
use crate::cpu::{Bus, Cpu, FLAG_I, FLAG_T, LR, PC, SP};
use serde::{Deserialize, Serialize};

/// The offset of the SWI vector from the BIOS base, and of the IRQ vector.
const SWI_VECTOR: u32 = 0x08;
const IRQ_VECTOR: u32 = 0x18;
/// ninge's own address for the user handler's return (`hle-swi.md`, "The vectors").
// TODO(hw-unverified): GBATEK does not state the BIOS's own address (`hle-swi.md`, "Unverified").
const IRQ_RETURN_VECTOR: u32 = 0x20;
/// ninge's own address where IntrWait loops (`hle-swi.md`, "The wait SWIs").
// TODO(hw-unverified): the address is ninge's, invisible unless a game reads `LR` inside IntrWait.
const INTR_WAIT_VECTOR: u32 = 0x24;
/// ninge's own addresses where WaitByLoop spends its delay: one for each of the four cycles a pass
/// of the BIOS's loop takes, the last of which counts the pass (`hle-swi.md`, "The wait SWIs").
// TODO(hw-unverified): the addresses are ninge's, and four cycles a pass is GBATEK's figure for the
// ARM7 and for the ARM9 with the cache on (`hle-swi.md`, "Unverified").
const WAIT_LOOP_VECTOR: u32 = 0x28;
const WAIT_LOOP_LAST: u32 = WAIT_LOOP_VECTOR + 4 * (WAIT_LOOP_CYCLES - 1);
/// The cycles of the calling CPU one pass of WaitByLoop takes.
const WAIT_LOOP_CYCLES: u32 = 4;
/// R15 reads this far past the instruction at the head of the pipeline (`arm-branch-pipeline.md`).
const PIPELINE_AHEAD: u32 = 8;
/// The farthest trap address from the base, in R15 terms: a compare against it is the whole cost
/// of the check for any R15 outside the vectors.
const TRAP_SPAN: u32 = WAIT_LOOP_LAST + PIPELINE_AHEAD;

/// Where the I/O area starts; the BIOS reaches the handler pointer through it (`hle-swi.md`).
// TODO(hw-unverified): the value in R0 at the user handler (`hle-swi.md`, "Unverified").
const IO_BASE: u32 = 0x0400_0000;
/// The ARM7's handler pointer, and the offset of the ARM9's from its DTCM base
/// (`hle-swi.md`, "The IRQ dispatcher").
const HANDLER_POINTER7: u32 = 0x0380_FFFC;
const HANDLER_POINTER9_OFFSET: u32 = 0x3FFC;
/// The IRQ check flags IntrWait reads (`hle-swi.md`, "The wait SWIs").
const FLAGS7: u32 = 0x0380_FFF8;
const FLAGS9_OFFSET: u32 = 0x3FF8;
/// The registers the dispatcher saves, in the order they are stored (`hle-swi.md`).
const SAVED: [usize; 6] = [0, 1, 2, 3, 12, LR];
const FRAME_BYTES: u32 = 4 * SAVED.len() as u32;
/// `SUBS PC, LR, #4`: what the IRQ return subtracts from LR.
const IRQ_RETURN_OFFSET: u32 = 4;

/// What each BIOS entry costs, in instructions at the CPU's own instruction cost: one, as the
/// exception entry itself costs (`hle-swi.md`, "Cost"). The savestate's check that the ARM7
/// clock is at most one instruction past the master clock relies on a step never costing more.
// TODO(hw-unverified): a model, not a measurement (`hle-swi.md`, "Unverified").
const ENTRY_INSTRUCTIONS: u64 = 1;

/// What the debugger can read about one CPU's SWIs (`hle-swi.md`, "SWI").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwiRecord {
    pub(crate) last: Option<u8>,
    pub(crate) last_unimplemented: Option<u8>,
}

impl SwiRecord {
    /// The number of the last SWI the BIOS took, or `None` before the first.
    // The debugger's CPU panel (#56) will show it.
    pub fn last(&self) -> Option<u8> {
        self.last
    }

    /// The number of the last SWI no routine implements, or `None` if there was none.
    // The debugger's CPU panel (#56) will show it.
    pub fn last_unimplemented(&self) -> Option<u8> {
        self.last_unimplemented
    }
}

/// The BIOS state of both CPUs: plain data, saved with the machine.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BiosState {
    pub(crate) arm9: SwiRecord,
    pub(crate) arm7: SwiRecord,
}

/// The entries the machine takes over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trap {
    Swi,
    Irq,
    IrqReturn,
    IntrWaitLoop,
    /// One cycle of a WaitByLoop pass; `last` is the one that counts the pass.
    WaitLoop {
        last: bool,
    },
}

/// The trap R15 is at for CPU `ARM9`, if any. The common case, R15 outside the vectors, costs
/// one compare. Each CPU matches only inside its own BIOS area: the ARM9's R15 in
/// `FFFF0000h`-`FFFF7FFFh` (a game that keeps its vectors at 0 never reaches the BIOS,
/// `hle-swi.md`, "The game's own vectors"), the ARM7's from 0, where its vectors always are.
pub(crate) fn trap_at<const ARM9: bool>(pc: u32) -> Option<Trap> {
    let offset = pc.wrapping_sub(bios_base::<ARM9>());
    if offset > TRAP_SPAN {
        return None;
    }
    match offset.wrapping_sub(PIPELINE_AHEAD) {
        SWI_VECTOR => Some(Trap::Swi),
        IRQ_VECTOR => Some(Trap::Irq),
        IRQ_RETURN_VECTOR => Some(Trap::IrqReturn),
        INTR_WAIT_VECTOR => Some(Trap::IntrWaitLoop),
        // The range is tested whole: an R15 below `PIPELINE_AHEAD` wraps to a large `at`.
        at if (WAIT_LOOP_VECTOR..=WAIT_LOOP_LAST).contains(&at) && at % 4 == 0 => {
            Some(Trap::WaitLoop {
                last: at == WAIT_LOOP_LAST,
            })
        }
        _ => None,
    }
}

/// Does the BIOS's work if the CPU is at a trap, and returns the cost in master cycles; `None`
/// when the CPU is not at one and should execute normally.
pub(crate) fn enter<const ARM9: bool, B: Bus>(
    cpu: &mut Cpu<ARM9>,
    bus: &mut B,
    record: &mut SwiRecord,
) -> Option<u64> {
    let trap = trap_at::<ARM9>(cpu.regs().get(PC))?;
    // The vectors are entered in ARM state; Thumb code that jumped here is not the BIOS's.
    if cpu.regs().cpsr() & FLAG_T != 0 {
        return None;
    }
    match trap {
        Trap::Swi => software_interrupt(cpu, bus, record),
        Trap::Irq => irq_enter(cpu, bus),
        Trap::IrqReturn => irq_return(cpu, bus),
        Trap::IntrWaitLoop => intr_wait_loop(cpu, bus),
        Trap::WaitLoop { last } => wait_loop(cpu, bus, last),
    }
    Some(ENTRY_INSTRUCTIONS * Cpu::<ARM9>::INSTRUCTION_COST)
}

/// The SWI vector: finds the number, runs the routine and returns as `MOVS PC, LR`
/// (`hle-swi.md`, "SWI").
fn software_interrupt<const ARM9: bool, B: Bus>(
    cpu: &mut Cpu<ARM9>,
    bus: &mut B,
    record: &mut SwiRecord,
) {
    // TODO(hw-unverified): the byte at LR - 2 is bits 23-16 of an ARM SWI and bits 7-0 of a Thumb
    // one. A normal load, as the BIOS's own, so an I/O address would have its side effect
    // (`hle-swi.md`, "Unverified").
    let number = bus.read8(cpu.regs().get(LR).wrapping_sub(2));
    record.last = Some(number);
    match swi::run(number, cpu, bus, record) {
        Next::Return => cpu.return_from_exception(bus, 0),
        // The delay runs in SVC mode with I set, as the entry left the CPSR (`hle-swi.md`, "The
        // wait SWIs").
        Next::WaitByLoop => cpu.jump_to(bus, bios_base::<ARM9>().wrapping_add(WAIT_LOOP_VECTOR)),
        Next::IntrWait => {
            // TODO(hw-unverified): I is cleared during the wait (`hle-swi.md`, "Unverified").
            // The BIOS waits with interrupts open (`hle-swi.md`, "The wait SWIs", step 1); the
            // caller's CPSR comes back through `SPSR_svc` when the wait returns.
            let cpsr = cpu.regs().cpsr();
            cpu.regs_mut().set_cpsr(cpsr & !FLAG_I);
            cpu.jump_to(bus, bios_base::<ARM9>().wrapping_add(INTR_WAIT_VECTOR));
        }
    }
}

/// The base of CPU `ARM9`'s BIOS area.
const fn bios_base<const ARM9: bool>() -> u32 {
    match ARM9 {
        true => BIOS9_BASE,
        false => BIOS7_BASE,
    }
}

/// Where the IRQ check flags are: the ARM9's DTCM base + `3FF8h`, the ARM7's `0380FFF8h`.
// TODO(hw-unverified): the ARM9 address follows the DTCM base register even when the DTCM is
// disabled (`hle-swi.md`, "Unverified").
pub(super) fn flag_address<const ARM9: bool>(cpu: &Cpu<ARM9>) -> u32 {
    match ARM9 {
        true => cpu.cp15().dtcm().base.wrapping_add(FLAGS9_OFFSET),
        false => FLAGS7,
    }
}

/// One pass of IntrWait's loop (`hle-swi.md`, "The wait SWIs", step 2): consumes the waited bits
/// of the flag word and returns to the caller if any was set, else halts and stays here.
// TODO(hw-unverified): the flag word is rewritten even without a hit, a hit clears the waited
// bits, IME stays 1, a wait whose flag is already set never halts, and I is clear while waiting
// (`hle-swi.md`, "Unverified").
fn intr_wait_loop<const ARM9: bool, B: Bus>(cpu: &mut Cpu<ARM9>, bus: &mut B) {
    let wanted = cpu.regs().get(1);
    let address = flag_address(cpu);
    let flags = bus.read32(address);
    bus.write32(address, flags & !wanted);
    match flags & wanted != 0 {
        true => cpu.return_from_exception(bus, 0),
        // The pipeline is left alone: after the wake the next step is this trap again.
        false => cpu.set_halted(true),
    }
}

/// One cycle of WaitByLoop's loop (`hle-swi.md`, "The wait SWIs"): the first three of a pass only
/// move on; the last subtracts 1 from R0 and goes round again while R0 was above 1 as a signed
/// number, the test the BIOS's `BGT` makes on the flags of its subtraction. Otherwise the SWI
/// returns. R15 is the position, so the wait needs no state of its own.
// TODO(hw-unverified): R0 ends at zero, a count of zero or less makes one pass, and an interrupt
// waits for the return because I stays set (`hle-swi.md`, "Unverified").
fn wait_loop<const ARM9: bool, B: Bus>(cpu: &mut Cpu<ARM9>, bus: &mut B, last: bool) {
    if !last {
        // R15 is 8 past the address of this cycle, so 4 less is the next cycle's address.
        let next = cpu.regs().get(PC).wrapping_sub(PIPELINE_AHEAD - 4);
        cpu.jump_to(bus, next);
        return;
    }
    let count = cpu.regs().get(0);
    cpu.regs_mut().set(0, count.wrapping_sub(1));
    // The count is signed; the cast reinterprets the register bits.
    match count as i32 > 1 {
        true => cpu.jump_to(bus, bios_base::<ARM9>().wrapping_add(WAIT_LOOP_VECTOR)),
        false => cpu.return_from_exception(bus, 0),
    }
}

/// Where the user handler's address is kept: the ARM9's DTCM base + `3FFCh`, the ARM7's
/// `0380FFFCh` (`hle-swi.md`, "The IRQ dispatcher").
// TODO(hw-unverified): the ARM9 address follows the DTCM base register even when the DTCM is
// disabled (`hle-swi.md`, "Unverified").
fn handler_pointer<const ARM9: bool>(cpu: &Cpu<ARM9>) -> u32 {
    match ARM9 {
        true => cpu.cp15().dtcm().base.wrapping_add(HANDLER_POINTER9_OFFSET),
        false => HANDLER_POINTER7,
    }
}

/// The IRQ vector: saves R0-R3, R12 and LR on the IRQ stack and calls the user handler with
/// LR at the IRQ return address (`hle-swi.md`, "The IRQ dispatcher").
// TODO(hw-unverified): the registers saved, R0 and the return address are the GBA BIOS's, carried
// over (`hle-swi.md`, "Unverified").
fn irq_enter<const ARM9: bool, B: Bus>(cpu: &mut Cpu<ARM9>, bus: &mut B) {
    let sp = cpu.regs().get(SP).wrapping_sub(FRAME_BYTES);
    SAVED.iter().enumerate().for_each(|(slot, &register)| {
        bus.write32(sp.wrapping_add(4 * slot as u32), cpu.regs().get(register));
    });
    let pointer = bus.read32(handler_pointer(cpu));
    let regs = cpu.regs_mut();
    regs.set(SP, sp);
    regs.set(0, IO_BASE);
    regs.set(LR, bios_base::<ARM9>().wrapping_add(IRQ_RETURN_VECTOR));
    cpu.jump_to(bus, pointer);
}

/// The IRQ return: restores the saved registers and returns as `SUBS PC, LR, #4`
/// (`hle-swi.md`, "The IRQ dispatcher").
fn irq_return<const ARM9: bool, B: Bus>(cpu: &mut Cpu<ARM9>, bus: &mut B) {
    let sp = cpu.regs().get(SP);
    SAVED.iter().enumerate().for_each(|(slot, &register)| {
        let value = bus.read32(sp.wrapping_add(4 * slot as u32));
        cpu.regs_mut().set(register, value);
    });
    cpu.regs_mut().set(SP, sp.wrapping_add(FRAME_BYTES));
    cpu.return_from_exception(bus, IRQ_RETURN_OFFSET);
}
