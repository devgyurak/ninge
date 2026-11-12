// SPDX-License-Identifier: Apache-2.0
//! Affine layers on both engines, observed through our own `bgaffine` test ROM
//! (`testroms/bgaffine/`). The ROM builds tile data, three affine maps and a palette (layout at
//! the top of `arm9.s`); engine A shows BG2 (rotated, wrapping) behind BG3 (a 128x128 layer
//! that does not wrap and whose X the ROM changes after line 100) on the upper screen, engine B
//! shows an affine BG3 (512x512, rotated and scaled) behind a text layer on the lower one.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/bg-modes.md` ("Affine
//! layers", "Reference point and parameters", "When the picture is made") and `docs/hw/bg-text.md`
//! and the data the ROM's source lists. It is never read back from the emulator: the code below
//! shares nothing with `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected.
const FRAMES: u64 = 4;

/// A machine that booted the `bgaffine` ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("bgaffine")).expect("bgaffine ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("bgaffine ROM has a valid header");
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

/// The palette value of texel (`tx`, `ty`) of every affine layer (arm9.s, "Tile data", "Affine
/// maps"): the tile of cell (cx, cy) is `1 + ((cx + 2 cy) mod 4)`; its pixel (r, c) holds
/// `16 tile + ((3 r + c) mod 16)`.
fn texel_value(tx: i64, ty: i64) -> usize {
    let tile = 1 + ((tx / 8 + 2 * (ty / 8)) & 3);
    (16 * tile + ((3 * (ty % 8) + tx % 8) & 15)) as usize
}

/// One affine layer as the ROM sets it up.
struct Affine {
    size: i64,
    wrap: bool,
    pa: i64,
    pb: i64,
    pc: i64,
    pd: i64,
    /// X and Y at the top of the frame, 20.8.
    x: i64,
    y: i64,
    /// X from line 101 on, if the ROM changes it.
    x_after_100: Option<i64>,
}

fn a_bg2() -> Affine {
    Affine {
        size: 256,
        wrap: true,
        pa: 0xE0,
        pb: -0x80,
        pc: 0x80,
        pd: 0xE0,
        x: 0x4000,
        y: -0x2000,
        x_after_100: None,
    }
}

fn a_bg3() -> Affine {
    Affine {
        size: 128,
        wrap: false,
        pa: 0x100,
        pb: 0,
        pc: 0,
        pd: 0x100,
        x: -64 * 256,
        y: -32 * 256,
        x_after_100: Some(-96 * 256),
    }
}

fn b_bg3() -> Affine {
    Affine {
        size: 512,
        wrap: true,
        pa: 0xC0,
        pb: 0x60,
        pc: -0x60,
        pd: 0xC0,
        x: 0,
        y: 256 * 256,
        x_after_100: None,
    }
}

// ------------------------------------------------------------------ the note's rules

/// The palette index of layer pixel (`x`, `y`), or `None` where it is transparent.
fn affine_index(layer: &Affine, x: i64, y: i64) -> Option<usize> {
    // The line starts where `y` steps of PB and PD, from the top (or from the new X once the ROM
    // writes it: the write lands after line 100 started, so line 101 starts from it).
    let origin_x = match layer.x_after_100 {
        Some(new_x) if y > 100 => new_x + (y - 101) * layer.pb,
        _ => layer.x + y * layer.pb,
    };
    let origin_y = layer.y + y * layer.pd;
    let u = origin_x + layer.pa * x;
    let v = origin_y + layer.pc * x;
    let (tx, ty) = (u.div_euclid(256), v.div_euclid(256));
    let inside = (0..layer.size).contains(&tx) && (0..layer.size).contains(&ty);
    match (inside, layer.wrap) {
        (false, false) => None,
        _ => Some(texel_value(
            tx.rem_euclid(layer.size),
            ty.rem_euclid(layer.size),
        )),
    }
}

/// Engine A: BG3 (priority 0) over BG2 (priority 1), else the backdrop.
fn expected_a(x: usize, y: usize) -> u32 {
    let (x, y) = (x as i64, y as i64);
    [a_bg3(), a_bg2()]
        .iter()
        .find_map(|layer| affine_index(layer, x, y))
        .map_or(BACKDROP_A, |index| expand(palette(index)))
}

/// Engine B's BG0: text, 4 bits per pixel, tile 1 (all 9) at cells (2..4, 2), (2..4, 3) and, with
/// palette number 3, at (8, 2).
fn b_bg0_index(x: usize, y: usize) -> Option<usize> {
    match (x / 8, y / 8) {
        (2..=4, 2..=3) => Some(9),
        (8, 2) => Some(3 * 16 + 9),
        _ => None,
    }
}

/// Engine B: the text layer (priority 0) over BG3 (priority 1), else the backdrop.
fn expected_b(x: usize, y: usize) -> u32 {
    b_bg0_index(x, y)
        .or_else(|| affine_index(&b_bg3(), x as i64, y as i64))
        .map_or(BACKDROP_B, |index| expand(palette(index)))
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
fn given_bgaffine_rom_when_4_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    assert_eq!(mismatches(&machine, Screen::Top, expected_a), vec![]);
}

#[test]
fn given_bgaffine_rom_when_4_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    assert_eq!(mismatches(&machine, Screen::Bottom, expected_b), vec![]);
}

#[test]
fn given_bgaffine_rom_when_4_frames_run_then_both_screens_show_many_colors() {
    // Guards the two tests above against a scene that went blank.
    let machine = boot_frames();
    assert_eq!(
        (
            colors(&machine, Screen::Top) > 30,
            colors(&machine, Screen::Bottom) > 30
        ),
        (true, true)
    );
}

#[test]
fn given_bgaffine_rom_when_4_frames_run_then_the_x_written_in_mid_frame_moves_layer_a_bg3_after_line_100()
 {
    // BG3's area is x 64..192 above line 101 and x 96..224 from it (arm9.s, "Layers"): the pixel
    // (200, 120) is BG3's and (70, 120) is not; the first is BG3's last column group.
    let machine = boot_frames();
    let frame = machine.frame(Screen::Top);
    let at = |x: usize, y: usize| frame[y * SCREEN_WIDTH + x];
    let bg3_here = |x: usize, y: usize| affine_index(&a_bg3(), x as i64, y as i64).is_some();
    assert_eq!(
        (
            bg3_here(200, 120),
            bg3_here(70, 120),
            bg3_here(70, 40),
            bg3_here(200, 40),
            at(200, 120) == expand(palette(affine_index(&a_bg3(), 200, 120).unwrap_or(0)))
        ),
        (true, false, true, false, true)
    );
}

/// A layer's decoded registers: priority, size, wrap, PA-PD, X, Y.
type AffineState = (u8, u16, bool, [i16; 4], [i32; 2]);

fn affine_state(machine: &Machine, engine: EngineId, layer: usize) -> AffineState {
    let l = machine.inspect().gpu2d().affine_bg(engine, layer);
    (l.priority, l.size, l.wrap, l.params, l.reference)
}

#[test]
fn given_bgaffine_rom_when_4_frames_run_then_the_rom_wrote_the_registers_it_set() {
    // BG3 of engine A has X = -64.0 again: the ROM restores it once VCOUNT reaches 200.
    let machine = boot_frames();
    assert_eq!(
        (
            machine.inspect().gpu2d().dispcnt_a(),
            machine.inspect().gpu2d().dispcnt_b(),
            machine.inspect().gpu2d().powcnt1(),
            affine_state(&machine, EngineId::A, 2),
            affine_state(&machine, EngineId::A, 3),
            affine_state(&machine, EngineId::B, 3),
        ),
        (
            0x0001_0C02,
            0x0001_0901,
            0x8203,
            (1, 256, true, [0xE0, -0x80, 0x80, 0xE0], [0x4000, -0x2000]),
            (0, 128, false, [0x100, 0, 0, 0x100], [-64 * 256, -32 * 256]),
            (1, 512, true, [0xC0, 0x60, -0x60, 0xC0], [0, 0x1_0000]),
        )
    );
}

#[test]
fn given_bgaffine_rom_when_run_for_4_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that this frame is the right one: the hash pins it.
    assert_frame_hash("bgaffine", 4);
}

#[test]
fn given_bgaffine_rom_when_run_twice_for_4_frames_then_every_frame_hash_matches() {
    assert_deterministic("bgaffine", 4);
}

#[test]
fn given_bgaffine_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("bgaffine", 2, 2);
}
