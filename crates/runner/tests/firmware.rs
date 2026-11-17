// SPDX-License-Identifier: Apache-2.0
//! The SPI bus and the firmware, observed through our own `firmware` test ROM
//! (`testroms/firmware/`). The ARM7 reads the firmware flash over SPICNT and SPIDATA by hand
//! (RDID, the update counter of user-settings area 0, and the first 74h bytes of area 1), computes
//! the CRC-16 over the settings and records the results in main RAM; the ARM9 compares them with
//! what direct boot left there (the boot indicator at `027FFC40h` and the user-settings copy at
//! `027FFC80h`) and shows one band for each of four checks on the bottom screen, green for a pass
//! and red for a fail. Expected values come from `docs/hw/firmware.md`, `docs/hw/spi.md` and
//! `docs/hw/direct-boot.md`: the chip ID, the counters 0 and 1, the boot indicator 1, and the
//! settings bytes and CRC of the image `ninge_core::spi::build` makes from the default settings,
//! which the ROM's own computation must agree with.

use ninge_core::cart::crc16;
use ninge_core::spi::{FirmwareSettings, build};
use ninge_core::{Input, Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const BUF: u32 = 0x0200_8100;
/// The user-settings copy direct boot leaves in main RAM.
const USER_COPY: u32 = 0x027F_FC80;
const FRAMES: u64 = 3;
const GREEN: u32 = 0x0000_FF00;
/// Area 1 of the firmware: the newest, by the counters 0 and 1.
const AREA1: usize = 0x3_FF00;

fn rom_image() -> Vec<u8> {
    load_rom_file(&testrom_path("firmware")).expect("firmware ROM is built")
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
        .expect("firmware ROM has a valid header");
    run_on(&mut machine, frames);
    machine
}

fn boot() -> Machine {
    boot_for(FRAMES)
}

fn word(machine: &Machine, address: u32) -> u32 {
    machine.inspect().arm9_peek32(address)
}

/// The record slots 0 to 4, the progress and the sentinel.
fn record(machine: &Machine) -> Vec<u32> {
    [0, 1, 2, 3, 4, 30, 31]
        .iter()
        .map(|&slot| word(machine, REC + 4 * slot))
        .collect()
}

/// `len` bytes of main RAM at `address`.
fn bytes(machine: &Machine, address: u32, len: u32) -> Vec<u8> {
    (0..len)
        .map(|i| machine.inspect().arm9_peek8(address + i))
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

/// The firmware image the machine builds from the default settings.
fn image() -> Vec<u8> {
    build(&FirmwareSettings::default())
}

#[test]
fn given_the_firmware_rom_when_3_frames_run_then_the_record_shows_the_expected_results() {
    let area = image()[AREA1..AREA1 + 0x70].to_vec();
    let crc = u32::from(crc16(&area));
    // Slots: RDID 20h 40h 12h; the computed and the stored CRC (the same value, low and high
    // half); the counters 0 and 1; the boot indicator 1; no differing word; progress 1; "FWRM".
    assert_eq!(
        record(&boot()),
        vec![0x0012_4020, crc | crc << 16, 0x0100, 1, 0, 1, 0x4D52_5746]
    );
}

#[test]
fn given_the_firmware_rom_when_3_frames_run_then_every_band_is_green() {
    assert_eq!(bands(&boot()), vec![GREEN; 4]);
}

#[test]
fn given_the_firmware_rom_when_3_frames_run_then_the_arm7_read_equals_the_image_it_was_built_from()
{
    // The first 74h bytes of area 1, read over SPI.
    assert_eq!(
        bytes(&boot(), BUF, 0x74),
        image()[AREA1..AREA1 + 0x74].to_vec()
    );
}

#[test]
fn given_the_firmware_rom_when_3_frames_run_then_the_boot_copy_equals_the_newest_area() {
    assert_eq!(
        bytes(&boot(), USER_COPY, 0x70),
        image()[AREA1..AREA1 + 0x70].to_vec()
    );
}

#[test]
fn given_the_firmware_rom_when_3_frames_run_then_the_screens_match_the_golden() {
    assert_frame_hash("firmware", FRAMES);
}

#[test]
fn given_the_firmware_rom_when_run_twice_then_the_frames_agree() {
    assert_deterministic("firmware", FRAMES);
}

#[test]
fn given_the_firmware_rom_when_run_twice_then_the_states_are_identical() {
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
