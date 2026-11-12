// SPDX-License-Identifier: Apache-2.0
//! Engine A's VRAM display mode and POWCNT1 bit 15 (issue #20). Every expected value comes
//! from `docs/hw/display-mode-vram.md` (GBATEK "DS Video - LCD Control", "DS Power Control",
//! "DS Video - Color Format"), never from the implementation. The 5-bit to 8-bit expansion
//! `(c << 3) | (c >> 2)` is the note's host choice; table cases hold literal results.

use super::*;
use crate::cpu::Bus;
use crate::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};

/// VRAMCNT_A (`docs/hw/vram-banks.md`).
const VRAMCNT_A: u32 = 0x0400_0240;
/// Bank A's LCDC window.
const LCDC_A: u32 = 0x0680_0000;
/// Enabled, MST 0 (LCDC mapping).
const ENABLE_MST0: u8 = 0x80;
/// DISPCNT display mode 2 (VRAM display), block 0.
const MODE2: u32 = 2 << 16;
/// POWCNT1 power-on value with bit 15 set.
const POWCNT1_ON_TOP: u16 = 0x820F;
/// POWCNT1 with bit 15 clear.
const POWCNT1_ON_BOTTOM: u16 = 0x020F;
const PIXELS: usize = SCREEN_WIDTH * SCREEN_HEIGHT;

mod affine;
mod engine_b;
mod extended;
mod large;
mod mosaic;
mod text;

fn w8(m: &mut Machine, addr: u32, v: u8) {
    m.bus9().write8(addr, v);
}

fn w16(m: &mut Machine, addr: u32, v: u16) {
    m.bus9().write16(addr, v);
}

fn w32(m: &mut Machine, addr: u32, v: u32) {
    m.bus9().write32(addr, v);
}

fn r16(m: &Machine, addr: u32) -> u16 {
    m.inspect().arm9_peek16(addr)
}

fn r32(m: &Machine, addr: u32) -> u32 {
    m.inspect().arm9_peek32(addr)
}

/// A machine with bank A mapped at the LCDC window, mode 2 block 0, engine A on top.
fn mode2_machine() -> Machine {
    let mut m = Machine::new();
    w8(&mut m, VRAMCNT_A, ENABLE_MST0);
    w32(&mut m, DISPCNT_A_ADDR, MODE2);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    m
}

fn put_pixel(m: &mut Machine, x: usize, y: usize, halfword: u16) {
    w16(m, LCDC_A + ((y * SCREEN_WIDTH + x) * 2) as u32, halfword);
}

/// Fills the whole first 98,304 bytes of bank A with one halfword.
fn fill_bank(m: &mut Machine, halfword: u16) {
    (0..PIXELS).for_each(|i| w16(m, LCDC_A + (i * 2) as u32, halfword));
}

fn pixel(m: &Machine, screen: Screen, x: usize, y: usize) -> u32 {
    m.frame(screen)[y * SCREEN_WIDTH + x]
}

/// How many pixels of `screen` differ from `expected`.
fn not_equal_to(m: &Machine, screen: Screen, expected: u32) -> usize {
    m.frame(screen).iter().filter(|&&p| p != expected).count()
}

/// Cases (x, y, halfword, expected xRGB) that mismatch after rendering a mode-2 frame.
fn pixel_failures(cases: &[(usize, usize, u16, u32)]) -> Vec<(usize, usize, u32, u32)> {
    let mut m = mode2_machine();
    cases
        .iter()
        .for_each(|&(x, y, h, _)| put_pixel(&mut m, x, y, h));
    m.render_frame();
    cases
        .iter()
        .filter_map(|&(x, y, _, want)| {
            let got = pixel(&m, Screen::Top, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .collect()
}

/// Failed cases as (case number, got, wanted) for DISPCNT writes of each width.
fn dispcnt_failures() -> Vec<(u32, u32, u32)> {
    let mut failures = Vec::new();
    // word write
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 0x1234_5678);
    let got = r32(&m, DISPCNT_A_ADDR);
    if got != 0x1234_5678 {
        failures.push((1, got, 0x1234_5678));
    }
    // high halfword write changes only bits 16-31
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 0x1111_2222);
    w16(&mut m, DISPCNT_A_ADDR + 2, 0xABCD);
    let got = r32(&m, DISPCNT_A_ADDR);
    if got != 0xABCD_2222 {
        failures.push((2, got, 0xABCD_2222));
    }
    // byte write at +2 changes only bits 16-23
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 0x1122_3344);
    w8(&mut m, DISPCNT_A_ADDR + 2, 0xEE);
    let got = r32(&m, DISPCNT_A_ADDR);
    if got != 0x11EE_3344 {
        failures.push((3, got, 0x11EE_3344));
    }
    // all 32 bits kept
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 0xFFFF_FFFF);
    let got = r32(&m, DISPCNT_A_ADDR);
    if got != 0xFFFF_FFFF {
        failures.push((4, got, 0xFFFF_FFFF));
    }
    failures
}

/// Pixels (x, y) of a mode-2 frame whose colour differs from the note's expansion.
fn whole_frame_mismatches() -> Vec<(usize, usize)> {
    let expand = |c: u32| (c << 3) | (c >> 2);
    let halfword = |x: usize, y: usize| ((x * 7 + y * 13) & 0x7FFF) as u16;
    let mut m = mode2_machine();
    (0..SCREEN_HEIGHT)
        .for_each(|y| (0..SCREEN_WIDTH).for_each(|x| put_pixel(&mut m, x, y, halfword(x, y))));
    m.render_frame();
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let h = u32::from(halfword(x, y));
            let want = (expand(h & 0x1F) << 16)
                | (expand((h >> 5) & 0x1F) << 8)
                | expand((h >> 10) & 0x1F);
            pixel(&m, Screen::Top, x, y) != want
        })
        .collect()
}

#[test]
fn given_dispcnt_writes_of_each_width_when_read_back_then_only_covered_bytes_changed() {
    assert_eq!(dispcnt_failures(), Vec::<(u32, u32, u32)>::new());
}

#[test]
fn given_a_power_on_machine_when_powcnt1_is_read_then_it_is_820f() {
    assert_eq!(r16(&Machine::new(), POWCNT1_ADDR), 0x820F);
}

#[test]
fn given_a_halfword_write_to_powcnt1_when_read_back_then_all_16_bits_are_kept() {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, 0xFFFF);
    let all = r16(&m, POWCNT1_ADDR);
    w16(&mut m, POWCNT1_ADDR, 0x1234);
    assert_eq!((all, r16(&m, POWCNT1_ADDR)), (0xFFFF, 0x1234));
}

#[test]
fn given_a_byte_write_to_the_powcnt1_high_byte_when_read_back_then_the_low_byte_is_unchanged() {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, 0x1234);
    w8(&mut m, POWCNT1_ADDR + 1, 0xAB);
    assert_eq!(r16(&m, POWCNT1_ADDR), 0xAB34);
}

#[test]
fn given_a_power_on_machine_when_dispcnt_is_read_then_it_is_zero() {
    assert_eq!(r32(&Machine::new(), DISPCNT_A_ADDR), 0);
}

#[test]
fn given_mode_2_and_bank_a_when_pixels_are_rendered_then_corners_and_an_interior_pixel_match() {
    let failures = pixel_failures(&[
        (0, 0, 0x001F, 0x00FF_0000),
        (255, 0, 0x03E0, 0x0000_FF00),
        (0, 191, 0x7C00, 0x0000_00FF),
        (255, 191, 0x7FFF, 0x00FF_FFFF),
        (17, 5, 0x4A52, 0x0094_9494),
    ]);
    assert_eq!(failures, Vec::<(usize, usize, u32, u32)>::new());
}

#[test]
fn given_mode_2_when_channel_values_0_1_10_and_1f_are_rendered_then_they_expand_to_00_08_84_ff() {
    // halfword (R,G,B) -> expected; 0x10 expands to 0x84, 1 to 0x08, 0x1F to 0xFF.
    let failures = pixel_failures(&[
        (0, 0, 0x0000, 0x0000_0000),
        (1, 0, 0x0001, 0x0008_0000),
        (2, 0, 0x0010, 0x0084_0000),
        (3, 0, 0x001F, 0x00FF_0000),
        (4, 0, 0x0020, 0x0000_0800),
        (5, 0, 0x0200, 0x0000_8400),
        (6, 0, 0x03E0, 0x0000_FF00),
        (7, 0, 0x0400, 0x0000_0008),
        (8, 0, 0x4000, 0x0000_0084),
        (9, 0, 0x7C00, 0x0000_00FF),
        (10, 0, 0x7FFF, 0x00FF_FFFF),
    ]);
    assert_eq!(failures, Vec::<(usize, usize, u32, u32)>::new());
}

#[test]
fn given_halfwords_with_and_without_bit_15_when_rendered_then_the_pixels_are_equal() {
    let failures = pixel_failures(&[
        (0, 0, 0x001F, 0x00FF_0000),
        (1, 0, 0x801F, 0x00FF_0000),
        (2, 0, 0xFFFF, 0x00FF_FFFF),
        (3, 0, 0x8000, 0x0000_0000),
    ]);
    assert_eq!(failures, Vec::<(usize, usize, u32, u32)>::new());
}

#[test]
fn given_mode_0_and_vram_full_of_data_when_rendered_then_engine_a_screen_is_white() {
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x7FFF);
    w32(&mut m, DISPCNT_A_ADDR, 0);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, WHITE), 0);
}

#[test]
fn given_mode_1_and_vram_full_of_data_when_rendered_then_engine_a_screen_is_black() {
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x7FFF);
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, BLACK), 0);
}

#[test]
fn given_mode_3_and_vram_full_of_data_when_rendered_then_engine_a_screen_is_black() {
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x7FFF);
    w32(&mut m, DISPCNT_A_ADDR, 3 << 16);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, BLACK), 0);
}

#[test]
fn given_bank_a_disabled_after_writing_when_mode_2_is_rendered_then_the_screen_is_black() {
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x7FFF);
    w8(&mut m, VRAMCNT_A, 0);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, BLACK), 0);
}

#[test]
fn given_bank_a_enabled_with_mst_1_when_mode_2_is_rendered_then_the_screen_is_black() {
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x7FFF);
    w8(&mut m, VRAMCNT_A, 0x81);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, BLACK), 0);
}

/// For each block 1..=3, the count of non-black engine A pixels in mode 2.
fn nonblack_for_blocks_1_to_3() -> Vec<usize> {
    (1u32..=3)
        .map(|block| {
            let mut m = mode2_machine();
            fill_bank(&mut m, 0x7FFF);
            w32(&mut m, DISPCNT_A_ADDR, MODE2 | (block << 18));
            m.render_frame();
            not_equal_to(&m, Screen::Top, BLACK)
        })
        .collect()
}

#[test]
fn given_dispcnt_blocks_1_to_3_with_banks_b_to_d_disabled_when_mode_2_is_rendered_then_the_screen_is_black()
 {
    assert_eq!(nonblack_for_blocks_1_to_3(), vec![0, 0, 0]);
}

/// The top-left pixel of engine A in mode 2 for each block 0..=3 when banks A-D are all enabled
/// in LCDC mode and bank X holds the halfword `0x0001 << X` at its first pixel
/// (`docs/hw/display-mode-vram.md`: block N is bank A + N).
fn first_pixel_per_block() -> Vec<u32> {
    (0u32..4)
        .map(|block| {
            let mut m = mode2_machine();
            (0..4u32).for_each(|bank| {
                w8(&mut m, VRAMCNT_A + bank, ENABLE_MST0);
                w16(&mut m, LCDC_A + bank * 0x2_0000, 1 << bank);
            });
            w32(&mut m, DISPCNT_A_ADDR, MODE2 | (block << 18));
            m.render_frame();
            pixel(&m, Screen::Top, 0, 0)
        })
        .collect()
}

#[test]
fn given_banks_a_to_d_enabled_when_each_block_is_displayed_then_the_pixel_comes_from_that_bank() {
    // Red 1 / 2 / 4 / 8 expands (c << 3 | c >> 2) to 8, 16, 33 and 66, in the high byte.
    assert_eq!(
        first_pixel_per_block(),
        vec![0x0008_0000, 0x0010_0000, 0x0021_0000, 0x0042_0000]
    );
}

/// The red of 15-bit `001Fh`, and the picture the three swap tests draw.
const RED_PIXEL: u32 = 0x00FF_0000;

#[test]
fn given_powcnt1_bit_15_set_when_mode_2_is_rendered_then_top_shows_the_picture_and_bottom_is_engine_b_off()
 {
    // Engine B's DISPCNT is 0, display off: white (`display-mode-vram.md`).
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x001F);
    m.render_frame();
    assert_eq!(
        (
            not_equal_to(&m, Screen::Top, RED_PIXEL),
            not_equal_to(&m, Screen::Bottom, WHITE)
        ),
        (0, 0)
    );
}

#[test]
fn given_powcnt1_bit_15_clear_when_mode_2_is_rendered_then_bottom_shows_the_picture_and_top_is_engine_b_off()
 {
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x001F);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_BOTTOM);
    m.render_frame();
    assert_eq!(
        (
            not_equal_to(&m, Screen::Bottom, RED_PIXEL),
            not_equal_to(&m, Screen::Top, WHITE)
        ),
        (0, 0)
    );
}

#[test]
fn given_a_rendered_frame_when_bit_15_is_cleared_and_rendered_again_then_the_picture_moves_to_bottom()
 {
    let mut m = mode2_machine();
    fill_bank(&mut m, 0x001F);
    m.render_frame();
    let first_top_red = not_equal_to(&m, Screen::Top, RED_PIXEL);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_BOTTOM);
    m.render_frame();
    assert_eq!(
        (
            first_top_red,
            not_equal_to(&m, Screen::Bottom, RED_PIXEL),
            not_equal_to(&m, Screen::Top, WHITE)
        ),
        (0, 0, 0)
    );
}

#[test]
fn given_a_pattern_in_bank_a_when_mode_2_is_rendered_then_every_pixel_matches_its_halfword() {
    assert_eq!(whole_frame_mismatches(), Vec::<(usize, usize)>::new());
}

#[test]
fn given_a_rendered_mode_2_frame_when_lengths_are_read_then_both_screens_are_256_by_192() {
    let mut m = mode2_machine();
    m.render_frame();
    assert_eq!(
        (m.frame(Screen::Top).len(), m.frame(Screen::Bottom).len()),
        (PIXELS, PIXELS)
    );
}

#[test]
fn given_two_machines_with_identical_writes_when_rendered_then_their_frames_are_equal() {
    let build = || {
        let mut m = mode2_machine();
        put_pixel(&mut m, 3, 4, 0x1234);
        put_pixel(&mut m, 200, 100, 0x6ABC);
        m.render_frame();
        m
    };
    let (a, b) = (build(), build());
    assert_eq!(
        (
            a.frame(Screen::Top) == b.frame(Screen::Top),
            a.frame(Screen::Bottom) == b.frame(Screen::Bottom),
            a.frame(Screen::Top).len()
        ),
        (true, true, PIXELS)
    );
    assert_eq!(a.frame(Screen::Top), b.frame(Screen::Top));
}

#[test]
fn given_a_saved_machine_when_loaded_onto_a_fresh_one_then_registers_and_rendered_frames_match() {
    let mut saved = mode2_machine();
    put_pixel(&mut saved, 9, 9, 0x2BCD);
    put_pixel(&mut saved, 255, 191, 0x7FFF);
    let bytes = saved.save_state();
    let mut loaded = Machine::new();
    let result = loaded.load_state(&bytes);
    saved.render_frame();
    loaded.render_frame();
    assert_eq!(
        (
            result,
            r32(&loaded, DISPCNT_A_ADDR),
            r16(&loaded, POWCNT1_ADDR),
            loaded.frame(Screen::Top) == saved.frame(Screen::Top),
            loaded.frame(Screen::Bottom) == saved.frame(Screen::Bottom),
            pixel(&loaded, Screen::Top, 9, 9),
        ),
        (Ok(()), MODE2, POWCNT1_ON_TOP, true, true, 0x006B_F752)
    );
}

#[test]
fn given_a_machine_with_no_program_when_a_frame_runs_then_both_screens_stay_black() {
    let mut m = Machine::new();
    m.run_frame();
    assert_eq!(
        (
            not_equal_to(&m, Screen::Top, BLACK),
            not_equal_to(&m, Screen::Bottom, BLACK)
        ),
        (0, 0)
    );
}
