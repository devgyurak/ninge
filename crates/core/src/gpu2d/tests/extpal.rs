// SPDX-License-Identifier: Apache-2.0
//! BG extended palettes (issue #32). Every expected value comes from
//! `docs/hw/extended-palettes.md`: with DISPCNT bit 30 set a 256-colour layer whose map entries
//! are 16 bits reads palette `entry >> 12`, index = the pixel value, from the slot of its layer
//! (BG0 and BG1 move to slots 2 and 3 with BGxCNT bit 13); everything else keeps the standard
//! palette. The extended colours are the test's own: slot `s`, palette `p`, index 7 holds
//! `1 + 16 s + p`, a 15-bit value, so each (slot, palette) pair is a different colour.

use super::*;
use crate::debug::EngineId;

const VRAMCNT_C: u32 = 0x0400_0242;
const VRAMCNT_E: u32 = 0x0400_0244;
const VRAMCNT_F: u32 = 0x0400_0245;
const VRAMCNT_G: u32 = 0x0400_0246;
const VRAMCNT_H: u32 = 0x0400_0248;
const LCDC_E: u32 = 0x0688_0000;
const LCDC_F: u32 = 0x0689_0000;
const LCDC_G: u32 = 0x0689_4000;
const LCDC_H: u32 = 0x0689_8000;
/// Bank A as engine A BG VRAM (MST 1, OFS 0) and bank C as engine B's (MST 4).
const BG_BANK_A: u8 = 0x81;
const BG_BANK_C: u8 = 0x84;
const BG_A: u32 = 0x0600_0000;
const BG_B: u32 = 0x0620_0000;
const BG_PALETTE_A: u32 = 0x0500_0000;
const BG_PALETTE_B: u32 = 0x0500_0400;
const BG0CNT_B: u32 = BG_REGS_B_ADDR;
const DISPCNT_B: u32 = DISPCNT_B_ADDR;
/// DISPCNT: display mode 1, extended palettes for the BG (bit 30).
const MODE1: u32 = 1 << 16;
const EXT_ON: u32 = 1 << 30;
const BG_ON: [u32; 4] = [1 << 8, 1 << 9, 1 << 10, 1 << 11];
/// BGxCNT: 256 colours, screen block `n` (bits 8-12), priority 0, and the slot bit 13.
const EIGHT_BPP: u16 = 1 << 7;
const SLOT_BIT: u16 = 1 << 13;
/// The backdrop (standard palette entry 0) and the standard colour of palette entry 7.
const BACKDROP: u16 = 0x7C00;
const STANDARD_7: u16 = 0x001F;

/// The code of extended colour (`slot`, `palette`): `1 + 16 slot + palette`.
fn code(slot: u32, palette: u32) -> u16 {
    (1 + 16 * slot + palette) as u16
}

/// The xRGB pixel a 15-bit colour gives (`display-mode-vram.md`, "Colour expansion").
fn xrgb(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// Fills `slots` 8 KiB slots of the bank at LCDC address `lcdc` (it must be in LCDC mode) with the
/// test colours of slots `first..`, then writes `value` to its VRAMCNT.
fn ext_bank(m: &mut Machine, vramcnt: u32, lcdc: u32, (first, slots): (u32, u32), value: u8) {
    w8(m, vramcnt, 0x80);
    (0..slots).for_each(|s| {
        (0..16).for_each(|p| w16(m, lcdc + s * 0x2000 + p * 512 + 14, code(first + s, p)));
    });
    w8(m, vramcnt, value);
}

/// Writes `count` words of `value` from `at`.
fn fill_words(m: &mut Machine, at: u32, count: u32, value: u32) {
    (0..count).for_each(|n| w32(m, at + n * 4, value));
}

/// Tile 1 of 8 bits per pixel at byte 64 of the BG VRAM at `base`: every pixel 7.
fn solid_tile(m: &mut Machine, base: u32) {
    fill_words(m, base + 64, 16, 0x0707_0707);
}

/// Row 0 of 32 cells of the 16-bit map at byte `map` of the BG VRAM at `base`: tile 1 with palette
/// number `cell mod 16`.
fn palette_row(m: &mut Machine, base: u32, map: u32) {
    (0..16u32).for_each(|n| {
        let entry = |cell: u32| u32::from(1 | ((cell % 16) << 12) as u16);
        w32(m, base + map + n * 4, entry(2 * n) | entry(2 * n + 1) << 16);
    });
}

/// Engine A scene: bank A as BG VRAM, tile 1 solid 7, the 16-bit map of layer `layer`'s screen
/// block `block` holds a row of palette numbers `cell mod 16`, standard palette entry 7 red, the
/// backdrop blue, `bgcnt` for the layer, and DISPCNT `MODE1 | extra | layer on`.
fn scene_a(layer: usize, bgcnt: u16, extra: u32) -> Machine {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w8(&mut m, VRAMCNT_A, BG_BANK_A);
    w16(&mut m, BG_PALETTE_A, BACKDROP);
    w16(&mut m, BG_PALETTE_A + 14, STANDARD_7);
    solid_tile(&mut m, BG_A);
    palette_row(&mut m, BG_A, 0x800 * (layer as u32 + 1));
    w16(&mut m, BG_REGS_ADDR + 2 * layer as u32, bgcnt);
    w32(&mut m, DISPCNT_A_ADDR, MODE1 | extra | BG_ON[layer]);
    m
}

/// The colour of the first line at the middle of cell `cell` on the top screen after a frame.
fn cell_color(m: &mut Machine, cell: usize) -> u32 {
    m.render_frame();
    pixel(m, Screen::Top, cell * 8 + 3, 3)
}

/// The BGxCNT of a 256-colour text layer with its map in screen block `layer + 1`.
fn text_cnt(layer: usize, extra: u16) -> u16 {
    EIGHT_BPP | (layer as u16 + 1) << 8 | extra
}

/// The colours of cells 0, 5 and 15 of the layer on the top screen.
fn sample(m: &mut Machine) -> [u32; 3] {
    [0, 5, 15].map(|cell| cell_color(m, cell))
}

fn ext(slot: u32, palettes: [u32; 3]) -> [u32; 3] {
    palettes.map(|p| xrgb(code(slot, p)))
}

#[test]
fn given_a_256_colour_text_layer_with_bit_30_when_drawn_then_each_cell_reads_its_palette_number_from_slot_0()
 {
    let mut m = scene_a(0, text_cnt(0, 0), EXT_ON);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    // Cell c has palette number c mod 16.
    assert_eq!(sample(&mut m), ext(0, [0, 5, 15]));
}

#[test]
fn given_a_256_colour_text_layer_with_bit_30_clear_when_drawn_then_it_reads_the_standard_palette() {
    let mut m = scene_a(0, text_cnt(0, 0), 0);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(sample(&mut m), [xrgb(STANDARD_7); 3]);
}

#[test]
fn given_bg0_with_bit_13_when_drawn_then_it_reads_slot_2_and_without_it_slot_0() {
    let mut own = scene_a(0, text_cnt(0, 0), EXT_ON);
    ext_bank(&mut own, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    let mut moved = scene_a(0, text_cnt(0, SLOT_BIT), EXT_ON);
    ext_bank(&mut moved, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(
        (sample(&mut own), sample(&mut moved)),
        (ext(0, [0, 5, 15]), ext(2, [0, 5, 15]))
    );
}

#[test]
fn given_bg1_when_drawn_then_it_reads_slot_1_and_with_bit_13_slot_3() {
    let mut own = scene_a(1, text_cnt(1, 0), EXT_ON);
    ext_bank(&mut own, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    let mut moved = scene_a(1, text_cnt(1, SLOT_BIT), EXT_ON);
    ext_bank(&mut moved, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(
        (sample(&mut own), sample(&mut moved)),
        (ext(1, [0, 5, 15]), ext(3, [0, 5, 15]))
    );
}

#[test]
fn given_bg2_and_bg3_as_text_layers_when_drawn_then_they_read_their_own_slot_whatever_bit_13_says()
{
    let layer = |n: usize, bit: u16| {
        let mut m = scene_a(n, text_cnt(n, bit), EXT_ON);
        ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
        sample(&mut m)
    };
    assert_eq!(
        [
            layer(2, 0),
            layer(2, SLOT_BIT),
            layer(3, 0),
            layer(3, SLOT_BIT)
        ],
        [
            ext(2, [0, 5, 15]),
            ext(2, [0, 5, 15]),
            ext(3, [0, 5, 15]),
            ext(3, [0, 5, 15])
        ]
    );
}

#[test]
fn given_bit_13_with_bit_30_clear_when_a_text_layer_is_drawn_then_it_changes_nothing_unverified() {
    // TODO(hw-unverified): extended-palettes.md "Unverified", bit 13 with bit 30 clear.
    let mut m = scene_a(0, text_cnt(0, SLOT_BIT), 0);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(sample(&mut m), [xrgb(STANDARD_7); 3]);
}

#[test]
fn given_a_16_colour_text_layer_with_bit_30_when_drawn_then_it_keeps_the_standard_palette() {
    // 16 colours (BGxCNT bit 7 clear): the 4 bits per pixel tile 1 at byte 32 is all 7.
    let mut m = scene_a(0, 1 << 8, EXT_ON);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    fill_words(&mut m, BG_A + 32, 8, 0x7777_7777);
    // Palette number n of the map entry picks standard palette n: cell 0 is entry 7 (red), cell 5
    // is entry 16 * 5 + 7 (set to green) and cell 15 entry 16 * 15 + 7 (still black).
    w16(&mut m, BG_PALETTE_A + 2 * (16 * 5 + 7), 0x03E0);
    assert_eq!(sample(&mut m), [xrgb(STANDARD_7), xrgb(0x03E0), xrgb(0)]);
}

#[test]
fn given_value_0_pixels_with_bit_30_when_drawn_then_they_stay_transparent() {
    // Tile 0 is all zero: a layer of tile 0 shows the backdrop whatever the slots hold.
    let mut m = scene_a(0, text_cnt(0, 0), EXT_ON);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    fill_words(&mut m, BG_A + 0x800, 16, 0);
    assert_eq!(sample(&mut m), [xrgb(BACKDROP); 3]);
}

#[test]
fn given_bit_30_and_no_bank_in_an_extended_mapping_when_drawn_then_pixels_read_colour_0_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): extended-palettes.md "Unverified", an empty slot: black, not the
    // backdrop (the pixel value is not 0, so it is not transparent).
    let mut m = scene_a(0, text_cnt(0, 0), EXT_ON);
    assert_eq!(sample(&mut m), [xrgb(0); 3]);
}

#[test]
fn given_banks_f_and_g_when_bg0_is_drawn_then_slots_0_and_1_come_from_f_and_2_and_3_from_g() {
    let layer = |bit: u16| {
        let mut m = scene_a(0, text_cnt(0, bit), EXT_ON);
        ext_bank(&mut m, VRAMCNT_F, LCDC_F, (0, 2), 0x84);
        ext_bank(&mut m, VRAMCNT_G, LCDC_G, (2, 2), 0x84 | 1 << 3);
        sample(&mut m)
    };
    assert_eq!(
        [layer(0), layer(SLOT_BIT)],
        [ext(0, [0, 5, 15]), ext(2, [0, 5, 15])]
    );
}

#[test]
fn given_banks_e_and_f_on_the_same_slots_when_bg0_is_drawn_then_e_wins_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): the earlier letter wins; F would give slot 0 codes 1 + 16 * 5 + p.
    let mut m = scene_a(0, text_cnt(0, 0), EXT_ON);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, (5, 2), 0x84);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(sample(&mut m), ext(0, [0, 5, 15]));
}

#[test]
fn given_a_bank_switched_back_to_lcdc_when_bg0_is_drawn_then_the_slot_is_empty_and_the_colours_come_back_with_the_mapping()
 {
    let mut m = scene_a(0, text_cnt(0, 0), EXT_ON);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    let mapped = sample(&mut m);
    w8(&mut m, VRAMCNT_E, 0x80);
    let unmapped = sample(&mut m);
    w8(&mut m, VRAMCNT_E, 0x84);
    assert_eq!(
        (mapped, unmapped, sample(&mut m)),
        (ext(0, [0, 5, 15]), [xrgb(0); 3], ext(0, [0, 5, 15]))
    );
}

#[test]
fn given_engine_b_with_bank_h_when_bg0_is_drawn_then_it_reads_h_s_slots() {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w8(&mut m, VRAMCNT_C, BG_BANK_C);
    w16(&mut m, BG_PALETTE_B, BACKDROP);
    solid_tile(&mut m, BG_B);
    palette_row(&mut m, BG_B, 0x800);
    w16(&mut m, BG0CNT_B, text_cnt(0, SLOT_BIT));
    w32(&mut m, DISPCNT_B, MODE1 | EXT_ON | BG_ON[0]);
    ext_bank(&mut m, VRAMCNT_H, LCDC_H, (0, 4), 0x82);
    m.render_frame();
    let at = |cell: usize| pixel(&m, Screen::Bottom, cell * 8 + 3, 3);
    assert_eq!([at(0), at(5), at(15)], ext(2, [0, 5, 15]));
}

#[test]
fn given_the_slots_when_the_layers_are_decoded_then_ext_slot_says_which_slot_each_reads() {
    let mut m = scene_a(0, text_cnt(0, SLOT_BIT), EXT_ON);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    w16(&mut m, BG_REGS_ADDR + 2, text_cnt(1, 0));
    w16(&mut m, BG_REGS_ADDR + 4, 1 << 8);
    let gpu = m.inspect().gpu2d();
    let slots: Vec<_> = (0..3)
        .map(|n| gpu.text_bg(EngineId::A, n).ext_slot)
        .collect();
    assert_eq!(slots, vec![Some(2), Some(1), None]);
}

#[test]
fn given_banks_in_extended_mappings_when_inspected_then_the_slots_report_their_banks() {
    let mut m = Machine::new();
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    let banks = m.inspect().ext_palette_banks(EngineId::A);
    assert_eq!(
        (
            banks.bg,
            banks.obj,
            m.inspect().ext_palette_banks(EngineId::B).bg
        ),
        ([Some(4); 4], None, [None; 4])
    );
}

// --------------------------------------------------------- the extended and affine tile layers

/// Engine A in BG mode `mode` with bank A as BG VRAM, tile 1 solid 7 at char base 0, the standard
/// palette as in [`scene_a`], identity PA and PD for BG2 and BG3, and `extra` DISPCNT bits.
fn affine_scene(mode: u32, extra: u32) -> Machine {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w8(&mut m, VRAMCNT_A, BG_BANK_A);
    w16(&mut m, BG_PALETTE_A, BACKDROP);
    w16(&mut m, BG_PALETTE_A + 14, STANDARD_7);
    solid_tile(&mut m, BG_A);
    [AFFINE_REGS_ADDR, AFFINE_REGS_ADDR + 0x10]
        .iter()
        .fold((), |(), &at| {
            w16(&mut m, at, 0x100);
            w16(&mut m, at + 6, 0x100);
        });
    w32(&mut m, DISPCNT_A_ADDR, MODE1 | mode | extra);
    m
}

/// A map of 128x128 (16 x 16 cells) at screen block 1, byte 0x800, with row 0 of 16-bit entries
/// holding tile 1 and palette number = the column.
fn tiles16_row(m: &mut Machine) {
    (0..8u32).for_each(|n| {
        let entry = |cell: u32| u32::from(1 | (cell << 12) as u16);
        w32(
            m,
            BG_A + 0x800 + n * 4,
            entry(2 * n) | entry(2 * n + 1) << 16,
        );
    });
}

/// The colours at the middle of cells 0, 5 and 15 of row 0 of a 128-wide layer.
fn affine_sample(m: &mut Machine) -> [u32; 3] {
    m.render_frame();
    [0, 5, 15].map(|cell| pixel(m, Screen::Top, cell * 8 + 3, 3))
}

#[test]
fn given_an_extended_layer_with_16_bit_tile_entries_and_bit_30_when_drawn_then_it_reads_its_slot() {
    // BG mode 3: BG3 is extended; BGxCNT bit 7 clear is the tile format with 16-bit entries.
    let mut m = affine_scene(3 | 1 << 11, EXT_ON);
    tiles16_row(&mut m);
    w16(&mut m, BG_REGS_ADDR + 6, 1 << 8);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(affine_sample(&mut m), ext(3, [0, 5, 15]));
}

#[test]
fn given_the_same_extended_layer_with_bit_30_clear_when_drawn_then_it_reads_the_standard_palette() {
    let mut m = affine_scene(3 | 1 << 11, 0);
    tiles16_row(&mut m);
    w16(&mut m, BG_REGS_ADDR + 6, 1 << 8);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(affine_sample(&mut m), [xrgb(STANDARD_7); 3]);
}

#[test]
fn given_bg2_as_an_extended_tile_layer_in_mode_5_when_drawn_then_it_reads_slot_2() {
    let mut m = affine_scene(5 | 1 << 10, EXT_ON);
    tiles16_row(&mut m);
    w16(&mut m, BG_REGS_ADDR + 4, 1 << 8);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(affine_sample(&mut m), ext(2, [0, 5, 15]));
}

#[test]
fn given_an_affine_layer_with_8_bit_entries_and_bit_30_when_drawn_then_it_keeps_the_standard_palette()
 {
    // BG mode 2: BG2 is affine with one-byte map entries; the row of tile 1 is words of 0x01010101.
    let mut m = affine_scene(2 | 1 << 10, EXT_ON);
    fill_words(&mut m, BG_A + 0x800, 4, 0x0101_0101);
    w16(&mut m, BG_REGS_ADDR + 4, 1 << 8);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(affine_sample(&mut m), [xrgb(STANDARD_7); 3]);
}

#[test]
fn given_a_256_colour_bitmap_and_bit_30_when_drawn_then_it_keeps_the_standard_palette() {
    // BG mode 5: BG2 as a 256-colour bitmap (bit 7), 128 wide; its first 8 rows are all 7.
    let mut m = affine_scene(5 | 1 << 10, EXT_ON);
    fill_words(&mut m, BG_A, 256, 0x0707_0707);
    w16(&mut m, BG_REGS_ADDR + 4, 1 << 7);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    assert_eq!(affine_sample(&mut m), [xrgb(STANDARD_7); 3]);
}

#[test]
fn given_the_backdrop_and_bit_30_when_nothing_covers_a_pixel_then_it_is_standard_palette_entry_0() {
    let mut m = scene_a(0, text_cnt(0, 0), EXT_ON);
    ext_bank(&mut m, VRAMCNT_E, LCDC_E, (0, 4), 0x84);
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 200, 100), xrgb(BACKDROP));
}

/// The extended-affine layer's decode, as the debugger reads it.
#[test]
fn given_the_extended_tile_layer_when_decoded_then_ext_slot_is_its_number_and_other_formats_have_none()
 {
    let mut m = affine_scene(5 | 3 << 10, EXT_ON);
    w16(&mut m, BG_REGS_ADDR + 4, 0);
    w16(&mut m, BG_REGS_ADDR + 6, 1 << 7);
    let gpu = m.inspect().gpu2d();
    assert_eq!(
        (
            gpu.affine_bg(EngineId::A, 2).ext_slot,
            gpu.affine_bg(EngineId::A, 3).ext_slot
        ),
        (Some(2), None)
    );
}
