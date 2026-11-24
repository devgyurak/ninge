// SPDX-License-Identifier: Apache-2.0
//! The keypad and hinge interrupts, observed through our own `keyirq` test ROM
//! (`testroms/keyirq/`). The runner holds one input per frame (below); the ARM9, with its own
//! KEYCNT in OR mode over A and B and its IRQ taken in the ROM, counts the requests, and the ARM7,
//! with its own KEYCNT in AND mode and IE clear, reads IF. Both record in main RAM (slot layouts at
//! the top of `arm9.s` and `arm7.s`) and the ARM9 shows one band for each of six checks on the
//! bottom screen, green for a pass.
//!
//! Expected values come from `docs/hw/keypad.md` ("The keypad IRQ", "The hinge IRQ"), never from
//! running the ROM:
//! - OR: a selected key held at a frame's latch requests IF bit 12 once for that frame, so A (frame
//!   one) and A with B (frame two) make two; nothing held adds none (frame 3); B alone adds one
//!   (frame 4), and Start, which KEYCNT does not select, adds none (frame 5).
//! - AND: A alone gives no request (frame 1), A with B does (frame 2), and after an acknowledge B
//!   alone gives none again (frame 4).
//! - The hinge: closing it (frame 6) requests nothing, opening it again (frame 7) sets the ARM7's
//!   IF bit 22 although its IE is clear.
//!
//! The sampling points and the closing edge are the model's choices (the note's "Unverified"); the
//! IRQ-enable-clear and empty-selection rules are pinned by the keypad unit tests, not by this ROM:
//! a green band says the model follows the note, not that hardware was measured.

use ninge_core::{Buttons, Input, Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{
    assert_deterministic_with_input, assert_frame_hash_with_input,
    assert_savestate_round_trip_with_input,
};
use ninge_runner::log::InputLog;
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 10;
const GREEN: u32 = 0x0000_FF00;
/// The record slots the ROM writes: the ARM9's 0-4, the ARM7's 8-13, the progress and the sentinel.
const SLOTS: [u32; 13] = [0, 1, 2, 3, 4, 8, 9, 10, 11, 12, 13, 30, 31];

/// Frame 0 nothing (the ROM sets up), then A, A and B, nothing, B, Start, hinge closed, hinge open
/// again, and nothing.
fn log() -> InputLog {
    let held = |buttons| Input::new().with_buttons(buttons);
    InputLog::new(vec![
        Input::new(),
        held(Buttons::A),
        held(Buttons::A | Buttons::B),
        Input::new(),
        held(Buttons::B),
        held(Buttons::START),
        Input::new().with_hinge_closed(true),
        Input::new(),
        Input::new(),
        Input::new(),
    ])
}

fn rom_image() -> Vec<u8> {
    load_rom_file(&testrom_path("keyirq")).expect("keyirq ROM is built")
}

/// A booted machine that has run frames `from..to` of `log`.
fn boot_range(log: &InputLog, from: u64, to: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(rom_image())
        .expect("keyirq ROM has a valid header");
    (from..to).for_each(|frame| machine.run_frame(log.input_for(frame)));
    machine
}

fn boot() -> Machine {
    boot_range(&log(), 0, FRAMES)
}

fn record(machine: &Machine) -> Vec<(u32, u32)> {
    SLOTS
        .iter()
        .map(|&slot| (slot, machine.inspect().arm9_peek32(REC + 4 * slot)))
        .collect()
}

/// The colour of each band's middle pixel on the bottom screen, top band first.
fn bands(machine: &Machine) -> Vec<u32> {
    let frame = machine.frame(Screen::Bottom);
    [16, 48, 80, 112, 144, 176]
        .iter()
        .map(|&y| frame[y * SCREEN_WIDTH + 128])
        .collect()
}

#[test]
fn given_the_keyirq_rom_when_10_frames_run_then_the_record_shows_the_expected_results() {
    assert_eq!(
        record(&boot()),
        vec![
            (0, 1),          // ARM9 count with A held: one request, for frame 1
            (1, 2),          // with A and B held: a second, for frame 2
            (2, 2),          // nothing held: none
            (3, 3),          // B alone (OR): one more
            (4, 3),          // Start is not selected: none
            (8, 0),          // ARM7 IF bit 12 with A alone (AND): none
            (9, 0x1000),     // with A and B: set
            (10, 0),         // B alone after the acknowledge: still clear
            (11, 0),         // IF bit 22 with the hinge closed: closing requests nothing
            (12, 0x40_0000), // the hinge open again: set
            (13, 0),         // although IE is clear
            (30, 1),
            (31, 0x4B49_5237), // "KIR7"
        ]
    );
}

#[test]
fn given_the_keyirq_rom_when_10_frames_run_then_every_band_is_green() {
    assert_eq!(bands(&boot()), vec![GREEN; 6]);
}

#[test]
fn given_the_keyirq_rom_with_no_input_when_10_frames_run_then_the_handler_counts_no_request() {
    // The handler's counter (0x0200_6000) is the observable: a request with nothing held would
    // have run it (`keypad.md`, "The keypad IRQ").
    let machine = boot_range(&InputLog::default(), 0, FRAMES);
    assert_eq!(machine.inspect().arm9_peek32(0x0200_6000), 0);
}

#[test]
fn given_the_keyirq_rom_when_10_frames_run_then_the_screens_match_the_golden() {
    assert_frame_hash_with_input("keyirq", FRAMES, &log());
}

#[test]
fn given_the_keyirq_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic_with_input("keyirq", FRAMES, &log());
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    // Saved with the hinge closed, before it reopens; the resumed run reopens it and finishes as
    // the uninterrupted one does.
    assert_savestate_round_trip_with_input("keyirq", 7, 3, &log());
}
