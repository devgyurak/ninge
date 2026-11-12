// SPDX-License-Identifier: Apache-2.0
//! Engine A's text backgrounds (issue #28). Every expected value comes from
//! `docs/hw/bg-text.md` (GBATEK "DS 2D Video - BG Control", "Text Background Screen", "Color
//! Palettes", "Display Priority"), worked out by hand, never from running the renderer. Colors are
//! chosen so the 5-bit to 8-bit expansion of `display-mode-vram.md` gives round numbers.

use super::*;
use crate::cart::test_image::{Image, words};

const BG_PAL: u32 = 0x0500_0000;
const BG_VRAM: u32 = 0x0600_0000;
const BG0CNT: u32 = 0x0400_0008;
const BG0HOFS: u32 = 0x0400_0010;
/// Display mode 1: graphics display, with no layer switched on.
const MODE1: u32 = 1 << 16;
/// A 256x256 layer, one 2 KiB map block at screen block 1.
const SB1_4BPP: u16 = 0x0100;
const SB1_8BPP: u16 = 0x0180;
/// Where screen block 1 starts, and one map block's size.
const SB1: u32 = BG_VRAM + 0x800;
const BLOCK: u32 = 0x800;

const RED: u32 = 0x00FF_0000;
const GREEN: u32 = 0x0000_FF00;
const BLUE: u32 = 0x0000_00FF;
const YELLOW: u32 = 0x00FF_FF00;
const MAGENTA: u32 = 0x00FF_00FF;
const CYAN: u32 = 0x0000_FFFF;
const WHITE_PIXEL: u32 = 0x00FF_FFFF;
/// Red channel 16: `(16 << 3) | (16 >> 2)`.
const DARK_RED: u32 = 0x0084_0000;
/// The 15-bit colors that expand to the pixels above, in that order.
const PALETTE: [(u32, u16); 8] = [
    (RED, 0x001F),
    (GREEN, 0x03E0),
    (BLUE, 0x7C00),
    (YELLOW, 0x03FF),
    (MAGENTA, 0x7C1F),
    (CYAN, 0x7FE0),
    (WHITE_PIXEL, 0x7FFF),
    (DARK_RED, 0x0010),
];

/// Bank A in engine A's BG window at offset slot `ofs`, engine A on top, display mode 1, and the
/// palette entries 1..=8 holding the colors of [`PALETTE`].
fn text_machine_at(ofs: u8) -> Machine {
    let mut m = Machine::new();
    text_setup(&mut m, ofs);
    m
}

/// The setup of [`text_machine_at`] on a machine that exists already.
fn text_setup(m: &mut Machine, ofs: u8) {
    w8(m, VRAMCNT_A, ENABLE_MST0 | 1 | (ofs << 3));
    w32(m, DISPCNT_A_ADDR, MODE1);
    w16(m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    PALETTE
        .iter()
        .enumerate()
        .for_each(|(i, &(_, color))| set_pal(m, i as u32 + 1, color));
}

fn text_machine() -> Machine {
    text_machine_at(0)
}

fn set_pal(m: &mut Machine, index: u32, color: u16) {
    w16(m, BG_PAL + index * 2, color);
}

/// DISPCNT with display mode 1, BG mode `mode` and layers `layers` (bit n shows BG n).
fn dispcnt(mode: u32, layers: u32) -> u32 {
    MODE1 | mode | (layers << 8)
}

fn bgcnt_addr(layer: u32) -> u32 {
    BG0CNT + 2 * layer
}

/// One map cell of a 32x32 block at `block_base`.
fn cell(m: &mut Machine, block_base: u32, cx: u32, cy: u32, entry: u16) {
    w16(m, block_base + (cy * 32 + cx) * 2, entry);
}

/// An 8-bits-per-pixel tile whose 64 pixels all have the value `value`.
fn solid8(m: &mut Machine, data: u32, tile: u32, value: u8) {
    (0..16).for_each(|i| w32(m, data + tile * 64 + i * 4, u32::from(value) * 0x0101_0101));
}

/// One row of an 8-bits-per-pixel tile.
fn row8(m: &mut Machine, data: u32, tile: u32, row: u32, values: [u8; 8]) {
    let at = data + tile * 64 + row * 8;
    w32(
        m,
        at,
        u32::from_le_bytes([values[0], values[1], values[2], values[3]]),
    );
    w32(
        m,
        at + 4,
        u32::from_le_bytes([values[4], values[5], values[6], values[7]]),
    );
}

/// One row of a 4-bits-per-pixel tile: the left pixel is the low nibble of the first byte.
fn row4(m: &mut Machine, data: u32, tile: u32, row: u32, values: [u8; 8]) {
    let word = values
        .iter()
        .enumerate()
        .fold(0u32, |word, (i, &v)| word | (u32::from(v) << (4 * i)));
    w32(m, data + tile * 32 + row * 4, word);
}

fn top(m: &Machine, x: usize, y: usize) -> u32 {
    pixel(m, Screen::Top, x, y)
}

/// The cases (x, y, wanted pixel) whose pixel of the top screen differs, as (x, y, got, want).
fn differing(m: &Machine, cases: &[(usize, usize, u32)]) -> Vec<(usize, usize, u32, u32)> {
    cases
        .iter()
        .filter_map(|&(x, y, want)| {
            let got = top(m, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .collect()
}

/// Draws the frame and lists the pixel cases that differ.
fn drawn_failures(m: &mut Machine, cases: &[(usize, usize, u32)]) -> Vec<(usize, usize, u32, u32)> {
    m.render_frame();
    differing(m, cases)
}

type Diff = Vec<(usize, usize, u32, u32)>;

// ---------------------------------------------------------------- registers

#[test]
fn given_a_word_written_at_bg0cnt_when_read_then_bg0cnt_and_bg1cnt_hold_its_halves() {
    let mut m = Machine::new();
    w32(&mut m, BG0CNT, 0x2222_1111);
    assert_eq!(
        (r16(&m, BG0CNT), r16(&m, BG0CNT + 2), r32(&m, BG0CNT)),
        (0x1111, 0x2222, 0x2222_1111)
    );
}

#[test]
fn given_a_byte_written_to_bg2cnt_when_read_then_only_that_byte_changes() {
    let mut m = Machine::new();
    w16(&mut m, bgcnt_addr(2), 0x1234);
    w8(&mut m, bgcnt_addr(2) + 1, 0xAB);
    assert_eq!(r16(&m, bgcnt_addr(2)), 0xAB34);
}

#[test]
fn given_every_bg_control_bit_written_when_read_then_all_16_bits_are_kept() {
    let mut m = Machine::new();
    w16(&mut m, bgcnt_addr(3), 0xFFFF);
    assert_eq!(r16(&m, bgcnt_addr(3)), 0xFFFF);
}

#[test]
fn given_scroll_registers_written_when_read_then_they_read_zero_as_the_note_models_unverified() {
    // TODO(hw-unverified): bg-text.md "Unverified", reads of BGxHOFS and BGxVOFS.
    let mut m = Machine::new();
    w16(&mut m, BG0HOFS, 0x01FF);
    w16(&mut m, BG0HOFS + 2, 0x0123);
    assert_eq!((r16(&m, BG0HOFS), r16(&m, BG0HOFS + 2)), (0, 0));
}

#[test]
fn given_a_scroll_written_with_bits_above_8_when_decoded_then_only_the_low_9_bits_remain() {
    let mut m = Machine::new();
    w16(&mut m, BG0HOFS, 0xFFFF);
    w16(&mut m, BG0HOFS + 2, 0xFE12);
    let layer = m.inspect().gpu2d().text_bg(EngineId::A, 0);
    assert_eq!((layer.hofs, layer.vofs), (0x1FF, 0x012));
}

#[test]
fn given_a_scroll_with_a_byte_written_when_decoded_then_the_other_byte_is_kept() {
    let mut m = Machine::new();
    w16(&mut m, BG0HOFS, 0x01FF);
    w8(&mut m, BG0HOFS, 0x01);
    assert_eq!(m.inspect().gpu2d().text_bg(EngineId::A, 0).hofs, 0x101);
}

/// Scrolls layer n to (10 + n, 20 + n) and returns what each layer decodes to.
fn scrolled_layers() -> Vec<(u16, u16)> {
    let mut m = Machine::new();
    (0..4u32).for_each(|n| {
        w16(&mut m, BG0HOFS + 4 * n, 10 + n as u16);
        w16(&mut m, BG0HOFS + 4 * n + 2, 20 + n as u16);
    });
    (0..4)
        .map(|n| {
            (
                m.inspect().gpu2d().text_bg(EngineId::A, n).hofs,
                m.inspect().gpu2d().text_bg(EngineId::A, n).vofs,
            )
        })
        .collect()
}

#[test]
fn given_the_four_layers_scrolled_when_decoded_then_each_keeps_its_own_registers() {
    assert_eq!(
        scrolled_layers(),
        vec![(10, 20), (11, 21), (12, 22), (13, 23)]
    );
}

#[test]
fn given_the_registers_after_the_scrolls_when_written_then_nothing_is_stored_there_yet() {
    // The affine and window registers from 0400_0020h are Phase 4 (`TODO(phase4)`).
    let mut m = Machine::new();
    w32(&mut m, 0x0400_0020, u32::MAX);
    assert_eq!(r32(&m, 0x0400_0020), 0);
}

#[test]
fn given_bg_registers_written_when_decoded_then_the_layer_state_follows_the_note() {
    let mut m = Machine::new();
    // Character base 2 and screen base 3 in DISPCNT; BG1: priority 2, character block 5, 8 bits
    // per pixel, screen block 7, size 3.
    w32(&mut m, DISPCNT_A_ADDR, MODE1 | (2 << 24) | (3 << 27));
    w16(
        &mut m,
        bgcnt_addr(1),
        2 | (5 << 2) | 0x80 | (7 << 8) | (3 << 14),
    );
    let layer = m.inspect().gpu2d().text_bg(EngineId::A, 1);
    assert_eq!(
        (
            layer.priority,
            layer.char_base,
            layer.screen_base,
            layer.eight_bpp,
            layer.width,
            layer.height
        ),
        (2, 0x2_0000 + 0x1_4000, 0x3_0000 + 0x3800, true, 512, 512)
    );
}

#[test]
fn given_each_screen_size_when_decoded_then_the_layer_is_the_size_the_note_lists() {
    let mut m = Machine::new();
    let sizes: Vec<(u16, u16)> = (0..4u16)
        .map(|size| {
            w16(&mut m, BG0CNT, size << 14);
            (
                m.inspect().gpu2d().text_bg(EngineId::A, 0).width,
                m.inspect().gpu2d().text_bg(EngineId::A, 0).height,
            )
        })
        .collect();
    assert_eq!(sizes, vec![(256, 256), (512, 256), (256, 512), (512, 512)]);
}

// ---------------------------------------------------------------- one layer

#[test]
fn given_no_layer_on_in_mode_1_when_drawn_then_every_pixel_is_the_backdrop() {
    let mut m = text_machine();
    set_pal(&mut m, 0, 0x03E0);
    m.render_frame();
    assert_eq!(
        (
            not_equal_to(&m, Screen::Top, GREEN),
            // Engine B is in display mode 0: white.
            not_equal_to(&m, Screen::Bottom, WHITE)
        ),
        (0, 0)
    );
}

/// A 4-bits-per-pixel tile 1 whose row 0 holds the values 1-8, drawn at cell (0, 0).
fn tile4_machine() -> Machine {
    let mut m = text_machine();
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
    w16(&mut m, BG0CNT, SB1_4BPP);
    row4(&mut m, BG_VRAM, 1, 0, [1, 2, 3, 4, 5, 6, 7, 8]);
    cell(&mut m, SB1, 0, 0, 1);
    m
}

#[test]
fn given_a_4bpp_tile_when_drawn_then_each_pixel_of_its_first_row_takes_its_palette_color() {
    let cases: Vec<(usize, usize, u32)> = PALETTE
        .iter()
        .enumerate()
        .map(|(x, &(pixel, _))| (x, 0, pixel))
        .collect();
    assert_eq!(drawn_failures(&mut tile4_machine(), &cases), Diff::new());
}

#[test]
fn given_a_4bpp_tile_when_drawn_then_value_0_is_transparent_and_shows_the_backdrop() {
    // Row 1 of tile 1 and all of tile 0 (cell (1, 0)) hold value 0; the backdrop is entry 0, black.
    let cases = [
        (0, 1, BLACK),
        (7, 7, BLACK),
        (8, 0, BLACK),
        (255, 191, BLACK),
    ];
    assert_eq!(drawn_failures(&mut tile4_machine(), &cases), Diff::new());
}

#[test]
fn given_a_4bpp_cell_with_palette_number_2_when_drawn_then_it_uses_entries_32_and_up() {
    let mut m = tile4_machine();
    set_pal(&mut m, 33, 0x7C1F);
    set_pal(&mut m, 34, 0x7FE0);
    cell(&mut m, SB1, 0, 0, 1 | (2 << 12));
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, MAGENTA), (1, 0, CYAN), (2, 0, BLACK)]),
        Diff::new()
    );
}

/// A 8-bits-per-pixel tile 1 whose row 0 holds the values 1-8, drawn at cell (0, 0).
fn tile8_machine() -> Machine {
    let mut m = text_machine();
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
    w16(&mut m, BG0CNT, SB1_8BPP);
    row8(&mut m, BG_VRAM, 1, 0, [1, 2, 3, 4, 5, 6, 7, 8]);
    cell(&mut m, SB1, 0, 0, 1);
    m
}

#[test]
fn given_an_8bpp_tile_when_drawn_then_each_pixel_takes_the_palette_entry_of_its_byte() {
    let mut cases: Vec<(usize, usize, u32)> = PALETTE
        .iter()
        .enumerate()
        .map(|(x, &(pixel, _))| (x, 0, pixel))
        .collect();
    cases.push((0, 1, BLACK));
    cases.push((8, 0, BLACK));
    assert_eq!(drawn_failures(&mut tile8_machine(), &cases), Diff::new());
}

#[test]
fn given_an_8bpp_cell_with_a_palette_number_when_drawn_then_the_number_is_not_used() {
    let mut m = tile8_machine();
    cell(&mut m, SB1, 0, 0, 1 | (5 << 12));
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, RED), (1, 0, GREEN)]),
        Diff::new()
    );
}

#[test]
fn given_a_tile_number_above_255_when_drawn_then_its_data_is_found_at_tile_times_the_tile_size() {
    let mut m = text_machine();
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
    w16(&mut m, BG0CNT, SB1_8BPP);
    solid8(&mut m, BG_VRAM, 0x2AB, 3);
    cell(&mut m, SB1, 0, 0, 0x2AB);
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, BLUE), (7, 7, BLUE), (8, 0, BLACK)]),
        Diff::new()
    );
}

/// Tile 1 with row 0 = 1..8 and row 7 = 8..1, placed with each flip combination in cells 0-3.
fn flip_machine() -> Machine {
    let mut m = text_machine();
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
    w16(&mut m, BG0CNT, SB1_4BPP);
    row4(&mut m, BG_VRAM, 1, 0, [1, 2, 3, 4, 5, 6, 7, 8]);
    row4(&mut m, BG_VRAM, 1, 7, [8, 7, 6, 5, 4, 3, 2, 1]);
    cell(&mut m, SB1, 0, 0, 1);
    cell(&mut m, SB1, 1, 0, 1 | 0x400);
    cell(&mut m, SB1, 2, 0, 1 | 0x800);
    cell(&mut m, SB1, 3, 0, 1 | 0xC00);
    m
}

#[test]
fn given_a_tile_without_a_flip_when_drawn_then_row_0_and_row_7_are_as_stored() {
    let cases = [(0, 0, RED), (7, 0, DARK_RED), (0, 7, DARK_RED), (7, 7, RED)];
    assert_eq!(drawn_failures(&mut flip_machine(), &cases), Diff::new());
}

#[test]
fn given_a_tile_with_the_horizontal_flip_when_drawn_then_its_columns_are_mirrored() {
    // Cell 1 covers x 8-15: column 0 shows the stored column 7.
    let cases = [
        (8, 0, DARK_RED),
        (15, 0, RED),
        (8, 7, RED),
        (15, 7, DARK_RED),
    ];
    assert_eq!(drawn_failures(&mut flip_machine(), &cases), Diff::new());
}

#[test]
fn given_a_tile_with_the_vertical_flip_when_drawn_then_its_rows_are_swapped() {
    // Cell 2 covers x 16-23: row 0 shows the stored row 7.
    let cases = [
        (16, 0, DARK_RED),
        (23, 0, RED),
        (16, 7, RED),
        (23, 7, DARK_RED),
    ];
    assert_eq!(drawn_failures(&mut flip_machine(), &cases), Diff::new());
}

#[test]
fn given_a_tile_with_both_flips_when_drawn_then_it_is_turned_around() {
    // Cell 3 covers x 24-31: pixel (0, 0) shows the stored (7, 7), pixel (7, 0) the stored (0, 7).
    let cases = [
        (24, 0, RED),
        (31, 0, DARK_RED),
        (24, 7, DARK_RED),
        (31, 7, RED),
    ];
    assert_eq!(drawn_failures(&mut flip_machine(), &cases), Diff::new());
}

// ---------------------------------------------------------------- scrolling

/// 8 bits per pixel, a 256x256 layer, solid tiles k with the palette entry k: row 0 holds the
/// cells (0..=3, 0) = tiles 1-4 and (31, 0) = tile 5; column 0 holds (0, 1) = tile 6,
/// (0, 23) = tile 8 and (0, 31) = tile 7.
fn scroll_machine(cnt: u16, hofs: u16, vofs: u16) -> Machine {
    let mut m = text_machine();
    scroll_scene(&mut m, cnt, hofs, vofs);
    m
}

/// The scene of [`scroll_machine`] on a machine that has its bank and palette set up.
fn scroll_scene(m: &mut Machine, cnt: u16, hofs: u16, vofs: u16) {
    w32(m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
    w16(m, BG0CNT, cnt);
    (1..=8).for_each(|tile| solid8(m, BG_VRAM, tile, tile as u8));
    (0..4).for_each(|cx| cell(m, SB1, cx, 0, cx as u16 + 1));
    cell(m, SB1, 31, 0, 5);
    cell(m, SB1, 0, 1, 6);
    cell(m, SB1, 0, 31, 7);
    cell(m, SB1, 0, 23, 8);
    w16(m, BG0HOFS, hofs);
    w16(m, BG0HOFS + 2, vofs);
}

/// The scroll cases (hofs, vofs, x, y, wanted) that differ on the 256x256 layer.
fn scroll_failures(cases: &[(u16, u16, usize, usize, u32)]) -> Vec<(u16, u16, usize, usize, u32)> {
    cases
        .iter()
        .copied()
        .filter(|&(hofs, vofs, x, y, want)| {
            let mut m = scroll_machine(SB1_8BPP, hofs, vofs);
            m.render_frame();
            top(&m, x, y) != want
        })
        .collect()
}

#[test]
fn given_a_horizontal_scroll_when_drawn_then_the_layer_moves_left_by_that_many_pixels() {
    let cases = [
        (0, 0, 0, 0, RED),
        (0, 0, 8, 0, GREEN),
        (0, 0, 23, 0, BLUE),
        (0, 0, 24, 0, YELLOW),
        (8, 0, 0, 0, GREEN),
        (8, 0, 4, 0, GREEN),
        (3, 0, 4, 0, RED),
        (3, 0, 5, 0, GREEN),
    ];
    assert_eq!(scroll_failures(&cases), vec![]);
}

#[test]
fn given_a_scroll_past_the_right_edge_when_drawn_then_the_256_wide_layer_wraps_around() {
    let cases = [
        (255, 0, 0, 0, MAGENTA),
        (255, 0, 1, 0, RED),
        (256, 0, 0, 0, RED),
        (256, 0, 8, 0, GREEN),
    ];
    assert_eq!(scroll_failures(&cases), vec![]);
}

#[test]
fn given_a_vertical_scroll_when_drawn_then_the_layer_moves_up_and_wraps_at_256() {
    let cases = [
        (0, 8, 0, 0, CYAN),
        (0, 0, 0, 8, CYAN),
        (0, 255, 0, 0, WHITE_PIXEL),
        (0, 255, 0, 1, RED),
        (0, 256, 0, 0, RED),
    ];
    assert_eq!(scroll_failures(&cases), vec![]);
}

#[test]
fn given_a_scroll_on_one_layer_when_another_is_drawn_then_the_other_does_not_move() {
    let mut m = scroll_machine(SB1_8BPP, 8, 0);
    // BG1 (priority 1) has its own map at screen block 2 and no scroll: cell (0, 1) = tile 3.
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0011));
    w16(&mut m, bgcnt_addr(1), 0x0200 | 0x0080 | 1);
    cell(&mut m, SB1 + BLOCK, 0, 1, 3);
    // BG0 (priority 0, scrolled 8) shows GREEN at (0, 0) and is transparent at (0, 8), where BG1
    // shows its own cell (0, 1) = BLUE; had BG1 scrolled too it would show its cell (1, 1).
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, GREEN), (0, 8, BLUE), (8, 8, BLACK)]),
        Diff::new()
    );
}

/// The size cases: `cnt`, the block cells (block index, tile), then (hofs, vofs, wanted).
fn size_failures(
    cnt: u16,
    blocks: &[(u32, u16)],
    cases: &[(u16, u16, u32)],
) -> Vec<(u16, u16, u32, u32)> {
    cases
        .iter()
        .copied()
        .filter_map(|(hofs, vofs, want)| {
            let mut m = text_machine();
            w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
            w16(&mut m, BG0CNT, cnt);
            (1..=8).for_each(|tile| solid8(&mut m, BG_VRAM, tile, tile as u8));
            blocks
                .iter()
                .for_each(|&(block, tile)| cell(&mut m, SB1 + block * BLOCK, 0, 0, tile));
            w16(&mut m, BG0HOFS, hofs);
            w16(&mut m, BG0HOFS + 2, vofs);
            m.render_frame();
            let got = top(&m, 0, 0);
            (got != want).then_some((hofs, vofs, got, want))
        })
        .collect()
}

#[test]
fn given_a_512_by_256_layer_when_scrolled_into_its_second_block_then_the_block_to_the_right_shows()
{
    // Block 1 sits to the right of block 0, 2 KiB further on; the height is 256, so VOFS 256 wraps.
    let blocks = [(0, 1), (1, 2)];
    let cases = [(0, 0, RED), (256, 0, GREEN), (0, 256, RED)];
    assert_eq!(size_failures(0x4000 | SB1_8BPP, &blocks, &cases), vec![]);
}

#[test]
fn given_a_256_by_512_layer_when_scrolled_down_into_its_second_block_then_the_block_below_shows() {
    let blocks = [(0, 1), (1, 2)];
    let cases = [(0, 0, RED), (0, 256, GREEN), (256, 0, RED)];
    assert_eq!(size_failures(0x8000 | SB1_8BPP, &blocks, &cases), vec![]);
}

#[test]
fn given_a_512_by_512_layer_when_scrolled_to_each_corner_then_the_four_blocks_are_in_reading_order()
{
    // Blocks 0-3 are top-left, top-right, bottom-left and bottom-right.
    let blocks = [(0, 1), (1, 2), (2, 3), (3, 4)];
    let cases = [
        (0, 0, RED),
        (256, 0, GREEN),
        (0, 256, BLUE),
        (256, 256, YELLOW),
        (256, 511, BLACK),
    ];
    assert_eq!(size_failures(0xC000 | SB1_8BPP, &blocks, &cases), vec![]);
}

#[test]
fn given_a_512_wide_layer_when_scrolled_by_511_then_the_pixel_after_the_last_column_wraps_to_column_0()
 {
    // The pixel at x = 1 is column 512 mod 512 = 0 of block 0.
    let mut m = text_machine();
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
    w16(&mut m, BG0CNT, 0x4000 | SB1_8BPP);
    solid8(&mut m, BG_VRAM, 1, 1);
    solid8(&mut m, BG_VRAM, 2, 2);
    cell(&mut m, SB1, 0, 0, 1);
    cell(&mut m, SB1 + BLOCK, 31, 0, 2);
    w16(&mut m, BG0HOFS, 511);
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, GREEN), (1, 0, RED)]),
        Diff::new()
    );
}

// ---------------------------------------------------------------- bases

#[test]
fn given_character_and_screen_bases_in_dispcnt_and_bgcnt_when_drawn_then_both_add_up() {
    // Character base 1 (64 KiB) + block 1 (16 KiB); screen base 1 (64 KiB) + block 3 (6 KiB).
    let mut m = text_machine();
    w32(
        &mut m,
        DISPCNT_A_ADDR,
        dispcnt(0, 0b0001) | (1 << 24) | (1 << 27),
    );
    w16(&mut m, BG0CNT, 0x80 | (1 << 2) | (3 << 8));
    solid8(&mut m, BG_VRAM + 0x1_4000, 1, 2);
    cell(&mut m, BG_VRAM + 0x1_1800, 0, 0, 1);
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, GREEN), (8, 0, BLACK)]),
        Diff::new()
    );
}

#[test]
fn given_bank_a_at_offset_1_when_the_bases_point_into_its_slot_then_it_draws() {
    // OFS 1 puts the bank at 0602_0000h: character base 2 and screen base 3 land inside it.
    let mut m = text_machine_at(1);
    w32(
        &mut m,
        DISPCNT_A_ADDR,
        dispcnt(0, 0b0001) | (2 << 24) | (3 << 27),
    );
    w16(&mut m, BG0CNT, 0x80);
    solid8(&mut m, BG_VRAM + 0x2_0000, 1, 3);
    cell(&mut m, BG_VRAM + 0x3_0000, 0, 0, 1);
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, BLUE), (8, 0, BLACK)]),
        Diff::new()
    );
}

#[test]
fn given_bank_a_at_offset_1_when_the_bases_point_below_its_slot_then_nothing_is_there_to_draw() {
    // TODO(hw-unverified): bg-text.md "Unverified", reads past the mapped banks.
    let mut m = text_machine_at(1);
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 0b0001));
    w16(&mut m, BG0CNT, SB1_8BPP);
    solid8(&mut m, BG_VRAM + 0x2_0000, 1, 3);
    cell(&mut m, BG_VRAM + 0x2_0000 + BLOCK, 0, 0, 1);
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, BLACK), (4, 4, BLACK)]),
        Diff::new()
    );
}

#[test]
fn given_bank_a_in_lcdc_mode_when_a_layer_is_drawn_then_no_tile_data_is_found() {
    // TODO(hw-unverified): bg-text.md "Unverified", reads past the mapped banks.
    let mut m = tile8_machine();
    w8(&mut m, VRAMCNT_A, ENABLE_MST0);
    assert_eq!(
        drawn_failures(&mut m, &[(0, 0, BLACK), (3, 0, BLACK)]),
        Diff::new()
    );
}

// ---------------------------------------------------------------- priority

/// Layers 0-3 each show the solid tile (n + 1) at cell (12, 12) of their own map block (n + 1),
/// so pixel (100, 100) is RED, GREEN, BLUE, YELLOW for BG0-BG3, with the given priorities.
fn priority_machine(priorities: [u16; 4], layers: u32) -> Machine {
    let mut m = text_machine();
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, layers));
    (0..4u32).for_each(|n| {
        let block = n + 1;
        w16(
            &mut m,
            bgcnt_addr(n),
            0x80 | priorities[n as usize] | ((block as u16) << 8),
        );
        solid8(&mut m, BG_VRAM, n + 1, (n + 1) as u8);
        cell(&mut m, BG_VRAM + block * BLOCK, 12, 12, (n + 1) as u16);
    });
    m
}

/// The cases (priorities, layers, wanted pixel at (100, 100)) that differ.
fn priority_failures(cases: &[([u16; 4], u32, u32)]) -> Vec<([u16; 4], u32, u32, u32)> {
    cases
        .iter()
        .copied()
        .filter_map(|(priorities, layers, want)| {
            let mut m = priority_machine(priorities, layers);
            m.render_frame();
            let got = top(&m, 100, 100);
            (got != want).then_some((priorities, layers, got, want))
        })
        .collect()
}

#[test]
fn given_layers_with_different_priorities_when_drawn_then_the_lowest_priority_value_is_in_front() {
    let cases = [
        ([1, 0, 3, 3], 0b0011, GREEN),
        ([0, 1, 3, 3], 0b0011, RED),
        ([3, 2, 1, 0], 0b1111, YELLOW),
        ([3, 2, 0, 1], 0b1111, BLUE),
        ([2, 3, 1, 0], 0b0111, BLUE),
    ];
    assert_eq!(priority_failures(&cases), vec![]);
}

#[test]
fn given_layers_with_the_same_priority_when_drawn_then_the_lower_layer_number_is_in_front() {
    let cases = [
        ([2, 2, 3, 3], 0b0011, RED),
        ([1, 1, 1, 1], 0b1100, BLUE),
        ([1, 1, 1, 1], 0b1010, GREEN),
        ([0, 0, 0, 0], 0b1111, RED),
    ];
    assert_eq!(priority_failures(&cases), vec![]);
}

#[test]
fn given_a_layer_switched_off_when_drawn_then_the_layer_behind_shows_instead() {
    let cases = [([0, 1, 2, 3], 0b1110, GREEN), ([0, 1, 2, 3], 0b0000, BLACK)];
    assert_eq!(priority_failures(&cases), vec![]);
}

#[test]
fn given_a_transparent_pixel_in_the_front_layer_when_drawn_then_the_layer_behind_shows_through() {
    let mut m = priority_machine([0, 1, 3, 3], 0b0011);
    // BG0 at pixel (100, 100) keeps its tile; at (108, 100) its cell (13, 12) is tile 0.
    cell(&mut m, BG_VRAM + 2 * BLOCK, 13, 12, 2);
    assert_eq!(
        drawn_failures(
            &mut m,
            &[(100, 100, RED), (108, 100, GREEN), (116, 100, BLACK)]
        ),
        Diff::new()
    );
}

// ---------------------------------------------------------------- modes and layers

/// Whether layer `layer` (a solid tile covering pixel (0, 0)) is drawn in BG mode `mode`.
fn layer_is_drawn(mode: u32, layer: u32, extra_dispcnt: u32) -> bool {
    let mut m = text_machine();
    w32(
        &mut m,
        DISPCNT_A_ADDR,
        dispcnt(mode, 1 << layer) | extra_dispcnt,
    );
    w16(&mut m, bgcnt_addr(layer), SB1_8BPP);
    solid8(&mut m, BG_VRAM, 1, 1);
    cell(&mut m, SB1, 0, 0, 1);
    m.render_frame();
    top(&m, 0, 0) == RED
}

/// The (mode, layer) pairs for which `layer_is_drawn` differs from `drawn`.
fn kind_failures(cases: &[(u32, u32, bool)]) -> Vec<(u32, u32, bool)> {
    cases
        .iter()
        .copied()
        .filter(|&(mode, layer, drawn)| layer_is_drawn(mode, layer, 0) != drawn)
        .collect()
}

#[test]
fn given_bg_mode_0_when_each_layer_is_enabled_then_all_four_text_layers_draw() {
    let cases = [(0, 0, true), (0, 1, true), (0, 2, true), (0, 3, true)];
    assert_eq!(kind_failures(&cases), vec![]);
}

#[test]
fn given_bg_modes_1_to_5_when_the_text_layers_are_enabled_then_they_draw() {
    let cases = [
        (1, 0, true),
        (1, 1, true),
        (1, 2, true),
        (2, 0, true),
        (2, 1, true),
        (3, 0, true),
        (3, 1, true),
        (3, 2, true),
        (4, 0, true),
        (4, 1, true),
        (5, 0, true),
        (5, 1, true),
    ];
    assert_eq!(kind_failures(&cases), vec![]);
}

#[test]
fn given_bg_mode_6_when_the_layers_are_enabled_then_none_draws_as_bg0_is_the_3d_layer() {
    // TODO(phase7): BG0 is the 3D engine's layer in mode 6.
    let cases = [(6, 0, false), (6, 1, false), (6, 2, false), (6, 3, false)];
    assert_eq!(kind_failures(&cases), vec![]);
}

#[test]
fn given_bg_mode_7_when_bg0_is_enabled_then_nothing_draws_as_the_note_models_unverified() {
    // TODO(hw-unverified): bg-text.md "Unverified", mode 7 is not defined.
    assert_eq!(kind_failures(&[(7, 0, false)]), vec![]);
}

#[test]
fn given_bg0_set_to_3d_when_drawn_then_it_draws_nothing_yet() {
    // DISPCNT bit 3. TODO(phase7): the 3D engine's picture.
    assert!(!layer_is_drawn(0, 0, 1 << 3));
}

#[test]
fn given_display_mode_0_when_a_layer_is_enabled_then_the_screen_stays_white() {
    let mut m = tile8_machine();
    w32(&mut m, DISPCNT_A_ADDR, 0b0001 << 8);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, WHITE), 0);
}

// ---------------------------------------------------------------- lines

#[test]
fn given_a_scroll_changed_between_two_lines_when_they_are_drawn_then_each_line_uses_the_scroll_it_saw()
 {
    let mut m = scroll_machine(SB1_8BPP, 0, 0);
    m.render_line(2);
    w16(&mut m, BG0HOFS, 8);
    m.render_line(3);
    m.present_frame();
    // Line 4 was never drawn: the work buffer it comes from is still blank.
    assert_eq!(
        (top(&m, 0, 2), top(&m, 0, 3), top(&m, 0, 4)),
        (RED, GREEN, BLACK)
    );
}

#[test]
fn given_a_line_that_is_drawn_when_another_is_asked_for_then_the_first_is_untouched() {
    let mut m = scroll_machine(SB1_8BPP, 0, 0);
    m.render_line(0);
    m.render_line(191);
    m.present_frame();
    assert_eq!(
        (
            top(&m, 0, 0),
            top(&m, 255, 0),
            top(&m, 0, 191),
            top(&m, 0, 1)
        ),
        // Cell (31, 0) is tile 5, MAGENTA; line 191 is row 23, which holds tile 8 only at x < 8.
        (RED, MAGENTA, DARK_RED, BLACK)
    );
}

#[test]
fn given_a_line_past_the_last_visible_one_when_asked_for_then_nothing_is_drawn() {
    let mut m = scroll_machine(SB1_8BPP, 0, 0);
    m.render_line(192);
    m.render_line(usize::MAX);
    m.present_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, BLACK), 0);
}

#[test]
fn given_layers_on_the_bottom_screen_when_powcnt1_bit_15_is_clear_then_the_picture_moves_there() {
    let mut m = scroll_machine(SB1_8BPP, 0, 0);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_BOTTOM);
    m.render_frame();
    assert_eq!(
        (pixel(&m, Screen::Bottom, 0, 0), top(&m, 0, 0)),
        // Engine B, display off, is on the upper screen: white.
        (RED, WHITE)
    );
}

// ---------------------------------------------------------------- running

/// Runs rounds until the frame is at line `line`.
fn run_to_line(m: &mut Machine, line: u16) {
    while m.inspect().vcount() < line {
        crate::scheduler::run_round(m);
    }
}

/// A machine booted from a program that spins, with the scroll scene already in memory.
fn spinning_scene() -> Machine {
    let mut image = Image::valid();
    image.arm9_code = words(&[0xEAFF_FFFE]);
    let mut m = Machine::new();
    m.load_rom(image.build()).expect("synthetic image");
    text_setup(&mut m, 0);
    scroll_scene(&mut m, SB1_8BPP, 0, 0);
    m
}

#[test]
fn given_a_scene_in_memory_when_frames_run_then_the_presented_frame_is_the_whole_picture() {
    let mut m = spinning_scene();
    m.run_frame();
    m.run_frame();
    assert_eq!(
        (
            top(&m, 0, 0),
            top(&m, 8, 3),
            top(&m, 24, 7),
            top(&m, 0, 191)
        ),
        // The last line is row 23 of the map: cell (0, 23) holds tile 8.
        (RED, GREEN, YELLOW, DARK_RED)
    );
}

#[test]
fn given_a_scroll_changed_after_a_frame_when_the_next_runs_then_line_0_keeps_the_old_scroll() {
    // Line 0 of a frame is drawn at the start of that line, which ends the frame before it; the
    // change comes after, so it shows from line 1 (`bg-text.md`, "Lines").
    let mut m = spinning_scene();
    m.run_frame();
    w16(&mut m, BG0HOFS, 8);
    m.run_frame();
    assert_eq!(
        (top(&m, 0, 0), top(&m, 0, 1), top(&m, 0, 7)),
        (RED, GREEN, GREEN)
    );
}

#[test]
fn given_a_frame_in_progress_when_the_presented_frame_is_read_then_it_is_the_last_complete_one() {
    let mut m = spinning_scene();
    m.run_frame();
    m.run_frame();
    w16(&mut m, BG0HOFS, 8);
    // Part of the next frame: lines are drawn, but the presented frame holds the finished one.
    run_to_line(&mut m, 100);
    assert_eq!((top(&m, 0, 3), top(&m, 0, 7)), (RED, RED));
}

/// A machine saved at line 100 of a frame whose registers changed at line 50, loaded onto a fresh
/// one; both then run to the end of the frame. Returns whether both screens are equal.
fn raster_save_matches() -> (Result<(), crate::savestate::SavestateError>, bool, bool) {
    let mut m = spinning_scene();
    m.run_frame();
    run_to_line(&mut m, 50);
    w16(&mut m, BG0HOFS, 8);
    run_to_line(&mut m, 100);
    let mut loaded = Machine::new();
    loaded.load_rom(m.rom().to_vec()).expect("same image");
    let result = loaded.load_state(&m.save_state());
    m.run_frame();
    loaded.run_frame();
    (
        result,
        loaded.frame(Screen::Top) == m.frame(Screen::Top),
        loaded.frame(Screen::Bottom) == m.frame(Screen::Bottom),
    )
}

#[test]
fn given_a_machine_saved_after_a_mid_frame_register_change_when_loaded_and_run_then_both_screens_equal_the_uninterrupted_ones()
 {
    assert_eq!(raster_save_matches(), (Ok(()), true, true));
}

#[test]
fn given_a_machine_saved_mid_frame_when_loaded_then_the_last_finished_frame_is_still_shown() {
    let mut m = spinning_scene();
    m.run_frame();
    m.run_frame();
    run_to_line(&mut m, 100);
    let mut loaded = Machine::new();
    loaded.load_rom(m.rom().to_vec()).expect("same image");
    let result = loaded.load_state(&m.save_state());
    assert_eq!(
        (result, top(&loaded, 0, 3), top(&loaded, 8, 3)),
        (Ok(()), RED, GREEN)
    );
}

#[test]
fn given_a_machine_with_a_rom_when_it_boots_then_line_0_is_drawn_from_the_power_on_registers() {
    // Display mode 0 is a white screen; the first line start is line 1, so line 0 is drawn here.
    let mut image = Image::valid();
    image.arm9_code = words(&[0xEAFF_FFFE]);
    let mut m = Machine::new();
    m.load_rom(image.build()).expect("synthetic image");
    m.present_frame();
    assert_eq!((top(&m, 0, 0), top(&m, 0, 1)), (WHITE, BLACK));
}

/// Engine A's first pixel with all text data in bank `bank` (A = 0 .. I = 8) mapped by `control`:
/// an 8-bits-per-pixel tile of palette entry 1 (red) at character block 1 and one map cell at
/// screen block 1 of the BG window (`docs/hw/vram-banks.md`, "Where each bank goes").
fn first_pixel_from_bank(bank: u32, control: u8) -> u32 {
    let mut m = Machine::new();
    text_setup(&mut m, 0);
    w8(&mut m, VRAMCNT_A, 0);
    w8(&mut m, VRAMCNT_A + bank + u32::from(bank > 6), control);
    solid8(&mut m, BG_VRAM + 0x4000, 1, 1);
    cell(&mut m, SB1, 0, 0, 1);
    w16(&mut m, BG0CNT, SB1_8BPP | (1 << 2));
    w32(&mut m, DISPCNT_A_ADDR, dispcnt(0, 1));
    m.render_frame();
    pixel(&m, Screen::Top, 0, 0)
}

#[test]
fn given_text_data_in_bank_e_with_mst_1_when_rendered_then_the_layer_draws_it() {
    assert_eq!(first_pixel_from_bank(4, 0x81), RED);
}

#[test]
fn given_bank_f_at_ofs_0_when_the_tile_data_lies_past_its_16_kib_then_the_backdrop_shows() {
    // F covers the first 16 KiB of the window. The map cell at +0x800 is inside it, the tile data
    // at +0x4000 is not, so the tile reads as transparent and the backdrop (black) shows.
    assert_eq!(first_pixel_from_bank(5, 0x81), BLACK);
}

#[test]
fn given_text_data_in_bank_b_with_mst_1_when_bank_a_is_off_then_the_layer_draws_it() {
    assert_eq!(first_pixel_from_bank(1, 0x81), RED);
}
