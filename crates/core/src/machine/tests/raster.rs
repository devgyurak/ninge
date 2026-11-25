// SPDX-License-Identifier: Apache-2.0
//! When the rendering engine draws a frame and whether its picture survives a savestate
//! (`docs/hw/rasterizer.md`, "When the hardware draws", "Where the buffer lives and whether it is
//! saved"). The rear plane stands in for a scene: CLEAR_COLOR is a register the program writes at
//! a time of its choosing, and the picture shows it wherever no polygon draws.

use super::timing::{line_start, machine, run_to};
use crate::cpu::Bus;
use crate::gpu2d::Pixel3d;
use crate::machine::*;

const CLEAR_COLOR: u32 = 0x0400_0350;
const RENDER_START: u64 = 214;
const LINES: u64 = 263;
/// Opaque red.
const RED: u32 = (31 << 16) | 0x1F;
const GREEN: u32 = (31 << 16) | 0x3E0;

fn set_clear(m: &mut Machine, value: u32) {
    m.bus9().write32(CLEAR_COLOR, value);
}

fn pixel(m: &Machine) -> Pixel3d {
    m.frame3d[0]
}

fn red() -> Pixel3d {
    Pixel3d {
        color: 0x1F,
        alpha: 31,
    }
}

#[test]
fn given_a_clear_colour_written_in_vblank_when_line_214_starts_then_the_picture_has_it() {
    let mut m = machine();
    run_to(&mut m, line_start(200));
    set_clear(&mut m, RED);
    run_to(&mut m, line_start(RENDER_START));
    assert_eq!(pixel(&m), red());
}

#[test]
fn given_a_clear_colour_written_in_vblank_when_the_line_before_214_ends_then_the_picture_is_unchanged()
 {
    let mut m = machine();
    run_to(&mut m, line_start(200));
    set_clear(&mut m, RED);
    run_to(&mut m, line_start(RENDER_START) - 1);
    assert_eq!(pixel(&m), Pixel3d::default());
}

#[test]
fn given_a_clear_colour_written_after_line_214_when_the_frame_ends_then_the_picture_is_unchanged() {
    let mut m = machine();
    run_to(&mut m, line_start(RENDER_START + 1));
    set_clear(&mut m, RED);
    run_to(&mut m, line_start(LINES - 1));
    assert_eq!(pixel(&m), Pixel3d::default());
}

#[test]
fn given_a_clear_colour_written_after_line_214_when_the_next_frame_reaches_214_then_it_shows() {
    let mut m = machine();
    run_to(&mut m, line_start(RENDER_START + 1));
    set_clear(&mut m, RED);
    run_to(&mut m, line_start(LINES + RENDER_START));
    assert_eq!(pixel(&m), red());
}

#[test]
fn given_a_state_saved_between_line_192_and_214_when_loaded_and_run_on_then_it_matches_a_run_without_a_stop()
 {
    let mut whole = machine();
    run_to(&mut whole, line_start(200));
    set_clear(&mut whole, RED);
    let bytes = whole.save_state();
    run_to(&mut whole, line_start(LINES + 10));

    let mut resumed = machine();
    resumed
        .load_state(&bytes)
        .expect("a state this build saved");
    run_to(&mut resumed, line_start(LINES + 10));
    assert_eq!(
        (
            resumed.frame3d == whole.frame3d,
            resumed.save_state() == whole.save_state()
        ),
        (true, true)
    );
}

#[test]
fn given_a_state_saved_after_the_draw_when_loaded_then_the_picture_comes_back_as_it_was() {
    let mut saved = machine();
    set_clear(&mut saved, GREEN);
    run_to(&mut saved, line_start(RENDER_START + 1));
    let mut loaded = machine();
    loaded
        .load_state(&saved.save_state())
        .expect("a state this build saved");
    assert_eq!(loaded.frame3d, saved.frame3d);
}
