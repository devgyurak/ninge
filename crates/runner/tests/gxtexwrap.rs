// SPDX-License-Identifier: Apache-2.0
//! Texture repeat, flip and clamp, observed through our own `gxtexwrap` test ROM
//! (`testroms/gxtexwrap/`): one 8x8 256-colour texture drawn with S running -4 to 12 texels and T
//! -3 to 9 under six flag combinations. The expected picture is worked out here from
//! `docs/hw/textures.md` ("From S and T to a texel") and never read back from the emulator: nothing
//! below shares code with `ninge-core`.
//!
//! A region is 64x48 pixels, so S moves a quarter texel and T a quarter texel per pixel: pixel
//! `(k, j)` of a region samples texel `(-4 + k / 4, -3 + j / 4)` before the wrap.

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

/// One axis of a region: how a texel coordinate outside `0..8` is brought back.
#[derive(Clone, Copy)]
enum Wrap {
    Clamp,
    Repeat,
    Mirror,
}

/// The texel `n` on an axis of 8 texels under `wrap`: a mirrored axis runs backwards in every
/// odd-numbered tile (tile `n div 8`, counting down below 0).
fn texel(n: i32, wrap: Wrap) -> i32 {
    match wrap {
        Wrap::Clamp => n.clamp(0, 7),
        Wrap::Repeat => n.rem_euclid(8),
        Wrap::Mirror => {
            [n.rem_euclid(8), 7 - n.rem_euclid(8)][n.div_euclid(8).rem_euclid(2) as usize]
        }
    }
}

/// Region (`col`, `row`) painted with the texture's colours: texel `(x, y)` has index `8y + x` and
/// the palette's entry `k` is `colour(5 + k)`.
fn region(grid: &mut [u16], col: usize, row: usize, s: Wrap, t: Wrap) {
    paint(grid, col, row, |k, j| {
        let (x, y) = (texel(-4 + (k / 4) as i32, s), texel(-3 + (j / 4) as i32, t));
        colour(5 + (8 * y + x) as u16)
    });
}

/// The upper screen the ROM shows, as a grid of 15-bit colours.
fn expected() -> Vec<u16> {
    let mut grid = vec![BLUE; SCREEN_WIDTH * SCREEN_HEIGHT];
    region(&mut grid, 0, 0, Wrap::Clamp, Wrap::Clamp);
    region(&mut grid, 1, 0, Wrap::Repeat, Wrap::Repeat);
    region(&mut grid, 2, 0, Wrap::Mirror, Wrap::Mirror);
    region(&mut grid, 0, 1, Wrap::Mirror, Wrap::Clamp);
    // R5: the flip bits without repeat change nothing, both axes clamp.
    region(&mut grid, 1, 1, Wrap::Clamp, Wrap::Clamp);
    region(&mut grid, 2, 1, Wrap::Clamp, Wrap::Repeat);
    grid
}

#[test]
fn given_the_gxtexwrap_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(
        differences(&top_after("gxtexwrap", FRAMES), &expected()),
        vec![]
    );
}

#[test]
fn given_the_gxtexwrap_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxtexwrap", FRAMES);
}

#[test]
fn given_the_gxtexwrap_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxtexwrap", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxtexwrap", 2, 2);
}
