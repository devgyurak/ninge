// SPDX-License-Identifier: Apache-2.0
//! The nine VRAM banks in LCDC mode, observed through our own `vrambanks` test ROM
//! (`testroms/vrambanks/`). The ROM records what it read at 0x0200_4000 (one word per slot, the
//! layout is at the top of `arm9.s`); the expected values are computed here from
//! `docs/hw/vram-banks.md` ("The banks", "Behavior", "Edge cases"), never from running the ROM.
//!
//! Slots the note marks Unverified are in tests whose names say so (`TODO(hw-unverified)`).

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const RECORD: u32 = 0x0200_4000;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// The marker the ROM stores at a bank's first word, and at its last word. Both are our own
/// test values, listed in the ROM's header comment.
fn first_marker(bank: u32) -> u32 {
    0x5600_0000 | bank
}
fn last_marker(bank: u32) -> u32 {
    0xE000_0000 | bank
}

/// A machine that booted the `vrambanks` ROM and ran one frame.
fn boot_one_frame() -> Machine {
    let rom = load_rom_file(&testrom_path("vrambanks")).expect("vrambanks ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("vrambanks ROM has a valid header");
    machine.run_frame();
    machine
}

/// The expected record: (slot, name, value).
fn expected_table() -> Vec<(u32, &'static str, u32)> {
    let banks = 0..9u32;
    let mut table: Vec<(u32, &'static str, u32)> = Vec::new();
    table.extend(banks.clone().map(|b| (b, "first word", first_marker(b))));
    table.extend(banks.clone().map(|b| (9 + b, "last word", last_marker(b))));
    // The bytes around a window are another bank's, which is off, or nothing: zero.
    table.extend(banks.clone().map(|b| (18 + b, "word below the window", 0)));
    table.extend(banks.clone().map(|b| (27 + b, "word above the window", 0)));
    // A byte store is ignored, so the marker stays.
    table.extend(
        banks
            .clone()
            .map(|b| (36 + b, "byte store ignored", first_marker(b))),
    );
    table.push((45, "all banks off reads zero", 0));
    table.push((46, "contents survive off and on", first_marker(0)));
    table.push((47, "VRAMCNT reads zero", 0));
    table.push((48, "progress", 4));
    table.push((49, "sentinel", 0x5642_5421));
    table
}

/// Mismatches among the slots `wanted` accepts.
fn mismatches(machine: &Machine, wanted: impl Fn(u32) -> bool) -> Vec<Mismatch> {
    expected_table()
        .into_iter()
        .filter(|&(slot, _, _)| wanted(slot))
        .map(|(slot, name, expected)| {
            (
                slot,
                name,
                expected,
                machine.inspect().arm9_peek32(RECORD + slot * 4),
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

fn mismatches_in(first: u32, last: u32) -> Vec<Mismatch> {
    mismatches(&boot_one_frame(), |slot| (first..=last).contains(&slot))
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_every_bank_holds_its_own_first_word() {
    assert_eq!(mismatches_in(0, 8), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_every_bank_holds_its_own_last_word() {
    assert_eq!(mismatches_in(9, 17), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_a_bank_alone_does_not_map_the_word_below_its_window()
{
    assert_eq!(mismatches_in(18, 26), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_a_bank_alone_does_not_map_the_word_above_its_window()
{
    assert_eq!(mismatches_in(27, 35), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_a_byte_store_to_any_bank_is_ignored() {
    assert_eq!(mismatches_in(36, 44), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_a_disabled_bank_reads_zero_and_keeps_its_bytes_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): vram-banks.md "Unverified", a disabled bank reads zero.
    assert_eq!(mismatches_in(45, 46), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_vramcnt_reads_zero_as_the_note_models_unverified() {
    // TODO(hw-unverified): vram-banks.md "Unverified", the VRAMCNT registers read 0.
    assert_eq!(mismatches_in(47, 47), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_all_four_groups_finished_and_the_sentinel_was_written_last()
 {
    assert_eq!(mismatches_in(48, 49), Vec::<Mismatch>::new());
}
