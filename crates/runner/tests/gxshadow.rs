// SPDX-License-Identifier: Apache-2.0
//! The rasterizer's shadow polygons, observed through our own `gxshadow` test ROM
//! (`testroms/gxshadow/`): a mask pass of polygon mode 3 and ID 0, and a render pass over the
//! stencil it sets. The expected picture is worked out here from `docs/hw/rasterizer.md` ("Shadow
//! polygons"), pixel by pixel, from the coordinates the ROM's source lists. It is never read back
//! from the emulator: nothing below shares code with `ninge-core`.
//!
//! A shadow is black at alpha 15 blended over an opaque red floor, 63 x 16 / 32 = 31 in 6 bits, 15
//! in 5; with blending on a translucent polygon keeps its lower edge but not a vertical right one.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

const BLUE: u16 = 0x7C00;
const RED: u16 = 0x001F;

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// A 5-bit channel as 6 bits: `X x 2 + 1`, but 0 stays 0.
fn six(c: u16) -> u16 {
    if c == 0 { 0 } else { c * 2 + 1 }
}

/// `new` blended over `old` (both 15-bit) with the 5-bit alpha `alpha`.
fn blend(old: u16, new: u16, alpha: u16) -> u16 {
    let channel = |shift: u16| {
        let (o, n) = (six((old >> shift) & 31), six((new >> shift) & 31));
        ((n * (alpha + 1) + o * (31 - alpha)) / 32) >> 1
    };
    channel(0) | channel(5) << 5 | channel(10) << 10
}

/// The upper screen the ROM shows, as a grid of 15-bit colours.
fn expected() -> Vec<u16> {
    let mut grid = vec![BLUE; SCREEN_WIDTH * SCREEN_HEIGHT];
    let mut paint = |x0: usize, x1: usize, y0: usize, y1: usize, color: u16, alpha: u16| {
        (y0..y1).for_each(|y| {
            (x0..x1).for_each(|x| {
                let at = y * SCREEN_WIDTH + x;
                grid[at] = if alpha == 31 {
                    color
                } else {
                    blend(grid[at], color, alpha)
                };
            });
        });
    };
    let black = 0;
    // The four opaque floors (each drops its lower and right edges).
    paint(16, 112, 24, 72, RED, 31);
    paint(128, 224, 24, 72, RED, 31);
    paint(16, 112, 96, 144, RED, 31);
    paint(128, 224, 96, 144, RED, 31);
    // S1: shadowed (the mask is behind the floor, so it sets nothing). S2, S3: no shadow. S4: the
    // second render pass shadows, once.
    paint(32, 80, 36, 61, black, 15);
    paint(144, 192, 108, 133, black, 15);
    grid
}

fn top_after(frames: u64) -> Vec<u32> {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxshadow")).expect("gxshadow ROM is built"))
        .expect("gxshadow ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine.frame(Screen::Top).to_vec()
}

/// The first few pixels where the picture differs from the expected one: `(x, y, got, wanted)`.
fn differences(got: &[u32], wanted: &[u16]) -> Vec<(usize, usize, u32, u32)> {
    got.iter()
        .zip(wanted)
        .enumerate()
        .filter(|(_, (g, w))| **g != expand(**w))
        .map(|(at, (g, w))| (at % SCREEN_WIDTH, at / SCREEN_WIDTH, *g, expand(*w)))
        .take(8)
        .collect()
}

#[test]
fn given_the_gxshadow_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(differences(&top_after(FRAMES), &expected()), vec![]);
}

#[test]
fn given_the_gxshadow_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxshadow", FRAMES);
}

#[test]
fn given_the_gxshadow_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxshadow", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxshadow", 2, 2);
}
