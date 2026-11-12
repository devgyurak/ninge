// SPDX-License-Identifier: Apache-2.0
//! Object priority on both engines, observed through our own `objprio` test ROM
//! (`testroms/objprio/`). The ROM shows 8x8 objects of priorities 0-3 over and under text
//! backgrounds and over and under each other (the tables are at the end of `arm9.s`).
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/obj.md` ("Priority")
//! and `docs/hw/bg-text.md` ("Putting the layers together"): an object is in front of a background
//! layer whose priority value is the same or greater, and objects are ordered by (priority,
//! entry). They are never read back from the emulator: the code below shares nothing with
//! `ninge-core`.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const FRAMES: u64 = 3;

fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("objprio")).expect("objprio ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("objprio ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

// ------------------------------------------------------------------ the data of the ROM

/// Palette entry `index` as a 15-bit colour (arm9.s, "Palettes"): every palette is the same.
fn palette(index: usize) -> u16 {
    let red = index & 31;
    let green = (3 * index) & 31;
    let blue = (5 * index) & 31;
    (red | green << 5 | blue << 10) as u16
}

fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// Engine A's backdrop is 0x03E0 (green), engine B's 0x7C00 (blue).
const BACKDROP_A: u32 = 0x0000_FF00;
const BACKDROP_B: u32 = 0x0000_00FF;

/// An 8x8 object: Y, X, tile (its pixel value; 9 is the checker) and priority.
#[derive(Clone, Copy)]
struct Obj {
    y: usize,
    x: usize,
    tile: u8,
    priority: u8,
}

const fn obj(y: usize, x: usize, tile: u8, priority: u8) -> Obj {
    Obj {
        y,
        x,
        tile,
        priority,
    }
}

/// The value of an object's pixel (`row`, `column`): solid tiles have their own number, the checker
/// is 9 where row + column is odd and 0 (transparent) elsewhere.
fn object_value(o: &Obj, row: usize, column: usize) -> u8 {
    match (o.tile, (row + column) % 2) {
        (9, 1) => 9,
        (9, _) => 0,
        (tile, _) => tile,
    }
}

fn objects_a() -> Vec<Obj> {
    vec![
        obj(16, 8, 1, 0),
        obj(16, 22, 2, 1),
        obj(16, 36, 3, 2),
        obj(16, 50, 4, 3),
        obj(16, 72, 1, 0),
        obj(16, 84, 2, 1),
        obj(16, 96, 3, 2),
        obj(16, 108, 4, 3),
        obj(64, 16, 1, 0),
        obj(68, 20, 2, 0),
        obj(64, 48, 3, 1),
        obj(68, 52, 4, 0),
        obj(64, 80, 5, 0),
        obj(68, 84, 6, 1),
        obj(64, 112, 9, 0),
        obj(68, 116, 7, 0),
        obj(100, 16, 1, 2),
        obj(100, 16, 2, 1),
        obj(100, 16, 3, 1),
        obj(100, 60, 9, 1),
        obj(100, 60, 8, 1),
        obj(104, 64, 3, 0),
        obj(140, 100, 9, 0),
        obj(140, 140, 9, 0),
    ]
}

fn objects_b() -> Vec<Obj> {
    vec![
        obj(16, 8, 1, 0),
        obj(16, 22, 2, 1),
        obj(16, 36, 3, 2),
        obj(16, 50, 4, 3),
        obj(64, 16, 1, 0),
        obj(68, 20, 2, 0),
        obj(64, 48, 3, 1),
        obj(68, 52, 4, 0),
        obj(64, 80, 5, 0),
        obj(68, 84, 6, 1),
        obj(64, 112, 9, 0),
        obj(68, 116, 7, 0),
        obj(100, 16, 1, 2),
        obj(100, 16, 2, 1),
        obj(100, 16, 3, 1),
    ]
}

/// A background layer's pixel value at (`x`, `y`) and its priority: `None` where transparent.
type Background = fn(usize, usize) -> Option<(u8, u8)>;

/// Engine A's BG0: priority 2, value 5 everywhere; BG1: priority 1, value 6 in columns 64..127
/// (cells 8..15 of rows 0..23); the front-most background pixel.
fn backgrounds_a(x: usize, y: usize) -> Option<(u8, u8)> {
    match (x / 8, y / 8) {
        (8..=15, 0..=23) => Some((6, 1)),
        _ => Some((5, 2)),
    }
}

/// Engine B's BG0: priority 1, value 5 everywhere.
fn backgrounds_b(_x: usize, _y: usize) -> Option<(u8, u8)> {
    Some((5, 1))
}

// ------------------------------------------------------------------ the note's rules

/// The expected pixel: the object with the lowest (priority, entry) that has a pixel there, unless
/// the background in front has a smaller priority value; otherwise the background, otherwise the
/// backdrop.
fn expected_pixel(
    objects: &[Obj],
    background: Background,
    backdrop: u32,
    x: usize,
    y: usize,
) -> u32 {
    let object = objects
        .iter()
        .enumerate()
        .filter_map(|(entry, o)| {
            let (row, column) = (y.checked_sub(o.y)?, x.checked_sub(o.x)?);
            let value = (row < 8 && column < 8).then(|| object_value(o, row, column))?;
            (value != 0).then_some(((o.priority, entry), value))
        })
        .min_by_key(|&(key, _)| key);
    let background = background(x, y);
    let color = |value: u8| expand(palette(usize::from(value)));
    match (object, background) {
        (Some(((priority, _), value)), Some((_, bg_priority))) if priority <= bg_priority => {
            color(value)
        }
        (_, Some((value, _))) => color(value),
        (Some((_, value)), None) => color(value),
        (None, None) => backdrop,
    }
}

fn picture_mismatches(
    machine: &Machine,
    screen: Screen,
    objects: &[Obj],
    background: Background,
    backdrop: u32,
) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(screen);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| {
            (
                x,
                y,
                expected_pixel(objects, background, backdrop, x, y),
                frame[y * SCREEN_WIDTH + x],
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_objprio_rom_when_3_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(
        &machine,
        Screen::Top,
        &objects_a(),
        backgrounds_a,
        BACKDROP_A,
    );
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objprio_rom_when_3_frames_run_then_every_pixel_of_the_lower_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    let mismatches = picture_mismatches(
        &machine,
        Screen::Bottom,
        &objects_b(),
        backgrounds_b,
        BACKDROP_B,
    );
    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(8)]
    );
}

#[test]
fn given_objprio_rom_when_3_frames_run_then_the_rules_show_in_the_pixels_themselves() {
    // Spot checks that do not use the model above, straight from the note: the pixel value is
    // the palette entry of the winning object or background.
    let machine = boot_frames();
    let top = machine.frame(Screen::Top);
    let bottom = machine.frame(Screen::Bottom);
    let at = |frame: &[u32], x: usize, y: usize| frame[y * SCREEN_WIDTH + x];
    let color = |value: usize| expand(palette(value));
    assert_eq!(
        [
            // Engine A over BG0 (priority 2): priorities 0-2 win, priority 3 loses to the BG.
            at(top, 10, 18),
            at(top, 24, 18),
            at(top, 38, 18),
            at(top, 52, 18),
            // Over BG1 (priority 1): priorities 0 and 1 win, 2 and 3 lose.
            at(top, 74, 18),
            at(top, 86, 18),
            at(top, 98, 18),
            at(top, 110, 18),
            // A later entry with a smaller priority value (entry 11, priority 0) beats entry 10.
            at(top, 54, 70),
            // Engine B over BG0 (priority 1).
            at(bottom, 10, 18),
            at(bottom, 24, 18),
            at(bottom, 38, 18),
            at(bottom, 52, 18),
        ],
        [
            color(1),
            color(2),
            color(3),
            color(5),
            color(1),
            color(2),
            color(6),
            color(6),
            color(4),
            color(1),
            color(2),
            color(5),
            color(5),
        ]
    );
}

#[test]
fn given_objprio_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    assert_frame_hash("objprio", 3);
}

#[test]
fn given_objprio_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("objprio", 3);
}

#[test]
fn given_objprio_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("objprio", 2, 2);
}
