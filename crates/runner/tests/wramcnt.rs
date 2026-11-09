// SPDX-License-Identifier: Apache-2.0
//! Shared WRAM and WRAMCNT, observed through our own `wramcnt` test ROM (`testroms/wramcnt/`).
//! The ARM9 sets WRAMCNT to 0, 1, 2 and 3 in turn; each CPU records what it reads in main RAM
//! (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; mode m owns slots 8m.., layouts at the top of
//! `testroms/wramcnt/arm9.s` and `arm7.s`). The expected values come from `docs/hw/wramcnt.md`
//! ("Registers", "Behavior", "Edge cases"), never from running the ROM.
//!
//! Shared WRAM is 32 KiB; the "first" half is bytes 0..0x3FFF and the "second" 0x4000..0x7FFF.
//! The markers: the ARM9 writes `A9m` at 0x0300_0000 then `B9m` at 0x0300_4000 (`m` is the
//! mode, in the low byte); the ARM7 writes `C7m` at 0x0300_0000 then `D7m` at 0x0380_0010.
//! What is in memory when a mode starts follows from the earlier modes, because the bytes stay
//! where they are across a mode change:
//!
//! - Mode 0 (ARM9 all, ARM7 none): the ARM9 writes shared bytes 0 and 0x4000. The ARM7 sees its
//!   own 64 KiB WRAM at 0x0300_0000, 0x0300_4000 and 0x037F_8000 (all empty) and 0x0380_0000.
//!   Its writes go to that WRAM: offset 0 holds `C7_0`, offset 0x10 holds `D7_0`.
//! - Mode 1 (ARM9 second, ARM7 first): the ARM9's two writes land on the same byte (0x4000), so
//!   both read `B91`. The ARM7 reads shared byte 0, `A90`, at all three addresses (the half
//!   repeats every 16 KiB) and its own WRAM at 0x0380_0000 (`C70`). Its write at 0x0300_0000
//!   lands in shared byte 0.
//! - Mode 2 (ARM9 first, ARM7 second): the ARM9's writes land on byte 0, so the last, `B92`,
//!   wins. The ARM7 reads byte 0x4000, `B91`, at all three addresses.
//! - Mode 3 (ARM7 all): the ARM9's writes are dropped and its reads give 0 (an empty window).
//!   The ARM7 reads byte 0 (`B92`) at 0x0300_0000 and 0x037F_8000 and byte 0x4000 (`C72`) at
//!   0x0300_4000.
//! - Back to mode 0: the ARM9 reads byte 0 (`C73`, written by the ARM7 in mode 3) and byte
//!   0x4000 (`C72`, written in mode 2).
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

/// A machine that booted the `wramcnt` ROM and ran two frames (both CPUs finish well inside the
/// first; the second only adds spinning).
fn boot() -> Machine {
    let rom = load_rom_file(&testrom_path("wramcnt")).expect("wramcnt ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("wramcnt ROM has a valid header");
    machine.run_frame();
    machine.run_frame();
    machine
}

/// Mismatches between `rows` and the record that starts at `base`.
fn mismatches(machine: &Machine, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    rows.iter()
        .map(|&(slot, name, expected)| (slot, name, expected, machine.arm9_peek32(base + slot * 4)))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

/// Mismatches in the ARM9 record.
fn arm9_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM9_RECORD, rows)
}

/// Mismatches in the ARM7 record.
fn arm7_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM7_RECORD, rows)
}

// ------------------------------------------------------------------ the ARM9's records

const ARM9_MODE_0: &[Row] = &[
    (0, "mode 0: 0300_0000", 0xA900_0000),
    (1, "mode 0: 0300_4000", 0xB900_0000),
    // 037F_8000h repeats the 32 KiB: it is byte 0 again.
    (2, "mode 0: 037F_8000", 0xA900_0000),
    (3, "mode 0: 0300_0000 after the ARM7", 0xA900_0000),
    (4, "mode 0: 0300_4000 after the ARM7", 0xB900_0000),
];

const ARM9_MODE_1: &[Row] = &[
    (8, "mode 1: 0300_0000", 0xB900_0001),
    (9, "mode 1: 0300_4000", 0xB900_0001),
    (10, "mode 1: 037F_8000", 0xB900_0001),
    (11, "mode 1: 0300_0000 after the ARM7", 0xB900_0001),
    (12, "mode 1: 0300_4000 after the ARM7", 0xB900_0001),
];

const ARM9_MODE_2: &[Row] = &[
    (16, "mode 2: 0300_0000", 0xB900_0002),
    (17, "mode 2: 0300_4000", 0xB900_0002),
    (18, "mode 2: 037F_8000", 0xB900_0002),
    (19, "mode 2: 0300_0000 after the ARM7", 0xB900_0002),
    (20, "mode 2: 0300_4000 after the ARM7", 0xB900_0002),
];

// TODO(hw-unverified): an empty ARM9 window reads 0 and ignores writes; GBATEK says "undefined
// data" (`wramcnt.md`, "Unverified").
const ARM9_MODE_3: &[Row] = &[
    (24, "mode 3: 0300_0000", 0),
    (25, "mode 3: 0300_4000", 0),
    (26, "mode 3: 037F_8000", 0),
    (27, "mode 3: 0300_0000 after the ARM7", 0),
    (28, "mode 3: 0300_4000 after the ARM7", 0),
];

// TODO(hw-unverified): WRAMCNT reads back to the ARM9 (`wramcnt.md`, "Unverified"). The ROM
// writes 0xFC | m, so the bits 2-7 that are dropped show as a read of m.
const ARM9_WRAMCNT_READBACK: &[Row] = &[
    (5, "WRAMCNT after writing 0xFC", 0),
    (13, "WRAMCNT after writing 0xFD", 1),
    (21, "WRAMCNT after writing 0xFE", 2),
    (29, "WRAMCNT after writing 0xFF", 3),
];

const ARM9_BACK_TO_MODE_0: &[Row] = &[
    (32, "mode 0 again: 0300_0000", 0xC700_0003),
    (33, "mode 0 again: 0300_4000", 0xC700_0002),
];

#[test]
fn given_wramcnt_rom_when_run_then_mode_0_gives_the_arm9_all_32_kib_repeating_every_32_kib() {
    assert_eq!(arm9_mismatches(ARM9_MODE_0), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_mode_1_gives_the_arm9_one_16_kib_half_repeating_every_16_kib() {
    assert_eq!(arm9_mismatches(ARM9_MODE_1), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_mode_2_gives_the_arm9_one_16_kib_half_repeating_every_16_kib() {
    assert_eq!(arm9_mismatches(ARM9_MODE_2), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_mode_3_leaves_the_arm9_window_empty_as_the_note_models_unverified()
 {
    assert_eq!(arm9_mismatches(ARM9_MODE_3), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_the_arm9_reads_wramcnt_back_without_bits_2_to_7_as_the_note_models_unverified()
 {
    assert_eq!(
        arm9_mismatches(ARM9_WRAMCNT_READBACK),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_wramcnt_rom_when_run_then_the_arm9_gets_back_the_bytes_the_arm7_wrote_after_returning_to_mode_0()
 {
    assert_eq!(arm9_mismatches(ARM9_BACK_TO_MODE_0), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ the ARM7's records

const ARM7_MODE_0: &[Row] = &[
    // The ARM7 owns none of the shared WRAM: 0300_0000h shows its own WRAM, which is empty.
    (0, "mode 0: 0300_0000", 0),
    (1, "mode 0: 0300_4000", 0),
    (2, "mode 0: 037F_8000", 0),
    (3, "mode 0: 0380_0000", 0),
    (5, "mode 0: 0300_0000 after its own write", 0xC700_0000),
    (6, "mode 0: 0380_0010 after its own write", 0xD700_0000),
];

const ARM7_MODE_1: &[Row] = &[
    // The ARM7 has the first half; byte 0 is the ARM9's mode 0 marker, and the half repeats
    // every 16 KiB. 0380_0000h is the ARM7's own WRAM, which holds its mode 0 marker.
    (8, "mode 1: 0300_0000", 0xA900_0000),
    (9, "mode 1: 0300_4000", 0xA900_0000),
    (10, "mode 1: 037F_8000", 0xA900_0000),
    (11, "mode 1: 0380_0000", 0xC700_0000),
    (13, "mode 1: 0300_0000 after its own write", 0xC700_0001),
    (14, "mode 1: 0380_0010 after its own write", 0xD700_0001),
];

const ARM7_MODE_2: &[Row] = &[
    // The ARM7 has the second half; byte 0x4000 is the ARM9's mode 1 marker.
    (16, "mode 2: 0300_0000", 0xB900_0001),
    (17, "mode 2: 0300_4000", 0xB900_0001),
    (18, "mode 2: 037F_8000", 0xB900_0001),
    (19, "mode 2: 0380_0000", 0xC700_0000),
    (21, "mode 2: 0300_0000 after its own write", 0xC700_0002),
    (22, "mode 2: 0380_0010 after its own write", 0xD700_0002),
];

const ARM7_MODE_3: &[Row] = &[
    // The ARM7 has all 32 KiB: byte 0 is the ARM9's mode 2 marker, byte 0x4000 the ARM7's own
    // mode 2 write; 037F_8000h repeats byte 0.
    (24, "mode 3: 0300_0000", 0xB900_0002),
    (25, "mode 3: 0300_4000", 0xC700_0002),
    (26, "mode 3: 037F_8000", 0xB900_0002),
    (27, "mode 3: 0380_0000", 0xC700_0000),
    (29, "mode 3: 0300_0000 after its own write", 0xC700_0003),
    (30, "mode 3: 0380_0010 after its own write", 0xD700_0003),
];

// TODO(hw-unverified): WRAMSTAT is the stored 2-bit mode (`wramcnt.md`, "Unverified").
const ARM7_WRAMSTAT: &[Row] = &[
    (4, "WRAMSTAT in mode 0", 0),
    (12, "WRAMSTAT in mode 1", 1),
    (20, "WRAMSTAT in mode 2", 2),
    (28, "WRAMSTAT in mode 3", 3),
];

#[test]
fn given_wramcnt_rom_when_run_then_in_mode_0_the_arm7_sees_its_own_wram_in_place_of_shared_wram() {
    assert_eq!(arm7_mismatches(ARM7_MODE_0), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_in_mode_1_the_arm7_reads_the_first_half_repeating_every_16_kib()
{
    assert_eq!(arm7_mismatches(ARM7_MODE_1), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_in_mode_2_the_arm7_reads_the_second_half_repeating_every_16_kib()
{
    assert_eq!(arm7_mismatches(ARM7_MODE_2), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_in_mode_3_the_arm7_reads_all_32_kib_and_037f_8000_mirrors_the_start()
 {
    assert_eq!(arm7_mismatches(ARM7_MODE_3), Vec::<Mismatch>::new());
}

#[test]
fn given_wramcnt_rom_when_run_then_the_arm7_reads_the_mode_through_wramstat_as_the_note_models_unverified()
 {
    assert_eq!(arm7_mismatches(ARM7_WRAMSTAT), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ progress and determinism

const ARM9_DONE: &[Row] = &[(34, "modes finished", 4), (35, "sentinel", 0x5752_4D21)];
const ARM7_DONE: &[Row] = &[(32, "modes finished", 4), (33, "sentinel", 0x5752_4D37)];

#[test]
fn given_wramcnt_rom_when_run_two_frames_then_both_cpus_finished_all_four_modes() {
    let machine = boot();
    assert_eq!(
        (
            mismatches(&machine, ARM9_RECORD, ARM9_DONE),
            mismatches(&machine, ARM7_RECORD, ARM7_DONE)
        ),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

#[test]
fn given_wramcnt_rom_when_run_twice_then_the_final_savestate_bytes_match() {
    ninge_runner::golden::assert_deterministic("wramcnt", 2);
}

#[test]
fn given_wramcnt_rom_when_resumed_from_a_savestate_after_1_frame_then_it_matches_an_uninterrupted_run()
 {
    ninge_runner::golden::assert_savestate_round_trip("wramcnt", 1, 2);
}
