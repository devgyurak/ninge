// SPDX-License-Identifier: Apache-2.0
//! Mosaic, observed through our own `bgmosaic` test ROM (`testroms/bgmosaic/`). Engine A (BG mode 4)
//! shows a text layer, an affine layer and a 256-colour bitmap, all with mosaic (blocks 5 wide and
//! 3 high), on the upper screen; engine B (BG mode 0) shows a text layer with mosaic (2 wide, 4 high)
//! over a text layer without it on the lower one. The text tiles include one with a hole in each row,
//! so blocks whose first pixel is transparent occur.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/bg-modes.md` ("Mosaic",
//! "Affine layers", "Extended layers") and `docs/hw/bg-text.md` and the data the ROM's source
//! lists. It is never read back from the emulator: the code below shares nothing with `ninge-core`.

use ninge_core::debug::{EngineId, Mosaic};
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected; the ROM finishes its setup inside the first three.
const FRAMES: u64 = 4;

/// A machine that booted the `bgmosaic` ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("bgmosaic")).expect("bgmosaic ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("bgmosaic ROM has a valid header");
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

/// Engine B's backdrop (palette entry 0 = 0x7C00). Engine A's never shows: BG3 is opaque everywhere.
const BACKDROP_B: u32 = 0x0000_00FF;

/// The value of pixel (`row`, `column`) of text tile `tile` (arm9.s, "Text tiles"); 0 is
/// transparent.
fn tile_value(tile: u16, row: usize, column: usize) -> usize {
    match tile {
        1 => 1 + (3 * row + column) % 8,
        2 => 9,
        3 => (row + column) % 8,
        _ => 0,
    }
}

/// A text layer: 256x256, 4 bits per pixel, palette number 0 in every entry.
struct Text {
    /// The screen entry of cell (`cx`, `cy`); 0 holds tile 0 (transparent).
    entry: fn(usize, usize) -> u16,
    hofs: usize,
    vofs: usize,
}

/// The palette index of text layer pixel (`x`, `y`) (screen coordinates, before the mosaic has
/// picked the block's first pixel), or `None` where it is transparent.
fn text_index(layer: &Text, x: usize, y: usize) -> Option<usize> {
    let (px, py) = ((x + layer.hofs) % 256, (y + layer.vofs) % 256);
    let entry = (layer.entry)(px / 8, py / 8);
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
    match tile_value(entry & 0x3FF, row, column) {
        0 => None,
        value => Some(value),
    }
}

fn a_bg0_entry(cx: usize, cy: usize) -> u16 {
    match (cx, cy) {
        (3..=8, 2) => 0x0001,
        (3..=8, 3) => 0x0401,
        (3..=8, 4) | (3..=8, 9) => 0x0003,
        (10..=13, 6) => 0x0002,
        (10..=13, 7) => 0x0802,
        _ => 0,
    }
}

fn b_bg0_entry(cx: usize, cy: usize) -> u16 {
    match (cx, cy) {
        (1..=6, 1) => 0x0001,
        (1..=6, 2) => 0x0003,
        (8..=9, 4) => 0x0002,
        (8..=9, 5) => 0x0402,
        _ => 0,
    }
}

fn b_bg1_entry(cx: usize, cy: usize) -> u16 {
    match (cx, cy) {
        (_, 8) => 0x0001,
        (_, 9) => 0x0003,
        (2..=10, 1) => 0x0401,
        (2..=10, 2) => 0x0801,
        (2..=10, 3) => 0x0C01,
        _ => 0,
    }
}

/// The palette value of texel (`tx`, `ty`) of the affine layer's tiles and map (arm9.s, BG2): the
/// tile of cell (cx, cy) is `1 + ((cx + 2 cy) mod 4)`; its pixel (r, c) holds `16 tile + ((3r + c)
/// mod 16)`.
fn affine_value(tx: i64, ty: i64) -> usize {
    let tile = 1 + ((tx / 8 + 2 * (ty / 8)) & 3);
    (16 * tile + ((3 * (ty % 8) + tx % 8) & 15)) as usize
}

/// BG2: 256x256, not wrapping; PA, PB, PC, PD and X, Y as the ROM writes them. `None` where the
/// transform leaves the layer.
fn bg2_index(x: usize, y: usize) -> Option<usize> {
    // The line starts where `y` steps of PB and PD have walked from the top.
    let u = 0x1000 - 0x70 * y as i64 + 0xE0 * x as i64;
    let v = 0x0800 + 0xE0 * y as i64 + 0x70 * x as i64;
    let (tx, ty) = (u.div_euclid(256), v.div_euclid(256));
    ((0..256).contains(&tx) && (0..256).contains(&ty)).then(|| affine_value(tx, ty))
}

/// BG3: the 256-colour bitmap, identity transform, wrapping: `1 + ((5 tx + 3 ty) mod 255)`.
fn bg3_index(x: usize, y: usize) -> usize {
    1 + (5 * x + 3 * y) % 255
}

// ------------------------------------------------------------------ the note's rules

/// The first pixel of the block that holds (`x`, `y`) for blocks `w` by `h` from the top left.
fn block_origin(x: usize, y: usize, (w, h): (usize, usize)) -> (usize, usize) {
    (x - x % w, y - y % h)
}

/// Engine A: BG0 (priority 0), BG2 (1), BG3 (2), each showing the pixel at the first pixel of its
/// 5x3 block. BG2 does not wrap, so where it is outside its layer BG3, which is opaque everywhere,
/// shows.
fn expected_a(x: usize, y: usize) -> u32 {
    let (bx, by) = block_origin(x, y, (5, 3));
    let bg0 = Text {
        entry: a_bg0_entry,
        hofs: 3,
        vofs: 5,
    };
    let index = text_index(&bg0, bx, by)
        .or_else(|| bg2_index(bx, by))
        .unwrap_or_else(|| bg3_index(bx, by));
    expand(palette(index))
}

/// Engine B: BG0 (priority 0, 2x4 blocks) over BG1 (no mosaic), else the backdrop.
fn expected_b(x: usize, y: usize) -> u32 {
    let (bx, by) = block_origin(x, y, (2, 4));
    let bg0 = Text {
        entry: b_bg0_entry,
        hofs: 0,
        vofs: 0,
    };
    let bg1 = Text {
        entry: b_bg1_entry,
        hofs: 0,
        vofs: 0,
    };
    text_index(&bg0, bx, by)
        .or_else(|| text_index(&bg1, x, y))
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
fn given_bgmosaic_rom_when_4_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    assert_eq!(mismatches(&machine, Screen::Top, expected_a), vec![]);
}

#[test]
fn given_bgmosaic_rom_when_4_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    assert_eq!(mismatches(&machine, Screen::Bottom, expected_b), vec![]);
}

#[test]
fn given_bgmosaic_rom_when_4_frames_run_then_both_screens_show_blocks_and_many_colors() {
    // Guards the two tests above against a scene that went blank or lost its mosaic: some block
    // of the upper screen is uniform across 5x3 pixels and the lower screen has its backdrop.
    let machine = boot_frames();
    let top = machine.frame(Screen::Top);
    let uniform = |x: usize, y: usize| {
        (0..3).all(|dy| {
            (0..5).all(|dx| top[(y + dy) * SCREEN_WIDTH + x + dx] == top[y * SCREEN_WIDTH + x])
        })
    };
    assert_eq!(
        (
            colors(&machine, Screen::Top) > 30,
            colors(&machine, Screen::Bottom) > 5,
            uniform(100, 99),
            uniform(10, 30)
        ),
        (true, true, true, true)
    );
}

#[test]
fn given_bgmosaic_rom_when_4_frames_run_then_the_rom_wrote_the_registers_it_set() {
    let machine = boot_frames();
    let gpu = machine.inspect().gpu2d();
    let one = |h, v| Mosaic {
        bg_h: h,
        bg_v: v,
        obj_h: 1,
        obj_v: 1,
    };
    assert_eq!(
        (
            gpu.dispcnt_a(),
            gpu.dispcnt_b(),
            gpu.mosaic(EngineId::A),
            gpu.mosaic(EngineId::B),
            [
                gpu.text_bg(EngineId::A, 0).mosaic,
                gpu.affine_bg(EngineId::A, 2).mosaic,
                gpu.affine_bg(EngineId::A, 3).mosaic,
                gpu.text_bg(EngineId::B, 0).mosaic,
                gpu.text_bg(EngineId::B, 1).mosaic,
            ]
        ),
        (
            0x0001_0D04,
            0x0001_0300,
            one(5, 3),
            one(2, 4),
            [true, true, true, true, false]
        )
    );
}

#[test]
fn given_bgmosaic_rom_when_run_for_4_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that this frame is the right one: the hash pins it.
    assert_frame_hash("bgmosaic", 4);
}

#[test]
fn given_bgmosaic_rom_when_run_twice_for_4_frames_then_every_frame_hash_matches() {
    assert_deterministic("bgmosaic", 4);
}

#[test]
fn given_bgmosaic_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("bgmosaic", 2, 2);
}
