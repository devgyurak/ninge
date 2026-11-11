// SPDX-License-Identifier: Apache-2.0
//! The interrupt controller registers, observed through our own `irq` test ROM
//! (`testroms/irq/`). Each CPU records what it reads from its own IME, IE and IF in main RAM
//! (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; slot layouts at the top of `arm9.s` and `arm7.s`).
//! Expected values come from `docs/hw/interrupts.md` ("Registers", "IE and IF bits", "Behavior",
//! "Wider and narrower accesses", "Edge cases"), never from running the ROM.
//!
//! Valid bits: ARM9 `003F_3F7Fh`, ARM7 `01DF_3FFFh`. The ROM never requests an interrupt and
//! never clears the CPSR I flag, so IF stays 0 and no IRQ is delivered.
//!
//! The independence probe: the ARM9 writes `5555_5555h` to IE (kept: `0015_1555h`), then the
//! ARM7 reads its own IE (it has not written it yet: 0), then writes `AAAA_AAAAh` (kept:
//! `008A_2AAAh`); afterwards the ARM9 reads its IE again and still sees its own value.
//!
//! Slots the note marks Unverified are in tests whose names say so (`TODO(hw-unverified)`).

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_RECORD: u32 = 0x0200_4000;
const ARM7_RECORD: u32 = 0x0200_5000;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// Expected record: (slot within the record, name, expected value).
type Row = (u32, &'static str, u32);

/// A machine that booted the `irq` ROM and ran two frames (both CPUs finish well inside the
/// first; the second only adds spinning).
fn boot() -> Machine {
    let rom = load_rom_file(&testrom_path("irq")).expect("irq ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("irq ROM has a valid header");
    machine.run_frame();
    machine.run_frame();
    machine
}

/// Mismatches between `rows` and the record that starts at `base`.
fn mismatches(machine: &Machine, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    rows.iter()
        .map(|&(slot, name, expected)| {
            (
                slot,
                name,
                expected,
                machine.inspect().arm9_peek32(base + slot * 4),
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

fn arm9_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM9_RECORD, rows)
}

fn arm7_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM7_RECORD, rows)
}

// ------------------------------------------------------------------ reset values

// TODO(hw-unverified): the reset values are not stated; the note models all 0 (`interrupts.md`,
// "Registers" and "Unverified").
const ARM9_RESET: &[Row] = &[
    (0, "ARM9 IME at reset", 0),
    (1, "ARM9 IE at reset", 0),
    (2, "ARM9 IF at reset", 0),
];
const ARM7_RESET: &[Row] = &[
    (0, "ARM7 IME at reset", 0),
    (1, "ARM7 IE at reset", 0),
    (2, "ARM7 IF at reset", 0),
];

#[test]
fn given_irq_rom_when_run_then_the_arm9_reads_zero_from_ime_ie_and_if_at_reset_as_the_note_models_unverified()
 {
    assert_eq!(arm9_mismatches(ARM9_RESET), Vec::<Mismatch>::new());
}

#[test]
fn given_irq_rom_when_run_then_the_arm7_reads_zero_from_ime_ie_and_if_at_reset_as_the_note_models_unverified()
 {
    assert_eq!(arm7_mismatches(ARM7_RESET), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ IME

const ARM9_IME: &[Row] = &[
    // "IME keeps only bit 0; the other bits read 0."
    (3, "ARM9 IME after writing FFFF_FFFF", 1),
    (9, "ARM9 IME after a byte write of FF to byte 1", 0),
];
const ARM7_IME: &[Row] = &[
    (3, "ARM7 IME after writing FFFF_FFFF", 1),
    (9, "ARM7 IME after a byte write of FF to byte 1", 0),
];

#[test]
fn given_irq_rom_when_run_then_the_arm9_ime_keeps_only_bit_0() {
    assert_eq!(arm9_mismatches(ARM9_IME), Vec::<Mismatch>::new());
}

#[test]
fn given_irq_rom_when_run_then_the_arm7_ime_keeps_only_bit_0() {
    assert_eq!(arm7_mismatches(ARM7_IME), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ IE

// TODO(hw-unverified): a write to a bit that does not exist is dropped (`interrupts.md`,
// "Unverified"). The masks come from the table: ARM9 bits 0-6, 8-13, 16-21; ARM7 bits 0-13,
// 16-20, 22-24.
const ARM9_IE: &[Row] = &[
    (4, "ARM9 IE after writing FFFF_FFFF", 0x003F_3F7F),
    // A byte write acts on that byte only: bits 0-6 exist in byte 0 on the ARM9 (bit 7 not).
    (6, "ARM9 IE after a byte write of FF to byte 0", 0x7F),
    // A halfword write to the high half: bits 16-21 exist.
    (
        7,
        "ARM9 IE after a halfword write of FFFF to bits 16-31",
        0x003F_0000,
    ),
];
const ARM7_IE: &[Row] = &[
    (4, "ARM7 IE after writing FFFF_FFFF", 0x01DF_3FFF),
    // Bit 7 exists on the ARM7.
    (6, "ARM7 IE after a byte write of FF to byte 0", 0xFF),
    // Bits 16-20 and 22-24 exist.
    (
        7,
        "ARM7 IE after a halfword write of FFFF to bits 16-31",
        0x01DF_0000,
    ),
];

#[test]
fn given_irq_rom_when_run_then_the_arm9_ie_keeps_only_the_arm9_bits_at_word_byte_and_halfword_width_as_the_note_models_unverified()
 {
    assert_eq!(arm9_mismatches(ARM9_IE), Vec::<Mismatch>::new());
}

#[test]
fn given_irq_rom_when_run_then_the_arm7_ie_keeps_only_the_arm7_bits_at_word_byte_and_halfword_width_as_the_note_models_unverified()
 {
    assert_eq!(arm7_mismatches(ARM7_IE), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ IF

const ARM9_IF: &[Row] = &[
    // Nothing requests an interrupt, and software cannot set an IF bit by writing.
    (5, "ARM9 IF after IE was written", 0),
    (8, "ARM9 IF after writing FFFF_FFFF", 0),
];
const ARM7_IF: &[Row] = &[
    (5, "ARM7 IF after IE was written", 0),
    (8, "ARM7 IF after writing FFFF_FFFF", 0),
];

#[test]
fn given_irq_rom_when_run_then_the_arm9_if_stays_zero_though_ones_were_written_to_it() {
    assert_eq!(arm9_mismatches(ARM9_IF), Vec::<Mismatch>::new());
}

#[test]
fn given_irq_rom_when_run_then_the_arm7_if_stays_zero_though_ones_were_written_to_it() {
    assert_eq!(arm7_mismatches(ARM7_IF), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ independence

const ARM9_INDEPENDENCE: &[Row] = &[
    (11, "ARM9 IE right after writing 5555_5555", 0x0015_1555),
    // After the ARM7 wrote its own pattern, the ARM9's IE is unchanged.
    (10, "ARM9 IE after the ARM7 wrote its IE", 0x0015_1555),
];
const ARM7_INDEPENDENCE: &[Row] = &[
    // The ARM9 had written its pattern; the ARM7 never wrote its IE.
    (10, "ARM7 IE after the ARM9 wrote its IE", 0),
    (11, "ARM7 IE right after writing AAAA_AAAA", 0x008A_2AAA),
];

#[test]
fn given_irq_rom_when_run_then_the_arm9_ie_is_unchanged_by_the_arm7_writing_its_own() {
    assert_eq!(arm9_mismatches(ARM9_INDEPENDENCE), Vec::<Mismatch>::new());
}

#[test]
fn given_irq_rom_when_run_then_the_arm7_ie_is_unchanged_by_the_arm9_writing_its_own() {
    assert_eq!(arm7_mismatches(ARM7_INDEPENDENCE), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ progress and determinism

const ARM9_DONE: &[Row] = &[(14, "done", 1), (15, "sentinel", 0x4952_5139)];
const ARM7_DONE: &[Row] = &[(14, "done", 1), (15, "sentinel", 0x4952_5137)];

#[test]
fn given_irq_rom_when_run_two_frames_then_both_cpus_finished_their_sequences() {
    let machine = boot();
    assert_eq!(
        (
            mismatches(&machine, ARM9_RECORD, ARM9_DONE),
            mismatches(&machine, ARM7_RECORD, ARM7_DONE)
        ),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

#[test]
fn given_irq_rom_when_run_twice_then_the_final_savestate_bytes_match() {
    ninge_runner::golden::assert_deterministic("irq", 2);
}

#[test]
fn given_irq_rom_when_resumed_from_a_savestate_after_1_frame_then_it_matches_an_uninterrupted_run()
{
    ninge_runner::golden::assert_savestate_round_trip("irq", 1, 2);
}
