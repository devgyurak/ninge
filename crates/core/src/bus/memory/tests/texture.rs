// SPDX-License-Identifier: Apache-2.0
//! The texture slots and their reader (`docs/hw/vram-banks.md`). The banks sit one after another in
//! the VRAM buffer (A at 0, B at 0x2_0000, ..., E at 0x8_0000, F at 0x9_0000, G at 0x9_4000); a
//! texture image slot is a 128 KiB bank and a texture palette slot 16 KiB.

use super::vram::set;
use super::*;

const VRAMCNT_A: u32 = 0x0400_0240;
const VRAMCNT_B: u32 = 0x0400_0241;
const VRAMCNT_E: u32 = 0x0400_0244;
const VRAMCNT_F: u32 = 0x0400_0245;
const VRAMCNT_G: u32 = 0x0400_0246;
const B_AT: usize = 0x2_0000;
const E_AT: usize = 0x8_0000;
const F_AT: usize = 0x9_0000;
const G_AT: usize = 0x9_4000;

/// A VRAMCNT value: enabled, with `mst` and `ofs`.
fn cnt(mst: u8, ofs: u8) -> u8 {
    0x80 | mst | ofs << 3
}

/// A memory with `bytes` written at VRAM buffer offsets, then the VRAMCNT writes applied.
fn with(writes: &[(u32, u8)], bytes: &[(usize, u8)]) -> Memory {
    let mut memory = Memory::new();
    bytes.iter().for_each(|&(at, byte)| memory.vram[at] = byte);
    writes
        .iter()
        .for_each(|&(reg, value)| set(&mut memory, reg, value));
    memory
}

#[test]
fn given_bank_b_as_texture_slot_2_when_read_then_its_bytes_are_at_the_slot() {
    let memory = with(&[(VRAMCNT_B, cnt(3, 2))], &[(B_AT + 5, 0xAB)]);
    assert_eq!(memory.textures().image_byte(2 * 0x2_0000 + 5), 0xAB);
}

#[test]
fn given_no_bank_for_a_slot_when_read_then_it_reads_zero() {
    let memory = with(&[(VRAMCNT_A, cnt(3, 0))], &[(B_AT, 0xFF)]);
    assert_eq!(memory.textures().image_byte(0x2_0000), 0);
}

#[test]
fn given_an_offset_past_the_image_space_when_read_then_it_reads_zero() {
    let memory = with(&[(VRAMCNT_A, cnt(3, 0))], &[(0, 0xFF)]);
    assert_eq!(memory.textures().image_byte(4 * 0x2_0000), 0);
}

#[test]
fn given_bank_e_as_texture_palette_when_read_then_it_backs_the_first_four_slots() {
    let memory = with(
        &[(VRAMCNT_E, cnt(3, 0))],
        &[(E_AT + 0xC000, 0x34), (E_AT + 0xC001, 0x12)],
    );
    assert_eq!(memory.textures().palette_color(0xC000), 0x1234);
}

#[test]
fn given_bank_f_with_ofs_1_when_read_then_it_backs_palette_slot_1() {
    let memory = with(&[(VRAMCNT_F, cnt(3, 1))], &[(F_AT, 0x78), (F_AT + 1, 0x56)]);
    assert_eq!(memory.textures().palette_color(0x4000), 0x5678);
}

#[test]
fn given_bank_g_with_ofs_2_when_read_then_it_backs_palette_slot_4() {
    let memory = with(
        &[(VRAMCNT_G, cnt(3, 2))],
        &[(G_AT + 2, 0xCD), (G_AT + 3, 0x0A)],
    );
    assert_eq!(memory.textures().palette_color(4 * 0x4000 + 2), 0x0ACD);
}

#[test]
fn given_a_palette_slot_no_bank_backs_when_read_then_the_colour_is_zero() {
    let memory = with(&[(VRAMCNT_F, cnt(3, 0))], &[(F_AT, 0xFF)]);
    assert_eq!(memory.textures().palette_color(0x4000), 0);
}

#[test]
fn given_the_largest_offset_when_a_palette_colour_is_read_then_it_reads_zero_without_panicking() {
    let memory = with(&[(VRAMCNT_E, cnt(3, 0))], &[(E_AT, 0xFF)]);
    assert_eq!(memory.textures().palette_color(u32::MAX), 0);
}

#[test]
fn given_an_odd_offset_when_a_palette_colour_is_read_then_it_is_the_colour_that_holds_it() {
    let memory = with(
        &[(VRAMCNT_F, cnt(3, 0))],
        &[(F_AT + 0x3FFE, 0x34), (F_AT + 0x3FFF, 0x12)],
    );
    assert_eq!(memory.textures().palette_color(0x3FFF), 0x1234);
}

#[test]
fn given_two_banks_backing_one_image_slot_when_read_then_the_earlier_letter_wins() {
    let memory = with(
        &[(VRAMCNT_A, cnt(3, 1)), (VRAMCNT_B, cnt(3, 1))],
        &[(0, 0x11), (B_AT, 0x22)],
    );
    assert_eq!(memory.textures().image_byte(0x2_0000), 0x11);
}

#[test]
fn given_two_banks_backing_one_palette_slot_when_read_then_the_earlier_letter_wins() {
    let memory = with(
        &[(VRAMCNT_F, cnt(3, 1)), (VRAMCNT_G, cnt(3, 1))],
        &[(F_AT, 0x11), (G_AT, 0x22)],
    );
    assert_eq!(memory.textures().palette_color(0x4000), 0x0011);
}

#[test]
fn given_an_offset_past_the_palette_space_when_read_then_it_reads_colour_zero() {
    let memory = with(&[(VRAMCNT_E, cnt(3, 0))], &[(E_AT, 0xFF)]);
    assert_eq!(memory.textures().palette_color(6 * 0x4000), 0);
}

#[test]
fn given_a_texture_reader_when_formatted_with_debug_then_the_output_is_short() {
    let memory = with(&[(VRAMCNT_A, cnt(3, 1))], &[(0, 0x11)]);
    let text = format!("{:?}", memory.textures());
    assert!(text.len() < 1_000, "{} bytes", text.len());
}
