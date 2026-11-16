// SPDX-License-Identifier: Apache-2.0
//! The main-memory display through `Machine` (issue #36): a DMA channel in start mode 4 streams a
//! bitmap from main RAM into DISP_MMEM_FIFO, a line at a time, and display mode 3 shows it; start
//! mode 3 fires at the start of each frame. Expected values come from
//! `docs/hw/display-capture.md` ("The main-memory display", "ARM9 DMA start modes 3 and 4"),
//! never from running the code.

use super::dma::{
    DEST_FIXED, DST, ENABLE, REPEAT, SRC, WORD, busy9, machine, peek_words, poke_halfwords,
    poke_words, run_frames, run_until, set9,
};
use crate::cpu::Bus;
use crate::machine::*;

/// Where the bitmap lives, the FIFO register and DISPCNT.
const BITMAP: u32 = 0x0220_0000;
const DISP_MMEM_FIFO: u32 = 0x0400_0068;
const DISPCNT_A: u32 = 0x0400_0000;
/// Display mode 3.
const MODE3: u32 = 3 << 16;
/// Start mode 4 (bits 27-29).
const MAIN_MEMORY: u32 = 4 << 27;
/// Start mode 3.
const DISPLAY_START: u32 = 3 << 27;

/// The bitmap's pixel at (x, y): distinct for the first rows, in 15-bit colour.
fn bitmap(x: usize, y: usize) -> u16 {
    ((y * 7 + x * 3) & 0x7FFF) as u16
}

/// The 15-bit colour as xRGB8888 (`display-mode-vram.md`, "Colour expansion").
fn expand(color: u16) -> u32 {
    let channel = |c: u16| {
        let c = u32::from(c & 31);
        (c << 3) | (c >> 2)
    };
    (channel(color) << 16) | (channel(color >> 5) << 8) | channel(color >> 10)
}

/// A machine with the bitmap in main RAM and display mode 3.
fn bitmap_machine() -> Machine {
    let mut m = machine();
    (0..192).for_each(|y| {
        let row: Vec<u16> = (0..256).map(|x| bitmap(x, y)).collect();
        poke_halfwords(&mut m, BITMAP + (y * 512) as u32, &row);
    });
    m.bus9().write32(DISPCNT_A, MODE3);
    m
}

/// Enables a repeating main-memory DMA channel of 4-word requests from the bitmap to the FIFO.
fn start_streaming(m: &mut Machine) {
    set9(
        m,
        0,
        BITMAP,
        DISP_MMEM_FIFO,
        ENABLE | REPEAT | WORD | DEST_FIXED | MAIN_MEMORY | 4,
    );
}

/// The (x, y) pixels of the upper screen that differ from the bitmap.
fn misses(m: &Machine) -> Vec<(usize, usize)> {
    (0..192)
        .flat_map(|y| (0..256).map(move |x| (x, y)))
        .filter(|&(x, y)| m.frame(Screen::Top)[y * 256 + x] != expand(bitmap(x, y)))
        .collect()
}

/// How many pixels of the upper screen are not black.
fn lit(m: &Machine) -> usize {
    m.frame(Screen::Top).iter().filter(|&&p| p != 0).count()
}

#[test]
fn given_a_channel_enabled_in_the_middle_of_a_frame_when_that_frame_ends_then_the_screen_is_black()
{
    // "Transfer starts at next frame": nothing reaches the FIFO during the frame it was enabled in.
    let mut m = bitmap_machine();
    run_until(&mut m, 1, 100);
    start_streaming(&mut m);
    m.run_frame();
    assert_eq!(lit(&m), 0);
}

#[test]
fn given_a_channel_enabled_in_the_middle_of_a_frame_when_the_next_frame_ends_then_it_is_the_bitmap()
{
    let mut m = bitmap_machine();
    run_until(&mut m, 1, 100);
    start_streaming(&mut m);
    run_frames(&mut m, 2);
    assert_eq!(misses(&m), vec![]);
}

#[test]
fn given_the_bitmap_shown_for_a_frame_when_the_next_frame_ends_then_the_line_zero_pixel_is_what_the_source_holds_past_the_bitmap()
 {
    // The channel only repeats: its source runs on through main RAM after one frame (98,304 bytes),
    // so the second frame reads what lies past the bitmap, which is zeros here.
    let mut m = bitmap_machine();
    run_until(&mut m, 1, 100);
    start_streaming(&mut m);
    run_frames(&mut m, 3);
    assert_eq!(lit(&m), 0);
}

#[test]
fn given_a_start_of_display_channel_when_frame_2_has_started_then_it_ran_once_for_frames_1_and_2() {
    // The first frame's line 0 is not a start event (the machine begins in it), so the channel runs
    // at the start of frames 1 and 2.
    let mut m = machine();
    poke_words(&mut m, SRC, &[1, 2, 3, 4]);
    set9(
        &mut m,
        0,
        SRC,
        DST,
        ENABLE | REPEAT | WORD | DISPLAY_START | 1,
    );
    run_until(&mut m, 2, 1);
    assert_eq!(peek_words(&m, DST, 4), vec![1, 2, 0, 0]);
}

#[test]
fn given_a_start_of_display_channel_without_repeat_when_a_frame_has_run_then_it_moved_once_and_stopped()
 {
    let mut m = machine();
    poke_words(&mut m, SRC, &[1, 2, 3, 4]);
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | DISPLAY_START | 2);
    run_until(&mut m, 2, 1);
    assert_eq!((peek_words(&m, DST, 3), busy9(&m, 0)), (vec![1, 2, 0], 0));
}

#[test]
fn given_a_main_memory_channel_without_repeat_when_a_line_is_due_then_it_moves_32_requests_and_stops()
 {
    // One request is `count` = 4 words; a line is 32 of them, moved in one transfer
    // (`display-capture.md`, "Unverified"). The destination is plain RAM so the words can be read.
    let mut m = machine();
    let words: Vec<u32> = (1..=130).collect();
    poke_words(&mut m, SRC, &words);
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | MAIN_MEMORY | 4);
    run_frames(&mut m, 2);
    assert_eq!(
        (peek_words(&m, DST + 127 * 4, 3), busy9(&m, 0)),
        (vec![128, 0, 0], 0)
    );
}

/// A machine saved in the middle of frame 1 with a channel waiting for the next frame and a FIFO
/// holding a few pixels, loaded into a fresh machine.
fn saved_mid_frame_and_loaded() -> (Machine, Machine) {
    let mut m = bitmap_machine();
    run_until(&mut m, 1, 100);
    start_streaming(&mut m);
    m.bus9().write32(DISP_MMEM_FIFO, 0x0001_0001);
    let mut loaded = machine();
    let result = loaded.load_state(&m.save_state());
    assert_eq!(result, Ok(()));
    (m, loaded)
}

#[test]
fn given_a_state_saved_with_a_waiting_channel_and_a_filled_fifo_when_loaded_and_run_then_the_frames_match_an_uninterrupted_run()
 {
    let (mut straight, mut resumed) = saved_mid_frame_and_loaded();
    run_frames(&mut straight, 2);
    run_frames(&mut resumed, 2);
    assert!(resumed.frame(Screen::Top) == straight.frame(Screen::Top));
}

#[test]
fn given_a_state_saved_with_a_waiting_channel_when_loaded_then_the_channel_still_waits_for_the_next_frame()
 {
    let (_, mut resumed) = saved_mid_frame_and_loaded();
    // Enabled in frame 1, it must not feed frame 1's remaining lines: the screen has only what the
    // FIFO held, none of the bitmap.
    resumed.run_frame();
    assert!(lit(&resumed) <= 2);
}
