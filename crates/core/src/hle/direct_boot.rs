// SPDX-License-Identifier: Apache-2.0
//! Direct boot: places the binaries and starts both CPUs at their entry points. Note:
//! `docs/hw/direct-boot.md`.

use crate::bus::{Arm7View, Arm9View, Memory, TcmConfig};
use crate::cart::{BinarySpec, HEADER_LEN, Header};
use crate::cpu::{Bank, Cpu};

/// WRAMCNT, and the mode that gives all of the shared WRAM to the ARM7 (`wramcnt.md`).
const WRAMCNT_ADDR: u32 = 0x0400_0247;
const WRAMCNT_ARM7_ALL: u8 = 3;
/// Where the loader keeps the header copy (`direct-boot.md`, "Placement").
const HEADER_COPY_ADDRESS: u32 = 0x027F_FE00;

/// Register values at cartridge boot time (`direct-boot.md`, "ARM9 state at entry").
const SP_SYSTEM: u32 = 0x0300_2F7C;
const SP_IRQ: u32 = 0x0300_3F80;
const SP_SUPERVISOR: u32 = 0x0300_3FC0;
/// CPSR: System mode, ARM state, I and F set. Chosen so the machine can run; GBATEK does not
/// state it.
// TODO(hw-unverified): see `direct-boot.md`, "Unverified".
const CPSR_AT_ENTRY: u32 = 0x0000_00DF;
/// The ARM7's stacks (`direct-boot.md`, "ARM9 state at entry"; "ARM7 state at entry").
const ARM7_SP_SYSTEM: u32 = 0x0380_FD80;
const ARM7_SP_IRQ: u32 = 0x0380_FF80;
const ARM7_SP_SUPERVISOR: u32 = 0x0380_FFC0;

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

/// Copies `bytes` to `address` through the ARM7 data bus: main RAM, or the `037F_8000h` range,
/// which lands in shared or ARM7 WRAM as WRAMCNT says (`wramcnt.md`).
fn place7(memory: &mut Memory, address: u32, bytes: &[u8]) {
    bytes.iter().enumerate().for_each(|(index, &byte)| {
        memory.write8_arm7(address.wrapping_add(index as u32), byte);
    });
}

/// The bytes of one binary in the image, or `None` when its range is not inside it.
fn binary_bytes<'a>(rom: &'a [u8], spec: &BinarySpec) -> Option<&'a [u8]> {
    let start = spec.rom_offset as usize;
    rom.get(start..start.saturating_add(spec.size as usize))
}

/// Places the binaries and sets the ARM9 up at its entry point. `header` has been validated by
/// `cart::parse`, so every range it names is inside `rom` and main RAM.
// TODO(hw-unverified): CPSR at entry, CP15 left at power-on, the other register banks at zero,
// the ARM9-then-ARM7 placement order, and no BIOS/firmware values in `027FF800h` and up
// (`direct-boot.md`, "Unverified").
// TODO(hw-unverified): the ARM7's CPSR at entry is chosen like the ARM9's (`direct-boot.md`).
// TODO(phase3): the BIOS/firmware values arrive with their phases.
pub(crate) fn boot(
    arm9: &mut Cpu<true>,
    arm7: &mut Cpu<false>,
    memory: &mut Memory,
    rom: &[u8],
    header: &Header,
) {
    // TODO(hw-unverified): CP15 is left at power-on, so both TCMs are disabled here
    // (`direct-boot.md`, "CP15 state at entry").
    let tcm = TcmConfig::from_cp15(arm9.cp15());
    if let Some(copy) = rom.get(..HEADER_LEN) {
        place(memory, &tcm, HEADER_COPY_ADDRESS, copy);
    }
    // TODO(hw-unverified): the ARM9 binary is placed before the ARM7's; GBATEK gives no order.
    if let Some(bytes) = binary_bytes(rom, &header.arm9) {
        place(memory, &tcm, header.arm9.ram_address, bytes);
    }
    // All 32 KiB of shared WRAM go to the ARM7 first, so the boot area at `037F_8000h`, its stacks
    // at `0380_FD80h` and the IRQ vector at `0380_FFFCh` are different memory; with mode 0 they
    // would overlap through the ARM7's 64 KiB repetition (`wramcnt.md`, "Behavior").
    // TODO(hw-unverified): the value after boot is inferred, not stated, and leaves the ARM9
    // stacks at `0300_2F7Ch` unbacked (`wramcnt.md`, "Unverified").
    memory.write8(&tcm, WRAMCNT_ADDR, WRAMCNT_ARM7_ALL);
    if let Some(bytes) = binary_bytes(rom, &header.arm7) {
        place7(memory, header.arm7.ram_address, bytes);
    }

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

    let entry7 = header.arm7.entry;
    let regs7 = arm7.regs_mut();
    regs7.set_cpsr(CPSR_AT_ENTRY);
    regs7.set(STACK_REGISTER, ARM7_SP_SYSTEM);
    regs7.set(IP_REGISTER, entry7);
    regs7.set(LINK_REGISTER, entry7);
    regs7.set_banked(Bank::Irq, STACK_REGISTER, ARM7_SP_IRQ);
    regs7.set_banked(Bank::Supervisor, STACK_REGISTER, ARM7_SP_SUPERVISOR);
    let mut view7 = Arm7View::new(memory);
    arm7.start_at(&mut view7, entry7);
}
