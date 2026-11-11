// SPDX-License-Identifier: Apache-2.0
//! The libnds console ROM (`testroms/console/`, the Phase 3 DoD of #28). The ROM puts a text
//! console on engine A's BG0 (screen block 31, character base 0, 4 bits per pixel, 256x256) and
//! prints three lines with `printf`. The console writes one map cell per character, whose tile
//! number is the character's ASCII code minus 32 (the console font starts at the space), so the
//! text can be read back from the map: that is the memory condition checked here, independent of
//! how the glyphs are drawn. `docs/hw/bg-text.md` ("Memory") gives the map address.
//!
//! The libnds font is not copied into this repository: no test holds its pixels. Whether the
//! glyphs look right is a visual check by the maintainer in a terminal, recorded in the PR.

use ninge_core::{Machine, SCREEN_WIDTH, Screen};
use ninge_runner::{load_rom_file, testrom_path};

/// Screen block 31 of engine A's BG window: `0600_0000h + 31 * 2 KiB`.
const MAP: u32 = 0x0600_F800;
/// Cells in a map row.
const ROW_CELLS: u32 = 32;
/// Frames run: libnds starts both CPUs, handshakes, then `main` prints.
const FRAMES: u64 = 10;

/// A machine that booted the console ROM and ran `FRAMES` frames.
fn boot_frames() -> Machine {
    let rom = load_rom_file(&testrom_path("console")).expect("console ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("console ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    machine
}

/// Map row `row` as text: each cell's tile number plus 32 is a character, trailing blanks cut.
fn row_text(machine: &Machine, row: u32) -> String {
    let text: String = (0..ROW_CELLS)
        .map(|cell| {
            let entry = machine.arm9_peek16(MAP + (row * ROW_CELLS + cell) * 2);
            char::from_u32(u32::from(entry & 0x3FF) + 32).unwrap_or('?')
        })
        .collect();
    text.trim_end().to_string()
}

#[test]
fn given_console_rom_when_120_frames_run_then_the_map_rows_spell_the_printed_lines() {
    let machine = boot_frames();
    let rows: Vec<String> = (0..4).map(|row| row_text(&machine, row)).collect();
    assert_eq!(
        rows,
        vec![
            "ninge console test",
            "sum: 20 + 22 = 42",
            "hex: 00C0FFEE",
            ""
        ]
    );
}

#[test]
fn given_console_rom_when_120_frames_run_then_the_rows_below_the_text_are_blank() {
    let machine = boot_frames();
    let rows: Vec<String> = (4..24).map(|row| row_text(&machine, row)).collect();
    assert_eq!(rows, vec![String::new(); 20]);
}

#[test]
fn given_console_rom_when_120_frames_run_then_the_upper_screen_is_drawn_and_the_lower_is_black() {
    // The text is drawn on engine A, which POWCNT1 puts on the upper screen at power-on; engine B
    // is a black placeholder in Phase 3.
    let machine = boot_frames();
    let top = machine.frame(Screen::Top);
    let first = top[0];
    assert_eq!(
        (
            top.iter().any(|&pixel| pixel != first),
            machine
                .frame(Screen::Bottom)
                .iter()
                .all(|&pixel| pixel == 0),
            top.len() == SCREEN_WIDTH * 192
        ),
        (true, true, true)
    );
}

#[test]
fn given_console_rom_when_run_twice_for_10_frames_then_both_screens_are_identical() {
    let (a, b) = (boot_frames(), boot_frames());
    assert_eq!(
        (
            a.frame(Screen::Top) == b.frame(Screen::Top),
            a.frame(Screen::Bottom) == b.frame(Screen::Bottom)
        ),
        (true, true)
    );
}
