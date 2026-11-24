// SPDX-License-Identifier: Apache-2.0
//! Engine A's composed line and what the screen picks (issue #36, `docs/hw/display-capture.md`,
//! "Where the capture unit sits"): the graphics screen is composed whatever display mode the
//! engine shows, and the display mode only picks which picture reaches the screen. Expected values
//! are worked out by hand from the note.

use super::affine::ENG_A;
use super::window::{
    BG0, BG1, BG2, BG3, GRAPHICS_BGS, GREEN, MAGENTA, WIN0_ON, WIN0H, WIN0V, WININ, WINOUT,
    layers_scene, range,
};
use super::*;

/// Palette RAM entry 0 of engine A: the backdrop, the colour of a pixel no layer draws.
const BACKDROP: u32 = 0x0500_0000;
/// Pure red and pure blue in 15-bit colour.
const RED15: u16 = 0x001F;
const BLUE15: u16 = 0x7C00;

/// Display mode 2 on bank A full of blue, with a red backdrop: the layers hold red, the VRAM blue.
fn mode2_blue_over_red_backdrop() -> Machine {
    let mut m = mode2_machine();
    fill_bank(&mut m, BLUE15);
    w16(&mut m, BACKDROP, RED15);
    m
}

#[test]
fn given_display_mode_2_when_the_layers_are_composed_then_the_line_is_the_backdrop_not_the_vram() {
    let m = mode2_blue_over_red_backdrop();
    let inputs = m.memory.engine_inputs(EngineId::A);
    let mut composed = [0; SCREEN_WIDTH];
    m.io.gpu2d().compose_line_a(0, &inputs, &mut composed);
    assert_eq!(composed, [RED15; SCREEN_WIDTH]);
}

#[test]
fn given_display_mode_2_when_the_screen_picks_its_line_then_the_vram_wins_over_the_composed_line() {
    let m = mode2_blue_over_red_backdrop();
    let inputs = m.memory.engine_inputs(EngineId::A);
    let mut line = [0; SCREEN_WIDTH];
    m.io.gpu2d().select_line_a(
        0,
        m.memory.lcdc_banks(),
        &inputs,
        &[RED15; SCREEN_WIDTH],
        &[0; SCREEN_WIDTH],
        &mut line,
    );
    assert_eq!(line, [BLUE15; SCREEN_WIDTH]);
}

#[test]
fn given_display_mode_1_when_the_screen_picks_its_line_then_it_is_the_composed_line() {
    let mut m = mode2_blue_over_red_backdrop();
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    let inputs = m.memory.engine_inputs(EngineId::A);
    let mut line = [0; SCREEN_WIDTH];
    m.io.gpu2d().select_line_a(
        0,
        m.memory.lcdc_banks(),
        &inputs,
        &[0x03E0; SCREEN_WIDTH],
        &[0; SCREEN_WIDTH],
        &mut line,
    );
    assert_eq!(line, [0x03E0; SCREEN_WIDTH]);
}

#[test]
fn given_display_mode_0_when_asked_whether_it_shows_the_layers_then_it_does_not() {
    let m = Machine::new();
    let inputs = m.memory.engine_inputs(EngineId::A);
    let shows = m.io.gpu2d().shows_layers_a(&inputs);
    assert!(!shows);
}

#[test]
fn given_display_mode_1_when_asked_whether_it_shows_the_layers_then_it_does() {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    let inputs = m.memory.engine_inputs(EngineId::A);
    assert!(m.io.gpu2d().shows_layers_a(&inputs));
}

/// DISPCNT: display mode 1 (layers), BG0 on, and BG0 handed to the 3D engine.
const MODE1_BG0_3D: u32 = (1 << 16) | (1 << 8) | (1 << 3);
/// A green 3D pixel, opaque.
const GREEN_OPAQUE: Pixel3d = Pixel3d {
    color: 0x03E0,
    alpha: 31,
};

/// The line engine A composes at `y = 0` with `three_d` as the 3D layer, under `dispcnt`, over a red
/// backdrop.
fn composed_with(dispcnt: u32, three_d: &Line3d) -> [u16; SCREEN_WIDTH] {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, dispcnt);
    w16(&mut m, BACKDROP, RED15);
    let inputs = m.memory.engine_inputs(EngineId::A).with_three_d(three_d);
    let mut composed = [0; SCREEN_WIDTH];
    m.io.gpu2d().compose_line_a(0, &inputs, &mut composed);
    composed
}

/// A 3D line with `pixel` at column 5 and nothing elsewhere.
fn line_with(pixel: Pixel3d) -> Line3d {
    let mut line = TRANSPARENT_LINE;
    line[5] = pixel;
    line
}

#[test]
fn given_bg0_as_the_3d_layer_when_a_pixel_is_opaque_then_it_shows_over_the_backdrop() {
    let composed = composed_with(MODE1_BG0_3D, &line_with(GREEN_OPAQUE));
    assert_eq!(
        (composed[4], composed[5], composed[6]),
        (RED15, 0x03E0, RED15)
    );
}

#[test]
fn given_bg0_as_the_3d_layer_when_a_pixel_has_alpha_zero_then_the_backdrop_shows() {
    let transparent = Pixel3d {
        color: 0x03E0,
        alpha: 0,
    };
    let composed = composed_with(MODE1_BG0_3D, &line_with(transparent));
    assert_eq!(composed[5], RED15);
}

#[test]
fn given_bg0_not_handed_to_3d_when_a_3d_pixel_is_opaque_then_the_backdrop_shows() {
    let composed = composed_with((1 << 16) | (1 << 8), &line_with(GREEN_OPAQUE));
    assert_eq!(composed[5], RED15);
}

#[test]
fn given_the_3d_layer_off_in_dispcnt_when_a_3d_pixel_is_opaque_then_the_backdrop_shows() {
    let composed = composed_with((1 << 16) | (1 << 3), &line_with(GREEN_OPAQUE));
    assert_eq!(composed[5], RED15);
}

/// Offsets from an engine's DISPCNT: BG0CNT, BG0HOFS, and the blend control and alpha registers.
const BG0CNT: u32 = 0x08;
const BG0HOFS: u32 = 0x10;
const BG2CNT: u32 = 0x0C;
const BG3CNT: u32 = 0x0E;
const BLDCNT_OFFSET: u32 = 0x50;
const BLDALPHA_OFFSET: u32 = 0x52;
/// An opaque magenta 3D pixel, a colour none of the four layers has.
const MAGENTA_OPAQUE: Pixel3d = Pixel3d {
    color: 0x7C1F,
    alpha: 31,
};

/// The line engine A composes at `y = 0` over the four solid layers of `window.rs` (BG1 green, BG2
/// blue and BG3 yellow behind BG0, which `dispcnt` hands to 3D), with `regs` written first.
fn scene_composed(dispcnt: u32, regs: &[(u32, u16)], three_d: &Line3d) -> [u16; SCREEN_WIDTH] {
    let mut m = layers_scene(ENG_A, dispcnt, regs);
    m.io.gpu2d_mut().begin_line(0);
    let inputs = m.memory.engine_inputs(EngineId::A).with_three_d(three_d);
    let mut composed = [0; SCREEN_WIDTH];
    m.io.gpu2d().compose_line_a(0, &inputs, &mut composed);
    composed
}

/// A 3D line with `pixel` at columns 5 and 20.
fn line_at_5_and_20(pixel: Pixel3d) -> Line3d {
    let mut line = TRANSPARENT_LINE;
    line[5] = pixel;
    line[20] = pixel;
    line
}

#[test]
fn given_bg0hofs_of_3_when_the_3d_layer_is_composed_then_column_5_shows_at_x_2() {
    let line = line_at_5_and_20(MAGENTA_OPAQUE);
    let composed = scene_composed(GRAPHICS_BGS | 1 << 3, &[(BG0HOFS, 3)], &line);
    assert_eq!(
        (composed[2], composed[5], composed[17]),
        (MAGENTA, GREEN, MAGENTA)
    );
}

#[test]
fn given_bg0hofs_of_256_when_the_3d_layer_is_composed_then_the_picture_is_scrolled_out() {
    let line = line_at_5_and_20(MAGENTA_OPAQUE);
    let composed = scene_composed(GRAPHICS_BGS | 1 << 3, &[(BG0HOFS, 256)], &line);
    assert_eq!(composed, [GREEN; SCREEN_WIDTH]);
}

#[test]
fn given_bg0hofs_of_510_when_the_3d_layer_is_composed_then_the_picture_wraps_in_from_the_right() {
    let line = line_at_5_and_20(MAGENTA_OPAQUE);
    let composed = scene_composed(GRAPHICS_BGS | 1 << 3, &[(BG0HOFS, 510)], &line);
    assert_eq!(
        (composed[1], composed[7], composed[8]),
        (GREEN, MAGENTA, GREEN)
    );
}

#[test]
fn given_bg0_behind_bg1_when_the_3d_layer_is_composed_then_bg1_shows_over_it() {
    let line = line_at_5_and_20(MAGENTA_OPAQUE);
    let composed = scene_composed(GRAPHICS_BGS | 1 << 3, &[(BG0CNT, 1)], &line);
    assert_eq!((composed[5], composed[20]), (GREEN, GREEN));
}

#[test]
fn given_a_window_without_bg0_when_the_3d_layer_is_composed_then_it_is_hidden_inside_only() {
    let line = line_at_5_and_20(MAGENTA_OPAQUE);
    let regs = [
        (WIN0H, range(0, 10)),
        (WIN0V, range(0, 192)),
        (WININ, BG1 | BG2 | BG3),
        (WINOUT, BG0 | BG1 | BG2 | BG3),
    ];
    let composed = scene_composed(GRAPHICS_BGS | 1 << 3 | WIN0_ON, &regs, &line);
    assert_eq!((composed[5], composed[20]), (GREEN, MAGENTA));
}

#[test]
fn given_bg0_as_the_second_target_when_bg1_blends_then_the_3d_pixel_is_the_pixel_below() {
    let line = line_at_5_and_20(Pixel3d {
        color: RED15,
        alpha: 31,
    });
    // BG1 (green) in front of the 3D layer (priority 1, BG2 and BG3 behind it at 2), BG1 the first
    // target, alpha 8/16 each, BG0 the second target.
    let regs = [
        (BG0CNT, 1),
        (BG2CNT, 1 << 2 | 2 << 8 | 2),
        (BG3CNT, 1 << 2 | 3 << 8 | 2),
        (BLDCNT_OFFSET, 2 | 1 << 6 | 1 << 8),
        (BLDALPHA_OFFSET, 8 | 8 << 8),
    ];
    let composed = scene_composed(GRAPHICS_BGS | 1 << 3, &regs, &line);
    assert_eq!((composed[5], composed[6]), (15 | 15 << 5, 0x03E0));
}
