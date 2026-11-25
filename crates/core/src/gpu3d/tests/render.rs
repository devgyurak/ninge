// SPDX-License-Identifier: Apache-2.0
//! The rendering registers `0400_0330h`-`0400_03BFh`: stored, masked and write-only. Expected
//! values come from `docs/hw/geometry-engine.md` ("The rendering registers"), whose masks come from
//! GBATEK.

use super::*;

const RDLINES: u32 = 0x0400_0320 - 0x0400_0060;
const EDGE_COLOR: u32 = 0x0400_0330 - 0x0400_0060;
const ALPHA_TEST_REF: u32 = 0x0400_0340 - 0x0400_0060;
const CLEAR_COLOR: u32 = 0x0400_0350 - 0x0400_0060;
const CLEAR_DEPTH: u32 = 0x0400_0354 - 0x0400_0060;
const FOG_COLOR: u32 = 0x0400_0358 - 0x0400_0060;
const FOG_OFFSET: u32 = 0x0400_035C - 0x0400_0060;
const FOG_TABLE: u32 = 0x0400_0360 - 0x0400_0060;
const TOON_TABLE: u32 = 0x0400_0380 - 0x0400_0060;

/// The registers after the writes `(offset, width, value)`, in order.
fn stored(writes: &[(u32, Width, u32)]) -> RenderRegs {
    let mut gpu = Gpu3d::new();
    writes
        .iter()
        .for_each(|&(offset, width, value)| write(&mut gpu, offset, width, value));
    gpu.render_regs().clone()
}

#[test]
fn given_a_halfword_with_bit_15_set_when_written_to_an_edge_colour_then_bit_15_is_cleared() {
    let regs = stored(&[(EDGE_COLOR + 14, Width::Half, 0xFFFF)]);
    assert_eq!(regs.edge_color[7], 0x7FFF);
}

#[test]
fn given_a_byte_write_to_the_high_byte_when_an_edge_colour_is_written_then_the_low_byte_stays() {
    let regs = stored(&[
        (EDGE_COLOR, Width::Half, 0x1234),
        (EDGE_COLOR + 1, Width::Byte, 0x56),
    ]);
    assert_eq!(regs.edge_color[0], 0x5634 & 0x7FFF);
}

#[test]
fn given_a_byte_of_ff_when_written_to_alpha_test_ref_then_five_bits_are_stored() {
    let regs = stored(&[(ALPHA_TEST_REF, Width::Byte, 0xFF)]);
    assert_eq!(regs.alpha_test_ref, 0x1F);
}

#[test]
fn given_a_word_of_ones_when_written_to_clear_color_then_the_unused_bits_are_cleared() {
    let regs = stored(&[(CLEAR_COLOR, Width::Word, u32::MAX)]);
    assert_eq!(regs.clear_color, 0x3F1F_FFFF);
}

#[test]
fn given_a_word_of_ones_when_written_to_clear_depth_then_both_halves_keep_fifteen_bits() {
    let regs = stored(&[(CLEAR_DEPTH, Width::Word, u32::MAX)]);
    assert_eq!((regs.clear_depth, regs.clrimage_offset), (0x7FFF, 0x7FFF));
}

#[test]
fn given_a_word_of_ones_when_written_to_fog_color_then_colour_and_alpha_are_kept() {
    let regs = stored(&[(FOG_COLOR, Width::Word, u32::MAX)]);
    assert_eq!(regs.fog_color, 0x001F_7FFF);
}

#[test]
fn given_ones_when_written_to_fog_offset_then_fifteen_bits_are_stored() {
    let regs = stored(&[(FOG_OFFSET, Width::Half, 0xFFFF)]);
    assert_eq!(regs.fog_offset, 0x7FFF);
}

#[test]
fn given_a_word_of_ones_when_written_to_the_fog_table_then_four_densities_keep_seven_bits() {
    let regs = stored(&[(FOG_TABLE + 4, Width::Word, u32::MAX)]);
    assert_eq!(regs.fog_table[3..9], [0, 0x7F, 0x7F, 0x7F, 0x7F, 0]);
}

#[test]
fn given_a_halfword_of_ones_when_written_to_the_last_toon_entry_then_bit_15_is_cleared() {
    let regs = stored(&[(TOON_TABLE + 62, Width::Half, 0xFFFF)]);
    assert_eq!(regs.toon_table[31], 0x7FFF);
}

#[test]
fn given_a_write_to_rdlines_count_when_it_is_read_then_it_still_reads_46() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, RDLINES, Width::Byte, 5);
    assert_eq!(read(&gpu, RDLINES, Width::Byte), 46);
}

#[test]
fn given_a_register_written_when_it_is_read_then_it_reads_zero() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, CLEAR_COLOR, Width::Word, u32::MAX);
    assert_eq!(read(&gpu, CLEAR_COLOR, Width::Word), 0);
}

#[test]
fn given_the_rendering_engine_unpowered_when_a_register_is_written_then_the_write_is_dropped() {
    let mut gpu = Gpu3d::new();
    let mut cx = cx_at(0);
    cx.lines.rendering_powered = false;
    gpu.write(CLEAR_COLOR, Width::Word, u32::MAX, cx);
    assert_eq!(gpu.render_regs().clear_color, 0);
}

#[test]
fn given_a_register_with_an_unused_bit_set_when_checked_then_the_state_is_not_well_formed() {
    let mut gpu = Gpu3d::new();
    gpu.render.fog_table[0] = 0x80;
    assert!(!gpu.well_formed(StateTimes::at(0)));
}
