// SPDX-License-Identifier: Apache-2.0
//! The sound registers, observed through our own `sound` test ROM (`testroms/sound/`). The ARM7
//! writes the registers, polls the busy bits and times them with its timer 0, and records what it
//! read in main RAM; the ARM9 judges the record and shows six bands on the bottom screen, green for
//! a pass. Expected values come from `docs/hw/sound-registers.md`: the read-back masks (SOUNDCNT
//! `BF7Fh`, SOUNDBIAS `3FFh`, SOUNDxCNT `FF7F837Fh`, SNDCAPxCNT `8Fh`), write-only registers reading
//! 0, SoundBias levels `200h` and `000h`, POWCNT2 `1` then `3`. The two timings follow from a
//! sample taking `4 * (10000h - TMR)` master cycles (TMR `FC00h`: 4,096): a one-shot PCM8 channel of
//! 4 words is 16 samples, 65,536 master cycles, 32,768 ARM7 cycles; a one-shot PCM16 capture of
//! length 0 (one word) is 2 samples, 8,192 master cycles, 4,096 ARM7 cycles. The ARM7 starts its
//! timer a few instructions before the write and polls a few after, so the windows are
//! `[32700, 33100)` and `[4000, 4300)`. Several of these are the note's modelling choices, listed
//! under its "Unverified": a green band says the model follows the note, not that hardware was
//! measured.

use ninge_core::{Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 4;
const GREEN: u32 = 0x0000_FF00;
/// The record slots the ROM writes: 0-12, the progress and the sentinel.
const SLOTS: [u32; 15] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 30, 31];
const ONE_SHOT_WINDOW: std::ops::Range<u32> = 32_700..33_100;
const CAPTURE_WINDOW: std::ops::Range<u32> = 4_000..4_300;

fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("sound")).expect("sound ROM is built"))
        .expect("sound ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine
}

fn boot() -> Machine {
    boot_for(FRAMES)
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
fn given_the_sound_rom_when_4_frames_run_then_the_exact_slots_hold_the_note_values() {
    let slots = record(&boot());
    // Slots 0-5, then 7-9, then 11, 12, the progress and the sentinel; 6 and 10 are timings.
    let exact = [0, 1, 2, 3, 4, 5, 7, 8, 9, 11, 12, 13, 14].map(|i| slots[i]);
    assert_eq!(
        exact,
        [
            0xBF7F,      // SOUNDCNT
            0x03FF,      // SOUNDBIAS
            0xFF7F_837F, // a PSG channel started with all ones
            0,           // the same channel stopped
            0x8F,        // SNDCAP0CNT
            0,           // the write-only registers
            1,           // the loop channel is still busy
            0x200,       // SoundBias with r0 = 1
            0,           // SoundBias with r0 = 0
            1,           // POWCNT2 at power-on
            3,           // POWCNT2 after all ones
            1,           // progress
            0x4D53_4E44  // "SNDM"
        ]
    );
}

#[test]
fn given_the_sound_rom_when_a_one_shot_channel_runs_then_the_busy_bit_clears_after_32768_arm7_cycles()
 {
    let slots = record(&boot());
    assert!(
        ONE_SHOT_WINDOW.contains(&slots[6]),
        "measured {} ARM7 cycles",
        slots[6]
    );
}

#[test]
fn given_the_sound_rom_when_a_one_shot_capture_of_length_0_runs_then_it_clears_after_4096_arm7_cycles()
 {
    let slots = record(&boot());
    assert!(
        CAPTURE_WINDOW.contains(&slots[10]),
        "measured {} ARM7 cycles",
        slots[10]
    );
}

#[test]
fn given_the_sound_rom_when_4_frames_run_then_every_band_is_green() {
    assert_eq!(bands(&boot()), vec![GREEN; 6]);
}

#[test]
fn given_the_sound_rom_when_4_frames_run_then_the_screens_match_the_golden() {
    assert_frame_hash("sound", FRAMES);
}

#[test]
fn given_the_sound_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("sound", FRAMES);
}

#[test]
fn given_the_sound_rom_when_run_twice_then_the_states_are_identical() {
    assert_eq!(boot().save_state(), boot().save_state());
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("sound", 1, 3);
}
