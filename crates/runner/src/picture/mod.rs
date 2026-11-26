// SPDX-License-Identifier: Apache-2.0
//! Helpers for the runner tests that compare a test ROM's upper screen with a picture worked out by
//! hand (`gxtexpal`, `gxtexwrap`, `gxtexalpha`, `gxtex4x4`, `gxrearbmp`): run a ROM, expand the
//! expected 15-bit colours the way the display does, list the first pixels that differ, and paint
//! the 64x48 regions the texture ROMs share.
//!
//! The expected picture is built by the test from the note, never read back from the emulator;
//! these helpers only run, convert and compare.

use crate::{load_rom_file, testrom_path};
use ninge_core::{Machine, SCREEN_WIDTH, Screen};

/// The rear plane's colour in the texture ROMs (opaque blue), which transparent texels let through.
pub const BLUE: u16 = 0x7C00;
/// The left edges of the three columns of the texture ROMs' 64x48 regions.
pub const REGION_COLS: [usize; 3] = [16, 96, 176];
/// The top edges of their two rows.
pub const REGION_ROWS: [usize; 2] = [24, 96];

/// A 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
pub fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The palette colour the texture ROMs give entry `k`: a distinct colour for each `k` below 32.
pub fn palette_colour(k: u16) -> u16 {
    (k & 31) | (((k * 3) & 31) << 5) | (((k * 7) & 31) << 10)
}

/// Paints region (`col`, `row`) of `grid` (the 256x192 upper screen as 15-bit colours) with
/// `f(x, y)`, `x` and `y` being the pixel offsets inside the 64x48 region.
pub fn paint_region(grid: &mut [u16], col: usize, row: usize, f: impl Fn(usize, usize) -> u16) {
    (0..48).for_each(|y| {
        (0..64).for_each(|x| {
            grid[(REGION_ROWS[row] + y) * SCREEN_WIDTH + REGION_COLS[col] + x] = f(x, y);
        });
    });
}

/// The upper screen of test ROM `rom` after `frames` frames, as xRGB8888.
///
/// # Panics
/// When the ROM is not built or its header is refused: the test cannot run.
pub fn top_after(rom: &str, frames: u64) -> Vec<u32> {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path(rom)).expect("the ROM is built"))
        .expect("the ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine.frame(Screen::Top).to_vec()
}

/// The first few pixels where `got` differs from the expected 15-bit picture `wanted`:
/// `(x, y, got, wanted)` as xRGB8888.
pub fn differences(got: &[u32], wanted: &[u16]) -> Vec<(usize, usize, u32, u32)> {
    got.iter()
        .zip(wanted)
        .enumerate()
        .filter(|(_, (g, w))| **g != expand(**w))
        .map(|(at, (g, w))| (at % SCREEN_WIDTH, at / SCREEN_WIDTH, *g, expand(*w)))
        .take(8)
        .collect()
}

#[cfg(test)]
mod tests;
