// SPDX-License-Identifier: Apache-2.0
//! The SWI routines the HLE BIOS implements, by number. Note: `docs/hw/hle-swi.md`.
//!
//! A routine is a function generic over the CPU and its bus: it reads and writes memory through
//! the CPU's own view, so I/O writes land, and it never allocates (AGENTS.md §2 rule 3).

mod copy;
mod lz77;
mod math;
mod misc;

use super::bios::{SwiRecord, flag_address};
use crate::cpu::{Bus, Cpu};
use crate::io::irq::IME_ADDR;
use crate::spu::{SOUNDBIAS_ADDR, SOUNDBIAS_MASK};

/// The SWI numbers of the wait routines (`hle-swi.md`, "The wait SWIs").
const WAIT_BY_LOOP: u8 = 0x03;
const INTR_WAIT: u8 = 0x04;
const VBLANK_INTR_WAIT: u8 = 0x05;
const HALT: u8 = 0x06;
/// The data routines (`hle-swi.md`, "The copy, math and decompression SWIs").
/// SoundBias, ARM7 only (`hle-swi.md`, "The copy, math and decompression SWIs").
const SOUND_BIAS: u8 = 0x08;
const DIV: u8 = 0x09;
const CPU_SET: u8 = 0x0B;
const CPU_FAST_SET: u8 = 0x0C;
const SQRT: u8 = 0x0D;
const GET_CRC16: u8 = 0x0E;
const IS_DEBUGGER: u8 = 0x0F;
const LZ77_WRAM: u8 = 0x11;
const LZ77_VRAM: u8 = 0x12;
/// SOUNDBIAS level 200h, the medium level.
const BIAS_MID: u32 = 0x200;
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
// TODO(phase5): Sleep (07h) powers down the hardware; it and the numbers nothing implements are the
// unimplemented path.
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
        SOUND_BIAS if !ARM9 => {
            sound_bias(cpu, bus);
            Next::Return
        }
        DIV | CPU_SET | CPU_FAST_SET | SQRT | GET_CRC16 | IS_DEBUGGER | LZ77_WRAM | LZ77_VRAM => {
            data_routine(number, cpu, bus);
            Next::Return
        }
        _ => {
            // TODO(hw-unverified): every unimplemented number, those above 1Fh included, has no
            // effect (`hle-swi.md`, "Unverified").
            record.last_unimplemented = Some(number);
            Next::Return
        }
    }
}

/// The routines that work on registers and memory only: arguments in `R0`-`R2`, results in the
/// registers or in memory through the CPU's own view.
fn data_routine<const ARM9: bool, B: Bus>(number: u8, cpu: &mut Cpu<ARM9>, bus: &mut B) {
    let [r0, r1, r2] = [0, 1, 2].map(|index| cpu.regs().get(index));
    match number {
        DIV => {
            // The operands are signed; the casts reinterpret the register bits.
            let (quotient, remainder, absolute) = math::divide(r0 as i32, r1 as i32);
            let regs = cpu.regs_mut();
            regs.set(0, quotient as u32);
            regs.set(1, remainder as u32);
            regs.set(3, absolute);
        }
        SQRT => cpu.regs_mut().set(0, math::sqrt(r0)),
        CPU_SET => copy::cpu_set(bus, r0, r1, r2),
        CPU_FAST_SET => copy::cpu_fast_set(bus, r0, r1, r2),
        GET_CRC16 => {
            // The initial value is the low half of R0.
            let crc = misc::crc16_halfwords(bus, r0 as u16, r1, r2);
            cpu.regs_mut().set(0, u32::from(crc));
        }
        // Retail hardware has no debugger.
        IS_DEBUGGER => cpu.regs_mut().set(0, 0),
        LZ77_WRAM => lz77::decode_wram(bus, r0, r1),
        LZ77_VRAM => lz77::decode_vram(bus, r0, r1),
        // `run` sends only the numbers in its data arm, each matched above.
        _ => unreachable!("`run` sends data_routine only the numbers it matches itself"),
    }
}

/// SoundBias: SOUNDBIAS bits 0-9 become `000h` for `R0 = 0` and `200h` otherwise, the bits above
/// kept (`hle-swi.md`). The BIOS's stepping and delay (`R1`) are not modeled.
// TODO(hw-unverified): the register lands on the target at once (`hle-swi.md`, "Unverified").
fn sound_bias<const ARM9: bool, B: Bus>(cpu: &Cpu<ARM9>, bus: &mut B) {
    let level = if cpu.regs().get(0) == 0 { 0 } else { BIAS_MID };
    let kept = bus.read32(SOUNDBIAS_ADDR) & !u32::from(SOUNDBIAS_MASK);
    bus.write32(SOUNDBIAS_ADDR, kept | level);
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

#[cfg(test)]
mod tests;
