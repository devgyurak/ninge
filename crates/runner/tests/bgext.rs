// SPDX-License-Identifier: Apache-2.0
//! The extended layer kinds, observed through our own `bgext` test ROM (`testroms/bgext/`). The ROM
//! builds a 256-colour bitmap, a direct-colour bitmap, tile data, a map of 16-bit entries and a
//! palette (layout at the top of `arm9.s`). Engine A (BG mode 5) shows the direct-colour bitmap
//! (BG2, 128x128, no wrap) in front of the 256-colour one (BG3, 256x256, rotated, wrapping) on the
//! upper screen; engine B (BG mode 3) shows a tile layer with 16-bit entries (BG3, rotated and
//! scaled, wrapping) on the lower one.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/bg-modes.md` ("Extended
//! layers", "Affine layers", "Sizes and where the data is") and the data the ROM's source lists. It
//! is never read back from the emulator: the code below shares nothing with `ninge-core`.

use ninge_core::debug::{AffineFormat, EngineId};
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected; the ROM finishes its setup inside the first three.
const FRAMES: u64 = 4;

/// A machine that booted the `bgext` ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("bgext")).expect("bgext ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("bgext ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

/// The palette entry `index` as a 15-bit color (arm9.s, "Palette").
fn palette(index: usize) -> u16 {
    let red = index & 31;
    let green = (3 * index) & 31;
    let blue = (5 * index) & 31;
    (red | green << 5 | blue << 10) as u16
}

/// The 15-bit color as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// Engine A's backdrop (palette entry 0 = 0x03E0) and engine B's (0x7C00).
const BACKDROP_A: u32 = 0x0000_FF00;
const BACKDROP_B: u32 = 0x0000_00FF;

/// What a texel is.
#[derive(Clone, Copy)]
enum Texels {
    /// 1 + ((5 tx + 3 ty) mod 255), through the palette.
    Bitmap256,
    /// (37 tx + 91 ty) & 0x7FFF, shown when bit 15 is set, which it is unless (tx + ty) mod 3 = 0.
    Direct,
    /// 16-bit entries: tile 1 + ((cx + 2 cy) mod 4), flipped horizontally in odd columns and
    /// vertically in odd rows; pixel (r, c) of tile t holds 16 t + ((3 r + c) mod 16).
    Tiles16,
}

/// One extended layer as the ROM sets it up.
struct Layer {
    texels: Texels,
    width: i64,
    height: i64,
    wrap: bool,
    pa: i64,
    pb: i64,
    pc: i64,
    pd: i64,
    /// X and Y at the top of the frame, 20.8.
    x: i64,
    y: i64,
}

fn a_bg2() -> Layer {
    Layer {
        texels: Texels::Direct,
        width: 128,
        height: 128,
        wrap: false,
        pa: 0x100,
        pb: 0,
        pc: 0,
        pd: 0x100,
        x: -32 * 256,
        y: -16 * 256,
    }
}

fn a_bg3() -> Layer {
    Layer {
        texels: Texels::Bitmap256,
        width: 256,
        height: 256,
        wrap: true,
        pa: 0xF0,
        pb: 0x40,
        pc: -0x40,
        pd: 0xF0,
        x: 0x2000,
        y: 0,
    }
}

fn b_bg3() -> Layer {
    Layer {
        texels: Texels::Tiles16,
        width: 256,
        height: 256,
        wrap: true,
        pa: 0xC0,
        pb: -0x30,
        pc: 0x30,
        pd: 0xC0,
        x: 0x1000,
        y: 0x800,
    }
}

// ------------------------------------------------------------------ the note's rules

/// The 15-bit color of texel (`tx`, `ty`), or `None` where it is transparent.
fn texel(texels: Texels, tx: i64, ty: i64) -> Option<u16> {
    match texels {
        Texels::Bitmap256 => Some(palette((1 + (5 * tx + 3 * ty) % 255) as usize)),
        Texels::Direct => {
            let color = ((37 * tx + 91 * ty) & 0x7FFF) as u16;
            ((tx + ty) % 3 != 0).then_some(color)
        }
        Texels::Tiles16 => {
            let (cx, cy) = (tx / 8, ty / 8);
            let tile = 1 + ((cx + 2 * cy) & 3);
            let column = if cx & 1 == 1 { 7 - tx % 8 } else { tx % 8 };
            let row = if cy & 1 == 1 { 7 - ty % 8 } else { ty % 8 };
            Some(palette((16 * tile + ((3 * row + column) & 15)) as usize))
        }
    }
}

/// The color of the layer at screen pixel (`x`, `y`), or `None` where it is transparent.
fn layer_color(layer: &Layer, x: usize, y: usize) -> Option<u16> {
    // The line starts where `y` steps of PB and PD have walked from the top.
    let origin_x = layer.x + layer.pb * y as i64;
    let origin_y = layer.y + layer.pd * y as i64;
    let u = origin_x + layer.pa * x as i64;
    let v = origin_y + layer.pc * x as i64;
    let (tx, ty) = (u.div_euclid(256), v.div_euclid(256));
    let inside = (0..layer.width).contains(&tx) && (0..layer.height).contains(&ty);
    match (inside, layer.wrap) {
        (false, false) => None,
        _ => texel(
            layer.texels,
            tx.rem_euclid(layer.width),
            ty.rem_euclid(layer.height),
        ),
    }
}

/// Engine A: BG2 (priority 0) over BG3 (priority 1), else the backdrop.
fn expected_a(x: usize, y: usize) -> u32 {
    [a_bg2(), a_bg3()]
        .iter()
        .find_map(|layer| layer_color(layer, x, y))
        .map_or(BACKDROP_A, expand)
}

/// Engine B: BG3, else the backdrop.
fn expected_b(x: usize, y: usize) -> u32 {
    layer_color(&b_bg3(), x, y).map_or(BACKDROP_B, expand)
}

/// The pixels of `screen` that differ from `expected`: (x, y, expected, got), the first few.
fn mismatches(
    machine: &Machine,
    screen: Screen,
    expected: fn(usize, usize) -> u32,
) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(screen);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| (x, y, expected(x, y), frame[y * SCREEN_WIDTH + x]))
        .filter(|&(_, _, want, got)| want != got)
        .take(8)
        .collect()
}

/// How many different colors `screen` shows.
fn colors(machine: &Machine, screen: Screen) -> usize {
    let mut seen = machine.frame(screen).to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_bgext_rom_when_4_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    assert_eq!(mismatches(&machine, Screen::Top, expected_a), vec![]);
}

#[test]
fn given_bgext_rom_when_4_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    assert_eq!(mismatches(&machine, Screen::Bottom, expected_b), vec![]);
}

#[test]
fn given_bgext_rom_when_4_frames_run_then_both_screens_show_many_colors() {
    // Guards the two tests above against a scene that went blank.
    let machine = boot_frames();
    assert_eq!(
        (
            colors(&machine, Screen::Top) > 100,
            colors(&machine, Screen::Bottom) > 30
        ),
        (true, true)
    );
}

#[test]
fn given_bgext_rom_when_4_frames_run_then_the_direct_colour_layer_lets_the_bitmap_behind_it_show_through()
 {
    // Inside BG2's square (x 32..160, y 16..144) the pixels with (tx + ty) mod 3 = 0 are
    // transparent, so BG3's color shows there; the others show BG2's own.
    let machine = boot_frames();
    let frame = machine.frame(Screen::Top);
    let at = |x: usize, y: usize| frame[y * SCREEN_WIDTH + x];
    // Pixel (32, 16) is texel (0, 0): transparent. Pixel (33, 16) is texel (1, 0): its color.
    assert_eq!(
        (
            at(32, 16) == expand(layer_color(&a_bg3(), 32, 16).unwrap_or(0)),
            at(33, 16) == expand(((37) & 0x7FFF) as u16)
        ),
        (true, true)
    );
}

/// A layer's decoded registers: format, priority, width, height, wrap, PA-PD, X, Y.
type LayerState = (AffineFormat, u8, u16, u16, bool, [i16; 4], [i32; 2]);

fn layer_state(machine: &Machine, engine: EngineId, layer: usize) -> LayerState {
    let l = machine.inspect().gpu2d().affine_bg(engine, layer);
    (
        l.format,
        l.priority,
        l.width,
        l.height,
        l.wrap,
        l.params,
        l.reference,
    )
}

#[test]
fn given_bgext_rom_when_4_frames_run_then_the_rom_wrote_the_registers_it_set() {
    let machine = boot_frames();
    assert_eq!(
        (
            machine.inspect().gpu2d().dispcnt_a(),
            machine.inspect().gpu2d().dispcnt_b(),
            machine.inspect().gpu2d().powcnt1(),
            layer_state(&machine, EngineId::A, 2),
            layer_state(&machine, EngineId::A, 3),
            layer_state(&machine, EngineId::B, 3),
        ),
        (
            0x0001_0C05,
            0x0001_0803,
            0x8203,
            (
                AffineFormat::BitmapDirect,
                0,
                128,
                128,
                false,
                [0x100, 0, 0, 0x100],
                [-32 * 256, -16 * 256]
            ),
            (
                AffineFormat::Bitmap256,
                1,
                256,
                256,
                true,
                [0xF0, 0x40, -0x40, 0xF0],
                [0x2000, 0]
            ),
            (
                AffineFormat::Tiles16,
                0,
                256,
                256,
                true,
                [0xC0, -0x30, 0x30, 0xC0],
                [0x1000, 0x800]
            ),
        )
    );
}

#[test]
fn given_bgext_rom_when_run_for_4_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that this frame is the right one: the hash pins it.
    assert_frame_hash("bgext", 4);
}

#[test]
fn given_bgext_rom_when_run_twice_for_4_frames_then_every_frame_hash_matches() {
    assert_deterministic("bgext", 4);
}

#[test]
fn given_bgext_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("bgext", 2, 2);
}
