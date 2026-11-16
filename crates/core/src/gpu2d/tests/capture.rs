// SPDX-License-Identifier: Apache-2.0
//! Display capture of engine A's graphics screen (issue #36). Every expected value comes from
//! `docs/hw/display-capture.md` (GBATEK "DS Video Capture and Main Memory Display Mode"), worked
//! out by hand. The picture is the backdrop colour alone, so the captured halfword is known:
//! the 15-bit colour with bit 15 set (the note's alpha rule).

use super::*;

/// DISPCAPCNT (`docs/hw/display-capture.md`, "Registers").
const DISPCAPCNT: u32 = 0x0400_0064;
/// VRAMCNT_B, and bank B's LCDC window (bank B is 128 KiB after bank A).
const VRAMCNT_B: u32 = 0x0400_0241;
const LCDC_B: u32 = 0x0682_0000;
/// Engine A's backdrop: palette entry 0.
const BACKDROP: u32 = 0x0500_0000;
/// DISPCNT display mode 1 (graphics).
const MODE1_ONLY: u32 = 1 << 16;
/// MASTER_BRIGHT of engine A.
const MASTER_BRIGHT_A: u32 = 0x0400_006C;
const RED15: u16 = 0x001F;
const BLUE15: u16 = 0x7C00;
/// The stored form of a red source A pixel: colour and alpha.
const RED_STORED: u16 = 0x801F;
const ENABLE: u32 = 1 << 31;
/// Write block 1 (bank B), the field at bits 16-17.
const WRITE_B: u32 = 1 << 16;
const fn size(n: u32) -> u32 {
    n << 20
}
const fn write_offset(n: u32) -> u32 {
    n << 18
}

/// Bank B in LCDC mode, engine A showing the graphics (a red backdrop), engine A on top.
fn capture_machine() -> Machine {
    let mut m = Machine::new();
    w8(&mut m, VRAMCNT_B, ENABLE_MST0);
    w32(&mut m, DISPCNT_A_ADDR, MODE1_ONLY);
    w16(&mut m, POWCNT1_ADDR, POWCNT1_ON_TOP);
    w16(&mut m, BACKDROP, RED15);
    m
}

/// The halfword at `offset` of bank B.
fn bank_b(m: &Machine, offset: u32) -> u16 {
    r16(m, LCDC_B + offset)
}

/// Renders visible lines `lines` one by one, without the frame around them.
fn render_lines(m: &mut Machine, lines: std::ops::Range<usize>) {
    lines.for_each(|y| m.render_line(y));
}

/// Every halfword of bank B.
fn bank_b_words(m: &Machine) -> Vec<u16> {
    (0..0x1_0000).map(|i| bank_b(m, i * 2)).collect()
}

/// Offsets of bank B that differ from `(offset, halfword)` after one captured frame.
fn offset_failures(m: &Machine, cases: &[(u32, u16)]) -> Vec<(u32, u16, u16)> {
    cases
        .iter()
        .map(|&(offset, want)| (offset, bank_b(m, offset), want))
        .filter(|&(_, got, want)| got != want)
        .collect()
}

/// Captures one frame of `control` and returns the offsets that differ from `cases`.
fn captured_failures(control: u32, cases: &[(u32, u16)]) -> Vec<(u32, u16, u16)> {
    let mut m = capture_machine();
    w32(&mut m, DISPCAPCNT, control);
    m.render_frame();
    offset_failures(&m, cases)
}

#[test]
fn given_every_bit_written_when_dispcapcnt_is_read_then_only_the_existing_bits_read_back() {
    let mut m = Machine::new();
    w32(&mut m, DISPCAPCNT, 0xFFFF_FFFF);
    assert_eq!(r32(&m, DISPCAPCNT), 0xEF3F_1F1F);
}

#[test]
fn given_a_power_on_machine_when_dispcapcnt_is_read_then_it_is_zero() {
    assert_eq!(r32(&Machine::new(), DISPCAPCNT), 0);
}

#[test]
fn given_a_byte_write_to_the_high_byte_when_dispcapcnt_is_read_then_the_low_bytes_are_kept() {
    let mut m = Machine::new();
    w32(&mut m, DISPCAPCNT, 0x0001_0203);
    w8(&mut m, DISPCAPCNT + 3, 0x20);
    assert_eq!(r32(&m, DISPCAPCNT), 0x2001_0203);
}

#[test]
fn given_a_capture_of_the_whole_screen_when_a_frame_is_rendered_then_bank_b_holds_the_alpha_red() {
    let cases = [(0, RED_STORED), (0x17FFE, RED_STORED), (0x18000, 0)];
    assert_eq!(
        captured_failures(ENABLE | WRITE_B | size(3), &cases),
        vec![]
    );
}

#[test]
fn given_each_capture_size_when_a_frame_is_rendered_then_exactly_its_rows_and_columns_are_written()
{
    let failures: Vec<_> = [
        (size(0), 0x7FFE, 0x8000),
        (size(1), 0x7FFE, 0x8000),
        (size(2), 0xFFFE, 0x1_0000),
        (size(3), 0x1_7FFE, 0x1_8000),
    ]
    .iter()
    .flat_map(|&(s, last, next)| {
        captured_failures(ENABLE | WRITE_B | s, &[(last, RED_STORED), (next, 0)])
    })
    .collect();
    assert_eq!(failures, vec![]);
}

#[test]
fn given_a_write_offset_that_runs_past_the_bank_when_captured_then_the_picture_wraps_to_the_start()
{
    let cases = [
        (0x0, RED_STORED),
        (0xFFFE, RED_STORED),
        (0x1_0000, 0),
        (0x1_7FFE, 0),
        (0x1_8000, RED_STORED),
        (0x1_FFFE, RED_STORED),
    ];
    assert_eq!(
        captured_failures(ENABLE | WRITE_B | write_offset(3) | size(3), &cases),
        vec![]
    );
}

#[test]
fn given_a_small_size_at_a_write_offset_when_captured_then_it_starts_at_that_offset() {
    let cases = [
        (0x7FFE, 0),
        (0x8000, RED_STORED),
        (0xFFFE, RED_STORED),
        (0x1_0000, 0),
    ];
    assert_eq!(
        captured_failures(ENABLE | WRITE_B | write_offset(1) | size(0), &cases),
        vec![]
    );
}

#[test]
fn given_the_enable_bit_when_a_frame_is_rendered_then_it_reads_busy_until_the_next_frame_starts() {
    let mut m = capture_machine();
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    m.render_frame();
    let during = r32(&m, DISPCAPCNT) >> 31;
    m.io.gpu2d_mut().vblank_started();
    assert_eq!((during, r32(&m, DISPCAPCNT) >> 31), (1, 0));
}

#[test]
fn given_a_capture_enabled_in_line_100_when_that_frame_is_rendered_then_nothing_is_captured_yet() {
    let mut m = capture_machine();
    render_lines(&mut m, 0..100);
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    render_lines(&mut m, 100..SCREEN_HEIGHT);
    m.io.gpu2d_mut().vblank_started();
    assert_eq!((bank_b(&m, 100 * 512), r32(&m, DISPCAPCNT) >> 31), (0, 1));
}

#[test]
fn given_a_capture_enabled_in_line_100_when_the_next_frame_is_rendered_then_it_is_captured() {
    let mut m = capture_machine();
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    render_lines(&mut m, 0..SCREEN_HEIGHT);
    assert_eq!(bank_b(&m, 191 * 512), RED_STORED);
}

#[test]
fn given_the_enable_cleared_in_line_100_when_the_frame_is_rendered_then_the_capture_stops() {
    let mut m = capture_machine();
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    render_lines(&mut m, 0..100);
    w8(&mut m, DISPCAPCNT + 3, 0);
    render_lines(&mut m, 100..SCREEN_HEIGHT);
    assert_eq!(
        (bank_b(&m, 99 * 512), bank_b(&m, 100 * 512)),
        (RED_STORED, 0)
    );
}

#[test]
fn given_a_write_block_that_is_not_in_lcdc_mode_when_captured_then_nothing_is_written() {
    let mut m = capture_machine();
    w8(&mut m, VRAMCNT_B, 0);
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    m.render_frame();
    w8(&mut m, VRAMCNT_B, ENABLE_MST0);
    assert_eq!(bank_b(&m, 0), 0);
}

#[test]
fn given_display_mode_2_when_captured_then_the_layers_are_stored_not_the_vram_picture() {
    let mut m = capture_machine();
    w8(&mut m, VRAMCNT_A, ENABLE_MST0);
    w32(&mut m, DISPCNT_A_ADDR, MODE2);
    fill_bank(&mut m, BLUE15);
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    m.render_frame();
    assert_eq!(
        (bank_b(&m, 0), pixel(&m, Screen::Top, 0, 0)),
        (RED_STORED, 0x0000_00FF)
    );
}

#[test]
fn given_the_3d_screen_as_source_a_when_captured_then_a_transparent_zero_is_stored() {
    let mut m = capture_machine();
    w16(&mut m, LCDC_B, 0x1234);
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3) | (1 << 24));
    m.render_frame();
    assert_eq!(bank_b(&m, 0), 0);
}

#[test]
fn given_a_brightness_up_when_captured_then_the_capture_holds_the_unbrightened_colour() {
    let mut m = capture_machine();
    w16(&mut m, MASTER_BRIGHT_A, (1 << 14) | 16);
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    m.render_frame();
    assert_eq!(
        (bank_b(&m, 0), pixel(&m, Screen::Top, 0, 0)),
        (RED_STORED, 0x00FF_FFFF)
    );
}

#[test]
fn given_a_capture_saved_at_line_100_when_loaded_and_finished_then_the_bank_matches_an_uninterrupted_run()
 {
    let mut straight = capture_machine();
    w32(&mut straight, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    render_lines(&mut straight, 0..SCREEN_HEIGHT);
    let mut first = capture_machine();
    w32(&mut first, DISPCAPCNT, ENABLE | WRITE_B | size(3));
    render_lines(&mut first, 0..100);
    let mut resumed = Machine::new();
    let loaded = resumed.load_state(&first.save_state());
    render_lines(&mut resumed, 100..SCREEN_HEIGHT);
    assert_eq!(
        (
            loaded.is_ok(),
            bank_b_words(&resumed) == bank_b_words(&straight)
        ),
        (true, true)
    );
}
