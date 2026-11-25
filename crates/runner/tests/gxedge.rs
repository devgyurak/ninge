// SPDX-License-Identifier: Apache-2.0
//! The rasterizer's fill, observed through our own `gxedge` test ROM (`testroms/gxedge/`): seven
//! polygons drawn over an opaque blue rear plane and shown by engine A as its 3D layer. The
//! expected picture is worked out here from `docs/hw/rasterizer.md`, pixel by pixel, from the
//! coordinates the ROM's source lists. It is never read back from the emulator: nothing below
//! shares code with `ninge-core`.
//!
//! With the identity matrices a vertex's clip coordinates are its screen position, and the ROM's
//! numbers are chosen so that every vertex lands on a whole pixel. The rules the picture applies:
//! an opaque polygon keeps its upper and left edges and drops its lower and right ones; a polygon
//! has at least one pixel of width; a colour is interpolated across the span in 6 bits and narrowed
//! to 5; the nearer polygon wins and an equal depth keeps the first; a polygon of one point is a dot;
//! a wire frame (alpha 0) draws its outline only.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

const BLUE: u16 = 0x7C00;
const RED: u16 = 0x001F;
const GREEN: u16 = 0x03E0;
const WHITE: u16 = 0x7FFF;

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The upper screen the ROM shows, as a grid of 15-bit colours, built polygon by polygon in the
/// order the ROM sends them.
fn expected() -> Vec<u16> {
    let mut grid = vec![BLUE; SCREEN_WIDTH * SCREEN_HEIGHT];
    // `depth` rows alongside, so a later polygon can lose to an earlier, nearer one.
    let mut depth = vec![i32::MAX; SCREEN_WIDTH * SCREEN_HEIGHT];
    let mut put = |x: usize, y: usize, z: i32, color: u16| {
        let at = y * SCREEN_WIDTH + x;
        if z < depth[at] {
            depth[at] = z;
            grid[at] = color;
        }
    };
    // A: red quad, columns 16-47, lines 24-47.
    (24..48).for_each(|y| (16..48).for_each(|x| put(x, y, 0, RED)));
    // B: the triangle (100,24) (100,72) (148,72). Its right edge runs one column right per line.
    // The apex line has one pixel (a width of at least one); line 24 + k has k columns after it.
    // Line 72 is the lower edge and drops.
    (24..72).for_each(|y| {
        let last = if y == 24 { 100 } else { 99 + (y - 24) };
        (100..=last).for_each(|x| put(x, y, 0, GREEN));
    });
    // C: the gradient. The red channel runs 0 to 63 (6 bits) over 64 columns, one step each, then is
    // narrowed to 5 bits.
    (24..48).for_each(|y| {
        (160..224).for_each(|x| put(x, y, 0, ((63 * (x as u16 - 160) / 64) >> 1) & 31));
    });
    // D1 (near, green) is drawn first and D2 (far, red) second; D2 loses where they overlap.
    (120..168).for_each(|y| (20..84).for_each(|x| put(x, y, -1, GREEN)));
    (135..183).for_each(|y| (52..116).for_each(|x| put(x, y, 1, RED)));
    // E: one dot.
    put(240, 171, 0, WHITE);
    // F: the outline of (150,93)-(230,141), edges included.
    (93..=141).for_each(|y| {
        (150..=230)
            .filter(|&x| x == 150 || x == 230 || y == 93 || y == 141)
            .for_each(|x| put(x, y, 0, GREEN));
    });
    grid
}

fn top_after(frames: u64) -> Vec<u32> {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxedge")).expect("gxedge ROM is built"))
        .expect("gxedge ROM has a valid header");
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
fn given_the_gxedge_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(differences(&top_after(FRAMES), &expected()), vec![]);
}

#[test]
fn given_the_gxedge_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxedge", FRAMES);
}

#[test]
fn given_the_gxedge_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxedge", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxedge", 2, 2);
}
