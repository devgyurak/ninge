// SPDX-License-Identifier: Apache-2.0
//! The 4x4 compressed texture format, observed through our own `gxtex4x4` test ROM
//! (`testroms/gxtex4x4/`): the four block modes, a palette base, the block order of a wider texture,
//! texture data in slot 2 and in slot 1, and the format ignoring TEXIMAGE_PARAM bit 29. The expected
//! picture is worked out here from `docs/hw/textures.md` ("4x4 compressed texels"), pixel by pixel,
//! from the blocks, palette halfwords and palettes the ROM's source lists. It is never read back
//! from the emulator: nothing below shares code with `ninge-core`.
//!
//! A region is 64x48 pixels. An 8x8 texture covers 8x6 pixels a texel; the 16x8 one of R3 covers 4x6.
//! Every block row is `0xE4`, so the texels of a row are 0, 1, 2, 3 left to right.

use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::picture::{
    BLUE, differences, paint_region as paint, palette_colour as colour, top_after,
};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

/// A 15-bit colour from 5-bit channels.
fn rgb(r: u16, g: u16, b: u16) -> u16 {
    r | g << 5 | b << 10
}

/// `(a x wa + b x wb) / (wa + wb)` per 5-bit channel, truncating.
fn mix(a: u16, b: u16, wa: u16, wb: u16) -> u16 {
    let channel = |shift: u16| (((a >> shift) & 31) * wa + ((b >> shift) & 31) * wb) / (wa + wb);
    channel(0) | (channel(5) << 5) | (channel(10) << 10)
}

/// What texel `k` of a block of mode `mode` shows over the palette colours `c`; `None` is
/// transparent (the rear plane shows).
fn texel(mode: usize, k: usize, c: [u16; 4]) -> u16 {
    let shown = match (mode, k) {
        (0 | 1, 3) => None,
        (_, 0) => Some(c[0]),
        (_, 1) => Some(c[1]),
        (1, 2) => Some(mix(c[0], c[1], 1, 1)),
        (3, 2) => Some(mix(c[0], c[1], 5, 3)),
        (3, _) => Some(mix(c[0], c[1], 3, 5)),
        (_, 2) => Some(c[2]),
        _ => Some(c[3]),
    };
    shown.unwrap_or(BLUE)
}

/// The upper screen the ROM shows, as a grid of 15-bit colours.
fn expected() -> Vec<u16> {
    let mut grid = vec![BLUE; SCREEN_WIDTH * SCREEN_HEIGHT];
    // The palette at byte 0 holds P1; the one at byte 200h holds colour(90 + k).
    let p1 = [
        rgb(10, 20, 30),
        rgb(21, 5, 3),
        rgb(31, 31, 31),
        rgb(1, 2, 3),
    ];
    let p2 = [90, 91, 92, 93].map(colour);
    // The blocks of the 8x8 textures (two to a row) have modes 0, 1, 2, 3 in R1, R2 and R6.
    let eight = |x: usize, y: usize| ((y / 6) / 4 * 2 + (x / 8) / 4, (x / 8) % 4);
    paint(&mut grid, 0, 0, |x, y| {
        let (block, k) = eight(x, y);
        texel(block, k, p1)
    });
    paint(&mut grid, 1, 0, |x, y| {
        let (block, k) = eight(x, y);
        texel(block, k, p2)
    });
    // R3: 16x8, four blocks to a row, texel 0 of mode 2: block b shows palette entry 8b (its
    // palette offset is 16b bytes from PLTT_BASE 10h) of colour(k).
    paint(&mut grid, 2, 0, |x, y| {
        let block = (y / 6) / 4 * 4 + (x / 4) / 4;
        colour(8 * block as u16)
    });
    // R4: slot 2, mode 2, block j has the colours colour(120 + 8j + k).
    paint(&mut grid, 0, 1, |x, y| {
        let (j, k) = eight(x, y);
        colour(120 + 8 * j as u16 + k as u16)
    });
    // R5: slot 1, the halfword reads 0: mode 0 over P1.
    paint(&mut grid, 1, 1, |x, y| texel(0, eight(x, y).1, p1));
    // R6: bit 29 changes nothing for this format.
    paint(&mut grid, 2, 1, |x, y| {
        let (block, k) = eight(x, y);
        texel(block, k, p1)
    });
    grid
}

#[test]
fn given_the_gxtex4x4_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(
        differences(&top_after("gxtex4x4", FRAMES), &expected()),
        vec![]
    );
}

#[test]
fn given_the_gxtex4x4_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxtex4x4", FRAMES);
}

#[test]
fn given_the_gxtex4x4_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxtex4x4", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxtex4x4", 2, 2);
}
