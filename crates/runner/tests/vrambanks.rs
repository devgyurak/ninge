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
    table.extend(window_table());
    table.push((48, "progress", 9));
    table.push((49, "sentinel", 0x5642_5421));
    table
}

/// Every (bank, MST, OFS) that puts a bank into a BG or OBJ window, in the order the ROM visits
/// them, with the CPU address the note gives for the window start ("Where each bank goes"):
/// engine A BG, engine A OBJ, then engine B.
fn window_entries() -> Vec<(&'static str, u32, u32)> {
    let ofs = 0..4u32;
    // F and G: 16 KiB steps by OFS.0 and 64 KiB steps by OFS.1.
    let fg = |o: u32| 0x4000 * (o & 1) + 0x1_0000 * (o >> 1);
    let mut entries = Vec::new();
    entries.extend(ofs.clone().flat_map(|bank| {
        ofs.clone().map(move |o| {
            (
                "BG A: A-D at 20000h * OFS",
                bank,
                0x0600_0000 + 0x2_0000 * o,
            )
        })
    }));
    entries.push(("BG A: E", 4, 0x0600_0000));
    entries.extend(
        (5..7).flat_map(|bank| (0..4).map(move |o| ("BG A: F, G", bank, 0x0600_0000 + fg(o)))),
    );
    // A and B in OBJ mode: OFS.1 must be zero and is not used.
    entries.extend((0..2).flat_map(|bank| {
        (0..4).map(move |o| {
            (
                "OBJ A: A, B at 20000h * OFS.0",
                bank,
                0x0640_0000 + 0x2_0000 * (o & 1),
            )
        })
    }));
    entries.push(("OBJ A: E", 4, 0x0640_0000));
    entries.extend(
        (5..7).flat_map(|bank| (0..4).map(move |o| ("OBJ A: F, G", bank, 0x0640_0000 + fg(o)))),
    );
    entries.extend([
        ("BG B: C", 2, 0x0620_0000),
        ("BG B: H", 7, 0x0620_0000),
        ("BG B: I after H's 32 KiB", 8, 0x0620_8000),
        ("OBJ B: D", 3, 0x0660_0000),
        ("OBJ B: I", 8, 0x0660_0000),
    ]);
    entries
}

/// The record of group 5: for each entry the address, the first and last word of the window (the
/// bank's own markers) and the words just outside it (nothing else is on, so zero).
fn window_table() -> Vec<(u32, &'static str, u32)> {
    window_entries()
        .into_iter()
        .zip(0u32..)
        .flat_map(|((_, bank, addr), e)| {
            let slot = 100 + 5 * e;
            [
                (slot, "window start address", addr),
                (slot + 1, "first word of the window", first_marker(bank)),
                (slot + 2, "last word of the window", last_marker(bank)),
                (slot + 3, "word below the window", 0),
                (slot + 4, "word above the window", 0),
            ]
        })
        .chain(overlap_table())
        .chain(invisible_table())
        .chain(window_byte_table())
        .chain(moved_table())
        .collect()
}

/// Group 6: what the CPU reads where banks overlap; the earlier bank in the alphabet wins
/// ("Overlap"). The cases are the ROM's `cases`, in order.
fn overlap_table() -> Vec<(u32, &'static str, u32)> {
    [
        ("A over B at the same BG offset", first_marker(0)),
        ("B alone", first_marker(1)),
        ("A over E, first word", first_marker(0)),
        ("A over E, last word of A", last_marker(0)),
        ("F over G at the same 16 KiB", first_marker(5)),
        ("F beside G (OFS 1), F", first_marker(5)),
        ("F beside G (OFS 1), G", first_marker(6)),
        ("C over H and I, start", first_marker(2)),
        // Bank C's own word 32 KiB in, never written: zero, so I (a marker) is not showing.
        ("C over I at 8000h", 0),
        ("D over I in OBJ B", first_marker(3)),
        ("I after H, I", first_marker(8)),
        ("I after H, H", first_marker(7)),
        ("E beside A (OFS 1), E", first_marker(4)),
        ("E beside A (OFS 1), A", first_marker(0)),
    ]
    .into_iter()
    .zip(340u32..)
    .map(|((name, value), slot)| (slot, name, value))
    .collect()
}

/// Group 7: a bank in a texture, extended-palette or undefined mode is in no CPU window.
fn invisible_table() -> Vec<(u32, &'static str, u32)> {
    (360u32..372)
        .map(|slot| (slot, "no CPU address holds the bank", 0))
        .collect()
}

/// Group 8: a byte store to the start of each window changes nothing.
fn window_byte_table() -> Vec<(u32, &'static str, u32)> {
    [
        first_marker(0),
        first_marker(0),
        first_marker(2),
        first_marker(3),
    ]
    .into_iter()
    .zip(380u32..)
    .map(|(value, slot)| (slot, "byte store to a window ignored", value))
    .collect()
}

/// Group 9: bank A keeps a word stored through the BG window when it moves.
fn moved_table() -> Vec<(u32, &'static str, u32)> {
    (390u32..394)
        .map(|slot| (slot, "bytes survive the move", 0xC0FF_EE01))
        .collect()
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

/// The entries `first..last` of group 5 (slots `100 + 5 * e` onward).
fn entry_mismatches(first: u32, last: u32) -> Vec<Mismatch> {
    mismatches(&boot_one_frame(), |slot| {
        (100 + 5 * first..100 + 5 * last).contains(&slot)
    })
}

#[test]
fn given_the_note_when_the_window_entries_are_counted_then_there_are_47_matching_the_rom() {
    // 25 BG A + 17 OBJ A + 3 BG B + 2 OBJ B.
    assert_eq!(window_entries().len(), 47);
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_every_mst_and_ofs_puts_a_bank_in_engine_a_bg_vram_at_the_note_address()
 {
    assert_eq!(entry_mismatches(0, 25), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_every_mst_and_ofs_puts_a_bank_in_engine_a_obj_vram_at_the_note_address()
 {
    // TODO(hw-unverified): OFS.1 of A and B in OBJ mode is ignored (vram-banks.md "Unverified").
    assert_eq!(entry_mismatches(25, 42), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_banks_c_d_h_and_i_sit_where_the_note_puts_them_in_engine_b_vram()
 {
    assert_eq!(entry_mismatches(42, 47), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_the_earlier_bank_wins_where_banks_overlap_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): vram-banks.md "Unverified", overlapping banks.
    assert_eq!(mismatches_in(340, 353), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_a_bank_in_a_texture_or_extended_palette_mode_is_at_no_cpu_address()
 {
    assert_eq!(mismatches_in(360, 371), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_a_byte_store_to_each_window_is_ignored() {
    assert_eq!(mismatches_in(380, 383), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_a_bank_keeps_its_bytes_when_it_moves_between_mappings()
 {
    assert_eq!(mismatches_in(390, 393), Vec::<Mismatch>::new());
}

#[test]
fn given_vrambanks_rom_when_run_one_frame_then_all_groups_finished_and_the_sentinel_was_written_last()
 {
    assert_eq!(mismatches_in(48, 49), Vec::<Mismatch>::new());
}
