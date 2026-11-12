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
/// The ARM7's record; the tables below number its slots from `ARM7_SLOT`.
const ARM7_RECORD: u32 = 0x0200_5000;
const ARM7_SLOT: u32 = 1000;

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

/// Where a slot lives: the ARM9's record, or the ARM7's for slots from `ARM7_SLOT`.
fn slot_address(slot: u32) -> u32 {
    match slot.checked_sub(ARM7_SLOT) {
        Some(arm7) => ARM7_RECORD + arm7 * 4,
        None => RECORD + slot * 4,
    }
}

/// A machine that booted the `vrambanks` ROM and ran the frames both CPUs need to finish their
/// hand-shaken steps.
fn boot_one_frame() -> Machine {
    let rom = load_rom_file(&testrom_path("vrambanks")).expect("vrambanks ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("vrambanks ROM has a valid header");
    machine.run_frame();
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
    table.extend(arm7_table());
    table.push((48, "progress", 10));
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
        .chain(arm9_blind_table())
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

/// Group 10, the ARM9's side: what it reads while C and D are the ARM7's, and what the ARM7
/// stored once bank C is back in LCDC mode.
fn arm9_blind_table() -> Vec<(u32, &'static str, u32)> {
    let mut table: Vec<(u32, &'static str, u32)> = (500u32..524)
        .map(|slot| (slot, "the ARM9 sees nothing of the ARM7's banks", 0))
        .collect();
    // Step 5: C is LCDC (its marker at 0x06840000) and D is engine A BG VRAM (at 0x06000000).
    table[12] = (512, "D in BG A", first_marker(3));
    table[13] = (513, "C in LCDC", first_marker(2));
    table.push((530, "the ARM7's word and byte stores", 0x1122_33EE));
    table.push((531, "the ARM7's halfword store", 0x0000_BEEF));
    table
}

/// The ARM7's record: `(step, slot offset in the step, name, value)` from the note's
/// "VRAMSTAT (ARM7)" and "Where each bank goes", C = marker 2, D = marker 3.
fn arm7_table() -> Vec<(u32, &'static str, u32)> {
    let c = (first_marker(2), last_marker(2));
    let d = (first_marker(3), last_marker(3));
    let steps: [(u32, &'static str, Vec<u32>); 8] = [
        // MST 2, OFS 0: C at 0x06000000; ARM7 stores land, a byte store included.
        (
            0,
            "C at OFS 0",
            vec![1, c.0, c.1, 0, 0x1122_3344, 0x1122_33EE, 0xBEEF],
        ),
        // OFS 1: the slot moves, the stored word moves with it.
        (8, "C at OFS 1", vec![1, c.0, c.1, 0, 0x1122_33EE]),
        // C and D at the same slot: C wins.
        (16, "C and D at OFS 0", vec![3, c.0, c.1]),
        // C and D side by side.
        (24, "C OFS 0, D OFS 1", vec![3, c.0, d.0, d.1]),
        // C in LCDC and D in BG A: nothing is the ARM7's.
        (32, "C LCDC, D BG A", vec![0, 0, 0]),
        // MST 2 while disabled.
        (40, "disabled", vec![0, 0]),
        // OFS.1 is not used: OFS 3 acts as OFS 1, OFS 2 as OFS 0.
        (48, "OFS 3", vec![1, c.0, 0]),
        (56, "OFS 2", vec![1, c.0, 0]),
    ];
    steps
        .into_iter()
        .flat_map(|(base, name, values)| {
            values
                .into_iter()
                .zip(0u32..)
                .map(move |(value, k)| (ARM7_SLOT + base + k, name, value))
        })
        .chain([
            (ARM7_SLOT + 62, "the last finished step", 8),
            (ARM7_SLOT + 63, "sentinel", 0x5637_5421),
        ])
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
                machine.inspect().arm9_peek32(slot_address(slot)),
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

#[test]
fn given_vrambanks_rom_when_run_then_the_arm7_sees_c_and_d_in_mst_2_at_the_ofs_0_slot_and_stores_bytes()
 {
    assert_eq!(
        mismatches_in(ARM7_SLOT, ARM7_SLOT + 7),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_vrambanks_rom_when_run_then_the_arm7_sees_the_bank_move_with_its_bytes_and_ofs_0_and_1_side_by_side()
 {
    assert_eq!(
        mismatches_in(ARM7_SLOT + 8, ARM7_SLOT + 31),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_vrambanks_rom_when_run_then_the_arm7_sees_nothing_of_a_bank_in_another_mode_or_disabled() {
    assert_eq!(
        mismatches_in(ARM7_SLOT + 32, ARM7_SLOT + 47),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_vrambanks_rom_when_run_then_ofs_1_is_not_used_in_the_arm7_mapping_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): vram-banks.md "Unverified", OFS.1 where it must be zero.
    assert_eq!(
        mismatches_in(ARM7_SLOT + 48, ARM7_SLOT + 63),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_vrambanks_rom_when_run_then_the_arm9_sees_none_of_the_arm7s_banks_and_reads_the_arm7s_stores_in_lcdc_mode()
 {
    assert_eq!(mismatches_in(500, 531), Vec::<Mismatch>::new());
}
