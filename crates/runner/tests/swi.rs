// SPDX-License-Identifier: Apache-2.0
//! The HLE BIOS observed through our own `swi` test ROM (`testroms/swi/`): VBlank IRQs through the
//! BIOS dispatcher and SWI calls from ARM and Thumb state, on both CPUs. Each CPU records what it
//! saw in main RAM (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; the slot layout is at the top of
//! `arm9.s` and `arm7.s`). Expected values come from `docs/hw/hle-swi.md` ("SWI", "The IRQ
//! dispatcher", "Edge cases"), never from running the ROM. This is the first test ROM that takes
//! an ARM7 interrupt in the ROM instead of only latching IF.
//!
//! Slots the note marks Unverified are in tests whose names say so (`TODO(hw-unverified)`).

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_RECORD: u32 = 0x0200_4000;
const ARM7_RECORD: u32 = 0x0200_5000;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// Expected record: (slot within the record, name, expected value).
type Row = (u32, &'static str, u32);

/// A machine that booted the `swi` ROM and ran four frames: three VBlank IRQs per CPU take three
/// frames, and the rest finishes inside the fourth.
fn boot() -> Machine {
    let rom = load_rom_file(&testrom_path("swi")).expect("swi ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("swi ROM has a valid header");
    (0..4).for_each(|_| machine.run_frame());
    machine
}

/// Mismatches between `rows` and the record that starts at `base`.
fn mismatches(machine: &Machine, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    rows.iter()
        .map(|&(slot, name, expected)| (slot, name, expected, machine.arm9_peek32(base + slot * 4)))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

fn arm9_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM9_RECORD, rows)
}

fn arm7_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM7_RECORD, rows)
}

// ------------------------------------------------------------------ the ROM finished

const DONE: &[Row] = &[(30, "progress", 1), (31, "sentinel", 0x600D_F00D)];

#[test]
fn given_swi_rom_when_run_then_the_arm9_reaches_its_end() {
    assert_eq!(arm9_mismatches(DONE), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_reaches_its_end() {
    assert_eq!(arm7_mismatches(DONE), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ the IRQ dispatcher

/// "The dispatcher saves R0-R3, R12 and LR and restores them after the handler" (`hle-swi.md`);
/// the handler took the three VBlank IRQs, in IRQ mode with I set (CPSR low byte 92h).
const DISPATCH_COMMON: &[Row] = &[
    (0, "IRQs the user handler took", 3),
    (1, "r0 across the IRQs", 0x11),
    (2, "r1 across the IRQs", 0x22),
    (3, "r2 across the IRQs", 0x33),
    (4, "r3 across the IRQs", 0x44),
    (5, "r12 across the IRQs", 0x55),
    (6, "CPSR low byte inside the user handler", 0x92),
];

#[test]
fn given_swi_rom_when_run_then_the_arm9_handler_takes_three_irqs_and_the_dispatcher_keeps_the_registers()
 {
    assert_eq!(arm9_mismatches(DISPATCH_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_handler_takes_three_irqs_and_the_dispatcher_keeps_the_registers()
 {
    assert_eq!(arm7_mismatches(DISPATCH_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_sp_irq_is_back_where_each_cpu_set_it() {
    assert_eq!(
        arm9_mismatches(&[(8, "ARM9 SP_irq after the handlers", 0x027C_3F00)]),
        Vec::<Mismatch>::new()
    );
    assert_eq!(
        arm7_mismatches(&[(8, "ARM7 SP_irq after the handlers", 0x0380_FF80)]),
        Vec::<Mismatch>::new()
    );
}

// TODO(hw-unverified): R0 at the user handler is the GBA BIOS's value carried over
// (`hle-swi.md`, "Unverified").
const HANDLER_R0: &[Row] = &[(7, "r0 at the user handler's entry", 0x0400_0000)];

#[test]
fn given_swi_rom_when_run_then_r0_at_the_arm9_handler_is_the_io_base_as_the_note_models_unverified()
{
    assert_eq!(arm9_mismatches(HANDLER_R0), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_r0_at_the_arm7_handler_is_the_io_base_as_the_note_models_unverified()
{
    assert_eq!(arm7_mismatches(HANDLER_R0), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ SWI

/// "The return restores the CPSR (flags, T) and continues at the next instruction"
/// (`hle-swi.md`, "SWI"); the number 1Eh is not implemented and has no effect.
const SWI_COMMON: &[Row] = &[
    (10, "ARM SWI returned to the next instruction", 1),
    (11, "Thumb SWI returned to the next Thumb instruction", 1),
    (12, "CPSR flags nibble after the ARM SWI", 0xA),
];

#[test]
fn given_swi_rom_when_run_then_the_arm9_returns_from_arm_and_thumb_swis_with_the_flags_kept() {
    assert_eq!(arm9_mismatches(SWI_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_returns_from_arm_and_thumb_swis_with_the_flags_kept() {
    assert_eq!(arm7_mismatches(SWI_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_debugger_record_names_the_unimplemented_number_on_both_cpus() {
    let machine = boot();
    assert_eq!(
        (
            machine.swi_record9().last_unimplemented(),
            machine.swi_record7().last_unimplemented()
        ),
        (Some(0x1E), Some(0x1E))
    );
}
