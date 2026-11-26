// SPDX-License-Identifier: Apache-2.0
//! The palette texture formats, observed through our own `gxtexpal` test ROM
//! (`testroms/gxtexpal/`): the 256-, 16- and 4-colour texel layouts, the palette base units, colour 0
//! made transparent, and modulation by a grey vertex colour. The expected picture is worked out here
//! from `docs/hw/textures.md`, pixel by pixel, from the textures and palettes the ROM's source
//! lists. It is never read back from the emulator: nothing below shares code with `ninge-core`.
//!
//! Each region is a 64x48 quad showing an 8x8 texture, so texel `(x, y)` covers the pixels
//! `8x..8x+8` by `6y..6y+6` of the region (the quad drops its lower and right edges, which leaves
//! exactly those 64x48 pixels).

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

/// The rear plane's colour, which transparent texels let through.
const BLUE: u16 = 0x7C00;
/// The regions are 64 pixels wide and 48 lines tall; their corners are these columns and rows.
const COLS: [usize; 3] = [16, 96, 176];
const ROWS: [usize; 2] = [24, 96];

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The ROM's palette colour for entry `k`: a distinct colour for each entry.
fn colour(k: u16) -> u16 {
    (k & 31) | (((k * 3) & 31) << 5) | (((k * 7) & 31) << 10)
}

/// Paints region (`col`, `row`) of `grid` with `f(x, y)`, `x` and `y` being the pixel offsets
/// inside the 64x48 region.
fn paint(grid: &mut [u16], col: usize, row: usize, f: impl Fn(usize, usize) -> u16) {
    (0..48).for_each(|y| {
        (0..64).for_each(|x| grid[(ROWS[row] + y) * SCREEN_WIDTH + COLS[col] + x] = f(x, y));
    });
}

fn top_after(rom: &str, frames: u64) -> Vec<u32> {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path(rom)).expect("the ROM is built"))
        .expect("the ROM has a valid header");
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

/// A channel of a 5-bit colour as 6 bits: `X x 2 + 1`, but 0 stays 0.
fn six(c: u16) -> u16 {
    if c == 0 { 0 } else { c * 2 + 1 }
}

/// `texel` modulated by the grey vertex colour whose channels are all `vertex` (5 bits): per channel
/// `((t + 1) x (v + 1) - 1) / 64` in 6 bits, narrowed to 5.
fn modulate(texel: u16, vertex: u16) -> u16 {
    let channel = |shift: u16| {
        let (t, v) = (six((texel >> shift) & 31), six(vertex));
        (((t + 1) * (v + 1) - 1) / 64) >> 1
    };
    channel(0) | (channel(5) << 5) | (channel(10) << 10)
}

/// The upper screen the ROM shows, as a grid of 15-bit colours.
fn expected() -> Vec<u16> {
    let mut grid = vec![BLUE; SCREEN_WIDTH * SCREEN_HEIGHT];
    // The textures' indices, as the ROM builds them.
    let i256 = |x: usize, y: usize| ((3 * x + 5 * y) % 7) as u16;
    let i16 = |x: usize, y: usize| ((x + 2 * y) % 16) as u16;
    let i4 = |x: usize, y: usize| ((x + y) % 4) as u16;
    // The palettes: entry k of the 256-colour one is colour(10 + k), of the 16-colour one
    // colour(40 + k), of the 4-colour one colour(80 + k).
    let c256 = |x: usize, y: usize| colour(10 + i256(x / 8, y / 6));
    let c16 = |x: usize, y: usize| colour(40 + i16(x / 8, y / 6));
    let c4 = |x: usize, y: usize| colour(80 + i4(x / 8, y / 6));
    paint(&mut grid, 0, 0, c256);
    paint(&mut grid, 1, 0, c16);
    paint(&mut grid, 2, 0, c4);
    // R4 and R6: colour 0 is transparent, so index 0 shows the rear plane.
    paint(&mut grid, 0, 1, |x, y| {
        [c256(x, y), BLUE][usize::from(i256(x / 8, y / 6) == 0)]
    });
    // R5: the grey vertex colour 4210h has channels of 16.
    paint(&mut grid, 1, 1, |x, y| modulate(c16(x, y), 16));
    paint(&mut grid, 2, 1, |x, y| {
        [c4(x, y), BLUE][usize::from(i4(x / 8, y / 6) == 0)]
    });
    grid
}

#[test]
fn given_the_gxtexpal_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(
        differences(&top_after("gxtexpal", FRAMES), &expected()),
        vec![]
    );
}

#[test]
fn given_the_gxtexpal_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxtexpal", FRAMES);
}

#[test]
fn given_the_gxtexpal_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxtexpal", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxtexpal", 2, 2);
}
