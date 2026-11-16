// SPDX-License-Identifier: Apache-2.0
//! The master brightness through the whole machine (issue #34). Every expected value is worked out by
//! hand from `docs/hw/blending.md`, never from the implementation; see `brightness/tests.rs` for the
//! arithmetic. A colour is 15 bits.

use super::affine::{ENG_A, ENG_B};
use super::window::{GRAPHICS_BGS, RED, layers_scene, misses};
use super::*;
use crate::debug::{BrightnessMode, EngineId};

/// Offset of MASTER_BRIGHT from an engine's DISPCNT, and the mode bits.
const MASTER_BRIGHT: u32 = 0x6C;
const UP: u16 = 1 << 14;
const DOWN: u16 = 2 << 14;
const RESERVED: u16 = 3 << 14;
const BLDCNT: u32 = 0x50;
const BLDALPHA: u32 = 0x52;

/// A register value, a colour in the VRAM display and the pixel wanted.
struct Case {
    register: u16,
    color: u16,
    want: u32,
}

const fn case(register: u16, color: u16, want: u32) -> Case {
    Case {
        register,
        color,
        want,
    }
}

/// The (index, got) of the cases whose pixel (0, 0) of the upper screen is not the one wanted when
/// engine A shows `color` in display mode 2 under `register`.
fn vram_misses(cases: &[Case]) -> Vec<(usize, u32)> {
    cases
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let mut m = mode2_machine();
            put_pixel(&mut m, 0, 0, c.color);
            w16(&mut m, DISPCNT_A_ADDR + MASTER_BRIGHT, c.register);
            m.render_frame();
            let got = pixel(&m, Screen::Top, 0, 0);
            (got != c.want).then_some((i, got))
        })
        .collect()
}

#[test]
fn given_display_mode_2_when_the_master_brightness_is_set_then_the_vram_pixel_is_brightened() {
    let cases = [
        case(UP | 8, 0x0000, 0x007D_7D7D),
        case(UP | 16, 0x0000, 0x00FF_FFFF),
        case(DOWN | 8, 0x7FFF, 0x0082_8282),
        case(DOWN | 31, 0x7FFF, 0),
        case(UP | 8, 0x001F, 0x00FF_7D7D),
        // The unit off, or reserved: the plain expansion of 5-bit 16, 132.
        case(8, 0x4210, 0x0084_8484),
        case(RESERVED | 8, 0x4210, 0x0084_8484),
    ];
    assert_eq!(vram_misses(&cases), vec![]);
}

#[test]
fn given_the_layers_when_the_master_brightness_is_down_then_it_applies_after_the_layers() {
    let mut m = layers_scene(ENG_A, GRAPHICS_BGS, &[]);
    w16(&mut m, ENG_A.dispcnt + MASTER_BRIGHT, DOWN | 8);
    // BG0 red 31,0,0 darkened at 8/16: 63 -> 32 -> 130 in red, 0 elsewhere.
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 5, 7), 0x0082_0000);
}

#[test]
fn given_blending_when_the_master_brightness_is_up_then_the_blended_pixel_is_brightened() {
    let mut m = layers_scene(ENG_A, GRAPHICS_BGS, &[]);
    // BG0 over BG1 at 8/8: red 31,0,0 and green 0,31,0 mix to 15,15,0. Widened 30,30,0, moved up at 8/16:
    // 30 + 33 * 8 / 16 = 46 and 0 + 63 * 8 / 16 = 31, expanded 186 and 125.
    w16(&mut m, ENG_A.dispcnt + BLDCNT, 1 | 1 << 6 | 1 << 9);
    w16(&mut m, ENG_A.dispcnt + BLDALPHA, 8 | 8 << 8);
    w16(&mut m, ENG_A.dispcnt + MASTER_BRIGHT, UP | 8);
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 5, 7), 0x00BA_BA7D);
}

#[test]
fn given_display_mode_0_when_the_master_brightness_is_down_then_the_white_screen_darkens_hw_unverified()
 {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w16(&mut m, DISPCNT_A_ADDR + MASTER_BRIGHT, DOWN | 8);
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 100, 100), 0x0082_8282);
}

#[test]
fn given_engine_b_when_its_master_brightness_is_set_then_only_the_lower_screen_changes() {
    let mut m = layers_scene(ENG_B, GRAPHICS_BGS, &[]);
    w16(&mut m, ENG_B.dispcnt + MASTER_BRIGHT, DOWN | 8);
    m.render_frame();
    assert_eq!(
        (
            pixel(&m, Screen::Bottom, 5, 7),
            // Engine A is off (white) and has no brightness of its own.
            pixel(&m, Screen::Top, 5, 7)
        ),
        (0x0082_0000, 0x00FF_FFFF)
    );
}

#[test]
fn given_engine_a_with_a_master_brightness_when_engine_b_has_none_then_the_lower_screen_is_plain() {
    let mut m = layers_scene(ENG_B, GRAPHICS_BGS, &[]);
    w16(&mut m, ENG_A.dispcnt + MASTER_BRIGHT, DOWN | 16);
    assert_eq!(misses(&mut m, ENG_B, &[(5, 7, RED)]), vec![]);
}

#[test]
fn given_engine_b_powered_off_when_its_master_brightness_is_up_then_the_screen_stays_black() {
    let mut m = Machine::new();
    w16(&mut m, POWCNT1_ADDR, 0x820F & !(1 << 9));
    // MASTER_BRIGHT is not behind POWCNT1 bit 9 (`mod.rs`, `GATED_PORTS_END`), so this write lands.
    w16(&mut m, DISPCNT_B_ADDR + MASTER_BRIGHT, UP | 16);
    m.render_frame();
    assert_eq!(
        (
            r16(&m, DISPCNT_B_ADDR + MASTER_BRIGHT),
            pixel(&m, Screen::Bottom, 5, 7)
        ),
        (UP | 16, 0)
    );
}

#[test]
fn given_all_bits_written_when_master_bright_is_read_then_the_unused_bits_read_0() {
    let mut m = Machine::new();
    w16(&mut m, DISPCNT_A_ADDR + MASTER_BRIGHT, 0xFFFF);
    w16(&mut m, DISPCNT_B_ADDR + MASTER_BRIGHT, 0xFFFF);
    assert_eq!(
        (
            r16(&m, DISPCNT_A_ADDR + MASTER_BRIGHT),
            r16(&m, DISPCNT_B_ADDR + MASTER_BRIGHT)
        ),
        (0xC01F, 0xC01F)
    );
}

#[test]
fn given_a_byte_write_to_the_high_byte_when_read_then_the_factor_byte_is_kept() {
    let mut m = Machine::new();
    w8(&mut m, DISPCNT_A_ADDR + MASTER_BRIGHT, 0x08);
    w8(&mut m, DISPCNT_A_ADDR + MASTER_BRIGHT + 1, 0x80);
    assert_eq!(r16(&m, DISPCNT_A_ADDR + MASTER_BRIGHT), 0x8008);
}

#[test]
fn given_the_register_written_when_inspected_then_the_decoded_view_is_the_note_s() {
    let mut m = Machine::new();
    w16(&mut m, DISPCNT_A_ADDR + MASTER_BRIGHT, UP | 20);
    w16(&mut m, DISPCNT_B_ADDR + MASTER_BRIGHT, DOWN | 3);
    let gpu = m.inspect().gpu2d();
    let (a, b) = (
        gpu.master_brightness(EngineId::A),
        gpu.master_brightness(EngineId::B),
    );
    assert_eq!(
        ((a.mode, a.factor), (b.mode, b.factor)),
        ((BrightnessMode::Up, 16), (BrightnessMode::Down, 3))
    );
}

#[test]
fn given_a_master_brightness_when_the_native_line_is_made_then_it_is_unbrightened_and_the_factor_returned()
 {
    let mut m = mode2_machine();
    put_pixel(&mut m, 0, 0, 0x001F);
    w16(&mut m, DISPCNT_A_ADDR + MASTER_BRIGHT, UP | 8);
    let mut line = [0; SCREEN_WIDTH];
    let inputs = m.memory.engine_inputs(EngineId::A);
    let applied =
        m.io.gpu2d()
            .render_line_a(0, m.memory.lcdc_banks(), &inputs, &mut line);
    assert_eq!(
        (line.first().copied(), applied.mode, applied.factor),
        (Some(0x001F), BrightnessMode::Up, 8)
    );
}
