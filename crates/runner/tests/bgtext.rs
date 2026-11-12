// SPDX-License-Identifier: Apache-2.0
//! Engine A's text backgrounds, observed through our own `bgtext` test ROM (`testroms/bgtext/`).
//! The ROM builds tile data, maps and a palette (layout at the top of `arm9.s`) and shows four
//! layers with different sizes, color depths, priorities, scrolls and flips, plus a scroll that
//! changes at line 101.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/bg-text.md` ("The
//! picture of one layer at one pixel", "Putting the layers together", "Lines") and the data the
//! ROM's source lists. It is never read back from the emulator: the code below shares nothing
//! with `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected; the ROM finishes its setup inside the first.
const FRAMES: u64 = 4;

/// A machine that booted the `bgtext` ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("bgtext")).expect("bgtext ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("bgtext ROM has a valid header");
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

/// The backdrop: palette entry 0 is overwritten with 0x03E0 (arm9.s, "Palette").
const BACKDROP: u32 = 0x0000_FF00;

/// One layer as the ROM sets it up.
struct Layer {
    priority: u8,
    eight_bpp: bool,
    /// Size in pixels.
    width: usize,
    height: usize,
    /// Scroll at line `y`.
    hofs: fn(usize) -> usize,
    vofs: usize,
    /// The screen entry of map cell (`cx`, `cy`), counted over the whole layer.
    entry: fn(usize, usize) -> u16,
    /// The palette value of a tile's pixel: (tile, row, column) -> value.
    tile_value: fn(u16, usize, usize) -> u8,
}

/// "asym": row r, column c holds 1 + ((3r + c) mod 8).
fn asym(row: usize, column: usize) -> u8 {
    (1 + (3 * row + column) % 8) as u8
}

fn bg0_entry(cx: usize, cy: usize) -> u16 {
    match (cx, cy) {
        (2, 2) => 0x0001,
        (3, 2) => 0x0401,
        (4, 2) => 0x0801,
        (5, 2) => 0x0C01,
        (6, 2) => 0x3001,
        (6, 3) => 0x3401,
        (10..=12, 8..=10) => 0x0002,
        _ => 0,
    }
}

fn bg0_tile(tile: u16, row: usize, column: usize) -> u8 {
    match tile {
        1 => asym(row, column),
        2 => 9,
        _ => 0,
    }
}

fn bg1_entry(cx: usize, cy: usize) -> u16 {
    match (cx, cy) {
        (_, 4) => 0x0001,
        (31, 6) => 0x0801,
        (32, 6) => 0x0401,
        _ => 0,
    }
}

/// "grad": row r, column c holds 16 + 8r + c.
fn bg1_tile(tile: u16, row: usize, column: usize) -> u8 {
    match tile {
        1 => (16 + 8 * row + column) as u8,
        _ => 0,
    }
}

fn bg2_entry(cx: usize, cy: usize) -> u16 {
    match (cx, cy) {
        (5, 31) => 0x5001,
        (5, 32) => 0x5801,
        (5, 35) => 0x5001,
        _ => 0,
    }
}

fn bg2_tile(tile: u16, row: usize, column: usize) -> u8 {
    match tile {
        1 => asym(row, column),
        _ => 0,
    }
}

/// Tile b + 1 in cells (0..15, 0..23) of block b; blocks are top-left, top-right, bottom-left,
/// bottom-right.
fn bg3_entry(cx: usize, cy: usize) -> u16 {
    let inside = cx % 32 < 16 && cy % 32 < 24;
    let block = (cy / 32) * 2 + cx / 32;
    if inside { block as u16 + 1 } else { 0 }
}

/// Solid tiles with the values 100-103.
fn bg3_tile(tile: u16, _row: usize, _column: usize) -> u8 {
    match tile {
        1..=4 => 99 + tile as u8,
        _ => 0,
    }
}

fn no_scroll(_line: usize) -> usize {
    0
}

fn bg1_hofs(_line: usize) -> usize {
    252
}

/// BG3's scroll: 256 from line 101 on (the ROM writes it after VCOUNT reaches 100).
fn bg3_hofs(line: usize) -> usize {
    if line >= 101 { 256 } else { 0 }
}

fn layers() -> [Layer; 4] {
    [
        Layer {
            priority: 0,
            eight_bpp: false,
            width: 256,
            height: 256,
            hofs: no_scroll,
            vofs: 0,
            entry: bg0_entry,
            tile_value: bg0_tile,
        },
        Layer {
            priority: 1,
            eight_bpp: true,
            width: 512,
            height: 256,
            hofs: bg1_hofs,
            vofs: 3,
            entry: bg1_entry,
            tile_value: bg1_tile,
        },
        Layer {
            priority: 1,
            eight_bpp: false,
            width: 256,
            height: 512,
            hofs: no_scroll,
            vofs: 250,
            entry: bg2_entry,
            tile_value: bg2_tile,
        },
        Layer {
            priority: 2,
            eight_bpp: true,
            width: 512,
            height: 512,
            hofs: bg3_hofs,
            vofs: 0,
            entry: bg3_entry,
            tile_value: bg3_tile,
        },
    ]
}

// ------------------------------------------------------------------ the note's rules

/// The palette index of layer pixel (`x`, `y`), or `None` where it is transparent.
fn layer_index(layer: &Layer, x: usize, y: usize) -> Option<usize> {
    let px = (x + (layer.hofs)(y)) % layer.width;
    let py = (y + layer.vofs) % layer.height;
    let entry = (layer.entry)(px / 8, py / 8);
    let tile = entry & 0x3FF;
    let column = if entry & 0x400 != 0 {
        7 - px % 8
    } else {
        px % 8
    };
    let row = if entry & 0x800 != 0 {
        7 - py % 8
    } else {
        py % 8
    };
    let value = usize::from((layer.tile_value)(tile, row, column));
    match (value, layer.eight_bpp) {
        (0, _) => None,
        (value, true) => Some(value),
        (value, false) => Some(usize::from(entry >> 12) * 16 + value),
    }
}

/// The expected xRGB pixel: the front-most layer that is not transparent, else the backdrop.
fn expected_pixel(x: usize, y: usize) -> u32 {
    let layers = layers();
    let mut order: Vec<usize> = (0..4).collect();
    order.sort_by_key(|&n| (layers[n].priority, n));
    order
        .iter()
        .find_map(|&n| layer_index(&layers[n], x, y))
        .map_or(BACKDROP, |index| expand(palette(index)))
}

/// The pixels of the top screen that differ from the note's picture: (x, y, expected, got).
fn picture_mismatches(machine: &Machine) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(Screen::Top);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| (x, y, expected_pixel(x, y), frame[y * SCREEN_WIDTH + x]))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_bgtext_rom_when_4_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_bgtext_rom_when_4_frames_run_then_the_lower_screen_is_black() {
    // Engine B is a black placeholder in Phase 3 (`display-mode-vram.md`).
    let machine = boot_frames();
    assert!(
        machine
            .frame(Screen::Bottom)
            .iter()
            .all(|&pixel| pixel == 0)
    );
}

#[test]
fn given_bgtext_rom_when_4_frames_run_then_the_picture_uses_every_part_of_the_scene() {
    // Guards the test above against a scene that went blank: the backdrop, the tile colors of
    // each depth, both halves of the split scroll and a flipped tile all appear.
    let machine = boot_frames();
    let frame = machine.frame(Screen::Top);
    let at = |x: usize, y: usize| frame[y * SCREEN_WIDTH + x];
    assert_eq!(
        (
            at(200, 150),
            // BG0 cell (2, 2) pixel (0, 0): value 1 of "asym", palette entry 1.
            at(16, 16) == expand(palette(1)),
            // BG3 above and below the split: values 100 and 101.
            at(10, 100) == expand(palette(100)),
            at(10, 101) == expand(palette(101)),
            // BG0's horizontally flipped tile shows the stored column 7 first: value 8.
            at(24, 16) == expand(palette(8)),
        ),
        (BACKDROP, true, true, true, true)
    );
}

/// A layer's decoded registers: priority, character base, screen base, 8 bits per pixel, width,
/// height, HOFS, VOFS.
type LayerState = (u8, u32, u32, bool, u16, u16, u16, u16);

/// What the four layers hold. BG3's HOFS is 0 or 256 depending on the line the machine stopped
/// at (the ROM changes it from line 101), so it is reported as 0 here.
fn layer_states(machine: &Machine) -> Vec<LayerState> {
    (0..4)
        .map(|n| {
            let l = machine.inspect().gpu2d().text_bg(EngineId::A, n);
            let hofs = if n == 3 { 0 } else { l.hofs };
            (
                l.priority,
                l.char_base,
                l.screen_base,
                l.eight_bpp,
                l.width,
                l.height,
                hofs,
                l.vofs,
            )
        })
        .collect()
}

#[test]
fn given_bgtext_rom_when_4_frames_run_then_the_rom_wrote_the_registers_it_set() {
    let machine = boot_frames();
    assert_eq!(
        (
            machine.inspect().gpu2d().dispcnt_a(),
            machine.inspect().gpu2d().powcnt1(),
            layer_states(&machine)
        ),
        (
            0x0101_0F00,
            0x8003,
            vec![
                (0, 0x1_0000, 0x0000, false, 256, 256, 0, 0),
                (1, 0x1_4000, 0x1000, true, 512, 256, 252, 3),
                (1, 0x1_8000, 0x2000, false, 256, 512, 0, 250),
                (2, 0x1_C000, 0x3000, true, 512, 512, 0, 0),
            ]
        )
    );
}

#[test]
fn given_bgtext_rom_when_run_for_4_frames_then_both_screens_match_golden() {
    // The pixel test above is the evidence that this frame is the right one: the hash pins it.
    assert_frame_hash("bgtext", 4);
}

#[test]
fn given_bgtext_rom_when_run_twice_for_4_frames_then_every_frame_hash_matches() {
    assert_deterministic("bgtext", 4);
}

#[test]
fn given_bgtext_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("bgtext", 2, 2);
}
