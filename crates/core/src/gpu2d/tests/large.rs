// SPDX-License-Identifier: Apache-2.0
//! The large bitmap of BG mode 6 (issue #30). Every expected value comes from `docs/hw/bg-modes.md`
//! ("Large bitmap", "Sizes and where the data is", "Engine B"), worked out by the model of
//! `extended.rs`, never from running the renderer.

use super::affine::{Diff, ENG_B, IDENTITY, NONE, Params, SHEAR};
use super::extended::{BACKDROP, Ext, Fmt, MAP_A, build, expected, failures, with};
use super::*;
use crate::debug::AffineFormat;

/// Engine A, BG mode 6, BG2 the large bitmap: 512x1024 (size 0), wrapping, identity.
const LARGE_A: Ext = Ext {
    fmt: Fmt::Large,
    bg_mode: 6,
    size_code: 0,
    ..MAP_A
};

/// A step of two texels a pixel, from a point far into the bitmap.
const ZOOMED: Params = Params {
    pa: 0x200,
    pd: 0x200,
    x: 300 * 256,
    y: 700 * 256,
    ..IDENTITY
};

fn large(size_code: u16, wrap: bool, p: Params) -> Diff {
    failures(&with(LARGE_A, Fmt::Large, size_code, wrap, p))
}

// ------------------------------------------------------------------------ the picture

#[test]
fn given_a_512_by_1024_bitmap_when_drawn_then_every_pixel_is_the_bitmap_pixel_through_the_palette()
{
    assert_eq!(large(0, true, IDENTITY), NONE);
}

#[test]
fn given_a_1024_by_512_bitmap_when_drawn_then_every_pixel_is_the_bitmap_pixel_through_the_palette()
{
    assert_eq!(large(1, true, IDENTITY), NONE);
}

#[test]
fn given_a_zoomed_out_view_deep_in_a_512_by_1024_bitmap_when_drawn_then_the_picture_matches_the_model()
 {
    // Rows 700-1084 wrap at 1024, so the last rows of the bitmap and the first ones both show.
    assert_eq!(large(0, true, ZOOMED), NONE);
}

#[test]
fn given_a_zoomed_out_view_deep_in_a_1024_by_512_bitmap_when_drawn_then_the_picture_matches_the_model()
 {
    assert_eq!(large(1, true, ZOOMED), NONE);
}

#[test]
fn given_a_rotation_when_the_large_bitmap_wraps_then_the_picture_matches_the_model() {
    assert_eq!((large(0, true, SHEAR), large(1, true, SHEAR)), (NONE, NONE));
}

#[test]
fn given_a_rotation_when_the_large_bitmap_does_not_wrap_then_outside_it_is_transparent() {
    assert_eq!(
        (large(0, false, SHEAR), large(1, false, SHEAR)),
        (NONE, NONE)
    );
}

#[test]
fn given_a_view_across_the_end_of_the_bitmap_when_wrap_is_off_then_the_rest_is_transparent() {
    let p = Params {
        y: 1000 * 256,
        ..IDENTITY
    };
    assert_eq!(large(0, false, p), NONE);
}

#[test]
fn given_a_screen_block_in_bgcnt_when_the_large_bitmap_is_drawn_then_it_still_starts_at_the_window()
{
    // The screen block is not used by the large bitmap (`bg-modes.md`, "Sizes and where the data
    // is"); block 7 would be 112 KiB in for any other bitmap.
    let e = Ext {
        block: 7,
        ..LARGE_A
    };
    assert_eq!(failures(&e), NONE);
}

#[test]
fn given_the_dispcnt_bases_when_the_large_bitmap_is_drawn_then_they_are_not_added() {
    let e = Ext {
        extra: (3 << 24) | (5 << 27),
        ..LARGE_A
    };
    assert_eq!(failures(&e), NONE);
}

/// VRAMCNT_B..D off: only bank A's 128 KiB stay in the BG window.
fn disable_banks_b_to_d(m: &mut Machine) {
    [0x0400_0241, 0x0400_0242, 0x0400_0243]
        .iter()
        .for_each(|&bank| w8(m, bank, 0));
}

#[test]
fn given_only_bank_a_mapped_when_a_large_bitmap_is_drawn_then_rows_past_its_128_kib_are_transparent()
 {
    // 128 KiB hold 256 rows of a 512-wide bitmap: rows 200-255 show, the rest is backdrop.
    let e = Ext {
        p: Params {
            y: 200 * 256,
            ..IDENTITY
        },
        ..LARGE_A
    };
    let mut m = build(&e);
    disable_banks_b_to_d(&mut m);
    m.render_frame();
    assert_eq!(
        (
            pixel(&m, Screen::Top, 7, 3) == expected(&e, 7, 3),
            pixel(&m, Screen::Top, 7, 55) == expected(&e, 7, 55),
            pixel(&m, Screen::Top, 7, 56),
            pixel(&m, Screen::Top, 200, 191)
        ),
        (true, true, BACKDROP, BACKDROP)
    );
}

#[test]
fn given_the_layers_other_than_bg2_on_in_mode_6_when_drawn_then_they_draw_nothing() {
    // BG1 and BG3 are none in mode 6; BG0 is the 3D layer (`TODO(phase7)`). They are on, with
    // priority 0, and would cover the bitmap if they drew the bitmap's bytes as text maps.
    let e = Ext {
        extra: (1 << 8) | (1 << 9) | (1 << 11),
        ..LARGE_A
    };
    assert_eq!(failures(&e), NONE);
}

#[test]
fn given_a_size_that_is_not_defined_when_the_large_bitmap_is_drawn_then_only_the_backdrop_shows() {
    // TODO(hw-unverified): `bg-modes.md`, "Unverified": sizes 2 and 3 are not defined.
    let mut m = build(&Ext {
        size_code: 2,
        ..LARGE_A
    });
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, BACKDROP), 0);
}

// ------------------------------------------------------------------------- engine B

#[test]
fn given_bg_mode_6_on_engine_b_when_drawn_then_it_has_no_large_bitmap_and_shows_the_backdrop() {
    // Engine B has no BG mode 6 (`bg-modes.md`, "Engine B"). TODO(hw-unverified): what a
    // reserved mode draws is not stated; here, no layer.
    let mut m = build(&Ext {
        eng: ENG_B,
        ..LARGE_A
    });
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Bottom, BACKDROP), 0);
}

// -------------------------------------------------------------------------- decode

/// The decoded large bitmap of BG2 on engine A for BGxCNT `cnt`: format, width, height, base.
fn decoded(cnt: u16) -> (AffineFormat, u16, u16, u32) {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 6 | (2 << 27));
    w16(&mut m, BG_REGS_ADDR + 4, cnt);
    let l = m.inspect().gpu2d().affine_bg(EngineId::A, 2);
    (l.format, l.width, l.height, l.screen_base)
}

#[test]
fn given_bg_mode_6_when_bg2_is_decoded_then_it_is_a_256_colour_bitmap_of_the_sizes_in_the_note() {
    assert_eq!(
        [
            decoded(0x0080),
            decoded(0x4080 | (9 << 8)),
            decoded(0x8080),
            decoded(0xC080)
        ],
        [
            (AffineFormat::Bitmap256, 512, 1024, 0),
            (AffineFormat::Bitmap256, 1024, 512, 0),
            (AffineFormat::Bitmap256, 0, 0, 0),
            (AffineFormat::Bitmap256, 0, 0, 0),
        ]
    );
}
