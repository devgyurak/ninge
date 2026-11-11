// SPDX-License-Identifier: Apache-2.0
//! The HLE BIOS observed through our own `swi` test ROM (`testroms/swi/`): VBlank IRQs through the
//! BIOS dispatcher, SWI calls from ARM and Thumb state, VBlankIntrWait, SWI Halt and each CPU's
//! own halt (the ARM9's CP15 wait-for-interrupt, the ARM7's HALTCNT), and the data SWIs (CpuSet,
//! CpuFastSet, Div, Sqrt, GetCRC16, IsDebugger, LZ77 in both forms), on both CPUs. Each CPU records what it
//! saw in main RAM (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; the slot layout is at the top of
//! `arm9.s` and `arm7.s`). Expected values come from `docs/hw/hle-swi.md` ("SWI", "The IRQ
//! dispatcher", "Edge cases"), never from running the ROM. This is the first test ROM that takes
//! an ARM7 interrupt in the ROM instead of only latching IF.
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

/// A machine that booted the `swi` ROM and ran twelve frames: three VBlank IRQs per CPU take three
/// frames, and VBlankIntrWait and the three halts a frame each; the rest finishes inside the last.
fn boot() -> Machine {
    let rom = load_rom_file(&testrom_path("swi")).expect("swi ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("swi ROM has a valid header");
    (0..12).for_each(|_| machine.run_frame());
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

// ------------------------------------------------------------------ the ROM finished

const DONE: &[Row] = &[(30, "progress", 1), (31, "sentinel", 0x600D_F00D)];

#[test]
fn given_swi_rom_when_run_then_the_arm9_reaches_its_end() {
    assert_eq!(arm9_mismatches(DONE), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_reaches_its_end() {
    assert_eq!(arm7_mismatches(DONE), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ the IRQ dispatcher

/// "The dispatcher saves R0-R3, R12 and LR and restores them after the handler" (`hle-swi.md`);
/// the handler took the three VBlank IRQs, in IRQ mode with I set (CPSR low byte 92h).
const DISPATCH_COMMON: &[Row] = &[
    (0, "IRQs the user handler took", 3),
    (1, "r0 across the IRQs", 0x11),
    (2, "r1 across the IRQs", 0x22),
    (3, "r2 across the IRQs", 0x33),
    (4, "r3 across the IRQs", 0x44),
    (5, "r12 across the IRQs", 0x55),
    (6, "CPSR low byte inside the user handler", 0x92),
];

#[test]
fn given_swi_rom_when_run_then_the_arm9_handler_takes_three_irqs_and_the_dispatcher_keeps_the_registers()
 {
    assert_eq!(arm9_mismatches(DISPATCH_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_handler_takes_three_irqs_and_the_dispatcher_keeps_the_registers()
 {
    assert_eq!(arm7_mismatches(DISPATCH_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_sp_irq_is_back_where_each_cpu_set_it() {
    assert_eq!(
        arm9_mismatches(&[(8, "ARM9 SP_irq after the handlers", 0x027C_3F00)]),
        Vec::<Mismatch>::new()
    );
    assert_eq!(
        arm7_mismatches(&[(8, "ARM7 SP_irq after the handlers", 0x0380_FF80)]),
        Vec::<Mismatch>::new()
    );
}

// TODO(hw-unverified): R0 at the user handler is the GBA BIOS's value carried over
// (`hle-swi.md`, "Unverified").
const HANDLER_R0: &[Row] = &[(7, "r0 at the user handler's entry", 0x0400_0000)];

#[test]
fn given_swi_rom_when_run_then_r0_at_the_arm9_handler_is_the_io_base_as_the_note_models_unverified()
{
    assert_eq!(arm9_mismatches(HANDLER_R0), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_r0_at_the_arm7_handler_is_the_io_base_as_the_note_models_unverified()
{
    assert_eq!(arm7_mismatches(HANDLER_R0), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ SWI

/// "The return restores the CPSR (flags, T) and continues at the next instruction"
/// (`hle-swi.md`, "SWI"); the number 1Eh is not implemented and has no effect.
const SWI_COMMON: &[Row] = &[
    (10, "ARM SWI returned to the next instruction", 1),
    (11, "Thumb SWI returned to the next Thumb instruction", 1),
    (12, "CPSR flags nibble after the ARM SWI", 0xA),
];

#[test]
fn given_swi_rom_when_run_then_the_arm9_returns_from_arm_and_thumb_swis_with_the_flags_kept() {
    assert_eq!(arm9_mismatches(SWI_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_returns_from_arm_and_thumb_swis_with_the_flags_kept() {
    assert_eq!(arm7_mismatches(SWI_COMMON), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_debugger_record_names_the_unimplemented_number_on_both_cpus() {
    let machine = boot();
    assert_eq!(
        (
            machine.inspect().swi_record9().last_unimplemented(),
            machine.inspect().swi_record7().last_unimplemented()
        ),
        (Some(0x1E), Some(0x1E))
    );
}

// ------------------------------------------------------------------ the wait SWIs and halt

/// "VBlankIntrWait forces IME to 1, waits for the next V-blank and returns to the caller with its
/// CPSR" (`hle-swi.md`, "The wait SWIs"): VCOUNT is in the V-blank lines when it returns (192 and
/// up), the handler has served one more IRQ (the fourth), IME reads 1 and the CPSR low byte is
/// the caller's (System mode, I set, F clear: 9Fh).
const VBLANK_INTR_WAIT: &[Row] = &[
    (13, "VCOUNT >= 192 after VBlankIntrWait", 1),
    (14, "IRQs the handler took by then", 4),
    (15, "IME after VBlankIntrWait", 1),
    (16, "CPSR low byte after VBlankIntrWait", 0x9F),
];

#[test]
fn given_swi_rom_when_run_then_the_arm9_vblankintrwait_waits_for_vblank_and_returns_to_its_caller()
{
    assert_eq!(arm9_mismatches(VBLANK_INTR_WAIT), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_vblankintrwait_waits_for_vblank_and_returns_to_its_caller()
{
    assert_eq!(arm7_mismatches(VBLANK_INTR_WAIT), Vec::<Mismatch>::new());
}

// TODO(hw-unverified): the wake depends on IE & IF only, whatever IME and the I flag say
// (`hle-swi.md`, "Halt, and what wakes a CPU", "Unverified").
const HALT_WAKES: &[Row] = &[
    (17, "IF bit 0 on waking from SWI Halt, IME 1, I set", 1),
    (18, "IF bit 0 on waking from SWI Halt, IME 0", 1),
    (19, "IF bit 0 on waking from the CPU's own halt, IME 0", 1),
];

#[test]
fn given_swi_rom_when_run_then_the_arm9_wakes_from_swi_halt_and_wfi_on_ie_and_if_as_the_note_models_unverified()
 {
    assert_eq!(arm9_mismatches(HALT_WAKES), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_wakes_from_swi_halt_and_haltcnt_on_ie_and_if_as_the_note_models_unverified()
 {
    assert_eq!(arm7_mismatches(HALT_WAKES), Vec::<Mismatch>::new());
}

#[test]
fn given_swi_rom_when_run_then_neither_cpu_ends_halted() {
    let machine = boot();
    assert_eq!(
        (
            machine.inspect().arm9().halted(),
            machine.inspect().arm7().halted()
        ),
        (false, false)
    );
}

// ------------------------------------------------------------------ the data SWIs

const ARM9_RESULTS: u32 = 0x0200_8000;
const ARM7_RESULTS: u32 = 0x0200_9000;
/// The result area starts filled with AAh bytes (`arm9.s`).
const FILLER: u32 = 0xAAAA_AAAA;

/// A mismatch in the result area: (byte offset, name, expected, got).
type ResultMismatch = (u32, &'static str, u32, u32);

/// Expected result words: (byte offset, name, value).
type ResultRow = (u32, &'static str, u32);

fn result_mismatches(base: u32, rows: &[ResultRow]) -> Vec<ResultMismatch> {
    let machine = boot();
    rows.iter()
        .map(|&(offset, name, expected)| {
            (
                offset,
                name,
                expected,
                machine.inspect().arm9_peek32(base + offset),
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

/// CRC-16/MODBUS, bit by bit from the note (`cart-header.md`), for the ASCII digits.
fn crc_of_digits() -> u32 {
    let crc = b"12345678".iter().fold(0xFFFFu16, |mut crc, &byte| {
        crc ^= u16::from(byte);
        (0..8).for_each(|_| crc = (crc >> 1) ^ (0xA001 * (crc & 1)));
        crc
    });
    u32::from(crc)
}

/// The rows both CPUs share (`hle-swi.md`, "The copy, math and decompression SWIs"). Everything
/// is derived from the note: units copied or filled, the fast set's round-up to 8 words,
/// truncating signed division, the floor square root, the CRC and the LZ77 stream "ABABABABAB".
fn shared_results() -> Vec<ResultRow> {
    let mut rows: Vec<ResultRow> = vec![
        (0x00, "CpuSet copy16, first word", 0x2222_1111),
        (0x04, "CpuSet copy16, third unit only", 0xAAAA_3333),
        (0x10, "CpuSet fill32, first word", 0xDEAD_BEEF),
        (0x14, "CpuSet fill32, second word", 0xDEAD_BEEF),
        (0x18, "CpuSet fill32, third word", 0xDEAD_BEEF),
        (0x1C, "CpuSet fill32, fourth word untouched", FILLER),
        (
            0x60,
            "CpuFastSet copy of 9 words rounds to 16: the 17th is untouched",
            FILLER,
        ),
        (
            0xA0,
            "CpuFastSet fill of 1 word writes 8: the 9th is untouched",
            FILLER,
        ),
        (0xC0, "CpuSet with a count of 0 writes nothing", FILLER),
        (0xD0, "Div -7 / 2 quotient", 0xFFFF_FFFD),
        (0xD4, "Div -7 / 2 remainder", 0xFFFF_FFFF),
        (0xD8, "Div -7 / 2 absolute quotient", 3),
        (0xDC, "Div 7 / -2 quotient", 0xFFFF_FFFD),
        (0xE0, "Div 7 / -2 remainder", 1),
        (0xE4, "Div 7 / -2 absolute quotient", 3),
        (0xF0, "Sqrt of 1000000", 1000),
        (0xF4, "Sqrt of FFFFFFFFh", 0xFFFF),
        (0xF8, "Sqrt of 0", 0),
        (0x100, "GetCRC16 of the digits from FFFFh", crc_of_digits()),
        (0x104, "IsDebugger", 0),
        (0x110, "LZ77 byte writes, bytes 0-3", 0x4241_4241),
        (0x114, "LZ77 byte writes, bytes 4-7", 0x4241_4241),
        (
            0x118,
            "LZ77 byte writes, bytes 8-9 then untouched",
            0xAAAA_4241,
        ),
        (0x11C, "LZ77 byte writes, nothing past the size", FILLER),
        (0x130, "LZ77 halfword writes, bytes 0-3", 0x4241_4241),
        (0x134, "LZ77 halfword writes, bytes 4-7", 0x4241_4241),
        (
            0x138,
            "LZ77 halfword writes, bytes 8-9 then untouched",
            0xAAAA_4241,
        ),
        (0x13C, "LZ77 halfword writes, nothing past the size", FILLER),
    ];
    // CpuFastSet copy: the source words 1..=16 land at +20h.
    rows.extend((0..16).map(|i| (0x20 + 4 * i, "CpuFastSet copy word", i + 1)));
    // CpuFastSet fill: eight words of DEADBEEFh at +80h.
    rows.extend((0..8).map(|i| (0x80 + 4 * i, "CpuFastSet fill word", 0xDEAD_BEEF)));
    rows
}

#[test]
fn given_swi_rom_when_run_then_the_arm9_data_swis_leave_the_results_the_note_describes() {
    assert_eq!(
        result_mismatches(ARM9_RESULTS, &shared_results()),
        Vec::<ResultMismatch>::new()
    );
}

#[test]
fn given_swi_rom_when_run_then_the_arm7_data_swis_leave_the_results_the_note_describes() {
    assert_eq!(
        result_mismatches(ARM7_RESULTS, &shared_results()),
        Vec::<ResultMismatch>::new()
    );
}

/// "The 8-bit form loses its writes to VRAM, the 16-bit form lands" (`hle-swi.md`, "LZ77").
const VRAM_RESULTS: &[ResultRow] = &[
    (0x150, "VRAM first word after a byte-write LZ77", 0),
    (
        0x160,
        "VRAM words 0-3 after a halfword-write LZ77",
        0x4241_4241,
    ),
    (
        0x164,
        "VRAM words 4-7 after a halfword-write LZ77",
        0x4241_4241,
    ),
    (
        0x168,
        "VRAM words 8-9 after a halfword-write LZ77",
        0x0000_4241,
    ),
];

#[test]
fn given_swi_rom_when_run_then_lz77_to_vram_loses_byte_writes_and_keeps_halfword_writes() {
    assert_eq!(
        result_mismatches(ARM9_RESULTS, VRAM_RESULTS),
        Vec::<ResultMismatch>::new()
    );
}
