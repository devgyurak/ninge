// SPDX-License-Identifier: Apache-2.0
//! Capture blended with the previous capture, observed through our own `capblend` test ROM
//! (`testroms/capblend/`): engine A composes a scrolling text layer, blends it into VRAM bank B with
//! what bank B already holds (capture source A+B, the "trail" use of the note) and shows bank B in
//! display mode 2. Over eight steps the layer moves, the factors change (12/4, 16/16 which
//! saturates, 0/16 which keeps only the previous capture) and the capture stops.
//!
//! The expected pictures are worked out here, line by line, from `docs/hw/display-capture.md` and
//! the data the ROM's source lists: a line shows what the bank holds, then the capture blends the
//! composed line with that same bank content (read first) and stores the result. They are never
//! read back from the emulator: the code below shares nothing with `ninge-core`.
//!
//! The pictures depend on the choices the note lists under "Unverified" (a 2D pixel's alpha is 1,
//! a factor above 16 counts as 16, the sum saturates at 31): the golden pins those choices, and
//! changes with them.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run: some white frames of setup, then the first picture and eight steps.
const FRAMES: u64 = 14;

/// Both screens after each frame, in the order the frames were made.
fn pictures() -> Vec<Picture> {
    let rom = load_rom_file(&testrom_path("capblend")).expect("capblend ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("capblend ROM has a valid header");
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

/// One step: BG0HOFS and the factors of the capture, `None` when it does not capture
/// (arm9.s, "Steps"). The ROM starts with the first picture: no scroll, no capture.
type Step = (u32, Option<(u32, u32)>);
const FIRST: Step = (0, None);
const STEPS: [Step; 8] = [
    (0, Some((12, 4))),
    (16, Some((12, 4))),
    (32, Some((12, 4))),
    (48, Some((12, 4))),
    (64, Some((12, 4))),
    (80, Some((16, 16))),
    (96, Some((0, 16))),
    (96, None),
];

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// The graphics screen at (x, y) with the layer scrolled by `hofs` (arm9.s, header): rows 0-127 are
/// red in layer columns 0-127 and green in 128-255, and rows 128-191 are the blue backdrop.
fn layers(x: usize, y: usize, hofs: u32) -> u16 {
    match (y < 128, (x + hofs as usize) % 256 < 128) {
        (true, true) => RED,
        (true, false) => GREEN,
        (false, _) => BLUE,
    }
}

/// One channel of the blend of A and B (`display-capture.md`, "The capture pipeline"): the alphas
/// are 0 or 1, the sum is in sixteenths, truncated and saturating at 31.
fn channel(a: u32, alpha_a: u32, eva: u32, b: u32, alpha_b: u32, evb: u32) -> u16 {
    ((a * alpha_a * eva + b * alpha_b * evb) / 16).min(31) as u16
}

/// The halfword the capture stores for source A `a` and source B `b`, both with their alpha in bit 15.
fn blended(a: u16, b: u16, eva: u32, evb: u32) -> u16 {
    let (alpha_a, alpha_b) = (u32::from(a >> 15), u32::from(b >> 15));
    let at = |shift: u16| {
        channel(
            u32::from((a >> shift) & 31),
            alpha_a,
            eva,
            u32::from((b >> shift) & 31),
            alpha_b,
            evb,
        ) << shift
    };
    let alpha = u16::from((alpha_a == 1 && eva > 0) || (alpha_b == 1 && evb > 0));
    at(0) | at(5) | at(10) | alpha << 15
}

/// VRAM bank B as halfwords.
const BANK_HALFWORDS: usize = 0x1_0000;

/// The upper screen of one frame, drawn line by line: a line shows the bank as it is, then the
/// capture blends the composed line with the bank's pixel and stores it there.
fn frame(step: &Step, bank: &mut [u16]) -> Vec<u32> {
    let (hofs, capture) = *step;
    (0..SCREEN_HEIGHT)
        .flat_map(|y| {
            let shown: Vec<u32> = (0..SCREEN_WIDTH)
                .map(|x| expand(bank[y * SCREEN_WIDTH + x] & 0x7FFF))
                .collect();
            if let Some((eva, evb)) = capture {
                (0..SCREEN_WIDTH).for_each(|x| {
                    let at = y * SCREEN_WIDTH + x;
                    bank[at] = blended(layers(x, y, hofs) | 0x8000, bank[at], eva, evb);
                });
            }
            shown
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
fn given_capblend_rom_when_run_then_it_shows_the_first_picture_and_eight_steps() {
    assert_eq!(shown_pictures().len(), expected().len());
}

#[test]
fn given_capblend_rom_when_run_then_each_picture_matches_the_note() {
    assert_eq!(misses(&shown_pictures(), &expected()), vec![]);
}

#[test]
fn given_capblend_rom_when_run_then_the_trail_grows_until_the_factors_keep_only_the_old_capture() {
    // A guard against a scene that went flat. A picture shows the bank as the captures before it
    // left it: the first capture shows only in the second picture, each blending step after it
    // changes the picture, and the step with EVA 0 and EVB 16 (the one before the last picture)
    // stores the old capture again, so the last picture repeats the one before it.
    let pictures = expected();
    assert_eq!(
        (
            pictures[1] == pictures[0],
            pictures[2] != pictures[1],
            pictures[5] != pictures[4],
            pictures[7] != pictures[6],
            pictures[8] == pictures[7]
        ),
        (true, true, true, true, true)
    );
}

#[test]
fn given_capblend_rom_when_run_for_14_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these pictures are the right ones: the hash pins
    // the last of them.
    assert_frame_hash("capblend", FRAMES);
}

#[test]
fn given_capblend_rom_when_run_twice_for_14_frames_then_every_frame_hash_matches() {
    assert_deterministic("capblend", FRAMES);
}

#[test]
fn given_capblend_rom_when_saved_after_8_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    // Frame 8 is in the middle of the steps, so DISPCAPCNT and the bank are in the savestate.
    assert_savestate_round_trip("capblend", 8, 6);
}
