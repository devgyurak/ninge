// SPDX-License-Identifier: Apache-2.0
//! Plain tile objects and priority (issue #31). Every expected value comes from
//! `docs/hw/obj.md` ("The object table", "Where an object is on the screen", "A plain tile
//! object", "Priority"), never from the implementation. The 5-bit to 8-bit expansion
//! `(c << 3) | (c >> 2)` is the note's host choice.

use super::*;
use crate::debug::{EngineId, ObjAttrs};

mod affine;
mod bitmap;
mod extpal;
mod limit;
mod mosaic;

const VRAMCNT_B: u32 = 0x0400_0241;
const VRAMCNT_D: u32 = 0x0400_0243;
/// Bank A as engine A's OBJ VRAM (MST 2) and bank D as engine B's (MST 4).
const OBJ_BANK_A: u8 = 0x82;
const OBJ_BANK_D: u8 = 0x84;
/// Bank B as engine A's BG VRAM (MST 1, OFS 0).
const BG_BANK_B: u8 = 0x81;
const BG_A: u32 = 0x0600_0000;
const OAM_A: u32 = 0x0700_0000;
const OAM_B: u32 = 0x0700_0400;
const OBJ_PALETTE_A: u32 = 0x0500_0200;
const OBJ_PALETTE_B: u32 = 0x0500_0600;
const OBJ_VRAM_A: u32 = 0x0640_0000;
const OBJ_VRAM_B: u32 = 0x0660_0000;
/// DISPCNT: display mode 1, sprites on.
const GRAPHICS_OBJ: u32 = 1 << 16 | 1 << 12;
const MAPPING_1D: u32 = 1 << 4;
/// DISPCNT bits 8 (BG0 on) for the background tests.
const BG0_ON: u32 = 1 << 8;
const BACKDROP: u32 = 0;

/// One of the two engines and where its sprite memory is.
#[derive(Clone, Copy)]
struct Side {
    screen: Screen,
    dispcnt: u32,
    palette: u32,
    vramcnt: u32,
    bank: u8,
    vram: u32,
    oam: u32,
}

const SIDE_A: Side = Side {
    screen: Screen::Top,
    dispcnt: DISPCNT_A_ADDR,
    palette: OBJ_PALETTE_A,
    vramcnt: VRAMCNT_A,
    bank: OBJ_BANK_A,
    vram: OBJ_VRAM_A,
    oam: OAM_A,
};

const SIDE_B: Side = Side {
    screen: Screen::Bottom,
    dispcnt: DISPCNT_B_ADDR,
    palette: OBJ_PALETTE_B,
    vramcnt: VRAMCNT_D,
    bank: OBJ_BANK_D,
    vram: OBJ_VRAM_B,
    oam: OAM_B,
};

/// The colour of OBJ palette entry `i`: red `i mod 32`, blue `i / 32`, as a 15-bit value.
fn entry(i: u32) -> u16 {
    ((i & 31) | ((i >> 5) << 10)) as u16
}

/// The xRGB pixel entry `i` gives.
const fn color(i: u32) -> u32 {
    let (red, blue) = (i & 31, i >> 5);
    (((red << 3) | (red >> 2)) << 16) | ((blue << 3) | (blue >> 2))
}

/// A machine with both engines on and the side set up by [`enable`].
fn scene(side: Side, extra: u32) -> Machine {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    enable(&mut m, side, extra);
    m
}

/// Maps the side's OBJ bank, fills its OBJ palette, and turns sprites on in display mode 1 with
/// `extra` DISPCNT bits.
fn enable(m: &mut Machine, side: Side, extra: u32) {
    w8(m, side.vramcnt, side.bank);
    (0..256).for_each(|i| w16(m, side.palette + i * 2, entry(i)));
    w32(m, side.dispcnt, GRAPHICS_OBJ | extra);
}

/// Writes entry `index` of the side's OAM.
fn put_obj(m: &mut Machine, side: Side, index: u32, attrs: [u16; 3]) {
    attrs
        .iter()
        .enumerate()
        .for_each(|(n, &half)| w16(m, side.oam + index * 8 + n as u32 * 2, half));
}

/// Writes `value` into every pixel of the 4 bits per pixel tile at byte `at` of the OBJ VRAM.
fn solid4(m: &mut Machine, side: Side, at: u32, value: u32) {
    (0..8).for_each(|n| w32(m, side.vram + at + n * 4, value * 0x1111_1111));
}

/// Writes `value` into every pixel of the 8 bits per pixel tile at byte `at`.
fn solid8(m: &mut Machine, side: Side, at: u32, value: u32) {
    (0..16).for_each(|n| w32(m, side.vram + at + n * 4, value * 0x0101_0101));
}

/// The asymmetric tile: row r, column c holds `1 + ((3r + c) mod 8)`, at 4 bits per pixel.
fn asym4(r: u32, c: u32) -> u32 {
    1 + (3 * r + c) % 8
}

fn put_asym4(m: &mut Machine, side: Side, at: u32) {
    (0..8).for_each(|r| {
        let word = (0..8).fold(0u32, |word, c| word | asym4(r, c) << (4 * c));
        w32(m, side.vram + at + r * 4, word);
    });
}

fn get(m: &Machine, side: Side, x: usize, y: usize) -> u32 {
    pixel(m, side.screen, x, y)
}

/// The (x, y, got, want) of the cases that mismatch after a frame is drawn.
type Misses = Vec<(usize, usize, u32, u32)>;

fn misses(m: &mut Machine, side: Side, cases: &[(usize, usize, u32)]) -> Misses {
    m.render_frame();
    cases
        .iter()
        .filter_map(|&(x, y, want)| {
            let got = get(m, side, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .collect()
}

/// How many pixels of the side's screen are not the backdrop.
fn drawn_pixels(m: &Machine, side: Side) -> usize {
    not_equal_to(m, side.screen, BACKDROP)
}

/// The 64 pixel cases of an 8x8 asymmetric tile at (`x0`, `y0`): through `palette` (16-colour).
fn asym_cases(x0: usize, y0: usize, palette: u32, flip: (bool, bool)) -> Vec<(usize, usize, u32)> {
    (0..8u32)
        .flat_map(|r| (0..8u32).map(move |c| (r, c)))
        .map(|(r, c)| {
            let (sr, sc) = (
                if flip.1 { 7 - r } else { r },
                if flip.0 { 7 - c } else { c },
            );
            (
                x0 + c as usize,
                y0 + r as usize,
                color(palette * 16 + asym4(sr, sc)),
            )
        })
        .collect()
}

// ------------------------------------------------------------------------------ decoding

fn entry_bytes(attrs: [u16; 3]) -> Vec<u8> {
    attrs
        .iter()
        .flat_map(|half| half.to_le_bytes())
        .chain([0, 0])
        .collect()
}

#[test]
fn given_an_entry_of_a_plain_object_when_decoded_then_every_field_is_the_note_s() {
    // attr0: Y 0x34, mode 1, mosaic, 256 colours, shape 1 (wide). attr1: X 0x1F0 (-16), flips,
    // size 2. attr2: tile 0x155, priority 2, palette 9.
    let bytes = entry_bytes([
        0x34 | 1 << 10 | 1 << 12 | 1 << 13 | 1 << 14,
        0x1F0 | 1 << 12 | 1 << 13 | 2 << 14,
        0x155 | 2 << 10 | 9 << 12,
    ]);
    assert_eq!(
        ObjAttrs::decode(&bytes, 0),
        ObjAttrs {
            y: 0x34,
            x: -16,
            affine: false,
            double_size: false,
            disabled: false,
            mode: 1,
            mosaic: true,
            eight_bpp: true,
            shape: 1,
            size: 2,
            h_flip: true,
            v_flip: true,
            group: 0,
            tile: 0x155,
            priority: 2,
            palette: 9,
        }
    );
}

#[test]
fn given_an_entry_of_an_affine_object_when_decoded_then_bit_9_is_double_size_and_attr1_holds_the_group()
 {
    let bytes = entry_bytes([1 << 8 | 1 << 9, 21 << 9 | 3 << 14, 0]);
    let obj = ObjAttrs::decode(&bytes, 0);
    assert_eq!(
        (
            obj.affine,
            obj.double_size,
            obj.disabled,
            obj.group,
            obj.h_flip,
            obj.v_flip
        ),
        (true, true, false, 21, false, false)
    );
}

#[test]
fn given_a_plain_object_with_bit_9_when_decoded_then_it_is_disabled() {
    let obj = ObjAttrs::decode(&entry_bytes([1 << 9, 0, 0]), 0);
    assert_eq!((obj.disabled, obj.double_size), (true, false));
}

#[test]
fn given_x_values_around_the_sign_bit_when_decoded_then_they_are_9_bit_signed() {
    let x = |attr1: u16| ObjAttrs::decode(&entry_bytes([0, attr1, 0]), 0).x;
    assert_eq!([x(0), x(255), x(256), x(511)], [0, 255, -256, -1]);
}

#[test]
fn given_every_shape_and_size_when_asked_for_dimensions_then_they_are_the_table_s() {
    let dimensions = |shape: u16, size: u16| {
        ObjAttrs::decode(&entry_bytes([shape << 14, size << 14, 0]), 0).dimensions()
    };
    let table = [
        [(8, 8), (16, 16), (32, 32), (64, 64)],
        [(16, 8), (32, 8), (32, 16), (64, 32)],
        [(8, 16), (8, 32), (16, 32), (32, 64)],
    ];
    let got: Vec<_> = (0..3)
        .map(|shape| std::array::from_fn(|size| dimensions(shape, size as u16).unwrap_or((0, 0))))
        .collect();
    assert_eq!(got, table);
}

#[test]
fn given_shape_3_when_asked_for_dimensions_then_there_are_none() {
    assert_eq!(
        ObjAttrs::decode(&entry_bytes([3 << 14, 0, 0]), 0).dimensions(),
        None
    );
}

#[test]
fn given_a_short_oam_when_decoded_then_the_missing_entry_is_all_zero() {
    assert_eq!(
        ObjAttrs::decode(&[], 5),
        ObjAttrs::decode(&entry_bytes([0, 0, 0]), 0)
    );
}

#[test]
fn given_oam_written_through_the_bus_when_inspected_then_each_engine_has_its_own_entry() {
    let mut m = Machine::new();
    put_obj(&mut m, SIDE_A, 3, [0x0007, 0x0009, 0x0123]);
    put_obj(&mut m, SIDE_B, 3, [0x0005, 0x0002, 0x0111]);
    let (a, b) = (
        m.inspect().obj(EngineId::A, 3),
        m.inspect().obj(EngineId::B, 3),
    );
    assert_eq!(
        ((a.y, a.x, a.tile), (b.y, b.x, b.tile)),
        ((7, 9, 0x123), (5, 2, 0x111))
    );
}

// -------------------------------------------------------------------------- tile objects

#[test]
fn given_a_plain_8x8_16_colour_object_when_drawn_then_each_pixel_is_its_tile_value_in_palette_number()
 {
    let mut m = scene(SIDE_A, 0);
    put_asym4(&mut m, SIDE_A, 32);
    put_obj(&mut m, SIDE_A, 0, [20, 10, 1 | 2 << 12]);
    let cases = asym_cases(10, 20, 2, (false, false));
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_the_same_object_on_engine_b_when_drawn_then_the_lower_screen_shows_it() {
    let mut m = scene(SIDE_B, 0);
    put_asym4(&mut m, SIDE_B, 32);
    put_obj(&mut m, SIDE_B, 0, [20, 10, 1 | 2 << 12]);
    let cases = asym_cases(10, 20, 2, (false, false));
    assert_eq!(misses(&mut m, SIDE_B, &cases), Misses::new());
}

#[test]
fn given_each_flip_combination_when_an_object_is_drawn_then_its_pixels_are_mirrored() {
    let failures: Vec<_> = [(false, true), (true, false), (true, true)]
        .into_iter()
        .flat_map(|flip| {
            let mut m = scene(SIDE_A, 0);
            put_asym4(&mut m, SIDE_A, 32);
            let attr1 = 10 | u16::from(flip.0) << 12 | u16::from(flip.1) << 13;
            put_obj(&mut m, SIDE_A, 0, [20, attr1, 1]);
            misses(&mut m, SIDE_A, &asym_cases(10, 20, 0, flip))
        })
        .collect();
    assert_eq!(failures, Misses::new());
}

#[test]
fn given_a_256_colour_object_when_drawn_then_the_pixel_value_indexes_the_whole_palette() {
    let mut m = scene(SIDE_A, 0);
    // Tile 2 is at byte 64 in 2D mapping and takes 64 bytes: every pixel 200, except the first
    // pixel of the first row, which is 0 (transparent).
    solid8(&mut m, SIDE_A, 64, 200);
    w32(&mut m, SIDE_A.vram + 64, 0xC8C8_C800);
    put_obj(&mut m, SIDE_A, 0, [5 | 1 << 13, 6, 2]);
    let cases = [(6, 5, BACKDROP), (7, 5, color(200)), (13, 12, color(200))];
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_a_16_colour_pixel_of_value_0_when_drawn_then_it_is_transparent_in_any_palette() {
    let mut m = scene(SIDE_A, 0);
    solid4(&mut m, SIDE_A, 32, 0);
    put_obj(&mut m, SIDE_A, 0, [0, 0, 1 | 7 << 12]);
    m.render_frame();
    assert_eq!(drawn_pixels(&m, SIDE_A), 0);
}

// ------------------------------------------------------------------------------ mapping

/// A 16x16 object (shape 0, size 1) of tile `tile`, with each tile of the object's grid marked
/// by its own solid value 1-4 at the byte addresses `at` (top-left, top-right, bottom-left,
/// bottom-right), drawn at (0, 0) under `extra` DISPCNT bits. Returns the colour at the centre
/// of each quadrant.
fn quadrants(extra: u32, attr0_bits: u16, tile: u16, at: [u32; 4], eight: bool) -> [u32; 4] {
    let mut m = scene(SIDE_A, extra);
    at.iter().enumerate().for_each(|(n, &addr)| match eight {
        true => solid8(&mut m, SIDE_A, addr, n as u32 + 1),
        false => solid4(&mut m, SIDE_A, addr, n as u32 + 1),
    });
    put_obj(&mut m, SIDE_A, 0, [attr0_bits, 1 << 14, tile]);
    m.render_frame();
    [(4, 4), (12, 4), (4, 12), (12, 12)].map(|(x, y)| get(&m, SIDE_A, x, y))
}

const MARKS: [u32; 4] = [color(1), color(2), color(3), color(4)];

#[test]
fn given_2d_mapping_when_a_16x16_16_colour_object_of_tile_4_is_drawn_then_its_tiles_are_4_5_36_and_37()
 {
    let got = quadrants(0, 0, 4, [4 * 32, 5 * 32, 36 * 32, 37 * 32], false);
    assert_eq!(got, MARKS);
}

#[test]
fn given_2d_mapping_when_a_16x16_256_colour_object_of_tile_4_is_drawn_then_its_tiles_are_4_6_36_and_38()
 {
    let got = quadrants(0, 1 << 13, 4, [4 * 32, 6 * 32, 36 * 32, 38 * 32], true);
    assert_eq!(got, MARKS);
}

#[test]
fn given_2d_mapping_when_a_256_colour_object_has_an_odd_tile_number_then_bit_0_is_ignored() {
    let got = quadrants(0, 1 << 13, 5, [4 * 32, 6 * 32, 36 * 32, 38 * 32], true);
    assert_eq!(got, MARKS);
}

#[test]
fn given_1d_mapping_when_a_16x16_16_colour_object_of_tile_4_is_drawn_then_its_tiles_are_4_5_6_and_7()
 {
    let got = quadrants(MAPPING_1D, 0, 4, [4 * 32, 5 * 32, 6 * 32, 7 * 32], false);
    assert_eq!(got, MARKS);
}

#[test]
fn given_1d_mapping_when_a_16x16_256_colour_object_of_tile_2_is_drawn_then_its_tiles_follow_each_other_64_bytes_apart()
 {
    // Boundary 32: the first tile is at 2 * 32 = 64, the others at 128, 192 and 256.
    let got = quadrants(MAPPING_1D, 1 << 13, 2, [64, 128, 192, 256], true);
    assert_eq!(got, MARKS);
}

#[test]
fn given_1d_boundaries_when_a_16x16_object_of_tile_3_is_drawn_then_the_first_tile_is_at_tile_times_boundary()
 {
    let failures: Vec<_> = [(0u32, 32u32), (1, 64), (2, 128), (3, 256)]
        .into_iter()
        .filter_map(|(field, boundary)| {
            let first = 3 * boundary;
            let at = [first, first + 32, first + 64, first + 96];
            let got = quadrants(MAPPING_1D | field << 20, 0, 3, at, false);
            (got != MARKS).then_some((boundary, got))
        })
        .collect();
    assert_eq!(failures, vec![]);
}

#[test]
fn given_a_tile_beyond_the_obj_window_when_drawn_then_it_is_transparent_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): obj.md "Unverified", what an object reads past its VRAM window.
    // 1D mapping with boundary 256 and tile 1023 starts at 261,888; the window holds 256 KiB
    // and no bank covers the rest of the object.
    let mut m = scene(SIDE_A, MAPPING_1D | 3 << 20);
    put_obj(&mut m, SIDE_A, 0, [0, 1 << 14, 1023]);
    m.render_frame();
    assert_eq!(drawn_pixels(&m, SIDE_A), 0);
}

// ----------------------------------------------------------------------------- the box

/// For each of the 12 shape/size pairs: the width and height of the opaque box of an object whose
/// every tile is solid (a 2 KiB run of value 1 from tile 0, 1D mapping), as (shape, size, w, h)
/// where it differs from the table.
fn size_failures() -> Vec<(u16, u16, usize, usize)> {
    let table = [
        [(8, 8), (16, 16), (32, 32), (64, 64)],
        [(16, 8), (32, 8), (32, 16), (64, 32)],
        [(8, 16), (8, 32), (16, 32), (32, 64)],
    ];
    (0..3u16)
        .flat_map(|shape| (0..4u16).map(move |size| (shape, size)))
        .filter_map(|(shape, size)| {
            let mut m = scene(SIDE_A, MAPPING_1D);
            (0..64).for_each(|t| solid4(&mut m, SIDE_A, t * 32, 1));
            put_obj(&mut m, SIDE_A, 0, [shape << 14, size << 14, 0]);
            m.render_frame();
            let (w, h) = table[shape as usize][size as usize];
            let frame = m.frame(Screen::Top);
            let opaque: Vec<_> = (0..SCREEN_HEIGHT)
                .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| frame[y * SCREEN_WIDTH + x] != BACKDROP)
                .collect();
            let inside = opaque.iter().filter(|&&(x, y)| x < w && y < h).count();
            let total = opaque.len();
            (inside != w * h || total != w * h).then_some((shape, size, inside, total))
        })
        .collect()
}

#[test]
fn given_every_shape_and_size_when_drawn_then_the_object_fills_exactly_its_box() {
    assert_eq!(size_failures(), vec![]);
}

#[test]
fn given_an_object_left_of_the_screen_when_drawn_then_only_the_columns_from_0_on_show() {
    // X = 0x1F8 is -8: a 16x8 object shows its right tile in columns 0-7.
    let mut m = scene(SIDE_A, MAPPING_1D);
    solid4(&mut m, SIDE_A, 32, 1);
    solid4(&mut m, SIDE_A, 64, 2);
    put_obj(&mut m, SIDE_A, 0, [10 | 1 << 14, 0x1F8, 1]);
    let cases = [(0, 10, color(2)), (7, 10, color(2)), (8, 10, BACKDROP)];
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_an_object_past_the_right_edge_when_drawn_then_the_columns_do_not_wrap() {
    // X = 248 with a 16-wide box: columns 248-255 show, nothing appears at column 0.
    let mut m = scene(SIDE_A, MAPPING_1D);
    solid4(&mut m, SIDE_A, 32, 1);
    solid4(&mut m, SIDE_A, 64, 2);
    put_obj(&mut m, SIDE_A, 0, [10 | 1 << 14, 248, 1]);
    let cases = [
        (248, 10, color(1)),
        (255, 10, color(1)),
        (0, 10, BACKDROP),
        (7, 10, BACKDROP),
    ];
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

#[test]
fn given_an_object_past_line_255_when_drawn_then_its_lower_rows_show_at_the_top() {
    // Y = 250, 8x16 object: rows 0-5 are lines 250-255 (off the screen), rows 6-15 are lines 0-9.
    let mut m = scene(SIDE_A, MAPPING_1D);
    solid4(&mut m, SIDE_A, 32, 1);
    solid4(&mut m, SIDE_A, 64, 2);
    put_obj(&mut m, SIDE_A, 0, [250 | 2 << 14, 0, 1]);
    // Row 6 of the 8x16 object is still in its first tile (value 1), rows 8-15 in the second.
    let expected = [
        (0, 0, color(1)),
        (0, 1, color(1)),
        (0, 2, color(2)),
        (0, 9, color(2)),
        (0, 10, BACKDROP),
    ];
    assert_eq!(misses(&mut m, SIDE_A, &expected), Misses::new());
}

#[test]
fn given_an_object_with_the_disable_bit_when_drawn_then_nothing_shows() {
    let mut m = scene(SIDE_A, 0);
    solid4(&mut m, SIDE_A, 32, 1);
    put_obj(&mut m, SIDE_A, 0, [20 | 1 << 9, 10, 1]);
    m.render_frame();
    assert_eq!(drawn_pixels(&m, SIDE_A), 0);
}

#[test]
fn given_sprites_off_in_dispcnt_when_an_object_is_drawn_then_nothing_shows() {
    let mut m = scene(SIDE_A, 0);
    solid4(&mut m, SIDE_A, 32, 1);
    put_obj(&mut m, SIDE_A, 0, [20, 10, 1]);
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    m.render_frame();
    assert_eq!(drawn_pixels(&m, SIDE_A), 0);
}

/// Six objects one below the other at X 100, entry `i` at Y `10 * i`, each of the tile of value 1
/// with attr0 bits `bits[i]`; returns the cases (x, y, expected) of each object's first pixel,
/// given which of them draw.
fn mode_cases(bits: [u16; 6], draws: [bool; 6]) -> Vec<(usize, usize, u32)> {
    let mut m = scene(SIDE_A, 0);
    solid4(&mut m, SIDE_A, 32, 1);
    bits.iter()
        .enumerate()
        .for_each(|(i, &bits)| put_obj(&mut m, SIDE_A, i as u32, [(i as u16 * 10) | bits, 100, 1]));
    let cases: Vec<_> = draws
        .iter()
        .enumerate()
        .map(|(i, &draw)| (100, i * 10, if draw { color(1) } else { BACKDROP }))
        .collect();
    misses(&mut m, SIDE_A, &cases)
        .into_iter()
        .map(|(x, y, got, _)| (x, y, got))
        .collect()
}

#[test]
fn given_objects_of_each_mode_when_drawn_then_only_the_window_mode_and_shape_3_are_skipped() {
    // TODO(phase4): the OBJ window (mode 2, #33) is not drawn yet; mode 1 draws as a normal object
    // (#34). Shape 3 is not allowed. The affine object (entry 4) has zero parameters, so it
    // samples one pixel of its tile everywhere, and that pixel is 1. The bitmap object (entry 3)
    // reads halfwords of the tile data, which has bit 15 clear everywhere, so nothing shows
    // (`bitmap.rs` covers bitmap objects).
    let bits = [0, 1 << 10, 2 << 10, 3 << 10, 1 << 8, 3 << 14];
    assert_eq!(
        mode_cases(bits, [true, true, false, false, true, false]),
        vec![]
    );
}

// ------------------------------------------------------------------------------ priority

/// Two 8x8 objects at the same place: entry 0 of value 1 and priority `p0`, entry 1 of value 2 and
/// priority `p1`. The colour at their pixel.
fn front_of_two(p0: u16, p1: u16) -> u32 {
    let mut m = scene(SIDE_A, 0);
    solid4(&mut m, SIDE_A, 32, 1);
    solid4(&mut m, SIDE_A, 64, 2);
    put_obj(&mut m, SIDE_A, 0, [0, 0, 1 | p0 << 10]);
    put_obj(&mut m, SIDE_A, 1, [0, 0, 2 | p1 << 10]);
    m.render_frame();
    get(&m, SIDE_A, 3, 3)
}

#[test]
fn given_two_objects_of_the_same_priority_when_drawn_then_the_lower_entry_is_in_front() {
    assert_eq!(front_of_two(2, 2), color(1));
}

#[test]
fn given_a_later_entry_with_a_smaller_priority_value_when_drawn_then_it_is_in_front_of_the_earlier()
{
    // The DS orders by (priority, entry): entry 1 at priority 0 beats entry 0 at priority 1.
    assert_eq!(front_of_two(1, 0), color(2));
}

#[test]
fn given_an_earlier_entry_with_a_smaller_priority_value_when_drawn_then_it_is_in_front() {
    assert_eq!(front_of_two(0, 3), color(1));
}

#[test]
fn given_a_transparent_pixel_of_the_front_object_when_drawn_then_the_object_behind_shows() {
    let mut m = scene(SIDE_A, 0);
    solid4(&mut m, SIDE_A, 32, 0);
    w32(&mut m, SIDE_A.vram + 32, 0x0000_0012);
    solid4(&mut m, SIDE_A, 64, 3);
    put_obj(&mut m, SIDE_A, 0, [0, 0, 1]);
    put_obj(&mut m, SIDE_A, 1, [0, 0, 2]);
    // Front object pixels 0 and 1 are 2 and 1; the rest are transparent and show value 3.
    let cases = [
        (0, 0, color(2)),
        (1, 0, color(1)),
        (2, 0, color(3)),
        (0, 1, color(3)),
    ];
    assert_eq!(misses(&mut m, SIDE_A, &cases), Misses::new());
}

/// An engine A scene with a background (BG0, priority `bg`, a tile of value 5 everywhere) and an
/// 8x8 object of value 1 and priority `obj` at (0, 0): the colour at the object and beside it.
fn against_background(bg: u16, obj: u16) -> (u32, u32) {
    let mut m = scene(SIDE_A, BG0_ON);
    w8(&mut m, VRAMCNT_B, BG_BANK_B);
    w16(&mut m, BG_A_PALETTE, 0);
    (0..256).for_each(|i| w16(&mut m, BG_A_PALETTE + i * 2, entry(i + 64)));
    // BG0: character block 0, screen block 1, 16 colours; tile 1 solid value 5.
    w16(&mut m, BG_REGS_ADDR, 1 << 8 | bg);
    (0..8).for_each(|n| w32(&mut m, BG_A + 32 + n * 4, 0x5555_5555));
    (0..512).for_each(|n| w32(&mut m, BG_A + 0x800 + n * 4, 0x0001_0001));
    solid4(&mut m, SIDE_A, 32, 1);
    put_obj(&mut m, SIDE_A, 0, [0, 0, 1 | obj << 10]);
    m.render_frame();
    (get(&m, SIDE_A, 3, 3), get(&m, SIDE_A, 20, 20))
}

/// Entry 0 of the BG palette, which is where BG0 colours come from; the backdrop is entry 0.
const BG_A_PALETTE: u32 = 0x0500_0000;

#[test]
fn given_an_object_and_a_background_of_the_same_priority_when_drawn_then_the_object_is_in_front() {
    // BG colour: palette entry 5 + 64, written as entry(i + 64) at index 5.
    assert_eq!(against_background(1, 1), (color(1), color(5 + 64)));
}

#[test]
fn given_an_object_with_a_smaller_priority_value_than_the_background_when_drawn_then_the_object_is_in_front()
 {
    assert_eq!(against_background(2, 0), (color(1), color(5 + 64)));
}

#[test]
fn given_an_object_with_a_greater_priority_value_than_the_background_when_drawn_then_the_background_is_in_front()
 {
    assert_eq!(against_background(0, 1), (color(5 + 64), color(5 + 64)));
}
