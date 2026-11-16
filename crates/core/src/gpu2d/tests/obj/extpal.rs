// SPDX-License-Identifier: Apache-2.0
//! OBJ extended palettes (issue #32). Every expected value comes from
//! `docs/hw/extended-palettes.md` ("Objects"): with DISPCNT bit 31 set a 256-colour tile object,
//! plain or affine, reads palette `attr2 >> 12` of the OBJ slot, entry = the pixel value; a
//! 16-colour object keeps the standard palette and a bitmap object has no palette. The extended
//! colour of palette `p`, entry `i` is `1 + 256 p + i`, a 15-bit value, so every pair is different.

use super::affine::put_group;
use super::*;

const EXT_OBJ: u32 = 1 << 31;
const VRAMCNT_F: u32 = 0x0400_0245;
const VRAMCNT_G: u32 = 0x0400_0246;
const VRAMCNT_I: u32 = 0x0400_0249;
const LCDC_F: u32 = 0x0689_0000;
const LCDC_G: u32 = 0x0689_4000;
const LCDC_I: u32 = 0x068A_0000;
/// MST 5 (engine A OBJ extended palette, F or G) and MST 3 (engine B's, bank I), enabled.
const MST5: u8 = 0x85;
const MST3: u8 = 0x83;
const VALUE: u32 = 200;

/// The extended colour (palette `p`, entry `i`).
fn code(p: u32, i: u32) -> u16 {
    (1 + 256 * p + i) as u16
}

/// Fills entries (`p`, `VALUE`) of the palettes `palettes` in the bank at LCDC address `lcdc`,
/// then maps the bank with `value`.
fn ext_bank(m: &mut Machine, vramcnt: u32, lcdc: u32, palettes: &[u32], value: u8) {
    w8(m, vramcnt, 0x80);
    palettes.iter().fold((), |(), &p| {
        w16(m, lcdc + p * 512 + VALUE * 2, code(p, VALUE))
    });
    w8(m, vramcnt, value);
}

/// A 256-colour 8x8 object of tile 2 (byte 64 in 2D mapping, solid `VALUE`) at (`x`, 20) with
/// palette number `palette`, and the first pixel of tile 2 cleared so that value 0 is tested too.
fn solid_object(m: &mut Machine, side: Side, index: u32, x: u16, palette: u16) {
    solid8(m, side, 64, VALUE);
    w32(m, side.vram + 64, 0xC8C8_C800);
    put_obj(m, side, index, [20 | 1 << 13, x, 2 | palette << 12]);
}

/// The colours of the object at (`x`, 20) in the second pixel of its first row.
fn color_at(m: &mut Machine, side: Side, x: usize) -> u32 {
    m.render_frame();
    get(m, side, x + 1, 20)
}

fn xrgb(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

#[test]
fn given_a_256_colour_object_with_bit_31_when_drawn_then_the_palette_number_and_value_pick_the_extended_colour()
 {
    let mut m = scene(SIDE_A, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[2, 9], MST5);
    solid_object(&mut m, SIDE_A, 0, 10, 2);
    solid_object(&mut m, SIDE_A, 1, 40, 9);
    assert_eq!(
        [color_at(&mut m, SIDE_A, 10), color_at(&mut m, SIDE_A, 40)],
        [xrgb(code(2, VALUE)), xrgb(code(9, VALUE))]
    );
}

#[test]
fn given_the_same_object_with_bit_31_clear_when_drawn_then_it_reads_the_standard_palette() {
    let mut m = scene(SIDE_A, 0);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[2], MST5);
    solid_object(&mut m, SIDE_A, 0, 10, 2);
    // The standard OBJ palette holds `entry(i)`: the palette number is not used.
    assert_eq!(color_at(&mut m, SIDE_A, 10), color(VALUE));
}

#[test]
fn given_a_16_colour_object_with_bit_31_when_drawn_then_it_keeps_the_standard_palette() {
    // Tile 1 solid 5, palette number 2: standard entry 2 * 16 + 5.
    let mut m = scene(SIDE_A, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[2], MST5);
    solid4(&mut m, SIDE_A, 32, 5);
    put_obj(&mut m, SIDE_A, 0, [20, 10, 1 | 2 << 12]);
    assert_eq!(color_at(&mut m, SIDE_A, 10), color(2 * 16 + 5));
}

#[test]
fn given_a_bitmap_object_with_bit_31_when_drawn_then_its_direct_colour_is_unchanged() {
    // 1D mapping, tile 32 at byte 4096: pixel 1 is opaque 0x801F (red 31).
    let mut m = scene(SIDE_A, EXT_OBJ | MAPPING_1D | 1 << 6);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[0], MST5);
    w16(&mut m, SIDE_A.vram + 4098, 0x801F);
    put_obj(&mut m, SIDE_A, 0, [20 | 3 << 10, 10, 32]);
    assert_eq!(color_at(&mut m, SIDE_A, 10), 0x00FF_0000);
}

#[test]
fn given_an_affine_256_colour_object_with_bit_31_when_drawn_then_it_reads_the_extended_palette() {
    let mut m = scene(SIDE_A, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[7], MST5);
    solid8(&mut m, SIDE_A, 64, VALUE);
    put_obj(&mut m, SIDE_A, 0, [20 | 1 << 8 | 1 << 13, 10, 2 | 7 << 12]);
    put_group(&mut m, SIDE_A, 0, [0x100, 0, 0, 0x100]);
    assert_eq!(color_at(&mut m, SIDE_A, 10), xrgb(code(7, VALUE)));
}

#[test]
fn given_value_0_pixels_with_bit_31_when_drawn_then_they_stay_transparent() {
    // The first pixel of the object is 0: the backdrop shows there, whatever palette 2 holds at 0.
    let mut m = scene(SIDE_A, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[2], MST5);
    solid_object(&mut m, SIDE_A, 0, 10, 2);
    m.render_frame();
    assert_eq!(get(&m, SIDE_A, 10, 20), BACKDROP);
}

#[test]
fn given_bit_31_and_no_bank_when_a_256_colour_object_is_drawn_then_it_reads_colour_0_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): extended-palettes.md "Unverified", an empty slot: black, not transparent.
    let mut m = scene(SIDE_A, EXT_OBJ);
    solid_object(&mut m, SIDE_A, 0, 10, 2);
    assert_eq!(color_at(&mut m, SIDE_A, 10), 0);
}

#[test]
fn given_bank_g_alone_with_mst_5_when_a_256_colour_object_is_drawn_then_it_reads_g() {
    let mut m = scene(SIDE_A, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_G, LCDC_G, &[4], MST5);
    solid_object(&mut m, SIDE_A, 0, 10, 4);
    assert_eq!(color_at(&mut m, SIDE_A, 10), xrgb(code(4, VALUE)));
}

#[test]
fn given_banks_f_and_g_with_mst_5_when_a_256_colour_object_is_drawn_then_f_wins_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): the earlier letter wins. G's palette 4 holds a different colour.
    let mut m = scene(SIDE_A, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_G, LCDC_G, &[4], MST5);
    w16(&mut m, LCDC_G + 4 * 512 + VALUE * 2, 0x7FFF);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[4], MST5);
    solid_object(&mut m, SIDE_A, 0, 10, 4);
    assert_eq!(color_at(&mut m, SIDE_A, 10), xrgb(code(4, VALUE)));
}

#[test]
fn given_engine_b_with_bank_i_when_a_256_colour_object_is_drawn_then_it_reads_i() {
    let mut m = scene(SIDE_B, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_I, LCDC_I, &[6], MST3);
    solid_object(&mut m, SIDE_B, 0, 10, 6);
    assert_eq!(color_at(&mut m, SIDE_B, 10), xrgb(code(6, VALUE)));
}

#[test]
fn given_a_bank_for_engine_a_only_when_engine_b_draws_with_bit_31_then_its_slot_is_empty() {
    let mut m = scene(SIDE_B, EXT_OBJ);
    ext_bank(&mut m, VRAMCNT_F, LCDC_F, &[6], MST5);
    solid_object(&mut m, SIDE_B, 0, 10, 6);
    assert_eq!(color_at(&mut m, SIDE_B, 10), 0);
}

/// Engine A with BG0 (256 colours, tile 1 solid 7, bank B as BG VRAM), bank E as a BG extended
/// palette, and `extra` DISPCNT bits besides sprites and BG0.
fn bg_scene(extra: u32) -> Machine {
    let mut m = scene(SIDE_A, extra | 1 << 8);
    w8(&mut m, 0x0400_0241, 0x81);
    ext_bank(&mut m, 0x0400_0244, 0x0688_0000, &[0], 0x84);
    w16(&mut m, 0x0500_0000 + 14, 0x001F);
    (0..16).for_each(|n| w32(&mut m, BG_A + 64 + n * 4, 0x0707_0707));
    (0..512).for_each(|n| w32(&mut m, BG_A + 0x800 + n * 4, 0x0001_0001));
    w16(&mut m, BG_REGS_ADDR, 0x0080 | 1 << 8);
    m
}

#[test]
fn given_bit_31_alone_when_a_256_colour_background_is_drawn_then_it_reads_the_standard_palette() {
    // DISPCNT bits 30 and 31 are independent (`extended-palettes.md`, "Registers"): the BG layer
    // needs bit 30. With bit 30 too it reads slot 0 of bank E, whose palette 0 holds `code(0, VALUE)`
    // only at index 200 and 0 at index 7, so it shows black.
    let (mut only_31, mut both) = (bg_scene(EXT_OBJ), bg_scene(EXT_OBJ | 1 << 30));
    only_31.render_frame();
    both.render_frame();
    assert_eq!(
        (
            get(&only_31, SIDE_A, 100, 100),
            get(&both, SIDE_A, 100, 100)
        ),
        (xrgb(0x001F), 0)
    );
}

#[test]
fn given_banks_in_obj_mappings_when_inspected_then_the_obj_slots_report_them() {
    let mut m = scene(SIDE_A, 0);
    ext_bank(&mut m, VRAMCNT_G, LCDC_G, &[0], MST5);
    ext_bank(&mut m, VRAMCNT_I, LCDC_I, &[0], MST3);
    let (a, b) = (
        m.inspect().ext_palette_banks(EngineId::A),
        m.inspect().ext_palette_banks(EngineId::B),
    );
    assert_eq!((a.obj, b.obj, a.bg), (Some(6), Some(8), [None; 4]));
}
