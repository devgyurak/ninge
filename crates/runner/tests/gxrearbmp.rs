// SPDX-License-Identifier: Apache-2.0
//! The rear bitmaps (DISP3DCNT bit 14), observed through our own `gxrearbmp` test ROM
//! (`testroms/gxrearbmp/`): colour and alpha from texture slot 2, depth from slot 3, the
//! CLRIMAGE_OFFSET scroll with its wrap on both axes, and a polygon drawn over the bitmap's depth.
//! The expected picture is worked out here from `docs/hw/textures.md` ("The rear bitmap"), pixel by
//! pixel, from the formulas the ROM's setup code fills the bitmaps with. It is never read back from
//! the emulator: nothing below shares code with `ninge-core`.
//!
//! The ROM scrolls by X 40 and Y 100, so screen pixel `(x, y)` shows bitmap entry
//! `((x + 40) mod 256, (y + 100) mod 256)`. Entry `(bx, by)` has red `bx / 8`, green `by / 8`, and
//! alpha set where bit 5 of `bx` and of `by` differ; its depth is far for `bx` below 128 and
//! nearest from 128 on. A transparent rear pixel shows the backdrop (palette entry 0: black).

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// The swap waits for VBlank and the picture is drawn at line 214, so it shows from the next frame
/// on; the third frame is a whole one.
const FRAMES: u64 = 4;

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

fn top_after(rom: &str, frames: u64) -> Vec<u32> {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path(rom)).expect("the ROM is built"))
        .expect("the ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine.frame(Screen::Top).to_vec()
}

/// The first few pixels where the picture differs from the expected one: `(x, y, got, wanted)`.
fn differences(got: &[u32], wanted: &[u16]) -> Vec<(usize, usize, u32, u32)> {
    got.iter()
        .zip(wanted)
        .enumerate()
        .filter(|(_, (g, w))| **g != expand(**w))
        .map(|(at, (g, w))| (at % SCREEN_WIDTH, at / SCREEN_WIDTH, *g, expand(*w)))
        .take(8)
        .collect()
}

/// The upper screen the ROM shows, as a grid of 15-bit colours.
fn expected() -> Vec<u16> {
    (0..SCREEN_WIDTH * SCREEN_HEIGHT)
        .map(|i| {
            let (x, y) = (i % SCREEN_WIDTH, i / SCREEN_WIDTH);
            let (bx, by) = ((x + 40) % 256, (y + 100) % 256);
            let solid = ((bx >> 5) ^ (by >> 5)) & 1 == 1;
            let rear = [0, ((bx >> 3) | (by >> 3) << 5) as u16][usize::from(solid)];
            // The white quad covers columns 64..192 of lines 48..144 (it drops its lower and
            // right edges) and draws where the rear depth is far: bitmap columns below 128.
            let in_quad = (64..192).contains(&x) && (48..144).contains(&y);
            [rear, 0x7FFF][usize::from(in_quad && bx < 128)]
        })
        .collect()
}

#[test]
fn given_the_gxrearbmp_rom_when_4_frames_run_then_the_upper_screen_shows_the_worked_out_picture() {
    assert_eq!(
        differences(&top_after("gxrearbmp", FRAMES), &expected()),
        vec![]
    );
}

#[test]
fn given_the_gxrearbmp_rom_when_4_frames_run_then_the_hashes_match_the_golden_file() {
    assert_frame_hash("gxrearbmp", FRAMES);
}

#[test]
fn given_the_gxrearbmp_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxrearbmp", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxrearbmp", 2, 2);
}
