// SPDX-License-Identifier: Apache-2.0
//! Display capture of engine A's graphics screen (issue #36). Every expected value comes from
//! `docs/hw/display-capture.md` (GBATEK "DS Video Capture and Main Memory Display Mode"), worked
//! out by hand. The picture is the backdrop colour alone, so the captured halfword is known:
//! the 15-bit colour with bit 15 set (the note's alpha rule).

use super::*;

/// DISPCAPCNT (`docs/hw/display-capture.md`, "Registers").
const DISPCAPCNT: u32 = 0x0400_0064;
const BG0HOFS: u32 = 0x0400_0010;
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

/// A saved state with DISPCAPCNT at `0x0001_0B15` and no capture running, with the byte of that
/// register's encoding at `index` replaced by `byte`; `None` when the encoding is not found
/// exactly once. The encoding is the control as a varint (`95 B6 04`), then the running flag.
fn state_with_capture_byte(index: usize, byte: u8) -> Option<Vec<u8>> {
    let mut m = capture_machine();
    w32(&mut m, DISPCAPCNT, 0x0001_0B15);
    let needle = postcard::to_allocvec(&m.io.gpu2d().capture()).expect("plain data encodes");
    let bytes = m.save_state();
    let hits: Vec<usize> = bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle.as_slice())
        .map(|(at, _)| at)
        .collect();
    let at = *hits.first().filter(|_| hits.len() == 1)?;
    let mut patched = bytes;
    patched[at + index] = byte;
    Some(patched)
}

#[test]
fn given_a_state_whose_dispcapcnt_has_an_unused_bit_when_loaded_then_it_is_refused_as_corrupt() {
    // Bit 5 is unused: the low byte of the varint, 95h, becomes B5h.
    let bytes = state_with_capture_byte(0, 0xB5).expect("the register is found once");
    assert_eq!(
        Machine::new().load_state(&bytes),
        Err(crate::savestate::SavestateError::Corrupt)
    );
}

#[test]
fn given_a_state_that_runs_a_capture_with_the_enable_clear_when_loaded_then_it_is_refused_as_corrupt()
 {
    // The running flag is the byte after the three of the control.
    let bytes = state_with_capture_byte(3, 1).expect("the register is found once");
    assert_eq!(
        Machine::new().load_state(&bytes),
        Err(crate::savestate::SavestateError::Corrupt)
    );
}

#[test]
fn given_a_state_whose_dispcapcnt_is_valid_when_patched_to_the_same_value_then_it_loads() {
    // Patching the low byte to itself proves the harness finds the right bytes, so the refusals
    // above are about the patched values.
    let bytes = state_with_capture_byte(0, 0x95).expect("the register is found once");
    assert_eq!(Machine::new().load_state(&bytes), Ok(()));
}

// ------------------------------------------------------------------------ sources B and A+B

/// Capture source B (bits 29-30 = 1) and A+B blended (2).
const SOURCE_B: u32 = 1 << 29;
const SOURCE_BLEND: u32 = 2 << 29;
/// The read offset field at bits 26-27, and EVA and EVB at bits 0-4 and 8-12.
const fn read_offset(n: u32) -> u32 {
    n << 26
}
const fn factors(eva: u32, evb: u32) -> u32 {
    eva | evb << 8
}
const ALPHA: u16 = 0x8000;
const GREEN15: u16 = 0x03E0;

/// Bank A (the read block, block 0) and bank B (the write block) in LCDC mode, engine A showing the
/// graphics with `backdrop` as its backdrop colour.
fn blend_machine(backdrop: u16) -> Machine {
    let mut m = capture_machine();
    w8(&mut m, VRAMCNT_A, ENABLE_MST0);
    w16(&mut m, BACKDROP, backdrop);
    m
}

/// The halfword stored at pixel 0 of bank B when `control` captures one frame of `m`.
fn stored(mut m: Machine, control: u32) -> u16 {
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3) | control);
    m.render_frame();
    bank_b(&m, 0)
}

/// Blend cases: (backdrop, source B halfword, EVA, EVB, the stored halfword).
const BLEND_CASES: [(u16, u16, u32, u32, u16); 9] = [
    // Only A: full red, with the alpha rule (A has alpha and EVA > 0).
    (RED15, ALPHA | GREEN15, 16, 0, ALPHA | RED15),
    // Half and half: 31 * 8 / 16 = 15 in red and in green.
    (RED15, ALPHA | GREEN15, 8, 8, ALPHA | 0x01EF),
    // Both at 16: red and green each reach 31.
    (RED15, ALPHA | GREEN15, 16, 16, ALPHA | 0x03FF),
    // Both factors 0: no colour and no alpha.
    (RED15, ALPHA | GREEN15, 0, 0, 0),
    // Red from both: 31 + 31 saturates at 31.
    (RED15, ALPHA | RED15, 16, 16, ALPHA | RED15),
    // B without alpha adds nothing: A at 8 / 16 gives 15, and the alpha comes from A.
    (RED15, GREEN15, 8, 16, ALPHA | 0x000F),
    // B alone has the alpha: the result has it, and A's colour is gone at EVA 0.
    (RED15, ALPHA | GREEN15, 0, 16, ALPHA | GREEN15),
    // A factor above 16 counts as 16: 8 * 16 / 16, not 8 * 31 / 16.
    (0x0008, ALPHA, 31, 0, ALPHA | 0x0008),
    // A truncation: 5 * 9 / 16 = 2 (not 3).
    (0x0005, ALPHA, 9, 0, ALPHA | 0x0002),
];

/// The blend cases that store something else.
fn blend_failures() -> Vec<(usize, u16, u16)> {
    BLEND_CASES
        .iter()
        .enumerate()
        .map(|(i, &(backdrop, b, eva, evb, want))| {
            let mut m = blend_machine(backdrop);
            w16(&mut m, LCDC_A, b);
            (i, stored(m, SOURCE_BLEND | factors(eva, evb)), want)
        })
        .filter(|&(_, got, want)| got != want)
        .collect()
}

#[test]
fn given_the_blend_cases_when_a_and_b_are_captured_then_each_stores_the_notes_pixel() {
    assert_eq!(blend_failures(), vec![]);
}

#[test]
fn given_capture_source_3_when_captured_then_it_blends_like_source_2() {
    let mut m = blend_machine(RED15);
    w16(&mut m, LCDC_A, ALPHA | GREEN15);
    assert_eq!(stored(m, (3 << 29) | factors(8, 8)), ALPHA | 0x01EF);
}

#[test]
fn given_source_b_when_captured_then_the_vram_halfword_is_stored_with_its_own_alpha() {
    let mut m = blend_machine(RED15);
    w16(&mut m, LCDC_A, GREEN15);
    assert_eq!(stored(m, SOURCE_B), GREEN15);
}

#[test]
fn given_a_read_offset_when_source_b_is_captured_then_the_read_starts_at_that_offset() {
    let mut m = blend_machine(RED15);
    w16(&mut m, LCDC_A, GREEN15);
    w16(&mut m, LCDC_A + 0x1_0000, BLUE15);
    assert_eq!(stored(m, SOURCE_B | read_offset(2)), BLUE15);
}

#[test]
fn given_display_mode_2_when_source_b_is_captured_then_the_read_offset_is_ignored() {
    let mut m = blend_machine(RED15);
    w32(&mut m, DISPCNT_A_ADDR, MODE2);
    w16(&mut m, LCDC_A, GREEN15);
    w16(&mut m, LCDC_A + 0x1_0000, BLUE15);
    assert_eq!(stored(m, SOURCE_B | read_offset(2)), GREEN15);
}

#[test]
fn given_dispcnt_names_block_1_when_source_b_is_captured_then_it_reads_bank_b() {
    let mut m = blend_machine(RED15);
    w32(&mut m, DISPCNT_A_ADDR, MODE1_ONLY | (1 << 18));
    w16(&mut m, LCDC_A, GREEN15);
    w16(&mut m, LCDC_B, BLUE15);
    // Bank B is the write block too: pixel 0 is read as BLUE15, then overwritten with it.
    assert_eq!(stored(m, SOURCE_B), BLUE15);
}

#[test]
fn given_a_read_block_that_is_not_in_lcdc_mode_when_source_b_is_captured_then_zero_is_stored() {
    let mut m = blend_machine(RED15);
    w16(&mut m, LCDC_A, GREEN15);
    w8(&mut m, VRAMCNT_A, 0);
    w16(&mut m, LCDC_B, 0x1234);
    assert_eq!(stored(m, SOURCE_B), 0);
}

#[test]
fn given_a_small_size_when_source_b_is_captured_then_its_rows_use_the_capture_width_as_stride() {
    // Size 0 (128 wide): row 1 pixel 0 is read from byte 256 of the block and stored at byte 256.
    let mut m = blend_machine(RED15);
    w16(&mut m, LCDC_A + 256, GREEN15);
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(0) | SOURCE_B);
    m.render_frame();
    assert_eq!(bank_b(&m, 256), GREEN15);
}

#[test]
fn given_a_bg0_scroll_when_the_3d_screen_is_captured_then_the_line_is_not_scrolled() {
    // The capture takes the renderer's line as it is; only the composed layer scrolls
    // (`display-capture.md`, "Unverified").
    let mut m = capture_machine();
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3) | (1 << 24));
    w16(&mut m, BG0HOFS, 3);
    let mut three_d = TRANSPARENT_LINE;
    three_d[0] = Pixel3d {
        color: 0x1234,
        alpha: 31,
    };
    m.io.gpu2d_mut().begin_line(0);
    let capture = m.io.gpu2d().capture_line(0).expect("a capture is running");
    let pixels = capture.pixels(&[0x7FFF; SCREEN_WIDTH], &three_d, &[0; SCREEN_WIDTH]);
    assert_eq!(pixels[0], 0x9234);
}

#[test]
fn given_source_a_when_the_capture_unit_runs_then_the_vram_block_is_not_read() {
    let mut m = blend_machine(RED15);
    w16(&mut m, LCDC_A, GREEN15);
    assert_eq!(stored(m, 0), ALPHA | RED15);
}

#[test]
fn given_the_3d_screen_as_source_a_when_a_3d_pixel_is_opaque_then_it_is_captured_with_alpha_1() {
    let mut m = capture_machine();
    w32(&mut m, DISPCAPCNT, ENABLE | WRITE_B | size(3) | (1 << 24));
    let mut three_d = TRANSPARENT_LINE;
    three_d[1] = Pixel3d {
        color: 0x1234,
        alpha: 31,
    };
    m.io.gpu2d_mut().begin_line(0);
    let capture = m.io.gpu2d().capture_line(0).expect("a capture is running");
    let pixels = capture.pixels(&[0x7FFF; SCREEN_WIDTH], &three_d, &[0; SCREEN_WIDTH]);
    assert_eq!((pixels[0], pixels[1]), (0, 0x9234));
}
