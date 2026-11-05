// SPDX-License-Identifier: Apache-2.0
//! ARM9 memory map, observed through our own `memmap` test ROM (`testroms/memmap/`). The ROM
//! records what it read at 0x0200_4000 (one word per slot); the expected values below come from
//! the hardware notes, never from running the ROM:
//!
//! - `docs/hw/arm9-memory-map.md`: "Alignment", "Main RAM", "Tightly coupled memory",
//!   "Unmapped addresses and I/O"
//! - `docs/hw/arm-load-store.md`: "Alignment" (unaligned LDR rotates right by 8 * (addr & 3))
//! - `docs/hw/vram-banks.md`: bank read/write rules and its "Unverified" section
//! - `docs/hw/cp15.md`: TCM control bits
//!
//! Slots the note marks Unverified are in tests whose names say so (`TODO(hw-unverified)`).

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const RECORD: u32 = 0x0200_4000;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// A machine that booted the `memmap` ROM and ran one frame.
fn boot_one_frame() -> Machine {
    let rom = load_rom_file(&testrom_path("memmap")).expect("memmap ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("memmap ROM has a valid header");
    machine.run_frame();
    machine
}

/// Expected record: (slot, name, expected value).
const TABLE: &[(u32, &str, u32)] = &[
    (0, "word store/load", 0xA1B2_C3D4),
    (1, "LDRH at +0", 0xC3D4),
    (2, "LDRH at +2", 0xA1B2),
    (3, "LDRB at +0", 0xD4),
    (4, "LDRB at +3", 0xA1),
    (5, "STRH at +2 of ones", 0x1234_FFFF),
    (6, "LDRH of that halfword", 0x1234),
    (7, "STRB at +1 of zero", 0x0000_5A00),
    (8, "LDRB of that byte", 0x5A),
    (9, "unaligned LDR rotates", 0xD4A1_B2C3),
    (10, "mirror at +0x400000", 0x4D41_5252),
    (11, "mirror at +0x800000", 0x4D41_5252),
    (12, "mirror at +0xC00000", 0x4D41_5252),
    (13, "write through mirror", 0x0B0B_0B0B),
    (14, "last word of 4 MiB", 0xFEED_F00D),
    (15, "last word mirror 0x027FFFFC", 0xFEED_F00D),
    (16, "first word past 4 MiB", 0x6D65_6D6D),
    (17, "last word of 16 MiB block", 0xFEED_F00D),
    (18, "DTCM read/write", 0xD7C3_0001),
    (19, "DTCM off reaches memory", 0x1111_1111),
    (20, "DTCM region mirrors TCM", 0xD7C3_0002),
    (21, "just past DTCM region", 0x2222_2222),
    (22, "load mode read", 0x3333_3333),
    (23, "load-mode store not in RAM", 0x3333_3333),
    (24, "load-mode store in TCM", 0x4444_4444),
    (25, "ITCM read/write", 0x1BC0_DE01),
    (26, "ITCM mirror in 64 KiB", 0x1BC0_DE02),
    (27, "code fetched from ITCM", 0x0000_0042),
    (28, "VRAM word", 0x5641_5431),
    (29, "VRAM halfword", 0x0000_BEEF),
    (30, "STRB to VRAM ignored", 0x1122_3344),
    (31, "disabled bank reads zero", 0x0000_0000),
    (32, "undefined I/O reads zero", 0x0000_0000),
    (33, "progress", 7),
    (34, "sentinel", 0x6D65_6D21),
];

/// Mismatches among the given slots.
fn mismatches(machine: &Machine, slots: &[u32]) -> Vec<Mismatch> {
    TABLE
        .iter()
        .filter(|(slot, _, _)| slots.contains(slot))
        .map(|&(slot, name, expected)| {
            (slot, name, expected, machine.arm9_peek32(RECORD + slot * 4))
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

/// Mismatches among the slots in `first..=last`.
fn mismatches_in(first: u32, last: u32) -> Vec<Mismatch> {
    let slots: Vec<u32> = (first..=last).collect();
    mismatches(&boot_one_frame(), &slots)
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_main_ram_accesses_are_little_endian_and_unaligned_ldr_rotates()
 {
    assert_eq!(mismatches_in(0, 9), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_the_last_word_of_4_mib_reads_back() {
    assert_eq!(mismatches_in(14, 14), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_the_4_mib_mirrors_are_recorded_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): mirror layout of main RAM, arm9-memory-map.md "Main RAM".
    let machine = boot_one_frame();
    assert_eq!(
        mismatches(&machine, &[10, 11, 12, 13, 15, 16, 17]),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_dtcm_reads_writes_and_mirrors_follow_its_region() {
    assert_eq!(mismatches_in(18, 21), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_dtcm_load_mode_reads_ram_and_stores_to_tcm() {
    assert_eq!(mismatches_in(22, 24), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_itcm_mirrors_and_executes_code() {
    assert_eq!(mismatches_in(25, 27), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_vram_bank_a_stores_words_and_halfwords_and_ignores_byte_stores()
 {
    assert_eq!(mismatches_in(28, 30), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_a_disabled_vram_bank_reads_zero_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): vram-banks.md "Unverified".
    assert_eq!(mismatches_in(31, 31), Vec::<Mismatch>::new());
}

// Slot 32 reads 0x0400_0090, which no ARM9 I/O register occupies (`arm9-memory-map.md`, "Evidence").
#[test]
fn given_memmap_rom_when_run_one_frame_then_an_undefined_io_port_reads_zero() {
    assert_eq!(mismatches_in(32, 32), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_all_seven_groups_finished() {
    assert_eq!(mismatches_in(33, 33), Vec::<Mismatch>::new());
}

#[test]
fn given_memmap_rom_when_run_one_frame_then_the_sentinel_was_written_last() {
    assert_eq!(mismatches_in(34, 34), Vec::<Mismatch>::new());
}
