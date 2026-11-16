// SPDX-License-Identifier: Apache-2.0
//! Which bank backs each extended-palette slot (`docs/hw/extended-palettes.md`, "Where the palette
//! is" and "Which bank backs a slot"). Expected slot starts are the note's tables written out: the
//! banks sit one after another in the VRAM buffer, A first (E at 0x80000, F at 0x90000, G at
//! 0x94000, H at 0x98000, I at 0xA0000), and a slot is 8 KiB.

use super::vram::set;
use super::*;
use crate::bus::memory::vram::{ExtSlot, ExtSlots};

/// The VRAMCNT register address of banks A to I.
const VRAMCNT: [u32; 9] = [
    0x0400_0240,
    0x0400_0241,
    0x0400_0242,
    0x0400_0243,
    0x0400_0244,
    0x0400_0245,
    0x0400_0246,
    0x0400_0248,
    0x0400_0249,
];
const E: usize = 4;
const F: usize = 5;
const G: usize = 6;
const H: usize = 7;
const I: usize = 8;
const E_AT: u32 = 0x8_0000;
const F_AT: u32 = 0x9_0000;
const G_AT: u32 = 0x9_4000;
const H_AT: u32 = 0x9_8000;
const I_AT: u32 = 0xA_0000;
const SLOT: u32 = 0x2000;

/// A VRAMCNT value: enabled, with `mst` and `ofs`.
fn cnt(mst: u8, ofs: u8) -> u8 {
    0x80 | mst | ofs << 3
}

/// The slots after writing each `(bank, VRAMCNT)`.
fn resolved(writes: &[(usize, u8)]) -> (ExtSlots, Memory) {
    let mut memory = Memory::new();
    writes
        .iter()
        .for_each(|&(bank, value)| set(&mut memory, VRAMCNT[bank], value));
    (memory.ext_slots(), memory)
}

fn slot(bank: usize, start: u32) -> Option<ExtSlot> {
    Some(ExtSlot { bank, start })
}

const NONE: Option<ExtSlot> = None;

#[test]
fn given_power_on_banks_when_slots_are_resolved_then_every_slot_is_empty() {
    assert_eq!(resolved(&[]).0, ExtSlots::default());
}

#[test]
fn given_bank_e_with_mst_4_when_slots_are_resolved_then_it_backs_the_four_slots_of_engine_a_bg() {
    let expected = [
        slot(E, E_AT),
        slot(E, E_AT + SLOT),
        slot(E, E_AT + 2 * SLOT),
        slot(E, E_AT + 3 * SLOT),
    ];
    assert_eq!(resolved(&[(E, cnt(4, 0))]).0.bg_a, expected);
}

#[test]
fn given_bank_f_with_mst_4_and_ofs_0_when_slots_are_resolved_then_it_backs_slots_0_and_1() {
    let expected = [slot(F, F_AT), slot(F, F_AT + SLOT), NONE, NONE];
    assert_eq!(resolved(&[(F, cnt(4, 0))]).0.bg_a, expected);
}

#[test]
fn given_bank_g_with_mst_4_and_ofs_1_when_slots_are_resolved_then_it_backs_slots_2_and_3() {
    let expected = [NONE, NONE, slot(G, G_AT), slot(G, G_AT + SLOT)];
    assert_eq!(resolved(&[(G, cnt(4, 1))]).0.bg_a, expected);
}

#[test]
fn given_f_at_ofs_0_and_g_at_ofs_1_when_slots_are_resolved_then_each_backs_its_pair() {
    let expected = [
        slot(F, F_AT),
        slot(F, F_AT + SLOT),
        slot(G, G_AT),
        slot(G, G_AT + SLOT),
    ];
    assert_eq!(resolved(&[(F, cnt(4, 0)), (G, cnt(4, 1))]).0.bg_a, expected);
}

#[test]
fn given_f_and_g_on_the_same_pair_when_slots_are_resolved_then_the_earlier_letter_wins_unverified()
{
    // TODO(hw-unverified): extended-palettes.md "Unverified", two banks on one slot.
    let expected = [slot(F, F_AT), slot(F, F_AT + SLOT), NONE, NONE];
    assert_eq!(resolved(&[(F, cnt(4, 0)), (G, cnt(4, 0))]).0.bg_a, expected);
}

#[test]
fn given_e_and_f_on_the_same_slots_when_slots_are_resolved_then_e_wins_and_f_adds_nothing_unverified()
 {
    // TODO(hw-unverified): the earlier letter wins, E backs all four slots.
    let expected = [
        slot(E, E_AT),
        slot(E, E_AT + SLOT),
        slot(E, E_AT + 2 * SLOT),
        slot(E, E_AT + 3 * SLOT),
    ];
    assert_eq!(resolved(&[(E, cnt(4, 0)), (F, cnt(4, 0))]).0.bg_a, expected);
}

#[test]
fn given_banks_f_and_g_with_mst_5_when_slots_are_resolved_then_the_obj_slot_of_engine_a_is_the_first_8_kib_of_f()
 {
    // OFS does not count for MST 5; F is the earlier letter.
    let (slots, _) = resolved(&[(F, cnt(5, 0)), (G, cnt(5, 3))]);
    assert_eq!((slots.obj_a, slots.bg_a), (slot(F, F_AT), [NONE; 4]));
}

#[test]
fn given_bank_g_alone_with_mst_5_when_slots_are_resolved_then_it_backs_the_obj_slot() {
    assert_eq!(resolved(&[(G, cnt(5, 0))]).0.obj_a, slot(G, G_AT));
}

#[test]
fn given_bank_h_with_mst_2_when_slots_are_resolved_then_it_backs_the_four_slots_of_engine_b_bg() {
    let expected = [
        slot(H, H_AT),
        slot(H, H_AT + SLOT),
        slot(H, H_AT + 2 * SLOT),
        slot(H, H_AT + 3 * SLOT),
    ];
    let (slots, _) = resolved(&[(H, cnt(2, 0))]);
    assert_eq!((slots.bg_b, slots.bg_a), (expected, [NONE; 4]));
}

#[test]
fn given_bank_i_with_mst_3_when_slots_are_resolved_then_it_backs_the_obj_slot_of_engine_b() {
    let (slots, _) = resolved(&[(I, cnt(3, 0))]);
    assert_eq!((slots.obj_b, slots.obj_a), (slot(I, I_AT), NONE));
}

#[test]
fn given_banks_in_other_mappings_when_slots_are_resolved_then_none_backs_a_slot() {
    // E as texture palette (3), F as engine A BG VRAM (1), G in LCDC mode (0), H as engine B BG
    // VRAM (1) and I disabled with an extended-palette MST.
    let (slots, _) = resolved(&[
        (E, cnt(3, 0)),
        (F, cnt(1, 0)),
        (G, cnt(0, 0)),
        (H, cnt(1, 0)),
        (I, 3),
    ]);
    assert_eq!(slots, ExtSlots::default());
}

#[test]
fn given_a_bank_moved_out_of_its_mapping_when_slots_are_resolved_then_its_slots_are_empty_and_its_bytes_stay()
 {
    let mut memory = Memory::new();
    set(&mut memory, VRAMCNT[H], cnt(0, 0));
    memory.vram[H_AT as usize] = 0xAB;
    set(&mut memory, VRAMCNT[H], cnt(2, 0));
    let backed = memory.ext_slots().bg_b[0];
    set(&mut memory, VRAMCNT[H], cnt(0, 0));
    assert_eq!(
        (
            backed,
            memory.ext_slots().bg_b[0],
            memory.vram[H_AT as usize]
        ),
        (slot(H, H_AT), NONE, 0xAB)
    );
}

#[test]
fn given_extended_palettes_in_memory_when_a_colour_is_read_then_it_is_at_slot_palette_and_index() {
    // Slot 1, palette 3, index 200 of engine B's BG: byte 8192 + 3 * 512 + 200 * 2 of bank H.
    let (_, mut memory) = resolved(&[(H, cnt(2, 0))]);
    let at = (H_AT + SLOT + 3 * 512 + 400) as usize;
    memory.vram[at] = 0x34;
    memory.vram[at + 1] = 0x12;
    let palette = memory.ext_bg_palette(true);
    assert_eq!(
        (
            palette.color(1, 3, 200),
            palette.color(1, 3, 201),
            palette.color(0, 3, 200),
            memory.ext_bg_palette(false).color(1, 3, 200)
        ),
        (0x1234, 0, 0, 0)
    );
}

#[test]
fn given_a_slot_no_bank_backs_when_a_colour_is_read_then_it_is_0_as_the_note_models_unverified() {
    // TODO(hw-unverified): extended-palettes.md "Unverified", an empty slot.
    let (_, memory) = resolved(&[(F, cnt(4, 0))]);
    let palette = memory.ext_bg_palette(false);
    assert_eq!((palette.color(2, 0, 0), palette.color(9, 0, 0)), (0, 0));
}

#[test]
fn given_the_slots_when_the_banks_are_reported_then_each_engine_lists_its_own() {
    let (_, memory) = resolved(&[
        (F, cnt(4, 0)),
        (G, cnt(4, 1)),
        (H, cnt(2, 0)),
        (I, cnt(3, 0)),
    ]);
    let (a, b) = (
        memory.ext_palette_banks(false),
        memory.ext_palette_banks(true),
    );
    assert_eq!(
        (a.bg, a.obj, b.bg, b.obj),
        (
            [Some(F), Some(F), Some(G), Some(G)],
            None,
            [Some(H); 4],
            Some(I)
        )
    );
}
