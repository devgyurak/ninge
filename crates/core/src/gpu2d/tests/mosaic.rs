// SPDX-License-Identifier: Apache-2.0
//! Mosaic (issue #30). Every expected value comes from `docs/hw/bg-modes.md` ("Mosaic"): with block
//! size `h` by `v`, screen pixel (`x`, `y`) of a mosaic layer shows what the same layer shows,
//! without mosaic, at (`x - x mod h`, `y - y mod v`). The tests render each scene twice, once plain
//! and once with mosaic, and compare the second against the first by that rule; the plain picture
//! of every kind of layer is pinned by the tests of `text.rs`, `affine.rs`, `extended.rs` and
//! `large.rs`.

use super::affine::{
    BANK_A_BG, BANK_C_BG_B, Diff, ENG_A, ENG_B, Eng, NONE, SHEAR, VRAMCNT_C, color15, scene,
};
use super::extended::{BMP256_A, Ext, Fmt, MAP_A, build, with};
use super::*;

/// BGxCNT bit 6.
const MOSAIC_ON: u16 = 1 << 6;
/// The backdrop, palette entry 0.
const BACKDROP: u32 = 0x0000_00FF;

fn mosaic_addr(eng: Eng) -> u32 {
    match eng.screen {
        Screen::Top => MOSAIC_ADDR,
        Screen::Bottom => MOSAIC_B_ADDR,
    }
}

/// The pixels of `mosaic`'s `screen` that are not the pixel `plain` shows at the first pixel of the
/// block, for blocks of `h` by `v`, as (x, y, got, want). Both machines render a frame here.
fn block_failures(
    plain: &mut Machine,
    mosaic: &mut Machine,
    screen: Screen,
    (h, v): (usize, usize),
) -> Diff {
    plain.render_frame();
    mosaic.render_frame();
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let want = pixel(plain, screen, x - x % h, y - y % v);
            let got = pixel(mosaic, screen, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .take(8)
        .collect()
}

/// The block size a MOSAIC value gives (BG fields only).
fn block(mosaic: u16) -> (usize, usize) {
    (
        usize::from(mosaic & 0xF) + 1,
        usize::from((mosaic >> 4) & 0xF) + 1,
    )
}

/// Switches BGxCNT bit 6 of BG `layer` on and writes MOSAIC.
fn enable(m: &mut Machine, eng: Eng, layer: u32, mosaic: u16) {
    let cnt = eng.bg_regs + layer * 2;
    let old = r16(m, cnt);
    w16(m, cnt, old | MOSAIC_ON);
    w16(m, mosaic_addr(eng), mosaic);
}

// ------------------------------------------------------------------------- text layers

/// Four-bits-per-pixel tiles 1-3 where pixel (r, c) of tile t holds `(3 r + c + t) mod 16` (0 is
/// transparent, so every tile has holes), and a 32x32 map of tile `1 + ((cx + cy) mod 3)`, flipped
/// horizontally in odd columns and vertically in odd rows. BG0, character block 1, screen block 0,
/// scrolled by (`hofs`, `vofs`).
fn text_scene(eng: Eng, hofs: u16, vofs: u16) -> Machine {
    let mut m = Machine::new();
    let (bank_reg, bank) = match eng.screen {
        Screen::Top => (VRAMCNT_A, BANK_A_BG),
        Screen::Bottom => (VRAMCNT_C, BANK_C_BG_B),
    };
    w8(&mut m, bank_reg, bank);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w16(&mut m, eng.palette, 0x7C00);
    (1..16).for_each(|i| w16(&mut m, eng.palette + i * 2, color15(i)));
    (1..=3u32).for_each(|t| text_tile(&mut m, eng.vram + 0x4000 + t * 32, t));
    (0..32 * 32u32).for_each(|i| {
        let (cx, cy) = (i % 32, i / 32);
        let entry = (1 + (cx + cy) % 3) | (cx & 1) << 10 | (cy & 1) << 11;
        w16(&mut m, eng.vram + i * 2, entry as u16);
    });
    w16(&mut m, eng.bg_regs, 1 << 2);
    w16(&mut m, eng.bg_regs + 8, hofs);
    w16(&mut m, eng.bg_regs + 10, vofs);
    w32(&mut m, eng.dispcnt, (1 << 16) | (1 << 8));
    m
}

fn text_tile(m: &mut Machine, at: u32, t: u32) {
    (0..8u32).for_each(|r| {
        let word = (0..8u32).fold(0u32, |w, c| w | ((3 * r + c + t) & 15) << (4 * c));
        w32(m, at + r * 4, word);
    });
}

/// A plain and a mosaic frame of the text scene, compared by the block rule.
fn text_failures(eng: Eng, hofs: u16, vofs: u16, mosaic: u16) -> Diff {
    let mut plain = text_scene(eng, hofs, vofs);
    let mut with_mosaic = text_scene(eng, hofs, vofs);
    enable(&mut with_mosaic, eng, 0, mosaic);
    block_failures(&mut plain, &mut with_mosaic, eng.screen, block(mosaic))
}

#[test]
fn given_a_horizontal_mosaic_of_4_when_a_text_layer_is_drawn_then_each_block_shows_its_first_pixel()
{
    assert_eq!(text_failures(ENG_A, 0, 0, 0x0003), NONE);
}

#[test]
fn given_a_vertical_mosaic_of_3_when_a_text_layer_is_drawn_then_each_block_shows_its_first_line() {
    assert_eq!(text_failures(ENG_A, 0, 0, 0x0020), NONE);
}

#[test]
fn given_a_6_by_5_mosaic_when_a_text_layer_is_drawn_then_each_block_shows_its_first_pixel() {
    assert_eq!(text_failures(ENG_A, 0, 0, 0x0045), NONE);
}

#[test]
fn given_the_largest_mosaic_when_a_text_layer_is_drawn_then_blocks_are_16_by_16() {
    assert_eq!(text_failures(ENG_A, 0, 0, 0x00FF), NONE);
}

#[test]
fn given_a_scrolled_text_layer_when_a_mosaic_is_on_then_the_blocks_stay_on_the_screens_grid() {
    // The blocks are counted from the top left of the screen, whatever the layer's scroll
    // (`bg-modes.md`, "Mosaic"); the first pixel of each is the layer's own pixel there.
    assert_eq!(text_failures(ENG_A, 5, 3, 0x0033), NONE);
}

#[test]
fn given_a_text_layer_on_engine_b_when_a_mosaic_is_on_then_engine_bs_mosaic_applies() {
    assert_eq!(text_failures(ENG_B, 7, 2, 0x0024), NONE);
}

#[test]
fn given_a_mosaic_size_of_1_when_a_text_layer_is_drawn_then_the_picture_is_unchanged() {
    assert_eq!(text_failures(ENG_A, 3, 1, 0x0000), NONE);
}

#[test]
fn given_a_mosaic_size_but_bit_6_clear_when_a_text_layer_is_drawn_then_the_picture_is_unchanged() {
    let mut plain = text_scene(ENG_A, 0, 0);
    let mut other = text_scene(ENG_A, 0, 0);
    w16(&mut other, MOSAIC_ADDR, 0x00FF);
    assert_eq!(
        block_failures(&mut plain, &mut other, Screen::Top, (1, 1)),
        NONE
    );
}

#[test]
fn given_engine_as_mosaic_when_engine_bs_layer_has_bit_6_set_then_engine_b_uses_its_own_size() {
    // Engine B's layer has bit 6 but engine B's MOSAIC is 0: no mosaic, though engine A's is set.
    let mut plain = text_scene(ENG_B, 0, 0);
    let mut other = text_scene(ENG_B, 0, 0);
    w16(&mut other, MOSAIC_ADDR, 0x00FF);
    w16(&mut other, ENG_B.bg_regs, (1 << 2) | MOSAIC_ON);
    assert_eq!(
        block_failures(&mut plain, &mut other, Screen::Bottom, (1, 1)),
        NONE
    );
}

#[test]
fn given_a_block_whose_first_pixel_is_transparent_when_drawn_then_the_whole_block_is_the_backdrop()
{
    // Pixel (0, 0) of tile 1 in cell (0, 0) is `(0 + 0 + 1) mod 16` = 1; make it 0 by clearing the
    // low nibble of tile 1's first word, leaving the others opaque.
    let mut m = text_scene(ENG_A, 0, 0);
    w32(&mut m, ENG_A.vram + 0x4000 + 32, 0x6543_2100);
    enable(&mut m, ENG_A, 0, 0x0003);
    m.render_frame();
    assert_eq!(
        (
            pixel(&m, Screen::Top, 0, 0),
            pixel(&m, Screen::Top, 1, 0),
            pixel(&m, Screen::Top, 3, 0)
        ),
        (BACKDROP, BACKDROP, BACKDROP)
    );
}

// ----------------------------------------------------------------- affine and extended

/// A plain and a mosaic frame of `plain`'s scene (`mosaic` is the same scene with the mosaic on),
/// compared by the block rule.
fn layer_failures(
    plain: &mut Machine,
    mosaic: &mut Machine,
    eng: Eng,
    layer: u32,
    size: u16,
) -> Diff {
    enable(mosaic, eng, layer, size);
    block_failures(plain, mosaic, eng.screen, block(size))
}

#[test]
fn given_an_affine_layer_with_a_rotation_when_a_mosaic_is_on_then_each_block_shows_its_first_pixel()
{
    // The vertical size matters most: line y of a block must use the point of the block's first
    // line, so PB and PD are not added while the block lasts (`bg-modes.md`, "Mosaic").
    let (mut plain, mut mosaic) = (scene(ENG_A, 1, true, SHEAR), scene(ENG_A, 1, true, SHEAR));
    assert_eq!(
        layer_failures(&mut plain, &mut mosaic, ENG_A, 2, 0x0045),
        NONE
    );
}

#[test]
fn given_an_affine_layer_without_wrap_when_a_mosaic_is_on_then_the_outside_stays_transparent() {
    let (mut plain, mut mosaic) = (scene(ENG_A, 1, false, SHEAR), scene(ENG_A, 1, false, SHEAR));
    assert_eq!(
        layer_failures(&mut plain, &mut mosaic, ENG_A, 2, 0x0033),
        NONE
    );
}

#[test]
fn given_a_16_bit_tile_layer_when_a_mosaic_is_on_then_each_block_shows_its_first_pixel() {
    let e = with(MAP_A, Fmt::Map16, 1, true, SHEAR);
    assert_eq!(
        layer_failures(&mut build(&e), &mut build(&e), ENG_A, 2, 0x0024),
        NONE
    );
}

#[test]
fn given_a_256_colour_bitmap_when_a_mosaic_is_on_then_each_block_shows_its_first_pixel() {
    let e = with(BMP256_A, Fmt::Bitmap256, 1, true, SHEAR);
    assert_eq!(
        layer_failures(&mut build(&e), &mut build(&e), ENG_A, 2, 0x0057),
        NONE
    );
}

#[test]
fn given_a_direct_colour_bitmap_when_a_mosaic_is_on_then_each_block_shows_its_first_pixel() {
    let e = with(BMP256_A, Fmt::BitmapDirect, 1, true, SHEAR);
    assert_eq!(
        layer_failures(&mut build(&e), &mut build(&e), ENG_A, 2, 0x0075),
        NONE
    );
}

#[test]
fn given_a_large_bitmap_when_a_mosaic_is_on_then_each_block_shows_its_first_pixel() {
    let e = Ext {
        fmt: Fmt::Large,
        bg_mode: 6,
        size_code: 0,
        p: SHEAR,
        ..MAP_A
    };
    assert_eq!(
        layer_failures(&mut build(&e), &mut build(&e), ENG_A, 2, 0x0066),
        NONE
    );
}

#[test]
fn given_an_affine_layer_on_engine_b_when_a_mosaic_is_on_then_engine_bs_mosaic_applies() {
    let (mut plain, mut mosaic) = (scene(ENG_B, 1, true, SHEAR), scene(ENG_B, 1, true, SHEAR));
    assert_eq!(
        layer_failures(&mut plain, &mut mosaic, ENG_B, 2, 0x0053),
        NONE
    );
}

// ------------------------------------------------------------------------- registers

#[test]
fn given_mosaic_written_when_decoded_then_the_sizes_are_the_fields_plus_1() {
    let mut m = Machine::new();
    w16(&mut m, MOSAIC_ADDR, 0x4321);
    assert_eq!(
        m.inspect().gpu2d().mosaic(EngineId::A),
        Mosaic {
            bg_h: 2,
            bg_v: 3,
            obj_h: 4,
            obj_v: 5
        }
    );
}

#[test]
fn given_a_power_on_machine_when_mosaic_is_decoded_then_every_size_is_1() {
    let m = Machine::new();
    let one = Mosaic {
        bg_h: 1,
        bg_v: 1,
        obj_h: 1,
        obj_v: 1,
    };
    assert_eq!(
        (
            m.inspect().gpu2d().mosaic(EngineId::A),
            m.inspect().gpu2d().mosaic(EngineId::B)
        ),
        (one, one)
    );
}

#[test]
fn given_a_byte_written_to_mosaic_when_decoded_then_only_that_byte_changes() {
    let mut m = Machine::new();
    w16(&mut m, MOSAIC_ADDR, 0x1234);
    w8(&mut m, MOSAIC_ADDR + 1, 0xFF);
    assert_eq!(
        m.inspect().gpu2d().mosaic(EngineId::A),
        Mosaic {
            bg_h: 5,
            bg_v: 4,
            obj_h: 16,
            obj_v: 16
        }
    );
}

#[test]
fn given_a_word_written_at_mosaic_when_decoded_then_the_halfword_above_is_not_a_register() {
    let mut m = Machine::new();
    w32(&mut m, MOSAIC_ADDR, 0xFFFF_0021);
    assert_eq!(
        (
            m.inspect().gpu2d().mosaic(EngineId::A),
            r32(&m, MOSAIC_ADDR)
        ),
        (
            Mosaic {
                bg_h: 2,
                bg_v: 3,
                obj_h: 1,
                obj_v: 1
            },
            0
        )
    );
}

#[test]
fn given_mosaic_written_in_engine_b_when_decoded_then_engine_a_keeps_its_own() {
    let mut m = Machine::new();
    w16(&mut m, MOSAIC_B_ADDR, 0x0034);
    assert_eq!(
        (
            m.inspect().gpu2d().mosaic(EngineId::B).bg_h,
            m.inspect().gpu2d().mosaic(EngineId::A).bg_h
        ),
        (5, 1)
    );
}

#[test]
fn given_engine_b_powered_off_when_mosaic_is_written_then_it_stays_as_it_was() {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP & !(1 << 9));
    w16(&mut m, MOSAIC_B_ADDR, 0x00FF);
    assert_eq!(m.inspect().gpu2d().mosaic(EngineId::B).bg_h, 1);
}

#[test]
fn given_bgcnt_bit_6_when_layers_are_decoded_then_the_mosaic_flag_shows_it() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 2);
    w16(&mut m, BG_REGS_ADDR, MOSAIC_ON);
    w16(&mut m, BG_REGS_ADDR + 4, MOSAIC_ON);
    let gpu = m.inspect().gpu2d();
    assert_eq!(
        (
            gpu.text_bg(EngineId::A, 0).mosaic,
            gpu.text_bg(EngineId::A, 1).mosaic,
            gpu.affine_bg(EngineId::A, 2).mosaic,
            gpu.affine_bg(EngineId::A, 3).mosaic
        ),
        (true, false, true, false)
    );
}

#[test]
fn given_mosaic_registers_after_writes_when_read_then_they_read_zero_as_the_note_models_unverified()
{
    // TODO(hw-unverified): bg-modes.md "Unverified", reads of the write-only MOSAIC.
    let mut m = Machine::new();
    w16(&mut m, MOSAIC_ADDR, 0x00FF);
    assert_eq!(r16(&m, MOSAIC_ADDR), 0);
}

#[test]
fn given_a_mosaic_set_when_saved_and_loaded_then_the_loaded_machine_draws_the_same_frame() {
    let mut saver = text_scene(ENG_A, 3, 1);
    enable(&mut saver, ENG_A, 0, 0x0033);
    let mut target = Machine::new();
    let result = target.load_state(&saver.save_state());
    saver.render_frame();
    target.render_frame();
    assert_eq!(
        (
            result,
            target.frame(Screen::Top) == saver.frame(Screen::Top)
        ),
        (Ok(()), true)
    );
}
