// SPDX-License-Identifier: Apache-2.0
//! Direct boot: places the binaries and starts the ARM9 at its entry point. Note:
//! `docs/hw/direct-boot.md`.

use crate::bus::{Arm9View, Memory, TcmConfig};
use crate::cart::{BinarySpec, HEADER_LEN, Header};
use crate::cpu::{Bank, Cpu};

/// Where the loader keeps the header copy (`direct-boot.md`, "Placement").
const HEADER_COPY_ADDRESS: u32 = 0x027F_FE00;
/// The end of main RAM; a binary destination below it is main RAM. Only the ARM7 can have a
/// destination above it (the 0x037F_8000 range); every validated ARM9 address is below.
const MAIN_RAM_END: u32 = 0x0240_0000;

/// Register values at cartridge boot time (`direct-boot.md`, "ARM9 state at entry").
const SP_SYSTEM: u32 = 0x0300_2F7C;
const SP_IRQ: u32 = 0x0300_3F80;
const SP_SUPERVISOR: u32 = 0x0300_3FC0;
/// CPSR: System mode, ARM state, I and F set. Chosen so the machine can run; GBATEK does not
/// state it.
// TODO(hw-unverified): see `direct-boot.md`, "Unverified".
const CPSR_AT_ENTRY: u32 = 0x0000_00DF;

const STACK_REGISTER: usize = 13;
const LINK_REGISTER: usize = 14;
const IP_REGISTER: usize = 12;

/// Copies `bytes` to `address` through the ARM9 data bus, so they land where the ARM9 would see
/// them.
fn place(memory: &mut Memory, tcm: &TcmConfig, address: u32, bytes: &[u8]) {
    bytes.iter().enumerate().for_each(|(index, &byte)| {
        memory.write8(tcm, address.wrapping_add(index as u32), byte);
    });
}

/// Copies one binary to its RAM address when that is main RAM.
// TODO(phase3): an ARM7 destination in the 0x037F_8000 range is placed with WRAMCNT (#22).
fn place_binary(memory: &mut Memory, tcm: &TcmConfig, rom: &[u8], spec: &BinarySpec) {
    let start = spec.rom_offset as usize;
    let source = rom.get(start..start.saturating_add(spec.size as usize));
    if let (Some(bytes), true) = (source, spec.ram_address < MAIN_RAM_END) {
        place(memory, tcm, spec.ram_address, bytes);
    }
}

/// Places the binaries and sets the ARM9 up at its entry point. `header` has been validated by
/// `cart::parse`, so every range it names is inside `rom` and main RAM.
// TODO(hw-unverified): CPSR at entry, CP15 left at power-on, the other register banks at zero,
// the ARM9-then-ARM7 placement order, and no BIOS/firmware values in `027FF800h` and up
// (`direct-boot.md`, "Unverified").
// TODO(phase3): the ARM7 and the BIOS/firmware values arrive with their phases.
pub(crate) fn boot(arm9: &mut Cpu<true>, memory: &mut Memory, rom: &[u8], header: &Header) {
    // TODO(hw-unverified): CP15 is left at power-on, so both TCMs are disabled here
    // (`direct-boot.md`, "CP15 state at entry").
    let tcm = TcmConfig::from_cp15(arm9.cp15());
    if let Some(copy) = rom.get(..HEADER_LEN) {
        place(memory, &tcm, HEADER_COPY_ADDRESS, copy);
    }
    // TODO(hw-unverified): the ARM9 binary is placed before the ARM7's; GBATEK gives no order.
    place_binary(memory, &tcm, rom, &header.arm9);
    place_binary(memory, &tcm, rom, &header.arm7);

    let entry = header.arm9.entry;
    let regs = arm9.regs_mut();
    regs.set_cpsr(CPSR_AT_ENTRY);
    regs.set(STACK_REGISTER, SP_SYSTEM);
    regs.set(IP_REGISTER, entry);
    regs.set(LINK_REGISTER, entry);
    regs.set_banked(Bank::Irq, STACK_REGISTER, SP_IRQ);
    regs.set_banked(Bank::Supervisor, STACK_REGISTER, SP_SUPERVISOR);
    let mut view = Arm9View::new(memory, tcm);
    arm9.start_at(&mut view, entry);
}
