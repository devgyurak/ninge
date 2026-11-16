// SPDX-License-Identifier: Apache-2.0
//! The master brightness, observed through our own `mbright` test ROM (`testroms/mbright/`). Engine A
//! shows a gradient in display mode 2 and then the layers in display mode 1; engine B shows three
//! bands (BG0 red, the white backdrop, BG2 blue). The ROM steps through nine MASTER_BRIGHT settings,
//! one for each frame: up, down, off, reserved, a factor of 0 and above 16, in both display modes.
//!
//! The expected pictures are worked out here, pixel by pixel, from `docs/hw/blending.md` and the
//! data the ROM's source lists. They are never read back from the emulator: the code below shares
//! nothing with `ninge-core`. Which frame shows which setting is not modelled: the test keeps the
//! pictures in the order they appear, drops the white screens before the ROM sets DISPCNT and any
//! frame equal to the one before, and compares that list with the nine settings in order.
//!
//! The pictures depend on the choices `blending.md` lists under "Unverified" (the sixteenths are
//! truncated, a 5-bit channel is widened to 6 bits by repeating its top bit): the golden pins those
//! choices, and changes with them.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run: the ROM writes one setting a frame, nine in all, after a short setup.
const FRAMES: u64 = 14;

/// Both screens after each frame, in the order the frames were made.
fn pictures() -> Vec<Picture> {
    let rom = load_rom_file(&testrom_path("mbright")).expect("mbright ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("mbright ROM has a valid header");
    (0..FRAMES)
        .map(|_| {
            machine.run_frame(ninge_core::Input::new());
            Picture {
                top: machine.frame(Screen::Top).to_vec(),
                bottom: machine.frame(Screen::Bottom).to_vec(),
            }
        })
        .collect()
}

#[derive(Clone, PartialEq, Debug)]
struct Picture {
    top: Vec<u32>,
    bottom: Vec<u32>,
}

/// `list` without a picture equal to the one before it.
fn without_repeats(list: Vec<Picture>) -> Vec<Picture> {
    let mut kept: Vec<Picture> = Vec::new();
    list.into_iter().for_each(|picture| {
        if kept.last() != Some(&picture) {
            kept.push(picture);
        }
    });
    kept
}

// ------------------------------------------------------------------ the data of the ROM

const WHITE: u16 = 0x7FFF;
const RED: u16 = 0x001F;
const BLUE: u16 = 0x7C00;

/// MASTER_BRIGHT modes (bits 14-15).
const UP: u16 = 1 << 14;
const DOWN: u16 = 2 << 14;
const RESERVED: u16 = 3 << 14;

/// Engine A's DISPCNT in display mode 2 and in display mode 1 (arm9.s, "Steps").
const MODE_2: u32 = 0x0002_0000;
const MODE_1: u32 = 0x0001_6F00;

/// One step: engine A's display mode, then MASTER_BRIGHT of engine A and of engine B.
const STEPS: [(u32, u16, u16); 9] = [
    (MODE_2, UP | 8, UP | 8),
    (MODE_2, DOWN | 8, DOWN | 8),
    (MODE_2, 8, DOWN | 8),
    (MODE_2, RESERVED | 8, RESERVED | 8),
    (MODE_2, UP, DOWN | 31),
    (MODE_2, UP | 31, DOWN | 16),
    (MODE_1, UP | 8, 0),
    (MODE_1, DOWN | 4, UP | 4),
    (MODE_1, 0, DOWN | 16),
];

/// The 15-bit colour as xRGB8888 with the unit off (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The colour as xRGB8888 after MASTER_BRIGHT `register` (`blending.md`, "Master brightness"): with
/// mode 1 or 2 each channel is widened to 6 bits as `c << 1 | c >> 4`, moved toward 63 or 0 by the
/// factor (at most 16) in sixteenths with the quotient truncated, and expanded as `c6 << 2 | c6 >> 4`.
fn brightened(register: u16, color: u16) -> u32 {
    let (mode, factor) = (register >> 14, u32::from((register & 31).min(16)));
    let channel = |c: u16| {
        let five = u32::from(c & 31);
        let six = five << 1 | five >> 4;
        let moved = match mode {
            1 => six + (63 - six) * factor / 16,
            _ => six - six * factor / 16,
        };
        moved << 2 | moved >> 4
    };
    match mode {
        1 | 2 => channel(color) << 16 | channel(color >> 5) << 8 | channel(color >> 10),
        _ => expand(color),
    }
}

/// The gradient engine A shows in display mode 2 (arm9.s, "The gradient").
fn gradient(x: usize, y: usize) -> u16 {
    let (r, g) = ((x >> 3) as u16, (y >> 3) as u16);
    r | g << 5 | (r ^ g) << 10
}

/// The band of the layers at column `x`: BG0 red, the backdrop, BG2 blue (arm9.s, "Windows").
fn band(x: usize) -> u16 {
    match x {
        0..85 => RED,
        85..170 => WHITE,
        _ => BLUE,
    }
}

/// The screen of `pixel` under MASTER_BRIGHT `register`.
fn screen(register: u16, pixel: impl Fn(usize, usize) -> u16) -> Vec<u32> {
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| brightened(register, pixel(x, y)))
        .collect()
}

/// The pictures the ROM goes through: no brightness, then each step. Engine A is on the upper
/// screen, in display mode 2 until step 7.
fn expected() -> Vec<Picture> {
    let picture = |mode: u32, a: u16, b: u16| Picture {
        top: match mode == MODE_2 {
            true => screen(a, gradient),
            false => screen(a, |x, _| band(x)),
        },
        bottom: screen(b, |x, _| band(x)),
    };
    std::iter::once(picture(MODE_2, 0, 0))
        .chain(STEPS.iter().map(|&(mode, a, b)| picture(mode, a, b)))
        .collect()
}

/// The (picture index, screen, x, y, got, want) of the first pixels that differ.
fn misses(got: &[Picture], want: &[Picture]) -> Vec<(usize, &'static str, usize, usize, u32, u32)> {
    got.iter()
        .zip(want)
        .enumerate()
        .flat_map(|(i, (got, want))| {
            [
                ("top", &got.top, &want.top),
                ("bottom", &got.bottom, &want.bottom),
            ]
            .into_iter()
            .flat_map(move |(name, got, want)| {
                got.iter()
                    .zip(want)
                    .enumerate()
                    .filter_map(move |(p, (&g, &w))| {
                        (g != w).then_some((i, name, p % SCREEN_WIDTH, p / SCREEN_WIDTH, g, w))
                    })
            })
        })
        .take(8)
        .collect()
}

/// The pictures the ROM shows, without the white ones before DISPCNT is set and without repeats.
fn shown_pictures() -> Vec<Picture> {
    let white = expand(WHITE);
    without_repeats(
        pictures()
            .into_iter()
            .filter(|p| !(p.top.iter().chain(&p.bottom)).all(|&px| px == white))
            .collect(),
    )
}

// ------------------------------------------------------------------------------ the tests

#[test]
fn given_mbright_rom_when_run_then_it_goes_through_the_plain_picture_and_nine_settings() {
    assert_eq!(shown_pictures().len(), expected().len());
}

#[test]
fn given_mbright_rom_when_run_then_each_picture_of_both_engines_matches_the_note() {
    assert_eq!(misses(&shown_pictures(), &expected()), vec![]);
}

#[test]
fn given_mbright_rom_when_run_then_the_pictures_differ_from_each_other() {
    // A guard against a scene that went flat: the brightness changes the first picture, and the
    // switch to display mode 1 changes the upper screen.
    let (shown, plain) = (shown_pictures(), expected()[0].clone());
    assert_eq!(
        (
            shown[1] != plain,
            shown[6].top != shown[7].top,
            shown[9] != shown[8]
        ),
        (true, true, true)
    );
}

#[test]
fn given_mbright_rom_when_run_for_14_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these pictures are the right ones: the hash pins
    // the last of them.
    assert_frame_hash("mbright", FRAMES);
}

#[test]
fn given_mbright_rom_when_run_twice_for_14_frames_then_every_frame_hash_matches() {
    assert_deterministic("mbright", FRAMES);
}

#[test]
fn given_mbright_rom_when_saved_after_7_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    // Frame 7 is in the middle of the steps, so MASTER_BRIGHT is in the savestate.
    assert_savestate_round_trip("mbright", 7, 7);
}
