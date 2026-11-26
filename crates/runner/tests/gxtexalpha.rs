// SPDX-License-Identifier: Apache-2.0
//! The alpha texture formats and decal shading, observed through our own `gxtexalpha` test ROM
//! (`testroms/gxtexalpha/`): A3I5, A5I3 and direct colour textures blended with the opaque blue rear
//! plane, a translucent polygon alpha times a texel alpha, and decal mode. The expected picture is
//! worked out here from `docs/hw/textures.md` ("Texture alpha, translucency and the buffers",
//! "Blending the texel with the vertex colour"), pixel by pixel, from the textures and palettes the
//! ROM's source lists. It is never read back from the emulator: nothing below shares code with
//! `ninge-core`.
//!
//! Each region is a 64x48 quad showing an 8x8 texture, so texel `(x, y)` covers the pixels
//! `8x..8x+8` by `6y..6y+6` of the region. The 3D blend is
//! `(new x (A + 1) + old x (31 - A)) / 32` per 6-bit channel over the rear plane, narrowed to 5 bits;
//! alpha 0 draws nothing.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

/// The rear plane's colour, which transparent texels let through.
const BLUE: u16 = 0x7C00;
const RED: u16 = 0x001F;
const GREEN: u16 = 0x03E0;
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

/// The 6-bit channels of a 15-bit colour.
fn channels(color: u16) -> [u16; 3] {
    [0, 5, 10].map(|shift| six((color >> shift) & 31))
}

/// 6-bit channels narrowed to a 15-bit colour.
fn narrow(c: [u16; 3]) -> u16 {
    (c[0] >> 1) | ((c[1] >> 1) << 5) | ((c[2] >> 1) << 10)
}

/// A pixel of 15-bit colour `color` and alpha `alpha` drawn over the rear plane (opaque blue):
/// nothing at alpha 0, the blend otherwise.
fn over_blue(color: u16, alpha: u16) -> u16 {
    if alpha == 0 {
        return BLUE;
    }
    let (new, old) = (channels(color), channels(BLUE));
    narrow([0, 1, 2].map(|k| (new[k] * (alpha + 1) + old[k] * (31 - alpha)) / 32))
}

/// A3I5's 3-bit alpha as 5 bits.
fn alpha3(a: u16) -> u16 {
    a * 4 + a / 2
}

/// The alpha after multiplying the texel alpha `texel` (5 bits) by the polygon alpha `poly`, per
/// `((t + 1) x (p + 1) - 1) / 64` in 6 bits, narrowed to 5.
fn multiplied(texel: u16, poly: u16) -> u16 {
    (((six(texel) + 1) * (six(poly) + 1) - 1) / 64) >> 1
}

/// Decal: the texel colour `texel` mixed with the vertex colour `vertex` by the texel alpha
/// `alpha` (5 bits): the vertex colour alone at 0, the texel alone at 31, else
/// `(t x a + v x (63 - a)) / 64` per 6-bit channel with `a = 2 alpha + 1`.
fn decalled(texel: u16, alpha: u16, vertex: u16) -> u16 {
    match alpha {
        0 => vertex,
        31 => texel,
        _ => {
            let (t, v, a) = (channels(texel), channels(vertex), six(alpha));
            narrow([0, 1, 2].map(|k| (t[k] * a + v[k] * (63 - a)) / 64))
        }
    }
}

/// Paints the line below region (`col`, `row`) with `f(x, 47)`, the texel row 7: a polygon that draws
/// with the translucent ones keeps its lower edge when blending is on (`rasterizer.md`, "Which pixels
/// a polygon covers"), and T is exactly 8 there, which clamps to the last texel row.
fn lower_edge(grid: &mut [u16], col: usize, row: usize, f: impl Fn(usize, usize) -> u16) {
    (0..64).for_each(|x| grid[(ROWS[row] + 48) * SCREEN_WIDTH + COLS[col] + x] = f(x, 47));
}

/// The upper screen the ROM shows, as a grid of 15-bit colours.
fn expected() -> Vec<u16> {
    let mut grid = vec![BLUE; SCREEN_WIDTH * SCREEN_HEIGHT];
    let (tx, ty) = (|x: usize| (x / 8) as u16, |y: usize| (y / 6) as u16);
    // R1: A3I5, alpha y, index (4x + y) mod 32, palette colour(20 + index).
    let r1 = |x: usize, y: usize| over_blue(colour(20 + (4 * tx(x) + ty(y)) % 32), alpha3(ty(y)));
    paint(&mut grid, 0, 0, r1);
    lower_edge(&mut grid, 0, 0, r1);
    // R2: A5I3, alpha 4y, index x, palette base 4 (byte 40h) holding colour(60 + index).
    let r2 = |x: usize, y: usize| over_blue(colour(60 + tx(x)), 4 * ty(y));
    paint(&mut grid, 1, 0, r2);
    lower_edge(&mut grid, 1, 0, r2);
    // R3: direct, colour(x + 8y + 3) with the alpha bit clear where (x + y) mod 3 is 0; a set bit
    // is alpha 31 and a polygon alpha of 31 keeps it.
    let direct = |x: usize, y: usize| (tx(x) + ty(y)) % 3 != 0;
    paint(&mut grid, 2, 0, |x, y| {
        [BLUE, colour(tx(x) + 8 * ty(y) + 3)][usize::from(direct(x, y))]
    });
    // R4: A5I3 decal over a red vertex colour, texel alpha 4y; the polygon stays opaque.
    paint(&mut grid, 0, 1, |x, y| {
        decalled(colour(60 + tx(x)), 4 * ty(y), RED)
    });
    // R5: A3I5 with polygon alpha 15: the two alphas multiply.
    let r5 = |x: usize, y: usize| {
        over_blue(
            colour(20 + (4 * tx(x) + ty(y)) % 32),
            multiplied(alpha3(ty(y)), 15),
        )
    };
    paint(&mut grid, 1, 1, r5);
    lower_edge(&mut grid, 1, 1, r5);
    // R6: direct decal over a green vertex colour: a clear alpha bit shows the vertex colour.
    paint(&mut grid, 2, 1, |x, y| {
        decalled(
            colour(tx(x) + 8 * ty(y) + 3),
            [0, 31][usize::from(direct(x, y))],
            GREEN,
        )
    });
    grid
}

#[test]
fn given_the_gxtexalpha_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(
        differences(&top_after("gxtexalpha", FRAMES), &expected()),
        vec![]
    );
}

#[test]
fn given_the_gxtexalpha_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxtexalpha", FRAMES);
}

#[test]
fn given_the_gxtexalpha_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxtexalpha", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxtexalpha", 2, 2);
}
