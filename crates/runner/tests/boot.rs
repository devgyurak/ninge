// SPDX-License-Identifier: Apache-2.0
//! Direct boot of our own `boot` test ROM (`testroms/boot/`). The ROM loads at 0x0200_0000,
//! enters at 0x0200_0100, and records the register state at entry at 0x0200_0200. Expected
//! values are `docs/hw/direct-boot.md`, "ARM9 state at entry".

use ninge_core::Machine;
use ninge_core::cart::HeaderError;
use ninge_runner::{RunnerError, load_rom_file, run, testrom_path};

const RECORD: u32 = 0x0200_0200;
const ENTRY: u32 = 0x0200_0100;

/// A machine that booted the `boot` ROM and ran one frame.
fn boot_one_frame() -> Machine {
    let rom = load_rom_file(&testrom_path("boot")).expect("boot ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("boot ROM has a valid header");
    machine.run_frame();
    machine
}

fn record(machine: &Machine, slot: u32) -> u32 {
    machine.arm9_peek32(RECORD + slot * 4)
}

/// Recorded r0..r11 that are not zero, as (register, value).
fn nonzero_recorded_registers(machine: &Machine) -> Vec<(u32, u32)> {
    (0..12)
        .map(|r| (r, record(machine, r)))
        .filter(|&(_, v)| v != 0)
        .collect()
}

#[test]
fn given_boot_rom_when_run_one_frame_then_r0_to_r11_were_zero_at_entry() {
    assert_eq!(
        nonzero_recorded_registers(&boot_one_frame()),
        Vec::<(u32, u32)>::new()
    );
}

#[test]
fn given_boot_rom_when_run_one_frame_then_r12_was_the_entry_address() {
    assert_eq!(record(&boot_one_frame(), 12), ENTRY);
}

#[test]
fn given_boot_rom_when_run_one_frame_then_r13_was_03002f7c() {
    assert_eq!(record(&boot_one_frame(), 13), 0x0300_2F7C);
}

#[test]
fn given_boot_rom_when_run_one_frame_then_r14_was_the_entry_address() {
    assert_eq!(record(&boot_one_frame(), 14), ENTRY);
}

#[test]
fn given_boot_rom_when_run_one_frame_then_cpsr_was_system_mode_with_i_and_f_set() {
    assert_eq!(record(&boot_one_frame(), 15), 0xDF);
}

#[test]
fn given_boot_rom_when_run_one_frame_then_sp_svc_was_03003fc0() {
    assert_eq!(record(&boot_one_frame(), 16), 0x0300_3FC0);
}

#[test]
fn given_boot_rom_when_run_one_frame_then_sp_irq_was_03003f80() {
    assert_eq!(record(&boot_one_frame(), 17), 0x0300_3F80);
}

#[test]
fn given_boot_rom_when_loaded_then_header_entry_differs_from_the_ram_address() {
    let rom = load_rom_file(&testrom_path("boot")).expect("boot ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("valid header");
    let header = machine.header().expect("accepted ROM has a header");
    assert_eq!(
        (header.arm9.ram_address, header.arm9.entry),
        (0x0200_0000, ENTRY)
    );
}

#[test]
fn given_boot_rom_when_loaded_then_the_first_opcode_is_at_the_entry_address() {
    let rom = load_rom_file(&testrom_path("boot")).expect("boot ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("valid header");
    // `str r0, [pc, #0xF8]`: the record is 0x100 past the entry and pc reads entry + 8.
    assert_eq!(machine.arm9_peek32(ENTRY), 0xE58F_00F8);
}

#[test]
fn given_boot_rom_when_loaded_then_the_header_copy_is_at_027ffe00() {
    let rom = load_rom_file(&testrom_path("boot")).expect("boot ROM is built");
    let first_word = u32::from_le_bytes([rom[0], rom[1], rom[2], rom[3]]);
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("valid header");
    assert_eq!(machine.arm9_peek32(0x027F_FE00), first_word);
}

#[test]
fn given_refused_rom_when_running_through_the_runner_then_it_is_a_rom_error() {
    let result = run(vec![1, 2, 3], 1);
    assert!(matches!(
        result,
        Err(RunnerError::Rom(HeaderError::ImageTooShort { len: 3 }))
    ));
}
