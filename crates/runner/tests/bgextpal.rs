// SPDX-License-Identifier: Apache-2.0
//! BG extended palettes on both engines, observed through our own `bgextpal` test ROM
//! (`testroms/bgextpal/`). The ROM fills banks E, F, G and H with computed extended palettes while
//! they are in LCDC mode, then maps them, and shows four bands of layers on each screen: two 256-
//! colour text layers (BG0, and BG1 with BGxCNT bit 13 set, which moves it to slot 3), a layer that
//! stays on the standard palette (a 16-colour text layer on engine A, an affine layer with one-byte
//! entries on engine B) and an extended layer with 16-bit tile entries (BG3, slot 3). Engine A
//! reads bank E for lines 0-95 and F and G for lines 96-191 (the ROM changes VRAMCNT by polling
//! VCOUNT); engine B has DISPCNT bit 30 set for lines 0-95 and clear after.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/extended-palettes.md` and
//! the formulas at the top of the ROM's `arm9.s`. It is never read back from the emulator: the code
//! below shares nothing with `ninge-core`.

use ninge_core::debug::EngineId;
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected: the ROM fills 48K colours and then settles into its
/// per-frame split.
const FRAMES: u64 = 6;

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("bgextpal")).expect("bgextpal ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("bgextpal ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

/// The standard palette entry `index` as a 15-bit colour (arm9.s, "Standard palettes").
fn standard(index: usize) -> u16 {
    let red = index & 31;
    let green = (3 * index) & 31;
    let blue = (5 * index) & 31;
    (red | green << 5 | blue << 10) as u16
}

/// The extended colour of slot `slot`, palette `palette`, index `index` in a bank whose constant
/// is `k` (arm9.s, "Extended palette colours").
fn extended(k: usize, slot: usize, palette: usize, index: usize) -> u16 {
    ((slot * 0x1F1 + palette * 0xB3 + index * 0x47 + k) & 0x7FFF) as u16
}

fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

const K_E: usize = 0x000;
const K_FG: usize = 0x2A5;
const K_H: usize = 0x333;

/// The pixel value of tile 1 at (`row`, `column`), 8 or 4 bits per pixel (arm9.s, "Tiles").
fn tile_value(row: usize, column: usize) -> usize {
    1 + ((3 * row + column) & 7)
}

/// The expected colour of engine A's pixel (`x`, `y`): the band is `x / 64`, the palette number
/// `(y / 8) mod 16`, the value the tile's; the bank is E for lines 0-95 and F or G after.
fn expected_a(x: usize, y: usize) -> u32 {
    let value = tile_value(y % 8, x % 8);
    let palette = (y / 8) % 16;
    // Slot s comes from bank E, or from F (slots 0-1) / G (slots 2-3), with their own constants.
    let k = if y < 96 { K_E } else { K_FG };
    let color = match x / 64 {
        // BG0 reads slot 0.
        0 => extended(k, 0, palette, value),
        // BG1 has BGxCNT bit 13 set: slot 3.
        1 => extended(k, 3, palette, value),
        // BG2 is 16 colours: standard palette `palette * 16 + value`.
        2 => standard(palette * 16 + value),
        // BG3, 16-bit entries: slot 3.
        _ => extended(k, 3, palette, value),
    };
    expand(color)
}

/// The expected colour of engine B's pixel: extended palettes (bank H) for lines 0-95, the standard
/// palette (the pixel value) after.
fn expected_b(x: usize, y: usize) -> u32 {
    let value = tile_value(y % 8, x % 8);
    let palette = (y / 8) % 16;
    let on = y < 96;
    let color = match (x / 64, on) {
        (0, true) => extended(K_H, 0, palette, value),
        (1, true) | (3, true) => extended(K_H, 3, palette, value),
        // BG2 is an affine layer with one-byte entries: always the standard palette. With bit 30
        // clear the others read it too (a 256-colour layer: index = value).
        _ => standard(value),
    };
    expand(color)
}

fn picture_mismatches(
    machine: &Machine,
    screen: Screen,
    expected: fn(usize, usize) -> u32,
) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(screen);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| (x, y, expected(x, y), frame[y * SCREEN_WIDTH + x]))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_bgextpal_rom_when_6_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Top, expected_a);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_bgextpal_rom_when_6_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(&machine, Screen::Bottom, expected_b);
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_bgextpal_rom_when_6_frames_run_then_the_pictures_differ_from_the_standard_palette_ones() {
    // Guards the two tests above against a ROM whose extended palettes did nothing: the same layers
    // read through the standard palette would give a different picture, and the halves split at
    // line 96 differ from each other.
    let machine = boot_frames();
    let (top, bottom) = (machine.frame(Screen::Top), machine.frame(Screen::Bottom));
    let at = |frame: &[u32], x: usize, y: usize| frame[y * SCREEN_WIDTH + x];
    assert_eq!(
        (
            // BG0 (x 3, y 3) on engine A: the extended colour, not the standard one, and the
            // bank E and bank F/G halves give different colours.
            at(top, 3, 3) != expand(standard(tile_value(3, 3))),
            at(top, 3, 3) != at(top, 3, 99),
            // Engine B's upper half reads bank H, its lower half the standard palette.
            at(bottom, 3, 3) != expand(standard(tile_value(3, 3))),
            at(bottom, 3, 99) == expand(standard(tile_value(3, 3))),
        ),
        (true, true, true, true)
    );
}

#[test]
fn given_bgextpal_rom_when_6_frames_run_then_the_slots_report_the_banks_of_the_current_line() {
    // The machine stops at the end of a frame, after the ROM restored bank E (line 200), so the
    // frame-start state shows: E backs engine A's four slots, H engine B's, F and G are in LCDC mode.
    let machine = boot_frames();
    let view = machine.inspect();
    let (a, b) = (
        view.ext_palette_banks(EngineId::A),
        view.ext_palette_banks(EngineId::B),
    );
    assert_eq!(
        (a.bg, a.obj, b.bg, b.obj),
        ([Some(4); 4], None, [Some(7); 4], None)
    );
}

#[test]
fn given_bgextpal_rom_when_run_for_6_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these frames are the right ones: the hash pins
    // them.
    assert_frame_hash("bgextpal", 6);
}

#[test]
fn given_bgextpal_rom_when_run_twice_for_6_frames_then_every_frame_hash_matches() {
    assert_deterministic("bgextpal", 6);
}

#[test]
fn given_bgextpal_rom_when_saved_after_4_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("bgextpal", 4, 2);
}
