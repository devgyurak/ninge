// SPDX-License-Identifier: Apache-2.0
//! The copy, math, checksum and decompression SWIs through `Machine`: the number is found in the
//! instruction, the arguments are the registers, and memory is reached through the calling CPU's
//! own bus view (I/O registers and VRAM included). Expected values come from
//! `docs/hw/hle-swi.md` ("The copy, math and decompression SWIs"), never from running the code.

use super::bios::{PROGRAM, arm_swi, arm7_running, arm9_running, step7, step9};
use crate::cpu::Bus;
use crate::machine::*;

const SRC: u32 = 0x0200_0800;
const DST: u32 = 0x0200_0900;
const IME: u32 = 0x0400_0208;
const VRAMCNT_A: u32 = 0x0400_0240;
const VRAM_A: u32 = 0x0680_0000;
/// Control bits: fill and 32-bit units.
const FILL: u32 = 1 << 24;
const WORD: u32 = 1 << 26;
/// `mov r6, #0x99`, run when the SWI has returned.
const MOV_R6: u32 = 0xE3A0_6099;
const SPIN: u32 = PROGRAM[2];

/// An ARM9 that has run `swi number` with `r0`..`r3` set, stopped after the SWI returned.
fn arm9_after(number: u32, regs: [u32; 4]) -> Machine {
    let mut m = arm9_running(&[arm_swi(number << 16), MOV_R6, SPIN]);
    regs.iter()
        .enumerate()
        .for_each(|(i, &v)| m.arm9.regs_mut().set(i, v));
    step9(&mut m, 2);
    m
}

fn arm7_after(number: u32, regs: [u32; 4]) -> Machine {
    let mut m = arm7_running(&[arm_swi(number << 16), MOV_R6, SPIN]);
    regs.iter()
        .enumerate()
        .for_each(|(i, &v)| m.arm7.regs_mut().set(i, v));
    step7(&mut m, 2);
    m
}

fn r9(m: &Machine) -> [u32; 4] {
    [0, 1, 2, 3].map(|i| m.arm9().regs().get(i))
}

// ------------------------------------------------------------------------------------ Div

#[test]
fn given_swi_div_when_minus_7_is_divided_by_2_then_r0_r1_and_r3_hold_quotient_remainder_and_absolute()
 {
    let m = arm9_after(0x09, [-7i32 as u32, 2, 0, 0]);
    assert_eq!(r9(&m), [-3i32 as u32, -1i32 as u32, 0, 3]);
}

#[test]
fn given_swi_div_on_the_arm7_when_7_is_divided_by_minus_2_then_the_results_are_in_the_arm7_registers()
 {
    let m = arm7_after(0x09, [7, -2i32 as u32, 0, 0]);
    let regs = [0, 1, 2, 3].map(|i| m.arm7().regs().get(i));
    assert_eq!(regs, [-3i32 as u32, 1, 0, 3]);
}

// ------------------------------------------------------------------------------------ Sqrt

#[test]
fn given_swi_sqrt_when_a_million_is_given_then_r0_is_1000() {
    assert_eq!(r9(&arm9_after(0x0D, [1_000_000, 7, 8, 9]))[0], 1000);
}

#[test]
fn given_swi_sqrt_when_the_largest_value_is_given_then_r0_is_ffffh_and_r1_to_r3_are_left() {
    assert_eq!(
        r9(&arm9_after(0x0D, [u32::MAX, 7, 8, 9])),
        [0xFFFF, 7, 8, 9]
    );
}

// ------------------------------------------------------------------------ CpuSet, CpuFastSet

fn arm9_with_source(words: &[u32], number: u32, regs: [u32; 4]) -> Machine {
    let mut m = arm9_running(&[arm_swi(number << 16), MOV_R6, SPIN]);
    words
        .iter()
        .enumerate()
        .for_each(|(i, &w)| m.bus9().write32(SRC + 4 * i as u32, w));
    regs.iter()
        .enumerate()
        .for_each(|(i, &v)| m.arm9.regs_mut().set(i, v));
    step9(&mut m, 2);
    m
}

#[test]
fn given_swi_cpuset_when_a_word_is_copied_to_ime_then_the_write_reaches_the_register() {
    let m = arm9_with_source(&[1], 0x0B, [SRC, IME, WORD | 1, 0]);
    assert!(m.irq9().ime());
}

#[test]
fn given_swi_cpuset_when_it_returns_then_r0_to_r2_are_unchanged_and_the_caller_continues() {
    let mut m = arm9_with_source(&[5, 6], 0x0B, [SRC, DST, WORD | 2, 0]);
    step9(&mut m, 1);
    assert_eq!(
        (r9(&m)[..3].to_vec(), m.arm9().regs().get(6)),
        (vec![SRC, DST, WORD | 2], 0x99)
    );
}

#[test]
fn given_swi_cpuset_when_16_bit_units_are_copied_then_the_destination_holds_them() {
    let m = arm9_with_source(&[0x4444_3333, 0x2222_1111], 0x0B, [SRC, DST, 3, 0]);
    assert_eq!(
        (m.arm9_peek32(DST), m.arm9_peek16(DST + 4)),
        (0x4444_3333, 0x1111)
    );
}

#[test]
fn given_swi_cpuset_when_a_word_is_filled_then_count_words_hold_it() {
    let m = arm9_with_source(&[0xCAFE_F00D], 0x0B, [SRC, DST, FILL | WORD | 3, 0]);
    let words = [0, 4, 8, 12].map(|o| m.arm9_peek32(DST + o));
    assert_eq!(words, [0xCAFE_F00D, 0xCAFE_F00D, 0xCAFE_F00D, 0]);
}

#[test]
fn given_swi_cpufastset_when_nine_words_are_copied_then_sixteen_move() {
    let source: Vec<u32> = (1..=20).collect();
    let m = arm9_with_source(&source, 0x0C, [SRC, DST, 9, 0]);
    let words = [60, 64].map(|o| m.arm9_peek32(DST + o));
    assert_eq!(words, [16, 0]);
}

/// An ARM7 that has run `swi number` with `r0`, `r1` and `r2` set, after `word` was written at
/// `SRC` through its own view.
fn arm7_with_source(word: u32, number: u32, regs: [u32; 3]) -> Machine {
    let mut m = arm7_running(&[arm_swi(number << 16), MOV_R6, SPIN]);
    m.bus7(0).write32(SRC, word);
    set_arm7_registers(&mut m, &regs);
    step7(&mut m, 2);
    m
}

fn set_arm7_registers(m: &mut Machine, regs: &[u32]) {
    regs.iter()
        .enumerate()
        .for_each(|(i, &v)| m.arm7.regs_mut().set(i, v));
}

#[test]
fn given_swi_cpuset_on_the_arm7_when_words_are_copied_then_the_arm7_view_receives_them() {
    let m = arm7_with_source(0x1234_5678, 0x0B, [SRC, DST, WORD | 1]);
    assert_eq!(m.arm7_peek32(0, DST), 0x1234_5678);
}

// ------------------------------------------------------------------------------- GetCRC16

/// An ARM9 that has run GetCRC16 over "12345678" (bytes 31h-38h) from FFFFh.
fn arm9_after_getcrc16() -> Machine {
    let mut m = arm9_running(&[arm_swi(0x0E << 16), MOV_R6, SPIN]);
    m.bus9().write32(SRC, 0x3433_3231);
    m.bus9().write32(SRC + 4, 0x3837_3635);
    set_arm9_registers(&mut m, &[0xFFFF, SRC, 8]);
    step9(&mut m, 2);
    m
}

fn set_arm9_registers(m: &mut Machine, regs: &[u32]) {
    regs.iter()
        .enumerate()
        .for_each(|(i, &v)| m.arm9.regs_mut().set(i, v));
}

#[test]
fn given_swi_getcrc16_when_the_ascii_digits_are_summed_then_r0_is_the_crc_continued_from_r0() {
    // The CRC-16/MODBUS of "12345678" from FFFFh (`cart-header.md`).
    assert_eq!(arm9_after_getcrc16().arm9().regs().get(0), crc_of_digits());
}

/// CRC-16/MODBUS of "12345678", bit by bit.
fn crc_of_digits() -> u32 {
    let crc = b"12345678".iter().fold(0xFFFFu16, |mut crc, &byte| {
        crc ^= u16::from(byte);
        (0..8).for_each(|_| crc = (crc >> 1) ^ (0xA001 * (crc & 1)));
        crc
    });
    u32::from(crc)
}

// ----------------------------------------------------------------------------- IsDebugger

#[test]
fn given_swi_isdebugger_when_called_then_r0_is_zero() {
    assert_eq!(r9(&arm9_after(0x0F, [0x55, 0, 0, 0]))[0], 0);
}

// --------------------------------------------------------------------------------- LZ77

/// The hand-written stream for "ABABABABAB" (`hle-swi.md`, "LZ77"): header (size 10, type 1), flags
/// 0010_0000, `A`, `B`, a block of length 8 at distance 2.
const STREAM: [u32; 3] = [0x0000_0A10, 0x5042_4120, 0x0000_0001];

fn arm9_decoding(number: u32, destination: u32) -> Machine {
    let mut m = arm9_running(&[arm_swi(number << 16), MOV_R6, SPIN]);
    m.bus9().write8(VRAMCNT_A, 0x80);
    STREAM
        .iter()
        .enumerate()
        .for_each(|(i, &w)| m.bus9().write32(SRC + 4 * i as u32, w));
    [(0, SRC), (1, destination)]
        .iter()
        .for_each(|&(i, v)| m.arm9.regs_mut().set(i, v));
    step9(&mut m, 2);
    m
}

#[test]
fn given_swi_lz77_8bit_when_decoding_to_main_ram_then_the_overlapping_block_repeats() {
    let m = arm9_decoding(0x11, DST);
    let bytes = [0, 2, 4, 6, 8].map(|o| m.arm9_peek16(DST + o));
    assert_eq!(bytes, [0x4241; 5]);
}

#[test]
fn given_swi_lz77_16bit_when_decoding_to_vram_then_the_halfword_writes_land() {
    let m = arm9_decoding(0x12, VRAM_A);
    let halfwords = [0, 2, 4, 6, 8].map(|o| m.arm9_peek16(VRAM_A + o));
    assert_eq!(halfwords, [0x4241; 5]);
}

#[test]
fn given_swi_lz77_8bit_when_decoding_to_vram_then_the_byte_writes_are_ignored() {
    let m = arm9_decoding(0x11, VRAM_A);
    assert_eq!(m.arm9_peek16(VRAM_A), 0);
}

// ----------------------------------------------------------------- numbers left unimplemented

#[test]
fn given_the_implemented_data_numbers_when_called_then_none_is_recorded_as_unimplemented() {
    let recorded: Vec<Option<u8>> = [0x09, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x12]
        .iter()
        .map(|&n| {
            arm9_after(n, [0, 0, 0, 0])
                .swi_record9()
                .last_unimplemented()
        })
        .collect();
    assert_eq!(recorded, vec![None; 8]);
}

#[test]
fn given_sleep_soundbias_and_the_other_decompressors_when_called_then_each_is_recorded_unimplemented()
 {
    let recorded: Vec<Option<u8>> = [0x07, 0x08, 0x10, 0x13, 0x14, 0x15]
        .iter()
        .map(|&n| {
            arm7_after(n, [0, 0, 0, 0])
                .swi_record7()
                .last_unimplemented()
        })
        .collect();
    assert_eq!(
        recorded,
        [0x07, 0x08, 0x10, 0x13, 0x14, 0x15].map(Some).to_vec()
    );
}
