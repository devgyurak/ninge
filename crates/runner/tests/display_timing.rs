// SPDX-License-Identifier: Apache-2.0
//! Display timing (VCOUNT, DISPSTAT, VBlank, HBlank), observed through our own `disptime` test
//! ROM (`testroms/disptime/`). Each CPU records what it sees of its own DISPSTAT, VCOUNT and IF
//! in main RAM (ARM9 record at 0x0200_4000, ARM7 at 0x0200_5000; the slot layout is at the top
//! of `arm9.s` and `arm7.s`), and the VCOUNT values it polled over one frame (halfwords at
//! 0x0201_0000 and 0x0201_2000). Expected values come from `docs/hw/display-timing.md`
//! ("Frame layout", "VCOUNT", "Flags", "Interrupt requests") and `docs/hw/interrupts.md`
//! ("Behavior": IF latches, a 1 written to an IF bit clears it), never from running the ROM.
//!
//! DISPSTAT bit values: VBlank flag 1, HBlank flag 2, VCount-match flag 4. The ROM clears its
//! DISPSTAT (setting 0) before the flag reads at lines 100, 192, 261, 262 and 0, so line 0 is
//! the only line the setting matches there.
//!
//! The ARM9 also takes the VBlank and VCount IRQs through the vector at 0x18 with the ITCM
//! enabled by the ROM; the ARM7's vector is BIOS (#27), so the ARM7 side checks IF only.

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_RECORD: u32 = 0x0200_4000;
const ARM7_RECORD: u32 = 0x0200_5000;
const ARM9_SEQUENCE: u32 = 0x0201_0000;
const ARM7_SEQUENCE: u32 = 0x0201_2000;
const LINES: u16 = 263;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// Expected record: (slot within the record, name, expected value).
type Row = (u32, &'static str, u32);

/// A machine that booted the `disptime` ROM and ran eight frames (both CPUs finish inside).
fn boot() -> Machine {
    let rom = load_rom_file(&testrom_path("disptime")).expect("disptime ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("disptime ROM has a valid header");
    (0..8).for_each(|_| machine.run_frame());
    machine
}

fn mismatches(machine: &Machine, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    rows.iter()
        .map(|&(slot, name, expected)| (slot, name, expected, machine.arm9_peek32(base + slot * 4)))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

fn arm9_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM9_RECORD, rows)
}

fn arm7_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM7_RECORD, rows)
}

/// The 264 polled VCOUNT values at `base`.
fn sequence(machine: &Machine, base: u32) -> Vec<u16> {
    (0..264)
        .map(|i| machine.arm9_peek16(base + i * 2))
        .collect()
}

/// Positions where a polled value is not the previous one plus one, wrapping 262 to 0.
fn sequence_breaks(values: &[u16]) -> Vec<(usize, u16, u16)> {
    values
        .windows(2)
        .enumerate()
        .map(|(i, pair)| (i + 1, (pair[0] + 1) % LINES, pair[1]))
        .filter(|&(_, expected, got)| expected != got)
        .collect()
}

// ------------------------------------------------------------------ VCOUNT

#[test]
fn given_disptime_rom_when_run_then_the_arm9_polls_vcount_up_by_one_per_line_and_wraps_262_to_0() {
    let values = sequence(&boot(), ARM9_SEQUENCE);
    assert_eq!(sequence_breaks(&values), Vec::<(usize, u16, u16)>::new());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm7_polls_vcount_up_by_one_per_line_and_wraps_262_to_0() {
    let values = sequence(&boot(), ARM7_SEQUENCE);
    assert_eq!(sequence_breaks(&values), Vec::<(usize, u16, u16)>::new());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm9_sees_all_263_lines_in_one_frame() {
    let mut lines: Vec<u16> = sequence(&boot(), ARM9_SEQUENCE)[..263].to_vec();
    lines.sort_unstable();
    assert_eq!(lines, (0..LINES).collect::<Vec<u16>>());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm7_sees_all_263_lines_in_one_frame() {
    let mut lines: Vec<u16> = sequence(&boot(), ARM7_SEQUENCE)[..263].to_vec();
    lines.sort_unstable();
    assert_eq!(lines, (0..LINES).collect::<Vec<u16>>());
}

// ------------------------------------------------------------------ VBlank flag

// "VBlank flag: set in lines 192-261, not in line 262." The match flag (4) is set at line 0
// because the setting is 0 there.
const ARM9_VBLANK: &[Row] = &[
    (1, "ARM9 DISPSTAT at line 100", 0),
    (2, "ARM9 DISPSTAT at line 192", 1),
    (3, "ARM9 DISPSTAT at line 261", 1),
    (4, "ARM9 DISPSTAT at line 262", 0),
    (5, "ARM9 DISPSTAT at line 0", 4),
];
const ARM7_VBLANK: &[Row] = &[
    (1, "ARM7 DISPSTAT at line 100", 0),
    (2, "ARM7 DISPSTAT at line 192", 1),
    (3, "ARM7 DISPSTAT at line 261", 1),
    (4, "ARM7 DISPSTAT at line 262", 0),
    (5, "ARM7 DISPSTAT at line 0", 4),
];

#[test]
fn given_disptime_rom_when_run_then_the_arm9_vblank_flag_is_set_for_lines_192_to_261_only() {
    assert_eq!(arm9_mismatches(ARM9_VBLANK), Vec::<Mismatch>::new());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm7_vblank_flag_is_set_for_lines_192_to_261_only() {
    assert_eq!(arm7_mismatches(ARM7_VBLANK), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ HBlank flag

// The flag sets inside line 50 (bit 1 set, VBlank clear) and clears when line 51 starts.
const ARM9_HBLANK: &[Row] = &[
    (6, "ARM9 DISPSTAT when HBlank is first seen", 2),
    (7, "ARM9 VCOUNT when HBlank is first seen", 50),
    (8, "ARM9 VCOUNT when HBlank clears", 51),
];
const ARM7_HBLANK: &[Row] = &[
    (6, "ARM7 DISPSTAT when HBlank is first seen", 2),
    (7, "ARM7 VCOUNT when HBlank is first seen", 50),
    (8, "ARM7 VCOUNT when HBlank clears", 51),
];

#[test]
fn given_disptime_rom_when_run_then_the_arm9_hblank_flag_sets_inside_a_line_and_clears_at_the_next()
{
    assert_eq!(arm9_mismatches(ARM9_HBLANK), Vec::<Mismatch>::new());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm7_hblank_flag_sets_inside_a_line_and_clears_at_the_next()
{
    assert_eq!(arm7_mismatches(ARM7_HBLANK), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ VCount-match flag

// The setting 100 is in bits 8-15 (0x6400); the flag (4) is set only in line 100.
const ARM9_MATCH: &[Row] = &[
    (9, "ARM9 DISPSTAT at line 99", 0x6400),
    (10, "ARM9 DISPSTAT at line 100", 0x6404),
    (11, "ARM9 DISPSTAT at line 101", 0x6400),
];
const ARM7_MATCH: &[Row] = &[
    (9, "ARM7 DISPSTAT at line 99", 0x6400),
    (10, "ARM7 DISPSTAT at line 100", 0x6404),
    (11, "ARM7 DISPSTAT at line 101", 0x6400),
];

#[test]
fn given_disptime_rom_when_run_then_the_arm9_vcount_match_flag_is_set_only_in_the_set_line() {
    assert_eq!(arm9_mismatches(ARM9_MATCH), Vec::<Mismatch>::new());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm7_vcount_match_flag_is_set_only_in_the_set_line() {
    assert_eq!(arm7_mismatches(ARM7_MATCH), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ DISPSTAT writes

// "Writable bits are 3, 4, 5, 7 and 8-15": FFFF reads back FFB8 (no flag is set at line 5).
const ARM9_WRITE: &[Row] = &[(12, "ARM9 DISPSTAT after writing FFFF", 0xFFB8)];
const ARM7_WRITE: &[Row] = &[(12, "ARM7 DISPSTAT after writing FFFF", 0xFFB8)];

#[test]
fn given_disptime_rom_when_run_then_the_arm9_dispstat_keeps_only_the_writable_bits() {
    assert_eq!(arm9_mismatches(ARM9_WRITE), Vec::<Mismatch>::new());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm7_dispstat_keeps_only_the_writable_bits() {
    assert_eq!(arm7_mismatches(ARM7_WRITE), Vec::<Mismatch>::new());
}

// TODO(hw-unverified): DISPSTAT is not stated at reset; the note models 0
// (`display-timing.md`, "Unverified").
#[test]
fn given_disptime_rom_when_run_then_both_cpus_read_zero_from_dispstat_before_writing_it_as_the_note_models_unverified()
 {
    let machine = boot();
    assert_eq!(
        (
            mismatches(&machine, ARM9_RECORD, &[(0, "ARM9 DISPSTAT at reset", 0)]),
            mismatches(&machine, ARM7_RECORD, &[(0, "ARM7 DISPSTAT at reset", 0)])
        ),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

// ------------------------------------------------------------------ IF latching

// IME = 0, IE = 7, setting 120, all three enables, IF cleared at line 101. At line 125: HBlank
// (bit 1, every line) and VCount match (bit 2, line 120) are latched, VBlank (bit 0) is not
// yet; writing 2 clears the HBlank bit and leaves bit 2. After clearing IF at line 191, line
// 195 shows VBlank (bit 0, line 192) and HBlank (bit 1); writing 1 leaves bit 1.
const ARM9_IF: &[Row] = &[
    (13, "ARM9 IF at line 125", 0b110),
    (14, "ARM9 IF after writing 2", 0b100),
    (15, "ARM9 IF at line 195", 0b011),
    (16, "ARM9 IF after writing 1", 0b010),
];
const ARM7_IF: &[Row] = &[
    (13, "ARM7 IF at line 125", 0b110),
    (14, "ARM7 IF after writing 2", 0b100),
    (15, "ARM7 IF at line 195", 0b011),
    (16, "ARM7 IF after writing 1", 0b010),
];

#[test]
fn given_disptime_rom_when_run_then_the_arm9_if_latches_hblank_vcount_and_vblank_and_clears_by_writing_1()
 {
    assert_eq!(arm9_mismatches(ARM9_IF), Vec::<Mismatch>::new());
}

#[test]
fn given_disptime_rom_when_run_then_the_arm7_if_latches_hblank_vcount_and_vblank_and_clears_by_writing_1()
 {
    assert_eq!(arm7_mismatches(ARM7_IF), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ IRQ delivery (ARM9)

// With the VBlank and VCount enables, IE = 5 and IME = 1, the vector at 0x18 runs once for the
// VCount match (setting 30) and once for VBlank (line 192) before line 200.
const ARM9_DELIVERY: &[Row] = &[
    (20, "ARM9 IRQs taken between lines 5 and 200", 2),
    (21, "ARM9 IF bits the handler saw", 0b101),
];

#[test]
fn given_disptime_rom_when_run_then_the_arm9_takes_the_vcount_and_vblank_irqs_through_the_vector() {
    assert_eq!(arm9_mismatches(ARM9_DELIVERY), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ progress and determinism

const ARM9_DONE: &[Row] = &[(30, "done", 1), (31, "sentinel", 0x4449_5339)];
const ARM7_DONE: &[Row] = &[(30, "done", 1), (31, "sentinel", 0x4449_5337)];

#[test]
fn given_disptime_rom_when_run_eight_frames_then_both_cpus_finished_their_sequences() {
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
fn given_disptime_rom_when_run_twice_then_the_final_savestate_bytes_match() {
    ninge_runner::golden::assert_deterministic("disptime", 8);
}

#[test]
fn given_disptime_rom_when_resumed_from_a_savestate_after_3_frames_then_it_matches_an_uninterrupted_run()
 {
    ninge_runner::golden::assert_savestate_round_trip("disptime", 3, 5);
}
