// SPDX-License-Identifier: Apache-2.0
//! The rasterizer's depth rules, observed through our own `gxdepth` test ROM (`testroms/gxdepth/`):
//! W-buffering, the equal depth test with its tolerance, and the far-plane bit of POLYGON_ATTR. The
//! expected picture is worked out here from `docs/hw/rasterizer.md` and
//! `docs/hw/geometry-engine.md` ("Clipping"), pixel by pixel, from the coordinates the ROM's source
//! lists. It is never read back from the emulator: nothing below shares code with `ninge-core`.
//!
//! Every polygon loads its own W, so the depth the ROM compares is that W (SWAP_BUFFERS bit 1). The
//! rules the picture applies: with W-buffering the smaller W is nearer whatever the Z; a polygon
//! that asks for equal depth draws where the depth differs by at most 200h; a polygon with
//! POLYGON_ATTR bit 12 set that crosses the far plane draws its part inside, and one with it clear
//! is hidden.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

const BLUE: u16 = 0x7C00;
const RED: u16 = 0x001F;
const GREEN: u16 = 0x03E0;
const YELLOW: u16 = 0x03FF;
const MAGENTA: u16 = 0x7C1F;
const WHITE: u16 = 0x7FFF;

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The upper screen the ROM shows, as a grid of 15-bit colours, built region by region.
fn expected() -> Vec<u16> {
    let mut grid = vec![BLUE; SCREEN_WIDTH * SCREEN_HEIGHT];
    let mut paint = |x0: usize, x1: usize, y0: usize, y1: usize, color: u16| {
        (y0..y1).for_each(|y| (x0..x1).for_each(|x| grid[y * SCREEN_WIDTH + x] = color));
    };
    // R1: A (green, W 2.0) covers columns 16-79, B (red, W 1.0) covers 48-111 and wins the overlap
    // although its Z is the larger.
    paint(16, 48, 24, 72, GREEN);
    paint(48, 112, 24, 72, RED);
    // R2: P1 green 128-191. P2 asks for equal depth: it draws only over P1 (144-191), not over the
    // rear plane, whose depth is far from its W. P3 (less) draws where nothing is nearer: 192-223.
    paint(128, 144, 24, 72, GREEN);
    paint(144, 192, 24, 72, RED);
    paint(192, 224, 24, 72, WHITE);
    // R3: Q2 (W 100h above, equal) covers Q1; Q3 (W 300h above, equal) leaves Q4 yellow.
    paint(16, 80, 96, 144, RED);
    paint(96, 160, 96, 144, YELLOW);
    // R4: the quad from column 176 to 240 has its depth cross the far plane half way, at column
    // 208; the part inside draws (the clipped right edge is vertical and drops). The twin with bit
    // 12 clear is hidden.
    paint(176, 208, 96, 144, MAGENTA);
    grid
}

fn top_after(frames: u64) -> Vec<u32> {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxdepth")).expect("gxdepth ROM is built"))
        .expect("gxdepth ROM has a valid header");
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
fn given_the_gxdepth_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(differences(&top_after(FRAMES), &expected()), vec![]);
}

#[test]
fn given_the_gxdepth_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxdepth", FRAMES);
}

#[test]
fn given_the_gxdepth_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxdepth", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxdepth", 2, 2);
}
