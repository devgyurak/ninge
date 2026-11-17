// SPDX-License-Identifier: Apache-2.0
//! Talking to the save chip over AUXSPI, observed through our own `savechip` test ROM
//! (`testroms/savechip/`). The ROM drives an 8 KiB EEPROM by hand: a write behind WREN with a
//! wait on RDSR and a read back, a write with no WREN that must be ignored, a 33-byte write that
//! wraps inside a 32-byte page, and the write latch's three states. It shows one band for each
//! check on the bottom screen, green for a pass and red for a fail; the top screen is engine B's
//! black placeholder. Expected values come from `docs/hw/auxspi-saves.md`, never from running the
//! emulator. The chip is a blank EEPROM made here (`SaveType::Eeprom8K`, every byte `FFh`); the
//! test fits it the way the golden harness does, so no save file is read or written.

use ninge_core::cart::SaveType;
use ninge_core::{Input, Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::assert_frame_hash_with_save;
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 3;
const GREEN: u32 = 0x0000_FF00;

fn rom_image() -> Vec<u8> {
    load_rom_file(&testrom_path("savechip")).expect("savechip ROM is built")
}

/// A booted machine with a blank 8 KiB EEPROM that has run `frames` frames.
fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(rom_image())
        .expect("savechip ROM has a valid header");
    machine
        .insert_save(SaveType::Eeprom8K, vec![0xFF; 8192])
        .expect("8 KiB fits");
    (0..frames).for_each(|_| machine.run_frame(Input::new()));
    machine
}

/// Runs `frames` more frames.
fn run_on(machine: &mut Machine, frames: u64) {
    (0..frames).for_each(|_| machine.run_frame(Input::new()));
}

fn boot() -> Machine {
    boot_for(FRAMES)
}

fn word(machine: &Machine, slot: u32) -> u32 {
    machine.inspect().arm9_peek32(REC + 4 * slot)
}

/// The record slots 0 to 3, the progress and the sentinel.
fn record(machine: &Machine) -> Vec<u32> {
    [0, 1, 2, 3, 30, 31]
        .iter()
        .map(|&slot| word(machine, slot))
        .collect()
}

/// The colour of each band's middle pixel on the bottom screen, top band first.
fn bands(machine: &Machine) -> Vec<u32> {
    let frame = machine.frame(Screen::Bottom);
    [24, 72, 120, 168]
        .iter()
        .map(|&y| frame[y * SCREEN_WIDTH + 128])
        .collect()
}

#[test]
fn given_the_savechip_rom_when_3_frames_run_then_the_record_shows_the_four_expected_results() {
    // Slots: A1h B2h C3h D4h read back from 0040h; FFh FFh at 0080h after a write with no WREN;
    // 33 at 0100h and 32 at 011Fh after the page wrap; RDSR 00h, 02h, 00h; progress 1; "SAVE".
    assert_eq!(
        record(&boot()),
        vec![
            0xD4C3_B2A1,
            0x0000_FFFF,
            0x0000_2021,
            0x0000_0200,
            1,
            0x5341_5645
        ]
    );
}

#[test]
fn given_the_savechip_rom_when_3_frames_run_then_every_band_is_green() {
    assert_eq!(bands(&boot()), vec![GREEN; 4]);
}

#[test]
fn given_the_savechip_rom_when_3_frames_run_then_the_chip_holds_what_the_rom_wrote() {
    let machine = boot();
    let save = machine.save_data();
    // The four bytes at 0040h; nothing from the write with no WREN at 0080h; the page at 0100h with
    // its first byte replaced by the 33rd and its last still 32.
    assert_eq!(
        (
            save[0x40..0x44].to_vec(),
            save[0x80..0x82].to_vec(),
            save[0x100..0x103].to_vec(),
            save[0x11F],
            save[0x120]
        ),
        (
            vec![0xA1, 0xB2, 0xC3, 0xD4],
            vec![0xFF, 0xFF],
            vec![33, 2, 3],
            32,
            0xFF
        )
    );
}

#[test]
fn given_the_savechip_rom_when_3_frames_run_then_the_save_generation_has_moved() {
    // Four bytes at 0040h, 32 page bytes from 0100h, and the 33rd, which changes the first again.
    assert_eq!(boot().save_generation(), 4 + 32 + 1);
}

#[test]
fn given_the_savechip_rom_when_3_frames_run_then_the_screens_match_the_golden() {
    assert_frame_hash_with_save("savechip", FRAMES, SaveType::Eeprom8K);
}

#[test]
fn given_the_savechip_rom_when_run_twice_then_the_save_bytes_and_the_state_are_identical() {
    let (a, b) = (boot(), boot());
    assert_eq!(
        (a.save_data().to_vec(), a.save_state()),
        (b.save_data().to_vec(), b.save_state())
    );
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    let uninterrupted = boot_for(5);
    let saver = boot_for(1);
    let mut resumed = boot_for(0);
    resumed.load_state(&saver.save_state()).expect("same ROM");
    run_on(&mut resumed, 4);
    assert_eq!(
        (resumed.save_data().to_vec(), resumed.save_state()),
        (
            uninterrupted.save_data().to_vec(),
            uninterrupted.save_state()
        )
    );
}
