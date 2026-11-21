// SPDX-License-Identifier: Apache-2.0
//! The clock chip, observed through our own `rtc` test ROM (`testroms/rtc/`). The ARM7 drives RTCCNT
//! by hand, bit by bit, as `docs/hw/rtc.md` describes the transfer (the usual command bytes sent
//! most significant bit first, the parameter bytes least significant bit first), and records what it
//! read in main RAM; the ARM9 compares the record with the note's values and shows one band for each
//! of six checks on the bottom screen, green for a pass and red for a fail. The runner's host time is
//! the epoch, `Input::default`, so the chip starts at 2000-01-01 00:00:00. Expected values come from
//! the note and the calendar: status 1 is 02h; 2000-01-01 was a Saturday, day of week 5 with Monday
//! as 0; the hour byte's bit 6 is the PM flag from noon on; 12-hour mode shows 23h as 11 PM; the
//! alarm day byte keeps bits 0-2 and 7. Three of these are the note's own modelling choices, listed
//! under its "Unverified" (status 1 = 02h at power-on, day of week 5 for the first day, and every
//! bit of INT2 kept as written within those masks): a green band says the model follows the note,
//! not that hardware was measured.

use ninge_core::{Input, Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 3;
const GREEN: u32 = 0x0000_FF00;

fn rom_image() -> Vec<u8> {
    load_rom_file(&testrom_path("rtc")).expect("rtc ROM is built")
}

/// Runs `frames` more frames.
fn run_on(machine: &mut Machine, frames: u64) {
    (0..frames).for_each(|_| machine.run_frame(Input::new()));
}

/// A booted machine that has run `frames` frames.
fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(rom_image())
        .expect("rtc ROM has a valid header");
    run_on(&mut machine, frames);
    machine
}

fn boot() -> Machine {
    boot_for(FRAMES)
}

/// The record slots 0 to 7, the progress and the sentinel.
fn record(machine: &Machine) -> Vec<u32> {
    [0, 1, 2, 3, 4, 5, 6, 7, 30, 31]
        .iter()
        .map(|&slot| machine.inspect().arm9_peek32(REC + 4 * slot))
        .collect()
}

/// A booted machine that has run `frames` frames with the host's time at `wall_time`.
fn boot_at(wall_time: u32, frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(rom_image()).expect("valid header");
    (0..frames).for_each(|_| machine.run_frame(Input::new().with_wall_time(wall_time)));
    machine
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
fn given_the_rtc_rom_when_3_frames_run_then_the_record_shows_the_expected_results() {
    // Status 1; the starting date and time (00 01 01 05 / 00 00 00); the written date and time read
    // back (24 12 31 01 / 63 59 58); the 12-hour time (51 59 58); status 2 55h and the free
    // register A5h; INT2 (87 12 34); progress 1; "RTCM".
    assert_eq!(
        record(&boot()),
        vec![
            0x02,
            0x0501_0100,
            0x0000_0000,
            0x0131_1224,
            0x0058_5963,
            0x0058_5951,
            0x0000_A555,
            0x0034_1287,
            1,
            0x4D43_5452
        ]
    );
}

#[test]
fn given_the_rtc_rom_when_3_frames_run_then_every_band_is_green() {
    assert_eq!(bands(&boot()), vec![GREEN; 6]);
}

#[test]
fn given_the_rtc_rom_when_3_frames_run_then_the_screens_match_the_golden() {
    assert_frame_hash("rtc", FRAMES);
}

#[test]
fn given_the_rtc_rom_when_run_twice_then_the_frames_agree() {
    assert_deterministic("rtc", FRAMES);
}

#[test]
fn given_the_rtc_rom_when_run_twice_then_the_states_are_identical() {
    assert_eq!(boot().save_state(), boot().save_state());
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    let uninterrupted = boot_for(5);
    let saver = boot_for(1);
    let mut resumed = boot_for(0);
    resumed.load_state(&saver.save_state()).expect("same ROM");
    run_on(&mut resumed, 4);
    assert_eq!(resumed.save_state(), uninterrupted.save_state());
}

#[test]
fn given_the_rtc_rom_when_the_host_time_is_a_year_later_then_the_arm7_s_first_read_differs_but_the_written_time_does_not()
 {
    // A later host time shifts the chip's starting time (slot 1 is 2001-01-01 plus a weekday), but a
    // time the program writes is its own, so slots 3 and 4 do not change.
    let machine = boot_at(366 * 86_400, FRAMES);
    let slots = record(&machine);
    // Day 366: 366 mod 7 = 2, 5 + 2 -> weekday 0, so the date bytes are 01 01 01 00.
    assert_eq!(
        (slots[1], slots[3], slots[4]),
        (0x0001_0101, 0x0131_1224, 0x0058_5963)
    );
}
