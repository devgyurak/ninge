// SPDX-License-Identifier: Apache-2.0
//! Bitmap objects (issue #31). Every expected value comes from `docs/hw/obj.md` ("A bitmap
//! object"): attr0 mode 3, 15-bit direct colour with bit 15 set meaning opaque, and the 2D and 1D
//! mapping addresses worked out by hand for each case below.

use super::affine::put_group;
use super::*;

const BITMAP: u16 = 3 << 10;
/// DISPCNT bit 5 (2D source bitmap 256 wide), bit 6 (1D mapping) and bit 22 (1D boundary 256).
const WIDE: u32 = 1 << 5;
const ONE_D: u32 = 1 << 6;
const BOUNDARY_256: u32 = 1 << 22;

/// The halfword a bitmap pixel (`r`, `c`) of the test pictures holds: red `c + 1`, green `r + 1`,
/// bit 15 set.
fn picture(r: u32, c: u32) -> u16 {
    (0x8000 | (r + 1) << 5 | (c + 1)) as u16
}

/// The xRGB colour of `picture(r, c)` (red `c + 1`, green `r + 1`, blue 0).
fn picture_color(r: u32, c: u32) -> u32 {
    let channel = |v: u32| (v << 3) | (v >> 2);
    (channel(c + 1) << 16) | (channel(r + 1) << 8)
}

/// Writes a `rows` by `columns` picture at byte `at` of the side's OBJ VRAM, a row every `stride`
/// bytes.
fn put_picture(m: &mut Machine, side: Side, at: u32, rows: u32, columns: u32, stride: u32) {
    (0..rows).for_each(|r| {
        (0..columns).for_each(|c| w16(m, side.vram + at + r * stride + c * 2, picture(r, c)));
    });
}

/// The cases of an 8x8 box at (`x`, `y`) showing `picture(sr, sc)` for box pixel (`r`, `c`) with
/// the source `source`.
fn cases(x: usize, y: usize, source: impl Fn(u32, u32) -> (u32, u32)) -> Vec<(usize, usize, u32)> {
    (0..8u32)
        .flat_map(|r| (0..8u32).map(move |c| (r, c)))
        .map(|(r, c)| {
            let (sr, sc) = source(r, c);
            (x + c as usize, y + r as usize, picture_color(sr, sc))
        })
        .collect()
}

fn plain(r: u32, c: u32) -> (u32, u32) {
    (r, c)
}

/// An 8x8 bitmap object (shape 0, size 0) of tile `tile` at (10, 20) under `extra` DISPCNT bits.
fn eight_by_eight(side: Side, extra: u32, tile: u16, attr2_bits: u16) -> Machine {
    let mut m = scene(side, extra);
    put_obj(&mut m, side, 0, [20 | BITMAP, 10, tile | attr2_bits]);
    m
}

#[test]
fn given_2d_mapping_128_wide_when_a_bitmap_object_of_tile_0x11_is_drawn_then_it_reads_from_byte_0x810()
 {
    // MaskX 0Fh: (0x11 & 0xF) * 0x10 + (0x11 & ~0xF) * 0x80 = 0x10 + 0x800; a row is 256 bytes.
    let mut m = eight_by_eight(SIDE_A, 0, 0x11, 0);
    put_picture(&mut m, SIDE_A, 0x810, 8, 8, 256);
    assert_eq!(misses(&mut m, SIDE_A, &cases(10, 20, plain)), Misses::new());
}

#[test]
fn given_2d_mapping_256_wide_when_a_bitmap_object_of_tile_0x23_is_drawn_then_it_reads_from_byte_0x1030()
 {
    // MaskX 1Fh: (0x23 & 0x1F) * 0x10 + (0x23 & ~0x1F) * 0x80 = 0x30 + 0x1000; a row is 512 bytes.
    let mut m = eight_by_eight(SIDE_A, WIDE, 0x23, 0);
    put_picture(&mut m, SIDE_A, 0x1030, 8, 8, 512);
    assert_eq!(misses(&mut m, SIDE_A, &cases(10, 20, plain)), Misses::new());
}

#[test]
fn given_1d_mapping_when_a_bitmap_object_of_tile_5_is_drawn_then_it_reads_from_byte_640_with_rows_of_its_own_width()
 {
    // Boundary 128: 5 * 128 = 640; an 8-pixel-wide object has 16-byte rows.
    let mut m = eight_by_eight(SIDE_A, ONE_D, 5, 0);
    put_picture(&mut m, SIDE_A, 640, 8, 8, 16);
    assert_eq!(misses(&mut m, SIDE_A, &cases(10, 20, plain)), Misses::new());
}

#[test]
fn given_1d_mapping_when_a_16x16_bitmap_object_is_drawn_then_its_rows_are_32_bytes_apart() {
    // Shape 0 size 1: 16x16, rows of 32 bytes; tile 2 at 256.
    let mut m = scene(SIDE_A, ONE_D);
    put_picture(&mut m, SIDE_A, 256, 16, 16, 32);
    put_obj(&mut m, SIDE_A, 0, [20 | BITMAP, 10 | 1 << 14, 2]);
    let spots = [(0u32, 0u32), (0, 15), (15, 0), (15, 15), (9, 3)];
    let want: Vec<_> = spots
        .iter()
        .map(|&(r, c)| (10 + c as usize, 20 + r as usize, picture_color(r, c)))
        .collect();
    assert_eq!(misses(&mut m, SIDE_A, &want), Misses::new());
}

#[test]
fn given_1d_mapping_with_bit_22_when_a_bitmap_object_of_tile_3_is_drawn_then_engine_a_uses_a_256_byte_boundary()
 {
    // 3 * 256 = 768.
    let mut m = eight_by_eight(SIDE_A, ONE_D | BOUNDARY_256, 3, 0);
    put_picture(&mut m, SIDE_A, 768, 8, 8, 16);
    assert_eq!(misses(&mut m, SIDE_A, &cases(10, 20, plain)), Misses::new());
}

#[test]
fn given_1d_mapping_with_bit_22_when_engine_b_draws_a_bitmap_object_then_it_keeps_the_128_byte_boundary_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): obj.md "Unverified", engine B treats DISPCNT bit 22 as 0. 3 * 128 = 384.
    let mut m = eight_by_eight(SIDE_B, ONE_D | BOUNDARY_256, 3, 0);
    put_picture(&mut m, SIDE_B, 384, 8, 8, 16);
    assert_eq!(misses(&mut m, SIDE_B, &cases(10, 20, plain)), Misses::new());
}

#[test]
fn given_the_reserved_dispcnt_combination_when_a_bitmap_object_is_drawn_then_nothing_shows_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): obj.md "Unverified", bits 5 and 6 both set are reserved.
    // The picture sits at 4096 (tile 32 at boundary 128), away from tile 0, which the 127 zero
    // OAM entries draw at (0, 0).
    let mut m = eight_by_eight(SIDE_A, ONE_D | WIDE, 32, 0);
    put_picture(&mut m, SIDE_A, 4096, 8, 8, 16);
    m.render_frame();
    assert_eq!(drawn_pixels(&m, SIDE_A), 0);
}

#[test]
fn given_pixels_with_bit_15_clear_when_a_bitmap_object_is_drawn_then_they_are_transparent_and_0x8000_is_opaque_black()
 {
    // 1D, tile 32 at byte 4096: pixel 0 is 0x7FFF (bit 15 clear: transparent whatever the colour),
    // pixel 1 is 0x8000 (opaque black), pixel 2 is 0x801F (opaque red 31). The zero OAM entries
    // draw tile 0, so the data stays away from it.
    let mut m = eight_by_eight(SIDE_A, ONE_D, 32, 0);
    w16(&mut m, SIDE_A.vram + 4096, 0x7FFF);
    w16(&mut m, SIDE_A.vram + 4098, 0x8000);
    w16(&mut m, SIDE_A.vram + 4100, 0x801F);
    // Put a backdrop that is not black behind it so that opaque black is visible.
    w16(&mut m, 0x0500_0000, 0x03E0);
    let cases = [(10, 20, 0x0000_FF00), (11, 20, 0), (12, 20, 0x00FF_0000)];
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_alpha_bits_in_attr2_when_a_bitmap_object_is_drawn_then_the_colours_are_unchanged() {
    // TODO(phase4): the alpha field is not applied (#34); a palette number is not used either.
    let mut m = eight_by_eight(SIDE_A, ONE_D, 5, 0xF << 12);
    put_picture(&mut m, SIDE_A, 640, 8, 8, 16);
    assert_eq!(misses(&mut m, SIDE_A, &cases(10, 20, plain)), Misses::new());
}

#[test]
fn given_a_bitmap_object_with_both_flips_when_drawn_then_the_picture_is_mirrored_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): obj.md "Unverified", flips on a bitmap object.
    let mut m = scene(SIDE_A, ONE_D);
    put_picture(&mut m, SIDE_A, 640, 8, 8, 16);
    put_obj(&mut m, SIDE_A, 0, [20 | BITMAP, 10 | 3 << 12, 5]);
    assert_eq!(
        misses(&mut m, SIDE_A, &cases(10, 20, |r, c| (7 - r, 7 - c))),
        Misses::new()
    );
}

#[test]
fn given_an_affine_bitmap_object_with_the_identity_when_drawn_then_it_looks_like_a_plain_one() {
    let mut m = scene(SIDE_A, ONE_D);
    put_picture(&mut m, SIDE_A, 640, 8, 8, 16);
    put_obj(&mut m, SIDE_A, 0, [20 | BITMAP | 1 << 8, 10, 5]);
    put_group(&mut m, SIDE_A, 0, [0x100, 0, 0, 0x100]);
    assert_eq!(misses(&mut m, SIDE_A, &cases(10, 20, plain)), Misses::new());
}
