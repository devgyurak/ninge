// SPDX-License-Identifier: Apache-2.0
//! The rasterizer's translucency rules, observed through our own `gxalpha` test ROM
//! (`testroms/gxalpha/`): blending over opaque polygons and the rear plane, the alpha test, the
//! same-ID rule and the depth update bit. The expected picture is worked out here from
//! `docs/hw/rasterizer.md` ("Translucency, draw order and the rear plane"), pixel by pixel, from
//! the coordinates the ROM's source lists. It is never read back from the emulator: nothing below
//! shares code with `ninge-core`.
//!
//! The blend is `(poly x (A + 1) + buffer x (31 - A)) / 32` per 6-bit channel, narrowed to 5 bits.
//! Opaque polygons draw first, then the translucent ones in the order the note gives (the regions
//! do not overlap each other, so the sort key does not show here).

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

const BLUE: u16 = 0x7C00;
const RED: u16 = 0x001F;
const GREEN: u16 = 0x03E0;

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
    // R1: the opaque red quad first (it drops its lower and right edges), then the translucent
    // green quad over it and over the rear plane; with blending on a translucent polygon keeps its
    // lower edge (line 72) but a vertical right edge never draws.
    paint(16, 80, 24, 72, RED, 31);
    paint(48, 112, 24, 73, GREEN, 15);
    // R2: alpha 8 is not above the reference 8 and draws nothing; alpha 9 does.
    paint(176, 208, 24, 73, GREEN, 9);
    // R3: T1 green, then T2 red only where T1 did not draw (the same polygon ID).
    paint(16, 64, 96, 145, GREEN, 15);
    paint(64, 88, 96, 145, RED, 15);
    // R4: T3 green updates the depth; T4 red is farther and draws only past T3.
    paint(120, 168, 96, 145, GREEN, 15);
    paint(168, 192, 96, 145, RED, 15);
    grid
}

fn top_after(frames: u64) -> Vec<u32> {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxalpha")).expect("gxalpha ROM is built"))
        .expect("gxalpha ROM has a valid header");
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
fn given_the_gxalpha_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(differences(&top_after(FRAMES), &expected()), vec![]);
}

#[test]
fn given_the_gxalpha_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxalpha", FRAMES);
}

#[test]
fn given_the_gxalpha_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxalpha", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxalpha", 2, 2);
}
