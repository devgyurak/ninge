// SPDX-License-Identifier: Apache-2.0
//! Replaying hand-written input logs (`tests/replay/*.ninput`, format in
//! `docs/contributing/input-log.md`) on our own test ROMs.
//!
//! Each log is a file, not a Rust table, so the format's text is what is tested: the header must
//! fit the built ROM, the replay's final frame must match its golden (`tests/golden/README.md`),
//! the same log run twice or saved and resumed must give the same frames and savestate bytes
//! (AGENTS.md §5.5), and the log must change the run: a run with no input ends in another state.
//!
//! A log's `rom=` is the ROM's header CRC16, so a change to a test ROM's source changes it. The
//! mismatch error names the new value; put it in the log's header line.

use ninge_core::input_log;
use ninge_core::{Input, Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{
    GoldenError, InputLog, assert_deterministic_with_input, assert_savestate_round_trip_with_input,
    check_frame_hash, golden_path, parse_golden, run_frame_hashes,
};
use ninge_runner::replay::Replay;
use ninge_runner::{RunReport, load_rom_file, testrom_path};
use std::path::{Path, PathBuf};

const GREEN: u32 = 0x0000_FF00;
const RED: u32 = 0x00FF_0000;

fn log_path(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/replay")
        .join(file)
}

fn log_text(file: &str) -> String {
    std::fs::read_to_string(log_path(file)).expect("the log is in tests/replay")
}

fn rom(name: &str) -> Vec<u8> {
    load_rom_file(&testrom_path(name)).expect("the test ROM is built")
}

/// The log's inputs, as the golden helpers take them.
fn input_log_of(file: &str) -> InputLog {
    InputLog::new(
        input_log::parse(&log_text(file))
            .expect("a valid log")
            .frames,
    )
}

/// A replay of `text` on the ROM `name`, with no save chip.
fn begin(name: &str, text: &str) -> Replay {
    Replay::begin(rom(name), text, None).expect("the log fits the ROM")
}

/// The report of replaying the log in `file` on the ROM `name`.
fn report(name: &str, file: &str) -> RunReport {
    begin(name, &log_text(file)).run(None).report
}

/// Whether the report matches the golden line for its frame count in `tests/golden/<name>.txt`.
fn golden_result(name: &str, file: &str) -> Result<(), GoldenError> {
    let text = std::fs::read_to_string(golden_path(name)).expect("the golden file exists");
    check_frame_hash(&parse_golden(&text)?, &report(name, file))
}

/// The screens' hashes after the last frame of a run of `frames` frames with no input.
fn no_input_hashes(name: &str, frames: u64) -> (u64, u64) {
    run_frame_hashes(rom(name), frames)
        .last()
        .copied()
        .expect("at least one frame")
}

fn screens(report: &RunReport) -> (u64, u64) {
    (report.top_hash, report.bottom_hash)
}

/// The words of the ARM9's record from `slot` on, after the whole log has run.
fn record_after(name: &str, text: &str, base: u32, slots: &[u32]) -> Vec<u32> {
    let mut replay = begin(name, text);
    replay.run(None);
    slots
        .iter()
        .map(|slot| replay.machine().inspect().arm9_peek32(base + 4 * slot))
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

fn bands_after(name: &str, text: &str) -> Vec<u32> {
    let mut replay = begin(name, text);
    replay.run(None);
    bands(replay.machine())
}

// ---- keyirq ----

#[test]
fn given_the_keyirq_log_when_checked_against_the_rom_then_its_header_fits() {
    assert!(Replay::begin(rom("keyirq"), &log_text("keyirq.ninput"), None).is_ok());
}

#[test]
fn given_the_keyirq_log_when_replayed_then_the_final_frame_matches_the_golden() {
    assert_eq!(golden_result("keyirq", "keyirq.ninput"), Ok(()));
}

#[test]
fn given_the_keyirq_log_when_replayed_then_it_runs_ten_frames() {
    assert_eq!(report("keyirq", "keyirq.ninput").frames, 10);
}

#[test]
fn given_the_keyirq_log_when_run_twice_then_frames_and_savestates_match() {
    assert_deterministic_with_input("keyirq", 10, &input_log_of("keyirq.ninput"));
}

#[test]
fn given_the_keyirq_log_when_saved_loaded_and_continued_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip_with_input("keyirq", 4, 6, &input_log_of("keyirq.ninput"));
}

#[test]
fn given_the_keyirq_log_when_compared_with_a_run_of_no_input_then_the_screens_differ() {
    assert_ne!(
        screens(&report("keyirq", "keyirq.ninput")),
        no_input_hashes("keyirq", 10)
    );
}

// ---- touch ----

#[test]
fn given_the_touch_log_when_checked_against_the_rom_then_its_header_fits() {
    assert!(Replay::begin(rom("touch"), &log_text("touch.ninput"), None).is_ok());
}

#[test]
fn given_the_touch_log_when_replayed_then_the_final_frame_matches_the_golden() {
    assert_eq!(golden_result("touch", "touch.ninput"), Ok(()));
}

#[test]
fn given_the_touch_log_when_replayed_then_it_runs_four_frames() {
    assert_eq!(report("touch", "touch.ninput").frames, 4);
}

#[test]
fn given_the_touch_log_when_run_twice_then_frames_and_savestates_match() {
    assert_deterministic_with_input("touch", 4, &input_log_of("touch.ninput"));
}

#[test]
fn given_the_touch_log_when_saved_loaded_and_continued_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip_with_input("touch", 2, 2, &input_log_of("touch.ninput"));
}

#[test]
fn given_the_touch_log_when_compared_with_a_run_of_no_input_then_the_screens_differ() {
    assert_ne!(
        screens(&report("touch", "touch.ninput")),
        no_input_hashes("touch", 4)
    );
}

// ---- keypad ----

/// What the ARM9 recorded at 0x0200_4000: slot 1 is its halfword read of KEYINPUT (`keypad.md`).
const KEYPAD_RECORD: u32 = 0x0200_4000;

#[test]
fn given_the_keypad_log_when_checked_against_the_rom_then_its_header_fits() {
    assert!(Replay::begin(rom("keypad"), &log_text("keypad.ninput"), None).is_ok());
}

#[test]
fn given_the_keypad_log_when_replayed_then_the_arm9_read_a_held_in_keyinput() {
    // Frame 1 holds A: bit 0 reads 0, so the ten key bits read 0x3FE.
    assert_eq!(
        record_after("keypad", &log_text("keypad.ninput"), KEYPAD_RECORD, &[1]),
        vec![0x03FE]
    );
}

/// The keypad log with every frame line empty: the same header and comments, no input.
fn keypad_log_of_nothing() -> String {
    log_text("keypad.ninput")
        .lines()
        .filter(|line| !line.starts_with("f="))
        .map(|line| format!("{line}\n"))
        .chain((1..=4).map(|n| format!("f={n}\n")))
        .collect()
}

#[test]
fn given_the_keypad_log_when_compared_with_a_log_of_nothing_then_the_record_differs() {
    assert_ne!(
        record_after("keypad", &log_text("keypad.ninput"), KEYPAD_RECORD, &[1]),
        record_after("keypad", &keypad_log_of_nothing(), KEYPAD_RECORD, &[1])
    );
}

#[test]
fn given_the_keypad_log_when_run_twice_then_frames_and_savestates_match() {
    assert_deterministic_with_input("keypad", 4, &input_log_of("keypad.ninput"));
}

#[test]
fn given_the_keypad_log_when_saved_loaded_and_continued_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip_with_input("keypad", 2, 2, &input_log_of("keypad.ninput"));
}

// ---- rtc, and the recorded wall time ----

/// What the ARM7 recorded at 0x0200_8000 (`rtc.rs`): slot 1 is the date the chip started on.
const RTC_RECORD: u32 = 0x0200_8000;

#[test]
fn given_the_rtc_log_when_checked_against_the_rom_then_its_header_fits() {
    assert!(Replay::begin(rom("rtc"), &log_text("rtc-wall.ninput"), None).is_ok());
}

#[test]
fn given_the_rtc_log_when_replayed_then_the_final_frame_matches_the_golden() {
    assert_eq!(golden_result("rtc", "rtc-wall.ninput"), Ok(()));
}

#[test]
fn given_the_rtc_log_when_replayed_then_the_chip_started_on_the_recorded_date_not_the_epoch() {
    // 366 days after 2000-01-01: 2001-01-01, day of week 0 (`rtc.rs`, the host-time test).
    assert_eq!(
        record_after("rtc", &log_text("rtc-wall.ninput"), RTC_RECORD, &[1]),
        vec![0x0001_0101]
    );
}

#[test]
fn given_the_rtc_log_without_its_wall_fields_when_replayed_then_the_chip_started_on_the_epoch() {
    let without = log_text("rtc-wall.ninput").replace(" wall=31622400", "");
    assert_eq!(
        record_after("rtc", &without, RTC_RECORD, &[1]),
        vec![0x0501_0100]
    );
}

#[test]
fn given_the_rtc_log_when_replayed_then_only_the_start_date_band_is_red() {
    assert_eq!(
        bands_after("rtc", &log_text("rtc-wall.ninput")),
        vec![GREEN, RED, GREEN, GREEN, GREEN, GREEN]
    );
}

#[test]
fn given_the_rtc_log_when_compared_with_a_run_with_the_host_clock_at_the_epoch_then_the_screens_differ()
 {
    assert_ne!(
        screens(&report("rtc", "rtc-wall.ninput")),
        no_input_hashes("rtc", 4)
    );
}

#[test]
fn given_the_rtc_log_when_run_twice_then_frames_and_savestates_match() {
    assert_deterministic_with_input("rtc", 4, &input_log_of("rtc-wall.ninput"));
}

#[test]
fn given_the_rtc_log_when_saved_loaded_and_continued_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip_with_input("rtc", 2, 2, &input_log_of("rtc-wall.ninput"));
}

/// A machine driven frame by frame with the recorded wall time, as a replay drives it.
fn rtc_driven_by_hand() -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(rom("rtc")).expect("a valid header");
    machine
        .insert_save(ninge_core::cart::SaveType::None, Vec::new())
        .expect("no chip needs no bytes");
    (0..4).for_each(|_| machine.run_frame(Input::new().with_wall_time(31_622_400)));
    machine
}

#[test]
fn given_the_rtc_log_when_the_machine_is_driven_by_hand_then_the_savestate_equals_the_replay_s() {
    let mut replay = begin("rtc", &log_text("rtc-wall.ninput"));
    replay.run(None);
    assert_eq!(
        replay.machine().save_state(),
        rtc_driven_by_hand().save_state()
    );
}
