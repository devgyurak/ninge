// SPDX-License-Identifier: Apache-2.0
//! Texture repeat, flip and clamp, observed through our own `gxtexwrap` test ROM
//! (`testroms/gxtexwrap/`): one 8x8 256-colour texture drawn with S running -4 to 12 texels and T
//! -3 to 9 under six flag combinations. The expected picture is worked out here from
//! `docs/hw/textures.md` ("From S and T to a texel") and never read back from the emulator: nothing
//! below shares code with `ninge-core`.
//!
//! A region is 64x48 pixels, so S moves a quarter texel and T a quarter texel per pixel: pixel
//! `(k, j)` of a region samples texel `(-4 + k / 4, -3 + j / 4)` before the wrap.

use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::picture::{
    BLUE, differences, paint_region as paint, palette_colour as colour, top_after,
};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

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
