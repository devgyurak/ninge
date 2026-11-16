// SPDX-License-Identifier: Apache-2.0
//! The main-memory display, observed through our own `mmdisp` test ROM (`testroms/mmdisp/`): a DMA
//! channel in start mode 4 streams a bitmap from main RAM into DISP_MMEM_FIFO and engine A shows it
//! in display mode 3; the capture unit stores the FIFO's line into VRAM bank B (source B = FIFO) and
//! display mode 2 shows that bank back, with and without a master brightness; in the last two steps
//! the channel is not armed again, so the FIFO gets nothing and mode 3 shows black.
//!
//! The expected pictures are worked out here, line by line, from `docs/hw/display-capture.md` and
//! the data the ROM's source lists. They are never read back from the emulator: the code below
//! shares nothing with `ninge-core`.
//!
//! The pictures depend on the choices the note lists under "Unverified" (a FIFO pixel's alpha is 1,
//! the FIFO is one line deep and an underrun reads black): the golden pins those choices, and
//! changes with them. The start-of-display DMA mode has no picture here; its once-per-frame start
//! is pinned in `ninge-core`'s machine tests.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run: some white frames of setup (the bitmap is computed first), then the first picture
/// and six steps.
const FRAMES: u64 = 24;

/// Both screens after each frame, in the order the frames were made.
fn pictures() -> Vec<Picture> {
    let rom = load_rom_file(&testrom_path("mmdisp")).expect("mmdisp ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("mmdisp ROM has a valid header");
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

// ------------------------------------------------------------------ the data of the ROM

const WHITE: u16 = 0x7FFF;
const UP: u16 = 1 << 14;
const DOWN: u16 = 2 << 14;

/// One step: engine A's display mode (3 shows the FIFO, 2 shows bank B), whether the frame
/// captures the FIFO into bank B, MASTER_BRIGHT, and whether the channel is armed again before the
/// frame (arm9.s, "Steps"). The ROM starts with the first picture: display mode 3, armed.
#[derive(Clone, Copy)]
struct Step {
    mode: u32,
    capture: bool,
    bright: u16,
    armed: bool,
}

const FIRST: Step = Step {
    mode: 3,
    capture: false,
    bright: 0,
    armed: true,
};
const STEPS: [Step; 6] = [
    Step {
        mode: 3,
        capture: true,
        bright: 0,
        armed: true,
    },
    Step {
        mode: 2,
        capture: false,
        bright: 0,
        armed: true,
    },
    Step {
        mode: 3,
        capture: false,
        bright: DOWN | 8,
        armed: true,
    },
    Step {
        mode: 2,
        capture: false,
        bright: UP | 8,
        armed: true,
    },
    Step {
        mode: 3,
        capture: false,
        bright: 0,
        armed: false,
    },
    Step {
        mode: 2,
        capture: false,
        bright: 0,
        armed: false,
    },
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

/// The bitmap in main RAM at (x, y) (arm9.s, header).
fn bitmap(x: usize, y: usize) -> u16 {
    let (r, g) = ((x >> 3) as u16, (y >> 3) as u16);
    r | g << 5 | (r ^ g) << 10
}

/// VRAM bank B as halfwords, and its pixel count.
const BANK_HALFWORDS: usize = 0x1_0000;

/// The upper screen of one frame, drawn line by line. The FIFO delivers the bitmap's line when the
/// channel was armed for the frame and a black line otherwise; display mode 3 shows that line and
/// display mode 2 shows bank B as it is; the capture stores the FIFO's line (alpha bit set) after
/// the line is shown.
fn frame(step: &Step, bank: &mut [u16]) -> Vec<u32> {
    (0..SCREEN_HEIGHT)
        .flat_map(|y| {
            let fifo = |x: usize| if step.armed { bitmap(x, y) } else { 0 };
            let shown: Vec<u16> = (0..SCREEN_WIDTH)
                .map(|x| match step.mode {
                    3 => fifo(x),
                    _ => bank[y * SCREEN_WIDTH + x] & 0x7FFF,
                })
                .collect();
            if step.capture {
                (0..SCREEN_WIDTH).for_each(|x| bank[y * SCREEN_WIDTH + x] = fifo(x) | 0x8000);
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
fn given_mmdisp_rom_when_run_then_it_shows_the_first_picture_and_six_steps() {
    assert_eq!(shown_pictures().len(), expected().len());
}

#[test]
fn given_mmdisp_rom_when_run_then_each_picture_matches_the_note() {
    assert_eq!(misses(&shown_pictures(), &expected()), vec![]);
}

#[test]
fn given_mmdisp_rom_when_run_then_the_pictures_follow_the_fifo_the_capture_and_the_brightness() {
    // A guard against a scene that went flat: the bank shown in mode 2 is the bitmap the FIFO
    // delivered, the brightness changes it, and the screen goes black once the source runs past the bitmap (the FIFO is fed zeros,
    // not left to run empty) while the bank stays.
    let pictures = expected();
    let black = pictures[5].top.iter().all(|&p| p == 0);
    assert_eq!(
        (
            pictures[0].top == pictures[2].top,
            pictures[3].top != pictures[2].top,
            pictures[4].top != pictures[2].top,
            black,
            pictures[6].top == pictures[2].top
        ),
        (true, true, true, true, true)
    );
}

#[test]
fn given_mmdisp_rom_when_run_for_24_frames_then_both_screens_match_golden() {
    // The pixel tests above are the evidence that these pictures are the right ones: the hash pins
    // the last of them.
    assert_frame_hash("mmdisp", FRAMES);
}

#[test]
fn given_mmdisp_rom_when_run_twice_for_24_frames_then_every_frame_hash_matches() {
    assert_deterministic("mmdisp", FRAMES);
}

#[test]
fn given_mmdisp_rom_when_saved_after_14_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    // Frame 14 is in the middle of the steps, so the FIFO, DISPCAPCNT and the DMA channel are in the
    // savestate.
    assert_savestate_round_trip("mmdisp", 14, 8);
}
