// SPDX-License-Identifier: Apache-2.0
//! Extended layers (issue #30): tile layers with 16-bit entries, 256-colour bitmaps and
//! direct-colour bitmaps. Every expected value comes from `docs/hw/bg-modes.md` ("Extended layers",
//! "Sizes and where the data is"), worked out by a small model written here from the note's
//! formulas, never from running the renderer.

use super::affine::{
    BANK_A_BG, BANK_C_BG_B, Diff, ENG_A, ENG_B, Eng, IDENTITY, NONE, Params, SHEAR, VRAMCNT_C,
    WRAP, color15, expand15, set_params, tile_rows,
};
use super::*;
use crate::debug::AffineFormat;

/// VRAMCNT_B..D, for the 512 KiB of engine A's BG window: banks A-D at OFS 0-3.
const VRAMCNT_B: u32 = 0x0400_0241;
const VRAMCNT_D: u32 = 0x0400_0243;
const BANK_B_BG: u8 = 0x89;
const BANK_D_BG: u8 = 0x99;
const BANK_C_BG_A: u8 = 0x91;
/// BGxCNT bits 2 and 7: the extended format selection.
const CNT_BITMAP: u16 = 1 << 7;
const CNT_DIRECT: u16 = 1 << 2;
/// The backdrop, palette entry 0.
const BACKDROP: u32 = 0x0000_00FF;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fmt {
    Map16,
    Bitmap256,
    BitmapDirect,
}

/// One extended layer as a scene sets it up.
#[derive(Clone, Copy)]
struct Ext {
    eng: Eng,
    /// 2 or 3.
    layer: u32,
    /// BG mode 3-5, which makes the layer extended.
    bg_mode: u32,
    fmt: Fmt,
    size_code: u16,
    wrap: bool,
    /// BGxCNT bits 8-12: screen block (2 KiB for a tile layer, 16 KiB for a bitmap).
    block: u16,
    /// BGxCNT bits 2-5, the character block, for the tile format.
    char_block: u16,
    /// DISPCNT bits beyond the display mode, BG mode and layer enable.
    extra: u32,
    p: Params,
}

const MAP_A: Ext = Ext {
    eng: ENG_A,
    layer: 2,
    bg_mode: 5,
    fmt: Fmt::Map16,
    size_code: 1,
    wrap: true,
    block: 0,
    char_block: 4,
    extra: 0,
    p: IDENTITY,
};

impl Ext {
    /// The layer's size in pixels.
    fn dims(&self) -> (i64, i64) {
        match (self.fmt, self.size_code) {
            (Fmt::Map16, c) => (128 << c, 128 << c),
            (_, 2) => (512, 256),
            (_, 3) => (512, 512),
            (_, c) => (128 << c, 128 << c),
        }
    }

    fn bgcnt(&self) -> u16 {
        let format = match self.fmt {
            Fmt::Map16 => self.char_block << 2,
            Fmt::Bitmap256 => CNT_BITMAP,
            Fmt::BitmapDirect => CNT_BITMAP | CNT_DIRECT,
        };
        format | (self.block << 8) | (self.size_code << 14) | if self.wrap { WRAP } else { 0 }
    }

    /// Where the bitmap or the map starts in the engine's BG VRAM.
    fn base(&self) -> u32 {
        match self.fmt {
            Fmt::Map16 => u32::from(self.block) * 0x800,
            _ => u32::from(self.block) * 0x4000,
        }
    }
}

/// The palette value of the 256-colour bitmap at (`tx`, `ty`): never 0 except where the model
/// says so (value 0 is transparent).
fn bitmap_value(tx: i64, ty: i64) -> u32 {
    match (tx + ty) % 11 {
        0 => 0,
        _ => (1 + (5 * tx + 3 * ty) % 255) as u32,
    }
}

/// The direct-colour bitmap at (`tx`, `ty`): bit 15 is set except on every third diagonal.
fn direct_value(tx: i64, ty: i64) -> u16 {
    let color = ((tx * 37 + ty * 91) & 0x7FFF) as u16;
    match (tx + ty) % 3 {
        0 => color,
        _ => color | 0x8000,
    }
}

/// The 16-bit entry of cell (`cx`, `cy`): tile `1 + ((cx + 2 cy) mod 4)`, flipped horizontally in
/// odd columns and vertically in odd rows.
fn entry(cx: i64, cy: i64) -> u16 {
    (1 + ((cx + 2 * cy) & 3)) as u16 | ((cx & 1) as u16) << 10 | ((cy & 1) as u16) << 11
}

/// The model's 15-bit colour at texel (`tx`, `ty`) inside the layer, or `None` if transparent.
fn texel_color(e: &Ext, tx: i64, ty: i64) -> Option<u16> {
    match e.fmt {
        Fmt::Map16 => {
            let entry = entry(tx / 8, ty / 8);
            let tile = i64::from(entry & 0x3FF);
            let r = if entry & 0x800 != 0 {
                7 - ty % 8
            } else {
                ty % 8
            };
            let c = if entry & 0x400 != 0 {
                7 - tx % 8
            } else {
                tx % 8
            };
            Some(color15((16 * tile + ((3 * r + c) & 15)) as u32))
        }
        Fmt::Bitmap256 => match bitmap_value(tx, ty) {
            0 => None,
            value => Some(color15(value)),
        },
        Fmt::BitmapDirect => {
            let value = direct_value(tx, ty);
            (value & 0x8000 != 0).then_some(value & 0x7FFF)
        }
    }
}

/// The xRGB pixel the note gives at (`x`, `y`) for the layer drawn alone.
fn expected(e: &Ext, x: usize, y: usize) -> u32 {
    let (w, h) = e.dims();
    let p = e.p;
    let origin_x = i64::from(p.x) + i64::from(p.pb) * y as i64;
    let origin_y = i64::from(p.y) + i64::from(p.pd) * y as i64;
    let u = origin_x + i64::from(p.pa) * x as i64;
    let v = origin_y + i64::from(p.pc) * x as i64;
    let (tx, ty) = (u.div_euclid(256), v.div_euclid(256));
    let inside = (0..w).contains(&tx) && (0..h).contains(&ty);
    match (inside, e.wrap) {
        (false, false) => BACKDROP,
        _ => texel_color(e, tx.rem_euclid(w), ty.rem_euclid(h)).map_or(BACKDROP, expand15),
    }
}

/// Writes the scene's data and registers: palette, tile data or bitmap, BGxCNT, transform,
/// DISPCNT.
fn build(e: &Ext) -> Machine {
    let mut m = Machine::new();
    let eng = e.eng;
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    match eng.screen {
        Screen::Top => {
            [
                (VRAMCNT_A, BANK_A_BG),
                (VRAMCNT_B, BANK_B_BG),
                (VRAMCNT_C, BANK_C_BG_A),
            ]
            .iter()
            .for_each(|&(reg, value)| w8(&mut m, reg, value));
            w8(&mut m, VRAMCNT_D, BANK_D_BG);
        }
        Screen::Bottom => w8(&mut m, VRAMCNT_C, BANK_C_BG_B),
    }
    w16(&mut m, eng.palette, 0x7C00);
    (1..256).for_each(|i| w16(&mut m, eng.palette + i * 2, color15(i)));
    let base = eng.vram + e.base();
    let (w, h) = e.dims();
    match e.fmt {
        Fmt::Map16 => {
            let char_base = eng.vram + u32::from(e.char_block) * 0x4000;
            (1..=4u32).for_each(|tile| tile_rows(&mut m, char_base + tile * 64, tile));
            fill_map16(&mut m, base, (w / 8) as u32);
        }
        Fmt::Bitmap256 => fill_bitmap256(&mut m, base, w, h),
        Fmt::BitmapDirect => fill_direct(&mut m, base, w, h),
    }
    w16(&mut m, eng.bg_regs + e.layer * 2, e.bgcnt());
    set_params(&mut m, eng, e.layer - 2, e.p);
    w32(
        &mut m,
        eng.dispcnt,
        (1 << 16) | e.bg_mode | (1 << (8 + e.layer)) | e.extra,
    );
    m
}

fn fill_map16(m: &mut Machine, base: u32, cells: u32) {
    (0..cells * cells).for_each(|i| {
        let (cx, cy) = (i64::from(i % cells), i64::from(i / cells));
        w16(m, base + i * 2, entry(cx, cy));
    });
}

fn fill_bitmap256(m: &mut Machine, base: u32, w: i64, h: i64) {
    (0..w * h / 4).for_each(|word| {
        let pixel = |k: i64| bitmap_value((word * 4 + k) % w, (word * 4 + k) / w) as u8;
        w32(
            m,
            base + word as u32 * 4,
            u32::from_le_bytes([pixel(0), pixel(1), pixel(2), pixel(3)]),
        );
    });
}

fn fill_direct(m: &mut Machine, base: u32, w: i64, h: i64) {
    (0..w * h).for_each(|i| w16(m, base + i as u32 * 2, direct_value(i % w, i / w)));
}

/// The pixels of the drawn frame that differ from the model's, as (x, y, got, want).
fn failures(e: &Ext) -> Diff {
    let mut m = build(e);
    m.render_frame();
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let want = expected(e, x, y);
            let got = pixel(&m, e.eng.screen, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .take(8)
        .collect()
}

fn with(e: Ext, fmt: Fmt, size_code: u16, wrap: bool, p: Params) -> Ext {
    Ext {
        fmt,
        size_code,
        wrap,
        p,
        ..e
    }
}

const BMP256_A: Ext = Ext {
    fmt: Fmt::Bitmap256,
    ..MAP_A
};
const DIRECT_A: Ext = Ext {
    fmt: Fmt::BitmapDirect,
    ..MAP_A
};
const MAP_B: Ext = Ext {
    eng: ENG_B,
    layer: 3,
    bg_mode: 3,
    ..MAP_A
};

// ------------------------------------------------------------ tile layers, 16-bit entries

#[test]
fn given_a_16_bit_map_with_flips_when_drawn_then_every_pixel_is_the_flipped_texel() {
    assert_eq!(failures(&MAP_A), NONE);
}

#[test]
fn given_a_16_bit_map_with_a_rotation_when_drawn_with_wrap_then_the_picture_matches_the_model() {
    assert_eq!(failures(&with(MAP_A, Fmt::Map16, 1, true, SHEAR)), NONE);
}

#[test]
fn given_a_16_bit_map_with_a_rotation_when_drawn_without_wrap_then_outside_is_transparent() {
    assert_eq!(failures(&with(MAP_A, Fmt::Map16, 1, false, SHEAR)), NONE);
}

#[test]
fn given_a_16_bit_map_of_each_size_when_drawn_then_the_picture_matches_the_model() {
    let sizes = [0u16, 2, 3].map(|code| failures(&with(MAP_A, Fmt::Map16, code, true, SHEAR)));
    assert_eq!(sizes, [NONE, NONE, NONE]);
}

#[test]
fn given_a_character_block_with_bit_0_set_when_the_16_bit_map_is_drawn_then_it_is_still_a_tile_layer()
 {
    // Bit 2 of BGxCNT is the character base's lowest bit here and does not pick the bitmap kind
    // (`bg-modes.md`, "The mode table").
    let e = Ext {
        char_block: 5,
        ..MAP_A
    };
    assert_eq!(failures(&e), NONE);
}

#[test]
fn given_a_16_bit_map_with_a_screen_block_when_drawn_then_the_map_is_read_from_it() {
    let e = Ext { block: 6, ..MAP_A };
    assert_eq!(failures(&e), NONE);
}

#[test]
fn given_entries_with_palette_numbers_when_drawn_with_the_standard_palette_then_the_number_is_not_used()
 {
    // TODO(hw-unverified): `bg-modes.md`, "Unverified": the palette number belongs to the extended
    // palette (`TODO(phase4)`, #32); the standard palette is indexed by the value alone.
    let mut m = build(&MAP_A);
    w16(&mut m, ENG_A.vram, entry(0, 0) | (7 << 12));
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 0, 0), expected(&MAP_A, 0, 0));
}

// -------------------------------------------------------------------- 256-colour bitmaps

#[test]
fn given_a_256_colour_bitmap_when_drawn_then_every_pixel_is_the_bitmap_pixel_through_the_palette() {
    assert_eq!(failures(&BMP256_A), NONE);
}

#[test]
fn given_a_256_colour_bitmap_with_value_0_pixels_when_drawn_then_they_show_the_backdrop() {
    // The model has value 0 on every 11th diagonal: (0, 0) is one.
    let mut m = build(&BMP256_A);
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 0, 0), BACKDROP);
}

#[test]
fn given_a_256_colour_bitmap_of_each_size_when_drawn_then_the_picture_matches_the_model() {
    let sizes =
        [0u16, 2, 3].map(|code| failures(&with(BMP256_A, Fmt::Bitmap256, code, true, SHEAR)));
    assert_eq!(sizes, [NONE, NONE, NONE]);
}

#[test]
fn given_a_256_colour_bitmap_with_wrap_off_when_drawn_with_a_rotation_then_outside_is_transparent()
{
    assert_eq!(
        failures(&with(BMP256_A, Fmt::Bitmap256, 1, false, SHEAR)),
        NONE
    );
}

#[test]
fn given_a_256_colour_bitmap_at_a_screen_block_when_drawn_then_it_starts_16_kib_a_block_in() {
    let e = Ext {
        block: 3,
        ..BMP256_A
    };
    assert_eq!(failures(&e), NONE);
}

#[test]
fn given_a_dispcnt_screen_base_when_a_bitmap_is_drawn_then_it_is_not_added_to_the_bitmap_base() {
    // The screen base in DISPCNT applies to maps, not to bitmaps (`bg-modes.md`, "Sizes and where
    // the data is"): bits 27-29 = 5 would move a map 320 KiB.
    let e = Ext {
        extra: 5 << 27,
        ..BMP256_A
    };
    assert_eq!(failures(&e), NONE);
}

#[test]
fn given_a_dispcnt_character_base_when_a_bitmap_is_drawn_then_it_is_not_used() {
    let e = Ext {
        extra: 3 << 24,
        ..BMP256_A
    };
    assert_eq!(failures(&e), NONE);
}

// --------------------------------------------------------------- direct-colour bitmaps

#[test]
fn given_a_direct_colour_bitmap_when_drawn_then_pixels_with_bit_15_set_show_their_colour() {
    assert_eq!(failures(&DIRECT_A), NONE);
}

#[test]
fn given_a_direct_colour_bitmap_of_each_size_when_drawn_then_the_picture_matches_the_model() {
    let sizes =
        [0u16, 2, 3].map(|code| failures(&with(DIRECT_A, Fmt::BitmapDirect, code, true, SHEAR)));
    assert_eq!(sizes, [NONE, NONE, NONE]);
}

#[test]
fn given_a_direct_colour_bitmap_with_a_rotation_and_wrap_off_when_drawn_then_outside_is_transparent()
 {
    assert_eq!(
        failures(&with(DIRECT_A, Fmt::BitmapDirect, 1, false, SHEAR)),
        NONE
    );
}

#[test]
fn given_direct_colour_values_with_and_without_bit_15_when_drawn_then_only_set_ones_are_shown() {
    let mut m = build(&DIRECT_A);
    // 7FFFh (white, bit 15 clear) is transparent, 8000h is opaque black, FC1Fh is magenta.
    w16(&mut m, ENG_A.vram, 0x7FFF);
    w16(&mut m, ENG_A.vram + 2, 0x8000);
    w16(&mut m, ENG_A.vram + 4, 0xFC1F);
    m.render_frame();
    assert_eq!(
        (
            pixel(&m, Screen::Top, 0, 0),
            pixel(&m, Screen::Top, 1, 0),
            pixel(&m, Screen::Top, 2, 0)
        ),
        (BACKDROP, 0x0000_0000, 0x00FF_00FF)
    );
}

// ------------------------------------------------------------------------- engine B

#[test]
fn given_a_16_bit_map_on_engine_b_when_drawn_then_the_lower_screen_matches_the_model() {
    assert_eq!(failures(&with(MAP_B, Fmt::Map16, 1, true, SHEAR)), NONE);
}

#[test]
fn given_a_256_colour_bitmap_on_engine_b_when_drawn_then_the_lower_screen_matches_the_model() {
    assert_eq!(failures(&with(MAP_B, Fmt::Bitmap256, 2, true, SHEAR)), NONE);
}

#[test]
fn given_a_direct_colour_bitmap_on_engine_b_when_drawn_then_the_lower_screen_matches_the_model() {
    assert_eq!(
        failures(&with(MAP_B, Fmt::BitmapDirect, 1, false, SHEAR)),
        NONE
    );
}

// --------------------------------------------------------------------- modes and decode

/// The cases (BG mode, layer, BGxCNT, wanted format) that decode to another format on engine A.
fn format_failures(cases: &[(u32, usize, u16, AffineFormat)]) -> Vec<(u32, usize, u16)> {
    cases
        .iter()
        .filter_map(|&(mode, layer, cnt, want)| {
            let mut m = Machine::new();
            w32(&mut m, DISPCNT_A_ADDR, mode);
            w16(&mut m, BG_REGS_ADDR + layer as u32 * 2, cnt);
            let got = m.inspect().gpu2d().affine_bg(EngineId::A, layer).format;
            (got != want).then_some((mode, layer, cnt))
        })
        .collect()
}

#[test]
fn given_each_bg_mode_and_bgcnt_when_decoded_then_the_format_follows_the_modes_kind_and_bits_7_and_2()
 {
    use AffineFormat::{Bitmap256, BitmapDirect, Tiles8, Tiles16};
    let cases = [
        (1, 3, 0x0000, Tiles8),
        (2, 2, 0x0084, Tiles8),
        (2, 3, 0x0000, Tiles8),
        (4, 2, 0x0084, Tiles8),
        (3, 3, 0x0000, Tiles16),
        (3, 3, 0x0004, Tiles16),
        (3, 3, 0x0080, Bitmap256),
        (3, 3, 0x0084, BitmapDirect),
        (4, 3, 0x0080, Bitmap256),
        (5, 2, 0x0084, BitmapDirect),
        (5, 3, 0x0000, Tiles16),
        // In mode 3 BG2 is a text layer, so its bits do not make it a bitmap.
        (3, 2, 0x0084, Tiles8),
        (0, 3, 0x0084, Tiles8),
    ];
    assert_eq!(format_failures(&cases), vec![]);
}

#[test]
fn given_bitmap_sizes_when_decoded_then_the_dimensions_and_the_16_kib_base_follow_the_note() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 5 | (3 << 27));
    // BG2: 256-colour bitmap, size 2 (512x256), screen block 3. BG3: direct, size 3, block 31.
    w16(&mut m, BG_REGS_ADDR + 4, 0x0080 | (3 << 8) | (2 << 14));
    w16(&mut m, BG_REGS_ADDR + 6, 0x0084 | (31 << 8) | (3 << 14));
    let gpu = m.inspect().gpu2d();
    let (a, b) = (gpu.affine_bg(EngineId::A, 2), gpu.affine_bg(EngineId::A, 3));
    assert_eq!(
        (
            a.width,
            a.height,
            a.screen_base,
            b.width,
            b.height,
            b.screen_base
        ),
        (512, 256, 3 * 0x4000, 512, 512, 31 * 0x4000)
    );
}

#[test]
fn given_a_16_bit_tile_layer_when_decoded_then_it_is_square_and_counts_its_map_in_2_kib() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 5 | (2 << 27) | (1 << 24));
    w16(&mut m, BG_REGS_ADDR + 4, (2 << 8) | (3 << 14) | (5 << 2));
    let layer = m.inspect().gpu2d().affine_bg(EngineId::A, 2);
    assert_eq!(
        (
            layer.width,
            layer.height,
            layer.screen_base,
            layer.char_base
        ),
        (1024, 1024, 2 * 0x1_0000 + 2 * 0x800, 0x1_0000 + 5 * 0x4000)
    );
}

#[test]
fn given_engine_b_with_dispcnt_bases_when_a_bitmap_is_decoded_then_the_base_is_the_block_alone() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_B_ADDR, 5 | (5 << 27));
    w16(&mut m, BG_REGS_B_ADDR + 4, 0x0080 | (2 << 8));
    assert_eq!(
        m.inspect().gpu2d().affine_bg(EngineId::B, 2).screen_base,
        2 * 0x4000
    );
}
