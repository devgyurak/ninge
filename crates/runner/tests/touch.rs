// SPDX-License-Identifier: Apache-2.0
//! The touch-screen controller and the power-management device, observed through our own `touch`
//! test ROM (`testroms/touch/`). The ARM7 drives both through SPICNT and SPIDATA while the host
//! presses the panel at pixel (100, 50) for the first two frames and releases it from the third,
//! and records what it read in main RAM; the ARM9 compares the record with the note's values and
//! shows one band for each of six checks on the bottom screen, green for a pass. Expected words are
//! packed by hand from `docs/hw/touch-power.md`: X and Y are 16 times the pixel (640h and 320h); a
//! 12-bit result is rebuilt as `((hi << 8) | lo) >> 3`; the 8-bit result is the top eight bits of
//! the 12-bit one; Z1, Z2, both temperatures and the microphone are the note's fixed values; the
//! pen bit (EXTKEYIN bit 6) is 0 while the panel is touched and the last command left the pen output
//! enabled (modes 0 and 2), 1 otherwise; a released panel reads X 000h and Y FFFh; the power
//! device keeps bits 0-6 of register 0 (bit 6, shutdown, stays clear
//! in the ROM: it would end the run), reads the battery register as 0 and mirrors index 4 onto 0.
//! Many of these are the note's own modelling choices, listed under its "Unverified": a green band
//! says the model follows the note, not that hardware was measured.

use ninge_core::{Input, Machine, SCREEN_WIDTH, Screen, Touch};
use ninge_runner::golden::{
    InputLog, assert_deterministic_with_input, assert_frame_hash_with_input,
    assert_savestate_round_trip_with_input,
};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 4;
const GREEN: u32 = 0x0000_FF00;
/// The record slots the ROM writes: 0-17, the progress and the sentinel.
const SLOTS: [u32; 20] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 30, 31,
];

/// The panel pressed at (100, 50) for two frames, then released.
fn log() -> InputLog {
    let pressed = Input::new().with_touch(Some(Touch { x: 100, y: 50 }));
    InputLog::new(vec![pressed, pressed])
}

fn rom_image() -> Vec<u8> {
    load_rom_file(&testrom_path("touch")).expect("touch ROM is built")
}

/// A booted machine that has run frames `from..to` of `log`.
fn boot_range(log: &InputLog, from: u64, to: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(rom_image())
        .expect("touch ROM has a valid header");
    (from..to).for_each(|frame| machine.run_frame(log.input_for(frame)));
    machine
}

fn boot() -> Machine {
    boot_range(&log(), 0, FRAMES)
}

fn record(machine: &Machine) -> Vec<u32> {
    SLOTS
        .iter()
        .map(|&slot| machine.inspect().arm9_peek32(REC + 4 * slot))
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
fn given_the_touch_rom_when_4_frames_run_then_the_record_shows_the_expected_results() {
    assert_eq!(
        record(&boot()),
        vec![
            0x3F,        // EXTKEYIN at power-on, panel pressed, mode 0: pen bit clear
            0x0640,      // X = 100 * 16
            0x0320,      // Y = 50 * 16
            0x0600,      // Z1, touched
            0x0A00,      // Z2, touched
            0x0640,      // X in 8-bit mode, rebuilt: 100 << 4
            0x0320_0640, // X then Y with the next command in the low-byte slot
            0x068E_0600, // temperature 1 and 0
            0x0000_0800, // battery 000h, microphone 800h
            0x7F,        // pen output disabled after a mode 1 command
            0x3F,        // enabled again after mode 0
            0x7F,        // panel released
            0x0000,      // X released
            0x0FFF,      // Y released
            0x0C,        // power register 0 written 0Ch
            0x00,        // battery register
            0x3F, // register 0 written BFh: bit 7 is dropped, and bit 6 (shutdown) stays clear
            0x3F, // index 4 mirrors register 0
            1,
            0x4D48_4354 // "TCHM"
        ]
    );
}

#[test]
fn given_the_touch_rom_when_4_frames_run_then_every_band_is_green() {
    assert_eq!(bands(&boot()), vec![GREEN; 6]);
}

#[test]
fn given_the_touch_rom_with_the_panel_never_pressed_when_4_frames_run_then_the_pen_bit_at_power_on_reads_released()
 {
    // Slot 0 is EXTKEYIN read before any command: with no press the pen bit is set (7Fh), not 3Fh.
    let machine = boot_range(&InputLog::default(), 0, FRAMES);
    assert_eq!(record(&machine)[0], 0x7F);
}

#[test]
fn given_the_touch_rom_when_4_frames_run_then_the_screens_match_the_golden() {
    assert_frame_hash_with_input("touch", FRAMES, &log());
}

#[test]
fn given_the_touch_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic_with_input("touch", FRAMES, &log());
}

#[test]
fn given_the_touch_rom_when_run_twice_then_the_states_are_identical() {
    assert_eq!(boot().save_state(), boot().save_state());
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    // Saved after the press, while the ARM7 waits for the release; the resumed run sees the
    // release and finishes as the uninterrupted one does.
    assert_savestate_round_trip_with_input("touch", 2, 2, &log());
}
