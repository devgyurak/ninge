// SPDX-License-Identifier: Apache-2.0
//! The large bitmap of BG mode 6, observed through our own `bglarge` test ROM (`testroms/bglarge/`).
//! The ROM fills all 512 KiB of engine A's BG VRAM with a 1024x512 256-colour bitmap (layout at the
//! top of `arm9.s`) and shows it rotated and scaled, without wrap, on the upper screen; engine B
//! keeps its power-on display mode 0, so the lower screen is white.
//!
//! The expected picture is worked out here, pixel by pixel, from `docs/hw/bg-modes.md` ("Large
//! bitmap", "Affine layers", "Sizes and where the data is") and the data the ROM's source lists. It
//! is never read back from the emulator: the code below shares nothing with `ninge-core`.

use ninge_core::debug::{AffineFormat, EngineId};
use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// Frames run before the screens are inspected; the ROM finishes its setup inside the first three.
const FRAMES: u64 = 4;

/// A machine that booted the `bglarge` ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("bglarge")).expect("bglarge ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("bglarge ROM has a valid header");
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

/// Engine A's backdrop (palette entry 0 = 0x03E0), and the white of engine B's display mode 0.
const BACKDROP: u32 = 0x0000_FF00;
const WHITE: u32 = 0x00FF_FFFF;

/// The bitmap is 1024x512 (arm9.s, "Bitmap"); BG2 does not wrap.
const WIDTH: i64 = 1024;
const HEIGHT: i64 = 512;
/// PA, PB, PC, PD, X and Y as the ROM writes them (arm9.s), 8.8 and 20.8.
const PA: i64 = 0xA0;
const PB: i64 = 0x50;
const PC: i64 = -0x50;
const PD: i64 = 0xA0;
const X: i64 = -40 * 256;
const Y: i64 = 30 * 256;

// ------------------------------------------------------------------ the note's rules

/// Texel (`tx`, `ty`): `1 + (((tx >> 2) + 2 ty) & 127)`, the palette value.
fn texel_value(tx: i64, ty: i64) -> usize {
    (1 + (((tx >> 2) + 2 * ty) & 127)) as usize
}

/// The expected xRGB pixel of the upper screen: the bitmap, or the backdrop outside it.
fn expected_top(x: usize, y: usize) -> u32 {
    // The line starts where `y` steps of PB and PD have walked from the top.
    let u = X + PB * y as i64 + PA * x as i64;
    let v = Y + PD * y as i64 + PC * x as i64;
    let (tx, ty) = (u.div_euclid(256), v.div_euclid(256));
    match (0..WIDTH).contains(&tx) && (0..HEIGHT).contains(&ty) {
        true => expand(palette(texel_value(tx, ty))),
        false => BACKDROP,
    }
}

/// The pixels of the upper screen that differ from the note's picture: (x, y, expected, got).
fn mismatches(machine: &Machine) -> Vec<(usize, usize, u32, u32)> {
    let frame = machine.frame(Screen::Top);
    (0..SCREEN_HEIGHT)
        .flat_map(|y| (0..SCREEN_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| (x, y, expected_top(x, y), frame[y * SCREEN_WIDTH + x]))
        .filter(|&(_, _, want, got)| want != got)
        .take(8)
        .collect()
}

// ------------------------------------------------------------------ the tests

#[test]
fn given_bglarge_rom_when_4_frames_run_then_every_pixel_of_the_upper_screen_is_the_pictures_the_note_gives()
 {
    let machine = boot_frames();
    assert_eq!(mismatches(&machine), vec![]);
}

#[test]
fn given_bglarge_rom_when_4_frames_run_then_the_bitmap_and_its_transparent_edges_both_show() {
    // Guards the test above against a scene that went blank: the model has the backdrop at the
    // top left (negative texel x) and bitmap pixels elsewhere.
    let machine = boot_frames();
    let frame = machine.frame(Screen::Top);
    let backdrop = frame.iter().filter(|&&p| p == BACKDROP).count();
    assert_eq!(
        (
            backdrop > 1000,
            backdrop < frame.len() / 2,
            frame[0] == BACKDROP
        ),
        (true, true, true)
    );
}

#[test]
fn given_bglarge_rom_when_4_frames_run_then_the_lower_screen_is_white() {
    // Engine B is in display mode 0 (`display-mode-vram.md`).
    let machine = boot_frames();
    assert!(
        machine
            .frame(Screen::Bottom)
            .iter()
            .all(|&pixel| pixel == WHITE)
    );
}

#[test]
fn given_bglarge_rom_when_4_frames_run_then_the_rom_wrote_the_registers_it_set() {
    let machine = boot_frames();
    let layer = machine.inspect().gpu2d().affine_bg(EngineId::A, 2);
    assert_eq!(
        (
            machine.inspect().gpu2d().dispcnt_a(),
            machine.inspect().gpu2d().powcnt1(),
            layer.format,
            layer.priority,
            (layer.width, layer.height, layer.wrap),
            layer.params,
            layer.reference
        ),
        (
            0x0001_0406,
            0x8203,
            AffineFormat::Bitmap256,
            0,
            (1024, 512, false),
            [0xA0, 0x50, -0x50, 0xA0],
            [-40 * 256, 30 * 256]
        )
    );
}

#[test]
fn given_bglarge_rom_when_run_for_4_frames_then_both_screens_match_golden() {
    // The pixel test above is the evidence that this frame is the right one: the hash pins it.
    assert_frame_hash("bglarge", 4);
}

#[test]
fn given_bglarge_rom_when_run_twice_for_4_frames_then_every_frame_hash_matches() {
    assert_deterministic("bglarge", 4);
}

#[test]
fn given_bglarge_rom_when_saved_after_2_frames_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("bglarge", 2, 2);
}
