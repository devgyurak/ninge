// SPDX-License-Identifier: Apache-2.0
//! Affine objects (issue #31). Every expected value comes from `docs/hw/obj.md` ("An affine
//! object"): for box pixel (`bx`, `by`), `dx = bx - boxW / 2`, `dy = by - boxH / 2`,
//! `u = ((PA * dx + PB * dy) >> 8) + W / 2`, `v = ((PC * dx + PD * dy) >> 8) + H / 2`, and the
//! pixel shows the picture at (`u`, `v`) when that is inside it. Each table below was worked out
//! by hand from those lines for its parameters; none is computed by the code under test.

// TODO(hw-unverified): these tests pin the model's choice where GBATEK is silent: the affine
// arithmetic and its rounding (`obj.md`, "Unverified").
use super::*;

/// attr0 bit 8, and bit 9 for double-size.
const AFFINE: u16 = 1 << 8;
const DOUBLE: u16 = 1 << 9;
const ONE: i16 = 0x100;

/// Writes parameter group `group` of the side's OAM: PA, PB, PC and PD in the fourth halfword of
/// four consecutive entries.
pub(super) fn put_group(m: &mut Machine, side: Side, group: u32, params: [i16; 4]) {
    params.iter().enumerate().for_each(|(n, &value)| {
        w16(m, side.oam + (group * 4 + n as u32) * 8 + 6, value as u16);
    });
}

/// An 8x8 affine object (the asymmetric tile 1) at (`x`, `y`) using group `group`, with the
/// parameters `params`, in an engine A scene.
fn asym_scene(x: u16, y: u16, attr0_bits: u16, params: [i16; 4]) -> Machine {
    let mut m = scene(SIDE_A, 0);
    put_asym4(&mut m, SIDE_A, 32);
    put_obj(&mut m, SIDE_A, 0, [y | AFFINE | attr0_bits, x, 1]);
    put_group(&mut m, SIDE_A, 0, params);
    m
}

/// The cases of an 8x8 box at (`x`, `y`) whose pixel (`bx`, `by`) shows picture pixel
/// `source(bx, by)`, or the backdrop for `None`.
fn box_cases(
    x: usize,
    y: usize,
    box_size: usize,
    source: impl Fn(usize, usize) -> Option<(u32, u32)>,
) -> Vec<(usize, usize, u32)> {
    (0..box_size)
        .flat_map(|by| (0..box_size).map(move |bx| (bx, by)))
        .map(|(bx, by)| {
            let want = source(bx, by).map_or(BACKDROP, |(row, column)| color(asym4(row, column)));
            (x + bx, y + by, want)
        })
        .collect()
}

#[test]
fn given_the_identity_transform_when_an_affine_object_is_drawn_then_it_looks_like_a_plain_one() {
    let mut m = asym_scene(10, 20, 0, [ONE, 0, 0, ONE]);
    let cases = box_cases(10, 20, 8, |bx, by| Some((by as u32, bx as u32)));
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_the_identity_and_double_size_when_drawn_then_the_picture_sits_in_the_middle_of_a_16x16_box()
 {
    // Box 16x16, centre 8: u = bx - 8 + 4, so the picture covers box pixels 4-11.
    let mut m = asym_scene(10, 20, DOUBLE, [ONE, 0, 0, ONE]);
    let cases = box_cases(10, 20, 16, |bx, by| {
        let inside = |n: usize| (4..12).contains(&n);
        (inside(bx) && inside(by)).then(|| (by as u32 - 4, bx as u32 - 4))
    });
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_a_magnifying_transform_when_drawn_then_each_picture_pixel_covers_2x2_box_pixels() {
    // PA = PD = 0.5: u = floor(dx / 2) + 4 with dx = bx - 4, so box columns 0-7 show picture
    // columns 2, 2, 3, 3, 4, 4, 5, 5.
    let mut m = asym_scene(10, 20, 0, [ONE / 2, 0, 0, ONE / 2]);
    let table = [2u32, 2, 3, 3, 4, 4, 5, 5];
    let cases = box_cases(10, 20, 8, |bx, by| Some((table[by], table[bx])));
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_a_shrinking_transform_when_drawn_then_only_the_middle_of_the_box_shows_every_other_pixel()
{
    // PA = PD = 2.0: u = 2 * dx + 4 = 2 * bx - 4, inside the picture for box pixels 2-5 (u =
    // 0, 2, 4, 6); everything else is cut.
    let mut m = asym_scene(10, 20, 0, [2 * ONE, 0, 0, 2 * ONE]);
    let cases = box_cases(10, 20, 8, |bx, by| {
        let inside = |n: usize| (2..6).contains(&n);
        (inside(bx) && inside(by)).then(|| (2 * by as u32 - 4, 2 * bx as u32 - 4))
    });
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_a_quarter_turn_when_drawn_then_the_picture_is_rotated_about_the_box_centre() {
    // PA = 0, PB = 1, PC = -1, PD = 0: u = dy + 4 = by, v = -dx + 4 = 8 - bx. Column 0 of the box
    // has v = 8, outside the picture, so it is transparent.
    let mut m = asym_scene(10, 20, 0, [0, ONE, -ONE, 0]);
    let cases = box_cases(10, 20, 8, |bx, by| {
        (bx >= 1).then(|| (8 - bx as u32, by as u32))
    });
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

/// How many pixels of the side's screen are not the backdrop after an affine 16x8 solid object
/// is turned a quarter: with `attr0_bits` (0 or `DOUBLE`).
fn quarter_turn_pixels(attr0_bits: u16) -> usize {
    // Tile 0 stays transparent: the other 127 entries are zero, which is a visible 8x8 object of
    // tile 0 at (0, 0) (`obj.md`, "Unverified": the power-on contents).
    let mut m = scene(SIDE_A, MAPPING_1D);
    (2..4).for_each(|t| solid4(&mut m, SIDE_A, t * 32, 1));
    // Shape 1 (wide), size 0: 16x8, tiles 2 and 3.
    put_obj(
        &mut m,
        SIDE_A,
        0,
        [40 | AFFINE | attr0_bits | 1 << 14, 40, 2],
    );
    put_group(&mut m, SIDE_A, 0, [0, ONE, -ONE, 0]);
    m.render_frame();
    drawn_pixels(&m, SIDE_A)
}

#[test]
fn given_a_16x8_object_turned_a_quarter_when_drawn_without_and_with_double_size_then_the_box_cuts_it_only_without()
 {
    // Picture 16x8, u = dy + 8, v = -dx + 4. Box 16x8 (dx = bx - 8, dy = by - 4): u = by + 4 is
    // always inside, v = 12 - bx is inside for bx 5-12: 8 columns x 8 rows = 64 pixels.
    // Box 32x16 (dx = bx - 16, dy = by - 8): u = by, v = 20 - bx inside for bx 13-20: 8 columns x
    // 16 rows = 128 pixels, the whole turned picture.
    assert_eq!(
        (quarter_turn_pixels(0), quarter_turn_pixels(DOUBLE)),
        (64, 128)
    );
}

#[test]
fn given_two_groups_when_two_objects_select_them_then_each_uses_its_own_parameters() {
    // Group 0 is the identity, group 31 a quarter turn; both objects are 8x8 of tile 1.
    let mut m = scene(SIDE_A, 0);
    put_asym4(&mut m, SIDE_A, 32);
    put_obj(&mut m, SIDE_A, 0, [20 | AFFINE, 10, 1]);
    put_obj(&mut m, SIDE_A, 1, [20 | AFFINE, 40 | 31 << 9, 1]);
    put_group(&mut m, SIDE_A, 0, [ONE, 0, 0, ONE]);
    put_group(&mut m, SIDE_A, 31, [0, ONE, -ONE, 0]);
    let mut cases = box_cases(10, 20, 8, |bx, by| Some((by as u32, bx as u32)));
    cases.extend(box_cases(40, 20, 8, |bx, by| {
        (bx >= 1).then(|| (8 - bx as u32, by as u32))
    }));
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_the_same_group_number_in_each_engine_when_drawn_then_each_engine_reads_its_own_oam() {
    // Group 0 is a quarter turn in engine A's OAM and the identity in engine B's.
    let mut m = scene(SIDE_A, 0);
    enable(&mut m, SIDE_B, 0);
    put_asym4(&mut m, SIDE_A, 32);
    put_asym4(&mut m, SIDE_B, 32);
    put_obj(&mut m, SIDE_A, 0, [20 | AFFINE, 10, 1]);
    put_obj(&mut m, SIDE_B, 0, [20 | AFFINE, 10, 1]);
    put_group(&mut m, SIDE_A, 0, [0, ONE, -ONE, 0]);
    put_group(&mut m, SIDE_B, 0, [ONE, 0, 0, ONE]);
    let turned = box_cases(10, 20, 8, |bx, by| {
        (bx >= 1).then(|| (8 - bx as u32, by as u32))
    });
    let plain = box_cases(10, 20, 8, |bx, by| Some((by as u32, bx as u32)));
    assert_eq!(
        (
            misses(&mut m, SIDE_A, &turned),
            misses(&mut m, SIDE_B, &plain)
        ),
        (Misses::new(), Misses::new())
    );
}

#[test]
fn given_a_256_colour_affine_object_when_drawn_then_pixels_come_from_the_whole_palette() {
    // The identity on a 8x8 256-colour tile of value 200, tile 2 (2D mapping, byte 64).
    let mut m = scene(SIDE_A, 0);
    solid8(&mut m, SIDE_A, 64, 200);
    put_obj(&mut m, SIDE_A, 0, [20 | AFFINE | 1 << 13, 10, 2]);
    put_group(&mut m, SIDE_A, 0, [ONE, 0, 0, ONE]);
    let cases = [
        (10, 20, color(200)),
        (17, 27, color(200)),
        (18, 20, BACKDROP),
    ];
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_an_affine_object_with_the_flip_bits_set_in_attr1_when_drawn_then_they_are_the_group_number()
 {
    // attr1 bits 12-13 are group bits (here group 16 and 8 are not what is used: 3 << 12 is group
    // 24), so a group with the identity at 24 draws the plain picture.
    let mut m = scene(SIDE_A, 0);
    put_asym4(&mut m, SIDE_A, 32);
    put_obj(&mut m, SIDE_A, 0, [20 | AFFINE, 10 | 3 << 12, 1]);
    put_group(&mut m, SIDE_A, 24, [ONE, 0, 0, ONE]);
    let cases = box_cases(10, 20, 8, |bx, by| Some((by as u32, bx as u32)));
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}
