// SPDX-License-Identifier: Apache-2.0
//! Engine A's composed line and what the screen picks (issue #36, `docs/hw/display-capture.md`,
//! "Where the capture unit sits"): the graphics screen is composed whatever display mode the
//! engine shows, and the display mode only picks which picture reaches the screen. Expected values
//! are worked out by hand from the note.

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
