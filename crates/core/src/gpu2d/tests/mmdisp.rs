// SPDX-License-Identifier: Apache-2.0
//! The main-memory display FIFO, display mode 3 and capture source B = FIFO (issue #36). Expected
//! values come from `docs/hw/display-capture.md` ("The main-memory display", "The capture
//! pipeline"), worked out by hand. These tests fill the FIFO by hand and render lines directly;
//! the DMA that fills it in a running machine is `machine/tests/mmdisp.rs`.

use super::*;

/// DISP_MMEM_FIFO, DISPCAPCNT, VRAMCNT_B, bank B's window and MASTER_BRIGHT of engine A.
const FIFO: u32 = 0x0400_0068;
const DISPCAPCNT: u32 = 0x0400_0064;
const VRAMCNT_B: u32 = 0x0400_0241;
const LCDC_B: u32 = 0x0682_0000;
const MASTER_BRIGHT_A: u32 = 0x0400_006C;
/// Display mode 3.
const MODE3_ONLY: u32 = 3 << 16;
const ENABLE: u32 = 1 << 31;
/// Write block B, the whole screen, source B (bit 29) from the FIFO (bit 25).
const CAPTURE_FIFO: u32 = ENABLE | (1 << 16) | (3 << 20) | (1 << 29) | (1 << 25);

/// A machine in display mode 3 with engine A on top.
fn mode3_machine() -> Machine {
    let mut m = Machine::new();
    w32(&mut m, DISPCNT_A_ADDR, MODE3_ONLY);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    m
}

/// Pushes `pixels` into the FIFO, two to a word write.
fn push(m: &mut Machine, pixels: &[u16]) {
    pixels.chunks(2).for_each(|pair| {
        w32(
            m,
            FIFO,
            u32::from(pair[0]) | u32::from(*pair.get(1).unwrap_or(&0)) << 16,
        )
    });
}

/// A line of black pixels whose last pixel is blue.
fn line_ending_in_blue() -> Vec<u16> {
    (0..256)
        .map(|x| if x == 255 { 0x7C00 } else { 0 })
        .collect()
}

/// The FIFO's length after `writes` of `(width in bytes, value)` to its register.
fn fifo_len_after(writes: &[(u32, u32)]) -> usize {
    let mut m = Machine::new();
    writes.iter().for_each(|&(width, value)| match width {
        1 => w8(&mut m, FIFO, value as u8),
        2 => w16(&mut m, FIFO, value as u16),
        _ => w32(&mut m, FIFO, value),
    });
    m.io.gpu2d().fifo().len()
}

#[test]
fn given_a_word_write_when_the_fifo_is_counted_then_it_holds_two_pixels() {
    assert_eq!(fifo_len_after(&[(4, 0x7C00_001F)]), 2);
}

#[test]
fn given_a_halfword_write_when_the_fifo_is_counted_then_it_holds_one_pixel() {
    assert_eq!(fifo_len_after(&[(2, 0x001F)]), 1);
}

#[test]
fn given_a_byte_write_when_the_fifo_is_counted_then_it_holds_nothing_unverified() {
    assert_eq!(fifo_len_after(&[(1, 0x1F)]), 0);
}

#[test]
fn given_more_than_a_line_of_words_when_written_then_the_fifo_keeps_256_pixels() {
    let writes = vec![(4, 0x0001_0001); 130];
    assert_eq!(fifo_len_after(&writes), 256);
}

#[test]
fn given_a_written_fifo_when_the_register_is_read_then_it_reads_zero() {
    let mut m = Machine::new();
    w32(&mut m, FIFO, 0x7FFF_7FFF);
    assert_eq!(r32(&m, FIFO), 0);
}

#[test]
fn given_two_pixels_in_one_word_when_line_0_is_shown_then_the_low_halfword_is_pixel_0() {
    let mut m = mode3_machine();
    push(&mut m, &[0x001F, 0x7C00]);
    m.render_frame();
    assert_eq!(
        (pixel(&m, Screen::Top, 0, 0), pixel(&m, Screen::Top, 1, 0)),
        (0x00FF_0000, 0x0000_00FF)
    );
}

#[test]
fn given_a_full_line_when_line_0_is_shown_then_pixel_255_is_the_last_one_pushed() {
    let mut m = mode3_machine();
    push(&mut m, &line_ending_in_blue());
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 255, 0), 0x0000_00FF);
}

#[test]
fn given_a_fifo_pixel_with_bit_15_set_when_shown_then_the_bit_is_ignored() {
    let mut m = mode3_machine();
    push(&mut m, &[0xFC00]);
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 0, 0), 0x0000_00FF);
}

#[test]
fn given_one_line_of_pixels_when_a_frame_is_shown_then_only_line_0_has_them_and_the_rest_is_black()
{
    let mut m = mode3_machine();
    push(&mut m, &[0x7FFF; 256]);
    m.render_frame();
    assert_eq!(
        (pixel(&m, Screen::Top, 0, 0), pixel(&m, Screen::Top, 0, 1)),
        (0x00FF_FFFF, 0)
    );
}

#[test]
fn given_a_short_line_when_shown_then_the_missing_pixels_are_black() {
    let mut m = mode3_machine();
    push(&mut m, &[0x7FFF; 4]);
    m.render_frame();
    assert_eq!(
        (pixel(&m, Screen::Top, 3, 0), pixel(&m, Screen::Top, 4, 0)),
        (0x00FF_FFFF, 0)
    );
}

#[test]
fn given_a_master_brightness_up_when_mode_3_is_shown_then_the_fifo_pixels_are_brightened() {
    let mut m = mode3_machine();
    w16(&mut m, MASTER_BRIGHT_A, (1 << 14) | 16);
    push(&mut m, &[0x0000]);
    m.render_frame();
    assert_eq!(pixel(&m, Screen::Top, 0, 0), 0x00FF_FFFF);
}

#[test]
fn given_a_line_of_pixels_when_a_frame_is_rendered_in_mode_1_then_the_fifo_is_drained_unshown_unverified()
 {
    let mut m = mode3_machine();
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    push(&mut m, &[0x7FFF; 256]);
    m.render_frame();
    assert_eq!(m.io.gpu2d().fifo().len(), 0);
}

#[test]
fn given_source_b_from_the_fifo_when_captured_then_line_0_is_stored_with_alpha_and_the_underrun_with_alpha_too()
 {
    let mut m = mode3_machine();
    w8(&mut m, VRAMCNT_B, ENABLE_MST0);
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    push(&mut m, &[0x1234]);
    w32(&mut m, DISPCAPCNT, CAPTURE_FIFO);
    m.render_frame();
    // Pixel 0 of line 0 came from the FIFO; pixel 0 of line 1 is an underrun, a black pixel with
    // alpha (`display-capture.md`, "Unverified").
    assert_eq!((r16(&m, LCDC_B), r16(&m, LCDC_B + 512)), (0x9234, 0x8000));
}

#[test]
fn given_source_b_from_the_fifo_with_a_narrow_size_when_captured_then_the_pixels_of_the_width_are_stored()
 {
    let mut m = mode3_machine();
    w8(&mut m, VRAMCNT_B, ENABLE_MST0);
    w32(&mut m, DISPCNT_A_ADDR, 1 << 16);
    push(&mut m, &[0x0001; 256]);
    // Size 0: 128 wide, so pixel 128 of line 0 is not stored (it would land at byte 256).
    w32(&mut m, DISPCAPCNT, CAPTURE_FIFO & !(3 << 20));
    m.render_frame();
    assert_eq!(
        (r16(&m, LCDC_B + 254), r16(&m, LCDC_B + 256 * 128)),
        (0x8001, 0)
    );
}

#[test]
fn given_a_half_filled_fifo_when_saved_and_loaded_then_it_holds_the_same_pixels() {
    let mut m = mode3_machine();
    push(&mut m, &[0x1111, 0x2222, 0x3333, 0x4444]);
    let mut loaded = Machine::new();
    let result = loaded.load_state(&m.save_state());
    assert_eq!((result.is_ok(), loaded.io.gpu2d().fifo().len()), (true, 4));
}
