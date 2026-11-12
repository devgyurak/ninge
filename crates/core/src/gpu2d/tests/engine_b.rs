// SPDX-License-Identifier: Apache-2.0
//! Engine B (issue #30). Every expected value comes from `docs/hw/bg-modes.md` ("Engine B",
//! "Registers") and `docs/hw/palette-ram.md`, worked out by hand, never from running the renderer.

use super::*;

/// VRAMCNT_C (`docs/hw/vram-banks.md`).
const VRAMCNT_C: u32 = 0x0400_0242;
/// Bank C enabled with MST 4: engine B's BG VRAM, at offset 0.
const BANK_C_BG_B: u8 = 0x80 | 4;
/// Engine B's BG VRAM window, and its half of the palette RAM.
const BG_B_VRAM: u32 = 0x0620_0000;
const PAL_B: u32 = 0x0500_0400;
const PAL_A: u32 = 0x0500_0000;
/// DISPCNT with display mode 1 and BG0 on.
const MODE1_BG0: u32 = (1 << 16) | (1 << 8);
/// Character block 1 (16 KiB in), screen block 0.
const BG0CNT_CHAR1: u16 = 1 << 2;
const RED_15: u16 = 0x001F;
const BLUE_15: u16 = 0x7C00;
const GREEN_15: u16 = 0x03E0;
const RED: u32 = 0x00FF_0000;
const BLUE: u32 = 0x0000_00FF;
const GREEN: u32 = 0x0000_FF00;

fn bottom(m: &Machine, x: usize, y: usize) -> u32 {
    pixel(m, Screen::Bottom, x, y)
}

/// Engine B with a layer: bank C is engine B's BG VRAM, tile 1 of character block 1 is solid
/// value 1, map cell (0, 0) shows it, palette B entry 1 is red and entry 0 (the backdrop) blue.
/// Engine A is on the upper screen.
fn engine_b_machine() -> Machine {
    let mut m = Machine::new();
    w8(&mut m, VRAMCNT_C, BANK_C_BG_B);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w16(&mut m, PAL_B, BLUE_15);
    w16(&mut m, PAL_B + 2, RED_15);
    solid_tile_1(&mut m, BG_B_VRAM + 0x4000);
    w16(&mut m, BG_B_VRAM, 0x0001);
    w16(&mut m, BG_REGS_B_ADDR, BG0CNT_CHAR1);
    w32(&mut m, DISPCNT_B_ADDR, MODE1_BG0);
    m
}

/// Tile 1 of the 4-bits-per-pixel tile data at `data`: every pixel has the value 1.
fn solid_tile_1(m: &mut Machine, data: u32) {
    (0..8).for_each(|i| w32(m, data + 32 + i * 4, 0x1111_1111));
}

/// The cases (x, y, wanted) whose pixel of the lower screen differs, as (x, y, got, want).
fn bottom_failures(m: &Machine, cases: &[(usize, usize, u32)]) -> Vec<(usize, usize, u32, u32)> {
    cases
        .iter()
        .filter_map(|&(x, y, want)| {
            let got = bottom(m, x, y);
            (got != want).then_some((x, y, got, want))
        })
        .collect()
}

#[test]
fn given_a_word_written_at_dispcnt_b_when_read_then_it_is_kept_and_engine_a_is_unchanged() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_B_ADDR, 0x1234_5678);
    assert_eq!(
        (r32(&m, DISPCNT_B_ADDR), r32(&m, DISPCNT_A_ADDR)),
        (0x1234_5678, 0)
    );
}

#[test]
fn given_a_byte_written_to_dispcnt_b_when_read_then_only_that_byte_changes() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_B_ADDR, 0x1122_3344);
    w8(&mut m, DISPCNT_B_ADDR + 1, 0xEE);
    assert_eq!(r32(&m, DISPCNT_B_ADDR), 0x1122_EE44);
}

#[test]
fn given_bg_registers_written_in_both_engines_when_read_then_each_engine_keeps_its_own() {
    let mut m = Machine::new();
    w16(&mut m, BG_REGS_ADDR, 0x1111);
    w16(&mut m, BG_REGS_B_ADDR, 0x2222);
    assert_eq!(
        (r16(&m, BG_REGS_ADDR), r16(&m, BG_REGS_B_ADDR)),
        (0x1111, 0x2222)
    );
}

#[test]
fn given_a_scroll_written_in_engine_b_when_decoded_then_engine_b_has_it_and_engine_a_does_not() {
    let mut m = Machine::new();
    w16(&mut m, BG_REGS_B_ADDR + 8, 0x0123);
    let gpu = m.inspect().gpu2d();
    assert_eq!(
        (
            gpu.text_bg(EngineId::B, 0).hofs,
            gpu.text_bg(EngineId::A, 0).hofs
        ),
        (0x123, 0)
    );
}

#[test]
fn given_engine_b_registers_after_power_on_when_read_then_dispcnt_and_bgcnt_are_zero() {
    let m = Machine::new();
    assert_eq!((r32(&m, DISPCNT_B_ADDR), r16(&m, BG_REGS_B_ADDR)), (0, 0));
}

#[test]
fn given_dispcnt_b_written_when_inspected_then_dispcnt_b_shows_it() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_B_ADDR, 0x0001_0100);
    assert_eq!(m.inspect().gpu2d().dispcnt_b(), 0x0001_0100);
}

#[test]
fn given_a_power_on_machine_when_a_frame_is_rendered_then_both_screens_are_white() {
    // Engine B's DISPCNT is 0, display off, and display off is white (`display-mode-vram.md`).
    let mut m = Machine::new();
    m.render_frame();
    assert_eq!(
        (
            not_equal_to(&m, Screen::Top, WHITE),
            not_equal_to(&m, Screen::Bottom, WHITE)
        ),
        (0, 0)
    );
}

#[test]
fn given_a_text_layer_in_engine_b_when_rendered_then_the_lower_screen_shows_it_over_its_backdrop() {
    let mut m = engine_b_machine();
    m.render_frame();
    let failures = bottom_failures(
        &m,
        &[
            (0, 0, RED),
            (7, 7, RED),
            (8, 0, BLUE),
            (0, 8, BLUE),
            (255, 191, BLUE),
        ],
    );
    assert_eq!(failures, Vec::<(usize, usize, u32, u32)>::new());
}

#[test]
fn given_a_text_layer_in_engine_b_when_rendered_then_engine_a_screen_is_unaffected() {
    let mut m = engine_b_machine();
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Top, WHITE), 0);
}

#[test]
fn given_powcnt1_bit_15_clear_when_engine_b_is_rendered_then_the_upper_screen_shows_it() {
    let mut m = engine_b_machine();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_BOTTOM);
    m.render_frame();
    assert_eq!(
        (pixel(&m, Screen::Top, 0, 0), pixel(&m, Screen::Top, 8, 0)),
        (RED, BLUE)
    );
}

#[test]
fn given_a_backdrop_in_each_half_of_the_palette_when_rendered_then_each_engine_uses_its_own() {
    let mut m = engine_b_machine();
    w16(&mut m, PAL_A, GREEN_15);
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    m.render_frame();
    assert_eq!(
        (pixel(&m, Screen::Top, 100, 100), bottom(&m, 100, 100)),
        (GREEN, BLUE)
    );
}

#[test]
fn given_a_layer_in_engine_b_when_the_bg_window_bank_is_disabled_then_the_screen_is_the_backdrop() {
    // A bank that is off reads 0: no tile pixels, so only the backdrop shows.
    let mut m = engine_b_machine();
    w8(&mut m, VRAMCNT_C, 0);
    m.render_frame();
    assert_eq!(
        bottom_failures(&m, &[(0, 0, BLUE), (100, 100, BLUE)]),
        vec![]
    );
}

#[test]
fn given_engine_b_in_display_mode_0_when_rendered_then_its_screen_is_white() {
    let mut m = engine_b_machine();
    w32(&mut m, DISPCNT_B_ADDR, 1 << 8);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Bottom, WHITE), 0);
}

#[test]
fn given_engine_b_with_display_mode_bit_17_set_when_rendered_then_it_has_no_mode_2_and_acts_as_mode_0()
 {
    // Engine B has display modes 0-1 only (`bg-modes.md`, "Engine B"); bit 17 does not exist.
    let mut m = engine_b_machine();
    w32(&mut m, DISPCNT_B_ADDR, 2 << 16);
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Bottom, WHITE), 0);
}

#[test]
fn given_engine_b_with_display_mode_3_when_rendered_then_it_acts_as_mode_1() {
    // Bit 16 is there and bit 17 is not, so mode 3 is mode 1.
    let mut m = engine_b_machine();
    w32(&mut m, DISPCNT_B_ADDR, MODE1_BG0 | (2 << 16));
    m.render_frame();
    assert_eq!(bottom_failures(&m, &[(0, 0, RED), (8, 0, BLUE)]), vec![]);
}

#[test]
fn given_character_and_screen_bases_in_dispcnt_b_when_decoded_then_engine_b_ignores_them() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_B_ADDR, (3 << 24) | (5 << 27));
    w32(&mut m, DISPCNT_A_ADDR, (3 << 24) | (5 << 27));
    w16(&mut m, BG_REGS_B_ADDR, 0x0204);
    w16(&mut m, BG_REGS_ADDR, 0x0204);
    let gpu = m.inspect().gpu2d();
    let (a, b) = (gpu.text_bg(EngineId::A, 0), gpu.text_bg(EngineId::B, 0));
    assert_eq!(
        (b.char_base, b.screen_base, a.char_base, a.screen_base),
        (0x4000, 0x1000, 3 * 0x1_0000 + 0x4000, 5 * 0x1_0000 + 0x1000)
    );
}

#[test]
fn given_dispcnt_bit_3_set_in_engine_b_when_rendered_then_bg0_is_still_a_text_layer() {
    // Engine B has no 3D BG0 (`bg-modes.md`, "Engine B"); engine A would hand BG0 to the 3D engine.
    let mut m = engine_b_machine();
    w32(&mut m, DISPCNT_B_ADDR, MODE1_BG0 | (1 << 3));
    m.render_frame();
    assert_eq!(bottom_failures(&m, &[(0, 0, RED)]), vec![]);
}

#[test]
fn given_dispcnt_bit_3_set_in_engine_a_when_rendered_then_bg0_draws_nothing_yet() {
    // TODO(phase7): BG0 is the 3D layer; it draws nothing until then.
    let mut m = engine_b_machine();
    w8(&mut m, VRAMCNT_A, ENABLE_MST0 | 1);
    w16(&mut m, PAL_A, GREEN_15);
    w32(&mut m, DISPCNT_A_ADDR, MODE1_BG0 | (1 << 3));
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 0, 0), GREEN);
}

#[test]
fn given_powcnt1_bit_9_clear_when_engine_b_registers_are_written_then_they_stay_as_they_were() {
    // The ports of a powered-off engine are read-only (`bg-modes.md`, "Engine B").
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_B_ADDR, 0x0001_0100);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP & !(1 << 9));
    w32(&mut m, DISPCNT_B_ADDR, 0x0002_0200);
    w16(&mut m, BG_REGS_B_ADDR, 0x1234);
    assert_eq!(
        (r32(&m, DISPCNT_B_ADDR), r16(&m, BG_REGS_B_ADDR)),
        (0x0001_0100, 0)
    );
}

#[test]
fn given_powcnt1_bit_9_clear_when_rendered_then_the_screen_engine_b_feeds_is_black() {
    let mut m = engine_b_machine();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP & !(1 << 9));
    m.render_frame();
    assert_eq!(not_equal_to(&m, Screen::Bottom, BLACK), 0);
}

#[test]
fn given_engine_b_powered_off_and_on_again_when_rendered_then_the_registers_it_had_draw_again() {
    let mut m = engine_b_machine();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP & !(1 << 9));
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    m.render_frame();
    assert_eq!(bottom_failures(&m, &[(0, 0, RED)]), vec![]);
}

#[test]
fn given_engine_b_configured_when_saved_and_loaded_then_it_draws_the_same_frame() {
    let mut saver = engine_b_machine();
    saver.render_frame();
    let mut target = Machine::new();
    let bytes = saver.save_state();
    assert_eq!(target.load_state(&bytes), Ok(()));
    target.render_frame();
    assert_eq!(target.frame(Screen::Bottom), saver.frame(Screen::Bottom));
}

#[test]
fn given_two_layers_in_both_engines_when_rendered_then_each_screen_shows_its_own_engine() {
    let mut m = engine_b_machine();
    // Engine A: BG VRAM from bank A, a solid tile 1 of value 1 in palette entry 1 = green.
    w8(&mut m, VRAMCNT_A, ENABLE_MST0 | 1);
    w16(&mut m, PAL_A + 2, GREEN_15);
    solid_tile_1(&mut m, 0x0600_4000);
    w16(&mut m, 0x0600_0000, 0x0001);
    w16(&mut m, BG_REGS_ADDR, BG0CNT_CHAR1);
    w32(&mut m, DISPCNT_A_ADDR, MODE1_BG0);
    m.render_frame();
    assert_eq!(
        (pixel(&m, Screen::Top, 0, 0), bottom(&m, 0, 0)),
        (GREEN, RED)
    );
}
