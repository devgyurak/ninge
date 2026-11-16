// SPDX-License-Identifier: Apache-2.0
//! Display capture of engine A's graphics screen, observed through our own `capture` test ROM
//! (`testroms/capture/`). Engine A composes a text layer over a backdrop, captures it into VRAM
//! bank B in several sizes and offsets, and shows the bank back in display mode 2, while the layer's
//! colour changes and a master brightness is applied.
//!
//! The expected pictures are worked out here, line by line, from `docs/hw/display-capture.md` and
//! the data the ROM's source lists: the model keeps the bank's halfwords, reads what a line shows,
//! then captures the line, as the note's pipeline says. They are never read back from the emulator:
//! the code below shares nothing with `ninge-core`.
//!
//! The pictures depend on the choices the note lists under "Unverified" (the row stride of the
//! 128-wide size is 128 pixels, a 2D pixel's alpha is 1): the golden pins those choices, and
//! changes with them.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run: some white frames of setup, then the first picture and eight steps.
const FRAMES: u64 = 14;

/// Both screens after each frame, in the order the frames were made.
fn pictures() -> Vec<Picture> {
    let rom = load_rom_file(&testrom_path("capture")).expect("capture ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("capture ROM has a valid header");
    (0..FRAMES)
        .map(|_| {
            machine.run_frame();
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

// ------------------------------------------------------------------ the data of the ROM

const WHITE: u16 = 0x7FFF;
const BLUE: u16 = 0x7C00;
const RED: u16 = 0x001F;
const GREEN: u16 = 0x03E0;
const YELLOW: u16 = 0x03FF;

/// The ROM's steps (arm9.s, "Steps"): engine A's DISPCNT, DISPCAPCNT, MASTER_BRIGHT and palette
/// entry 1. The first row is the picture the ROM starts with.
const FIRST: Step = Step {
    dispcnt: 0x0001_0100,
    dispcapcnt: 0,
    bright: 0,
    pal1: RED,
};
const STEPS: [Step; 8] = [
    step(0x0001_0100, 0x8031_0000, 0x0000, RED),
    step(0x0006_0100, 0x8015_0000, 0x0000, YELLOW),
    step(0x0006_0100, 0, 0x0000, YELLOW),
    step(0x0006_0100, 0x8009_0000, 0x0000, YELLOW),
    step(0x0001_0100, 0x8031_0000, 0x4008, GREEN),
    step(0x0006_0100, 0, 0x0000, GREEN),
    step(0x0001_0100, 0x803D_0000, 0x0000, GREEN),
    step(0x0006_0100, 0, 0x0000, GREEN),
];

#[derive(Clone, Copy)]
struct Step {
    dispcnt: u32,
    dispcapcnt: u32,
    bright: u16,
    pal1: u16,
}

const fn step(dispcnt: u32, dispcapcnt: u32, bright: u16, pal1: u16) -> Step {
    Step {
        dispcnt,
        dispcapcnt,
        bright,
        pal1,
    }
}

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

/// The graphics screen at (x, y), given palette entry 1 (arm9.s, header): rows 0-127 are entry 1 in
/// columns 0-127 and green in 128-255; rows 128-191 are the blue backdrop.
fn layers(x: usize, y: usize, pal1: u16) -> u16 {
    match (y < 128, x < 128) {
        (true, true) => pal1,
        (true, false) => GREEN,
        (false, _) => BLUE,
    }
}

/// VRAM bank B as halfwords.
const BANK_HALFWORDS: usize = 0x1_0000;

/// A capture's size in pixels from DISPCAPCNT bits 20-21 (`display-capture.md`, "Registers").
fn capture_size(dispcapcnt: u32) -> (usize, usize) {
    match (dispcapcnt >> 20) & 3 {
        0 => (128, 128),
        1 => (256, 64),
        2 => (256, 128),
        _ => (256, 192),
    }
}

/// The upper screen of one frame, drawn line by line: a line shows what display mode 1 composes or
/// what display mode 2 reads from bank B, and then the capture, if enabled, stores the composed line
/// (alpha bit set) at its offset with the row stride of the capture width, wrapping at 128 KiB.
fn frame(step: &Step, bank: &mut [u16]) -> Vec<u32> {
    let (width, height) = capture_size(step.dispcapcnt);
    let offset = ((step.dispcapcnt >> 18) & 3) as usize * 0x4000;
    let capturing = step.dispcapcnt >> 31 == 1;
    (0..SCREEN_HEIGHT)
        .flat_map(|y| {
            let shown: Vec<u16> = (0..SCREEN_WIDTH)
                .map(|x| match (step.dispcnt >> 16) & 3 {
                    1 => layers(x, y, step.pal1),
                    _ => bank[y * SCREEN_WIDTH + x] & 0x7FFF,
                })
                .collect();
            if capturing && y < height {
                (0..width).for_each(|x| {
                    bank[(offset + y * width + x) % BANK_HALFWORDS] =
                        layers(x, y, step.pal1) | 0x8000;
                });
            }
            shown
                .into_iter()
                .map(|color| brightened(step.bright, color))
        })
        .collect()
}

/// The pictures the ROM goes through: the first one, then each step.
fn expected() -> Vec<Picture> {
    let mut bank = vec![0u16; BANK_HALFWORDS];
    let bottom = vec![expand(WHITE); SCREEN_WIDTH * SCREEN_HEIGHT];
    std::iter::once(&FIRST)
        .chain(STEPS.iter())
        .map(|step| Picture {
            top: frame(step, &mut bank),
            bottom: bottom.clone(),
        })
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

/// The pictures the ROM shows, without the white ones before DISPCNT is set.
fn shown_pictures() -> Vec<Picture> {
    let white = expand(WHITE);
    pictures()
        .into_iter()
        .filter(|p| !(p.top.iter().chain(&p.bottom)).all(|&px| px == white))
        .take(STEPS.len() + 1)
        .collect()
}

// ------------------------------------------------------------------------------ the tests

#[test]
fn given_capture_rom_when_run_then_it_shows_the_first_picture_and_eight_steps() {
    assert_eq!(shown_pictures().len(), expected().len());
}

#[test]
fn given_capture_rom_when_run_then_each_picture_matches_the_note() {
    assert_eq!(misses(&shown_pictures(), &expected()), vec![]);
}

#[test]
fn given_capture_rom_when_run_then_the_captured_pictures_differ_from_the_live_ones() {
    // A guard against a scene that went flat: the second step shows the first capture, not the
    // live layers (palette entry 1 changed), and the last picture is neither of them.
    let shown = expected();
    assert_eq!(
        (
            shown[2].top != shown[1].top,
            shown[8].top != shown[6].top,
            shown[4].top != shown[3].top
        ),
        (true, true, true)
    );
}

#[test]
fn given_capture_rom_when_run_for_14_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these pictures are the right ones: the hash pins
    // the last of them.
    assert_frame_hash("capture", FRAMES);
}

#[test]
fn given_capture_rom_when_run_twice_for_14_frames_then_every_frame_hash_matches() {
    assert_deterministic("capture", FRAMES);
}

#[test]
fn given_capture_rom_when_saved_after_8_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    // Frame 8 is in the middle of the steps, so DISPCAPCNT and the bank are in the savestate.
    assert_savestate_round_trip("capture", 8, 6);
}
