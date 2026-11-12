// SPDX-License-Identifier: Apache-2.0
//! Object mosaic (issue #31). Every expected value comes from `docs/hw/obj.md` ("Mosaic"): an
//! object with attr0 bit 12 shows, at screen pixel (`x`, `y`), what it shows without mosaic at
//! (`x - x mod h`, `y - y mod v`), where `h` and `v` are MOSAIC bits 8-11 and 12-15 plus 1. Each
//! scene is rendered twice, plain and with mosaic, and the second is compared with the first by
//! that rule; the plain pictures are pinned by the tests of `mod.rs`, `affine.rs` and `bitmap.rs`.

use super::affine::put_group;
use super::*;

const MOSAIC_BIT: u16 = 1 << 12;
const AFFINE: u16 = 1 << 8;

/// MOSAIC with the OBJ fields for `(h, v)` blocks and the BG fields `bg`.
fn mosaic_value((h, v): (u16, u16), bg: u16) -> u16 {
    (h - 1) << 8 | (v - 1) << 12 | bg
}

fn mosaic_addr(side: Side) -> u32 {
    match side.screen {
        Screen::Top => MOSAIC_ADDR,
        Screen::Bottom => MOSAIC_B_ADDR,
    }
}

/// The tile pattern with holes: row r, column c holds `(3r + c) mod 8`, so value 0 appears.
fn put_holey4(m: &mut Machine, side: Side, at: u32) {
    (0..8u32).for_each(|r| {
        let word = (0..8u32).fold(0u32, |word, c| word | ((3 * r + c) % 8) << (4 * c));
        w32(m, side.vram + at + r * 4, word);
    });
}

/// A machine with a 16x16 object at (`x`, `y`) with attr0 bits `attr0_bits` and MOSAIC `register`:
/// tiles 1, 2, 33 and 34 of the pattern (2D mapping), plus an affine group 0 of 0.75 magnified.
fn scene_of(side: Side, x: u16, y: u16, attr0_bits: u16, register: u16) -> Machine {
    let mut m = scene(side, 0);
    [1, 2, 33, 34].iter().fold((), |(), &t| {
        put_holey4(&mut m, side, t * 32);
    });
    put_obj(&mut m, side, 0, [y | attr0_bits, x | 1 << 14, 1 | 2 << 12]);
    put_group(&mut m, side, 0, [0xC0, 0x20, -0x20, 0xC0]);
    w16(&mut m, mosaic_addr(side), register);
    m
}

/// The pixels of `mosaic`'s screen that are not the pixel `plain` shows at the first pixel of the
/// block, as (x, y, got, want), at most 8.
fn block_failures(
    plain: &mut Machine,
    mosaic: &mut Machine,
    side: Side,
    (h, v): (usize, usize),
) -> Misses {
    plain.render_frame();
    mosaic.render_frame();
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let want = get(plain, side, x - x % h, y - y % v);
            let got = get(mosaic, side, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .take(8)
        .collect()
}

/// The failures of a mosaic object with attr0 bits `attr0_bits` (besides mosaic) at (`x`, `y`)
/// for blocks of `blocks`, on `side`.
fn failures_for(side: Side, (x, y): (u16, u16), attr0_bits: u16, blocks: (u16, u16)) -> Misses {
    let mut plain = scene_of(side, x, y, attr0_bits, 0);
    let mut mosaic = scene_of(side, x, y, attr0_bits | MOSAIC_BIT, mosaic_value(blocks, 0));
    block_failures(
        &mut plain,
        &mut mosaic,
        side,
        (usize::from(blocks.0), usize::from(blocks.1)),
    )
}

#[test]
fn given_a_mosaic_object_when_drawn_then_each_block_shows_its_first_pixel() {
    assert_eq!(failures_for(SIDE_A, (20, 30), 0, (5, 3)), Misses::new());
}

#[test]
fn given_blocks_of_other_sizes_when_a_mosaic_object_is_drawn_then_each_block_shows_its_first_pixel()
{
    let failures: Vec<_> = [(2u16, 1u16), (1, 4), (16, 16), (3, 7)]
        .into_iter()
        .flat_map(|blocks| failures_for(SIDE_A, (21, 29), 0, blocks))
        .collect();
    assert_eq!(failures, Misses::new());
}

#[test]
fn given_a_mosaic_object_on_engine_b_when_drawn_then_engine_b_s_own_register_sizes_the_blocks() {
    assert_eq!(failures_for(SIDE_B, (20, 30), 0, (2, 4)), Misses::new());
}

#[test]
fn given_a_flipped_mosaic_object_when_drawn_then_each_block_shows_its_first_screen_pixel() {
    // The pick is on the screen, the flips are part of what the object shows there.
    let flips = 1 << 12 | 1 << 13;
    let failures = {
        let mut plain = scene_of(SIDE_A, 20, 30, 0, 0);
        put_obj(
            &mut plain,
            SIDE_A,
            0,
            [30, 20 | 1 << 14 | flips, 1 | 2 << 12],
        );
        let mut mosaic = scene_of(SIDE_A, 20, 30, MOSAIC_BIT, mosaic_value((5, 3), 0));
        put_obj(
            &mut mosaic,
            SIDE_A,
            0,
            [30 | MOSAIC_BIT, 20 | 1 << 14 | flips, 1 | 2 << 12],
        );
        block_failures(&mut plain, &mut mosaic, SIDE_A, (5, 3))
    };
    assert_eq!(failures, Misses::new());
}

#[test]
fn given_an_affine_mosaic_object_when_drawn_then_each_block_shows_its_first_screen_pixel() {
    assert_eq!(
        failures_for(SIDE_A, (20, 30), AFFINE, (5, 3)),
        Misses::new()
    );
}

#[test]
fn given_a_double_size_affine_mosaic_object_when_drawn_then_each_block_shows_its_first_screen_pixel()
 {
    assert_eq!(
        failures_for(SIDE_A, (20, 30), AFFINE | 1 << 9, (4, 4)),
        Misses::new()
    );
}

#[test]
fn given_a_mosaic_object_at_a_negative_x_when_drawn_then_blocks_start_at_the_screen_s_column_0() {
    // X = 0x1F6 is -10: blocks are counted from the left of the screen, not of the object.
    assert_eq!(failures_for(SIDE_A, (0x1F6, 30), 0, (4, 3)), Misses::new());
}

#[test]
fn given_a_mosaic_object_past_line_255_when_drawn_then_blocks_are_counted_from_the_top_of_the_screen()
 {
    assert_eq!(failures_for(SIDE_A, (20, 250), 0, (4, 4)), Misses::new());
}

#[test]
fn given_an_object_without_the_mosaic_bit_when_the_register_has_sizes_then_it_is_unchanged() {
    let mut plain = scene_of(SIDE_A, 20, 30, 0, 0);
    let mut other = scene_of(SIDE_A, 20, 30, 0, mosaic_value((9, 9), 0));
    assert_eq!(
        block_failures(&mut plain, &mut other, SIDE_A, (1, 1)),
        Misses::new()
    );
}

#[test]
fn given_a_mosaic_object_with_only_bg_sizes_in_the_register_when_drawn_then_it_is_unchanged() {
    // MOSAIC bits 0-7 are the BG fields (`bg-modes.md`); the OBJ fields are 0.
    let mut plain = scene_of(SIDE_A, 20, 30, 0, 0);
    let mut other = scene_of(SIDE_A, 20, 30, MOSAIC_BIT, 0x00F7);
    assert_eq!(
        block_failures(&mut plain, &mut other, SIDE_A, (1, 1)),
        Misses::new()
    );
}

/// A 16x16 bitmap object (2D mapping 128 wide, tile 0x20, data from byte 0x1000, one pixel in five
/// transparent) at (20, 30) with attr0 bits `attr0_bits` and MOSAIC `register`.
fn bitmap_scene(attr0_bits: u16, register: u16) -> Machine {
    let mut m = scene(SIDE_A, 0);
    (0..16u32).for_each(|r| {
        (0..16u32).for_each(|c| {
            let half = if (r + 2 * c) % 5 == 0 {
                0
            } else {
                0x8000 | (r * 16 + c) as u16
            };
            w16(&mut m, SIDE_A.vram + 0x1000 + r * 256 + c * 2, half);
        });
    });
    put_obj(
        &mut m,
        SIDE_A,
        0,
        [30 | 3 << 10 | attr0_bits, 20 | 1 << 14, 0x20],
    );
    w16(&mut m, MOSAIC_ADDR, register);
    m
}

#[test]
fn given_a_bitmap_mosaic_object_when_drawn_then_each_block_shows_its_first_pixel() {
    // Bitmap objects share the code with tile objects.
    let mut plain = bitmap_scene(0, 0);
    let mut mosaic = bitmap_scene(MOSAIC_BIT, mosaic_value((4, 3), 0));
    assert_eq!(
        block_failures(&mut plain, &mut mosaic, SIDE_A, (4, 3)),
        Misses::new()
    );
}
