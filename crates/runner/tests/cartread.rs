// SPDX-License-Identifier: Apache-2.0
//! Reading the cartridge ROM, observed through our own `cartread` test ROM (`testroms/cartread/`).
//! The ROM reads a 400h-byte computed blob (at ROM offset 8000h, the first offset `B7` can reach)
//! back through the slot: once by polling the data port, once by DMA (ARM9 start mode 5), then
//! asks `B8` for the chip ID with the transfer-complete IRQ bit on and off and polls IF bit 19.
//! It shows one band for each check on the bottom screen, green for a pass and red for a fail; the
//! top screen is engine B's black placeholder. Expected values come from `docs/hw/cart-protocol.md`
//! ("Commands", "The chip ID", "Interrupt", "DMA start"), never from running the emulator: the blob
//! is regenerated here from its recurrence, and the ROM image is checked to hold it where the
//! ROM's addresses assume.

use ninge_core::{Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const BUF_A: u32 = 0x0210_0000;
const BUF_B: u32 = 0x0210_1000;
const BLOB_RAM: u32 = 0x0200_4000;
const BLOB_ROM: usize = 0x8000;
const FRAMES: u64 = 3;

const GREEN: u32 = 0x0000_FF00;
const RED: u32 = 0x00FF_0000;

/// The blob's 256 words: each is the last times 33 plus 12345 (mod 2^32), from the ROM's seed.
fn blob_words() -> Vec<u32> {
    std::iter::successors(Some(0x0123_4567_u32), |&v| {
        Some(v.wrapping_mul(33).wrapping_add(12_345))
    })
    .take(256)
    .collect()
}

fn rom_image() -> Vec<u8> {
    load_rom_file(&testrom_path("cartread")).expect("cartread ROM is built")
}

fn boot(rom: Vec<u8>) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("cartread ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame(ninge_core::Input::new()));
    machine
}

fn word(machine: &Machine, addr: u32) -> u32 {
    machine.inspect().arm9_peek32(addr)
}

fn words(machine: &Machine, addr: u32, count: u32) -> Vec<u32> {
    (0..count).map(|i| word(machine, addr + 4 * i)).collect()
}

/// The record slots 0 to 4, the progress and the sentinel.
fn record(machine: &Machine) -> Vec<u32> {
    [0, 1, 2, 3, 4, 5, 6, 30, 31]
        .iter()
        .map(|&slot| word(machine, REC + 4 * slot))
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
fn given_the_cartread_image_when_the_header_is_read_then_the_blob_sits_at_rom_offset_8000h() {
    // The ROM's addresses assume the ARM9 binary is at 4000h and the blob 4000h bytes into it.
    let rom = rom_image();
    let machine = boot(rom.clone());
    let header = machine.header().expect("the image booted");
    let from_image: Vec<u32> = rom[BLOB_ROM..BLOB_ROM + 0x400]
        .chunks(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    assert_eq!(
        (header.arm9.rom_offset, from_image),
        (0x4000, blob_words()[..256].to_vec())
    );
}

#[test]
fn given_the_cartread_rom_when_3_frames_run_then_the_arm9_polled_and_dma_reads_match_the_blob() {
    let machine = boot(rom_image());
    let words_expected = blob_words();
    assert_eq!(
        (
            words(&machine, BUF_A, 128),
            words(&machine, BUF_B, 128),
            words(&machine, BLOB_RAM, 256)
        ),
        (
            words_expected[..128].to_vec(),
            words_expected[128..].to_vec(),
            words_expected
        )
    );
}

#[test]
fn given_the_cartread_rom_when_3_frames_run_then_the_record_shows_four_passes() {
    // TODO(hw-unverified): the chip ID word pins the model's manufacturer byte (C2h) and its size
    // rounding, and slot 3 pins IF bit 19 at the end of the transfer on the owner; both are listed
    // in `cart-protocol.md`, "Unverified", and are not hardware facts.
    // Slots 5 and 6 are the chip ID boot left at 027FF800h and 027FFC00h (`direct-boot.md`, "The chip
    // ID"), the same word `B8` answers.
    // Slots: polled mismatches 0, DMA mismatches 0, chip ID C2h (a 1 MiB chip: size byte 0,
    // manufacturer C2h, no flags), IF bit 19 set with the IRQ bit on and clear with it off,
    // progress 1, sentinel "CARD".
    let machine = boot(rom_image());
    assert_eq!(
        record(&machine),
        vec![
            0,
            0,
            0x0000_00C2,
            1 << 19,
            0,
            0x0000_00C2,
            0x0000_00C2,
            1,
            0x4341_5244
        ]
    );
}

#[test]
fn given_the_cartread_rom_when_3_frames_run_then_every_band_is_green() {
    let machine = boot(rom_image());
    assert_eq!(bands(&machine), vec![GREEN; 4]);
}

#[test]
fn given_the_cartread_rom_when_3_frames_run_then_the_top_screen_is_black() {
    let machine = boot(rom_image());
    assert!(machine.frame(Screen::Top).iter().all(|&pixel| pixel == 0));
}

/// The image with the ARM9 size in the header cut to 4000h: the blob is no longer copied to RAM,
/// so the ROM's expected data is zeros while the card still serves the real bytes.
fn rom_without_the_ram_copy() -> Vec<u8> {
    let mut rom = rom_image();
    rom[0x2C..0x30].copy_from_slice(&0x4000_u32.to_le_bytes());
    rom
}

#[test]
fn given_a_blob_that_differs_from_the_cards_when_3_frames_run_then_the_two_read_bands_are_red() {
    // The chip ID and the IRQ checks do not depend on the data, so their bands stay green.
    let machine = boot(rom_without_the_ram_copy());
    assert_eq!(bands(&machine), vec![RED, RED, GREEN, GREEN]);
}

#[test]
fn given_a_blob_that_differs_from_the_cards_when_3_frames_run_then_the_record_counts_128_words() {
    let machine = boot(rom_without_the_ram_copy());
    assert_eq!(record(&machine)[..2], [128, 128]);
}

#[test]
fn given_the_cartread_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    assert_frame_hash("cartread", 3);
}

#[test]
fn given_the_cartread_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("cartread", 3);
}

#[test]
fn given_the_cartread_rom_when_saved_after_1_frame_and_resumed_then_it_matches_an_uninterrupted_run()
 {
    assert_savestate_round_trip("cartread", 1, 2);
}
