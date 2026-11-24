// SPDX-License-Identifier: Apache-2.0
//! The keypad stub, observed through our own `keypad` test ROM (`testroms/keypad/`). Each CPU
//! records what it reads in main RAM (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; the slot layouts
//! are at the top of `arm9.s` and `arm7.s`). Expected values come from `docs/hw/keypad.md`
//! ("Registers", "Behavior", "Edge cases"), never from running the ROM: KEYINPUT reads 0x03FF
//! (ten key bits, all released), EXTKEYIN reads 0x007F, KEYCNT keeps bits 0-9, 14 and 15
//! (0xC3FF), and what the note leaves open is in `docs/hw/keypad.md` "Unverified".

use ninge_core::{Buttons, Input, Machine};
use ninge_runner::golden::{
    assert_deterministic_with_input, assert_savestate_round_trip_with_input,
};
use ninge_runner::log::InputLog;
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_RECORD: u32 = 0x0200_4000;
const ARM7_RECORD: u32 = 0x0200_5000;

/// Both CPUs finish in far less than one frame.
const FRAMES: u32 = 2;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// Expected record: (slot, name, expected value).
type Row = (u32, &'static str, u32);

fn boot_with(input: Input) -> Machine {
    let rom = load_rom_file(&testrom_path("keypad")).expect("keypad ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("keypad ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame(input));
    machine
}

fn mismatches(base: u32, rows: &[Row]) -> Vec<Mismatch> {
    mismatches_with(Input::default(), base, rows)
}

fn mismatches_with(input: Input, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    let machine = boot_with(input);
    rows.iter()
        .map(|&(slot, name, want)| {
            (
                slot,
                name,
                want,
                machine.inspect().arm9_peek32(base + slot * 4),
            )
        })
        .filter(|&(_, _, want, got)| want != got)
        .collect()
}

#[test]
fn given_keypad_rom_when_run_then_the_arm9_finishes_and_writes_its_sentinel() {
    let rows: &[Row] = &[(30, "ARM9 progress", 1), (31, "ARM9 sentinel", 0x4B45_5939)];
    assert_eq!(mismatches(ARM9_RECORD, rows), Vec::<Mismatch>::new());
}

#[test]
fn given_keypad_rom_when_run_then_the_arm7_finishes_and_writes_its_sentinel() {
    let rows: &[Row] = &[(30, "ARM7 progress", 1), (31, "ARM7 sentinel", 0x4B45_5937)];
    assert_eq!(mismatches(ARM7_RECORD, rows), Vec::<Mismatch>::new());
}

#[test]
fn given_nothing_pressed_when_the_arm9_reads_keyinput_then_all_ten_key_bits_are_set() {
    // `keypad.md`, "KEYINPUT bits": 0 = pressed, so nothing pressed is 0x3FF.
    let rows: &[Row] = &[
        (0, "word read, KEYCNT still 0 above it", 0x0000_03FF),
        (1, "halfword read", 0x03FF),
    ];
    assert_eq!(mismatches(ARM9_RECORD, rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_write_when_the_arm9_reads_back_then_keycnt_keeps_bits_0_to_9_14_and_15_and_keyinput_is_unchanged()
 {
    // `keypad.md`, "KEYCNT bits" and "Edge cases": KEYINPUT is read-only.
    let rows: &[Row] = &[
        (2, "KEYCNT after writing FFFFh", 0xC3FF),
        (3, "KEYINPUT after writing 0", 0x03FF),
    ];
    assert_eq!(mismatches(ARM9_RECORD, rows), Vec::<Mismatch>::new());
}

#[test]
fn given_the_arm9_when_it_reads_the_extkeyin_address_then_nothing_is_there() {
    // `keypad.md`, "Edge cases": EXTKEYIN is ARM7 only; RCNT (0x0400_0134) is not modeled.
    let rows: &[Row] = &[
        (4, "word at 0x04000134", 0),
        (5, "halfword at 0x04000136 after a write", 0),
    ];
    assert_eq!(mismatches(ARM9_RECORD, rows), Vec::<Mismatch>::new());
}

#[test]
fn given_nothing_pressed_when_the_arm7_reads_keyinput_and_extkeyin_then_keys_are_released_and_the_hinge_is_open()
 {
    // `keypad.md`, "EXTKEYIN bits": X, Y, debug and pen released, bits 2, 4, 5 set, hinge 0.
    let rows: &[Row] = &[
        (0, "KEYINPUT", 0x03FF),
        (1, "EXTKEYIN", 0x007F),
        (2, "word at 0x04000130", 0x0000_03FF),
    ];
    assert_eq!(mismatches(ARM7_RECORD, rows), Vec::<Mismatch>::new());
}

#[test]
fn given_the_arm7_when_it_writes_keycnt_and_extkeyin_then_keycnt_is_kept_and_extkeyin_is_read_only()
{
    // `keypad.md`, "KEYCNT bits", "Edge cases"; a word at 0x04000134 spans RCNT (0) and EXTKEYIN.
    let rows: &[Row] = &[
        (3, "KEYCNT after writing FFFFh", 0xC3FF),
        (4, "word at 0x04000134", 0x007F_0000),
        (5, "EXTKEYIN after writing 0", 0x007F),
    ];
    assert_eq!(mismatches(ARM7_RECORD, rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_and_down_held_when_both_cpus_read_keyinput_then_bits_0_and_7_read_zero() {
    // `keypad.md`, "Where the bits come from": a held key reads 0; A is bit 0, Down bit 7.
    let input = Input::new().with_buttons(Buttons::A | Buttons::DOWN);
    let rows9: &[Row] = &[(1, "ARM9 halfword read", 0x037E)];
    let rows7: &[Row] = &[(0, "ARM7 KEYINPUT", 0x037E)];
    assert_eq!(
        (
            mismatches_with(input, ARM9_RECORD, rows9),
            mismatches_with(input, ARM7_RECORD, rows7)
        ),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

#[test]
fn given_x_held_and_the_hinge_closed_when_the_arm7_reads_extkeyin_then_bit_0_is_clear_and_bit_7_set()
 {
    // `keypad.md`, "EXTKEYIN bits": 0x007F with X held is 0x007E, and a closed hinge sets bit 7.
    let input = Input::new()
        .with_buttons(Buttons::X)
        .with_hinge_closed(true);
    let rows: &[Row] = &[(1, "EXTKEYIN", 0x00FE)];
    assert_eq!(
        mismatches_with(input, ARM7_RECORD, rows),
        Vec::<Mismatch>::new()
    );
}

/// One input for each of four frames: a log that changes under the ROM.
fn input_log() -> InputLog {
    InputLog::new(
        [0x0001, 0x0102, 0x0804, 0x0000]
            .map(|bits| {
                Input::new()
                    .with_buttons(Buttons::from_bits_truncate(bits))
                    .with_hinge_closed(bits & 1 == 1)
            })
            .to_vec(),
    )
}

#[test]
fn given_the_keypad_rom_and_an_input_log_when_run_twice_then_frames_and_savestates_match() {
    // Panics with the first difference if the two runs differ.
    assert_deterministic_with_input("keypad", 4, &input_log());
}

#[test]
fn given_the_keypad_rom_and_an_input_log_when_saved_loaded_and_continued_then_it_matches_a_straight_run()
 {
    // Panics with the first difference if the resumed run differs.
    assert_savestate_round_trip_with_input("keypad", 2, 2, &input_log());
}
