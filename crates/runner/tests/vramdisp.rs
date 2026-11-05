// SPDX-License-Identifier: Apache-2.0
//! Engine A's VRAM display, observed through our own `vramdisp` test ROM
//! (`testroms/vramdisp/`). The ROM sets POWCNT1 = 0x0003 (display swap clear), enables bank A
//! as LCDC, selects display mode 2 with block 0, and writes a computed picture (layout at the
//! top of `arm9.s`). Expected pixels come from `docs/hw/display-mode-vram.md`: the 15-bit
//! colour of a bar or ramp step, widened by `(c << 3) | (c >> 2)` per channel, never from
//! running the emulator.
//!
//! With the swap bit clear, engine A is on the BOTTOM screen; the top screen is engine B's
//! black placeholder. The golden pins both (`tests/golden/vramdisp.txt`).

use ninge_core::{Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

/// A mismatch: (x, y, expected, got).
type Mismatch = (usize, usize, u32, u32);

/// A machine that booted the `vramdisp` ROM and ran one frame.
fn boot_one_frame() -> Machine {
    let rom = load_rom_file(&testrom_path("vramdisp")).expect("vramdisp ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("vramdisp ROM has a valid header");
    machine.run_frame();
    machine
}

/// Representative pixels: (x, y, expected xRGB8888). Bars are 32 pixels wide; a ramp step is
/// 8 pixels wide and its 5-bit channel `k` widens to 8 bits as `(k << 3) | (k >> 2)`.
const PIXELS: &[(usize, usize, u32)] = &[
    // Bars, row 0: white, red, green, blue, cyan, magenta, yellow, black.
    (0, 0, 0x00FF_FFFF),
    (32, 0, 0x00FF_0000),
    (64, 0, 0x0000_FF00),
    (96, 0, 0x0000_00FF),
    (128, 0, 0x0000_FFFF),
    (160, 0, 0x00FF_00FF),
    (192, 0, 0x00FF_FF00),
    (224, 0, 0x0000_0000),
    // Odd rows also set pixel bit 15, which is not part of the colour.
    (0, 1, 0x00FF_FFFF),
    (32, 1, 0x00FF_0000),
    (224, 1, 0x0000_0000),
    // Last pixel of the first bar, and the last pixel of the bar rows (red bar).
    (31, 0, 0x00FF_FFFF),
    (63, 63, 0x00FF_0000),
    // Red ramp (rows 64-95): channel 0, 1, 16, 31.
    (0, 64, 0x0000_0000),
    (8, 64, 0x0008_0000),
    (128, 64, 0x0084_0000),
    (255, 64, 0x00FF_0000),
    // Green ramp (rows 96-127).
    (128, 96, 0x0000_8400),
    (255, 127, 0x0000_FF00),
    // Blue ramp (rows 128-159).
    (128, 128, 0x0000_0084),
    (255, 128, 0x0000_00FF),
    // Grey ramp (rows 160-191).
    (0, 191, 0x0000_0000),
    (128, 160, 0x0084_8484),
    (255, 191, 0x00FF_FFFF),
];

/// Pixels of `screen` that differ from the table.
fn pixel_failures(machine: &Machine, screen: Screen) -> Vec<Mismatch> {
    let frame = machine.frame(screen);
    PIXELS
        .iter()
        .map(|&(x, y, expected)| (x, y, expected, frame[y * SCREEN_WIDTH + x]))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

#[test]
fn given_vramdisp_rom_when_one_frame_runs_then_the_bottom_screen_shows_the_expected_pixels() {
    let machine = boot_one_frame();
    assert_eq!(
        pixel_failures(&machine, Screen::Bottom),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_vramdisp_rom_with_the_swap_bit_clear_when_one_frame_runs_then_the_top_screen_is_black() {
    let machine = boot_one_frame();
    assert!(machine.frame(Screen::Top).iter().all(|&pixel| pixel == 0));
}

#[test]
fn given_vramdisp_rom_when_one_frame_runs_then_the_rom_wrote_the_registers_it_set() {
    let machine = boot_one_frame();
    assert_eq!(
        (
            machine.gpu2d().powcnt1(),
            machine.gpu2d().dispcnt_a(),
            machine.vram_a().mst,
            machine.vram_a().enabled
        ),
        (0x0003, 0x0002_0000, 0, true)
    );
}

#[test]
fn given_vramdisp_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    assert_frame_hash("vramdisp", 3);
}

#[test]
fn given_vramdisp_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("vramdisp", 3);
}

#[test]
fn given_vramdisp_rom_when_saved_after_1_frame_and_resumed_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("vramdisp", 1, 2);
}
