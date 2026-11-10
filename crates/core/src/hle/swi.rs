// SPDX-License-Identifier: Apache-2.0
//! The SWI routines the HLE BIOS implements, by number. Note: `docs/hw/hle-swi.md`.
//!
//! A routine is a function generic over the CPU and its bus: it reads and writes memory through
//! the CPU's own view, so I/O writes land, and it never allocates (AGENTS.md §2 rule 3).

use super::bios::{SwiRecord, flag_address};
use crate::cpu::{Bus, Cpu};
use crate::io::irq::IME_ADDR;

/// The SWI numbers of the wait routines (`hle-swi.md`, "The wait SWIs").
const WAIT_BY_LOOP: u8 = 0x03;
const INTR_WAIT: u8 = 0x04;
const VBLANK_INTR_WAIT: u8 = 0x05;
const HALT: u8 = 0x06;
/// The V-blank bit of IE, IF and the IRQ check flags.
const VBLANK: u32 = 1;

/// What the BIOS does after a routine: leave the SWI, or go on waiting at the IntrWait loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Next {
    /// Return to the caller as `MOVS PC, LR`.
    Return,
    /// Continue in the IntrWait loop (`hle-swi.md`, "The wait SWIs").
    IntrWait,
}

/// Runs SWI `number` for `cpu`. A number no routine implements has no effect and is recorded
/// (`hle-swi.md`, "Unimplemented number").
// TODO(phase3): the copy, math and decompression routines arrive in the next #27 PR.
// TODO(hw-unverified): WaitByLoop does not delay; the SWI costs one instruction
// (`hle-swi.md`, "Unverified").
pub(super) fn run<const ARM9: bool, B: Bus>(
    number: u8,
    cpu: &mut Cpu<ARM9>,
    bus: &mut B,
    record: &mut SwiRecord,
) -> Next {
    match number {
        WAIT_BY_LOOP => Next::Return,
        HALT => {
            cpu.set_halted(true);
            Next::Return
        }
        INTR_WAIT => {
            intr_wait(cpu, bus);
            Next::IntrWait
        }
        VBLANK_INTR_WAIT => {
            // VBlankIntrWait is IntrWait with R0 = 1 (discard old flags) and R1 = 1 (V-blank).
            cpu.regs_mut().set(0, 1);
            cpu.regs_mut().set(1, VBLANK);
            intr_wait(cpu, bus);
            Next::IntrWait
        }
        _ => {
            // TODO(hw-unverified): every unimplemented number, those above 1Fh included, has no
            // effect (`hle-swi.md`, "Unverified").
            record.last_unimplemented = Some(number);
            Next::Return
        }
    }
}

/// The start of IntrWait: IME = 1, and with `R0` not zero the old flags of `R1` are discarded
/// (`hle-swi.md`, "The wait SWIs", step 1).
// TODO(hw-unverified): IME is forced to 1 and left there, and discarding clears the `R1` bits of
// the flag word (`hle-swi.md`, "Unverified").
fn intr_wait<const ARM9: bool, B: Bus>(cpu: &mut Cpu<ARM9>, bus: &mut B) {
    bus.write32(IME_ADDR, 1);
    if cpu.regs().get(0) != 0 {
        let address = flag_address(cpu);
        let flags = bus.read32(address);
        bus.write32(address, flags & !cpu.regs().get(1));
    }
}
