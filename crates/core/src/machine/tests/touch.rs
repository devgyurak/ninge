// SPDX-License-Identifier: Apache-2.0
//! The touch-screen controller and the power-management device through `Machine`
//! (`docs/hw/touch-power.md`): SPICNT and SPIDATA on the ARM7's bus, the host's panel reaching the
//! controller, the pen bit following touch and the power-down mode, and both devices in a
//! savestate. Expected words are packed by hand from the note's reply layout.

use super::dma::machine;
use super::*;
use crate::cpu::Bus;
use crate::input::{Input, Touch};

const SPICNT: u32 = 0x0400_01C0;
const SPIDATA: u32 = 0x0400_01C2;
const EXTKEYIN: u32 = 0x0400_0136;

/// SPICNT: bus on, touch selected, 2 MHz, 8-bit; with hold set for every byte but the last.
const TOUCH: u16 = 0x8000 | (2 << 8) | 1;
const POWER: u16 = 0x8000 | 2;
const HOLD: u16 = 1 << 11;
/// X position, 12-bit, differential, power-down mode 0.
const X: u8 = 0xD0;
/// Y position, the same.
const Y: u8 = 0x90;
/// Far more ARM7 clocks than a unit takes.
const GAP: u64 = 10_000;

/// A host panel at pixel (100, 50) for the frame.
fn touched() -> Input {
    Input::new().with_touch(Some(Touch { x: 100, y: 50 }))
}

/// The bytes `bytes` sent as one session to the device `cnt` selects, `GAP` apart; the replies.
fn session(m: &mut Machine, cnt: u16, bytes: &[u8]) -> Vec<u8> {
    let last = bytes.len() - 1;
    // The ARM7 is not stepped here, so a session starts after the last unit on the wire.
    let mut at = m.inspect().arm7_clock().max(m.inspect().spi().done_at());
    bytes
        .iter()
        .enumerate()
        .map(|(i, &byte)| {
            at += GAP;
            let hold = if i == last { 0 } else { HOLD };
            m.bus7(at).write16(SPICNT, cnt | hold);
            m.bus7(at).write8(SPIDATA, byte);
            at += GAP;
            m.bus7(at).read8(SPIDATA)
        })
        .collect()
}

/// The 12-bit result a program rebuilds from the two reply bytes.
fn rebuild(hi: u8, lo: u8) -> u16 {
    (((u16::from(hi) << 8) | u16::from(lo)) >> 3) & 0xFFF
}

#[test]
fn given_a_touched_frame_when_the_arm7_reads_x_and_y_then_they_are_16_times_the_pixel() {
    let mut m = machine();
    m.run_frame(touched());
    let x = session(&mut m, TOUCH, &[X, 0, 0]);
    let y = session(&mut m, TOUCH, &[Y, 0, 0]);
    assert_eq!(
        (rebuild(x[1], x[2]), rebuild(y[1], y[2])),
        (100 * 16, 50 * 16)
    );
}

#[test]
fn given_the_next_command_in_the_low_byte_slot_when_the_arm7_reads_then_both_positions_arrive() {
    let mut m = machine();
    m.run_frame(touched());
    let r = session(&mut m, TOUCH, &[X, 0, Y, 0, 0]);
    assert_eq!(
        (rebuild(r[1], r[2]), rebuild(r[3], r[4])),
        (100 * 16, 50 * 16)
    );
}

#[test]
fn given_a_released_frame_when_the_arm7_reads_x_and_y_then_x_is_zero_and_y_is_full_scale() {
    let mut m = machine();
    m.run_frame(Input::new());
    let x = session(&mut m, TOUCH, &[X, 0, 0]);
    let y = session(&mut m, TOUCH, &[Y, 0, 0]);
    assert_eq!((rebuild(x[1], x[2]), rebuild(y[1], y[2])), (0, 0xFFF));
}

#[test]
fn given_a_touch_then_a_release_when_two_frames_run_then_the_second_read_sees_the_release() {
    let mut m = machine();
    m.run_frame(touched());
    let first = session(&mut m, TOUCH, &[X, 0, 0]);
    m.run_frame(Input::new());
    let second = session(&mut m, TOUCH, &[X, 0, 0]);
    assert_eq!(
        (rebuild(first[1], first[2]), rebuild(second[1], second[2])),
        (1600, 0)
    );
}

/// EXTKEYIN as the ARM7 reads it.
fn extkeyin(m: &mut Machine) -> u16 {
    let at = m.inspect().arm7_clock().max(m.inspect().spi().done_at());
    m.bus7(at).read16(EXTKEYIN)
}

#[test]
fn given_a_touched_frame_when_the_power_down_mode_changes_then_the_pen_bit_follows_it() {
    let mut m = machine();
    m.run_frame(touched());
    let power_on = extkeyin(&mut m);
    session(&mut m, TOUCH, &[X | 1]);
    let mode_1 = extkeyin(&mut m);
    session(&mut m, TOUCH, &[X]);
    let mode_0 = extkeyin(&mut m);
    session(&mut m, TOUCH, &[X | 2]);
    let mode_2 = extkeyin(&mut m);
    assert_eq!((power_on, mode_1, mode_0, mode_2), (0x3F, 0x7F, 0x3F, 0x3F));
}

#[test]
fn given_an_untouched_frame_when_the_pen_output_is_enabled_then_the_pen_bit_reads_released() {
    let mut m = machine();
    m.run_frame(Input::new());
    session(&mut m, TOUCH, &[X]);
    assert_eq!(extkeyin(&mut m), 0x7F);
}

#[test]
fn given_the_arm9_when_it_reads_extkeyin_on_a_touched_frame_then_it_reads_zero() {
    let mut m = machine();
    m.run_frame(touched());
    assert_eq!(m.inspect().arm9_peek16(EXTKEYIN), 0);
}

#[test]
fn given_the_power_device_when_register_0_is_written_then_it_reads_back_and_the_debugger_shows_it()
{
    let mut m = machine();
    session(&mut m, POWER, &[0x00, 0x0C]);
    let read = session(&mut m, POWER, &[0x80, 0]);
    assert_eq!((read[1], m.inspect().spi().power().control()), (0x0C, 0x0C));
}

#[test]
fn given_the_unconnected_select_when_a_byte_is_sent_then_the_line_floats_high() {
    let mut m = machine();
    let reply = session(&mut m, 0x8000 | (3 << 8), &[0x80, 0]);
    assert_eq!(reply, [0xFF, 0xFF]);
}

#[test]
fn given_the_same_input_log_when_two_machines_run_it_then_their_savestates_are_identical() {
    let run = || {
        let mut m = machine();
        m.run_frame(touched());
        session(&mut m, TOUCH, &[X, 0, Y, 0, 0]);
        session(&mut m, POWER, &[0x00, 0x0C]);
        m.run_frame(Input::new());
        m.save_state()
    };
    assert_eq!(run(), run());
}

#[test]
fn given_a_conversion_interrupted_by_a_savestate_when_it_finishes_then_it_matches_an_uninterrupted_one()
 {
    // Hold the chip, send the control byte and the zero byte, let a frame pass (the clock moves
    // past the last unit, as it would for a real program), save, then clock the low byte.
    let start = |m: &mut Machine| {
        m.run_frame(touched());
        session(m, TOUCH | HOLD, &[X, 0]);
        m.run_frame(touched());
    };
    let mut whole = machine();
    start(&mut whole);
    let whole_lo = session(&mut whole, TOUCH, &[0]);

    let mut saved = machine();
    start(&mut saved);
    let state = saved.save_state();
    let mut loaded = machine();
    let loaded_ok = loaded.load_state(&state);
    let lo = session(&mut loaded, TOUCH, &[0]);
    assert_eq!((loaded_ok, lo), (Ok(()), whole_lo));
}

#[test]
fn given_the_pen_output_disabled_when_the_state_is_saved_and_loaded_then_the_pen_bit_stays_released()
 {
    let mut m = machine();
    m.run_frame(touched());
    session(&mut m, TOUCH, &[X | 1]);
    m.run_frame(touched());
    let state = m.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    loaded.run_frame(touched());
    assert_eq!((result, extkeyin(&mut loaded)), (Ok(()), 0x7F));
}

// ------------------------------------------------------------------- shutdown and backlights

/// `add r1, r1, #1` and `b` back to it: a counter the ARM9 keeps running.
const COUNT_R1: [u32; 2] = [0xE281_1001, 0xEAFF_FFFD];

/// `b .`: the ARM7 spins.
const SPIN7: [u32; 1] = [0xEAFF_FFFE];

fn counting_machine() -> Machine {
    super::bios::machine_with(&COUNT_R1, &SPIN7)
}

#[test]
fn given_direct_boot_when_asked_then_both_screens_are_lit_and_the_machine_is_running() {
    let m = counting_machine();
    assert_eq!(
        (
            m.backlight(Screen::Top),
            m.backlight(Screen::Bottom),
            m.is_shut_down()
        ),
        (true, true, false)
    );
}

#[test]
fn given_the_lower_backlight_bit_alone_when_asked_then_only_the_bottom_screen_is_lit() {
    let mut m = counting_machine();
    session(&mut m, POWER, &[0x00, 0x04]);
    assert_eq!(
        (m.backlight(Screen::Top), m.backlight(Screen::Bottom)),
        (false, true)
    );
}

#[test]
fn given_the_upper_backlight_bit_alone_when_asked_then_only_the_top_screen_is_lit() {
    let mut m = counting_machine();
    session(&mut m, POWER, &[0x00, 0x08]);
    assert_eq!(
        (m.backlight(Screen::Top), m.backlight(Screen::Bottom)),
        (true, false)
    );
}

#[test]
fn given_a_shutdown_write_when_a_frame_runs_then_the_machine_reports_it_and_the_arm9_does_not_move()
{
    let mut m = counting_machine();
    m.run_frame(Input::new());
    let before = (m.inspect().arm9().regs().get(1), m.frame_count());
    session(&mut m, POWER, &[0x00, 0x4C]);
    m.run_frame(Input::new());
    assert_eq!(
        (
            m.is_shut_down(),
            m.inspect().arm9().regs().get(1),
            m.frame_count()
        ),
        (true, before.0, before.1 + 1)
    );
}

/// Rounds until the clock has passed the units a `session` left on the wire, which it started
/// far ahead of the master clock: a savestate needs the machine settled.
fn run_rounds(m: &mut Machine, rounds: usize) {
    (0..rounds).for_each(|_| crate::scheduler::run_round(m));
}

#[test]
fn given_a_machine_that_is_shut_down_when_a_savestate_round_trips_then_it_is_still_shut_down() {
    let mut m = counting_machine();
    m.run_frame(Input::new());
    session(&mut m, POWER, &[0x00, 0x44]);
    run_rounds(&mut m, 2_000);
    let mut loaded = counting_machine();
    let loaded_ok = loaded.load_state(&m.save_state());
    assert_eq!(
        (
            loaded_ok,
            loaded.is_shut_down(),
            loaded.backlight(Screen::Top)
        ),
        (Ok(()), true, false)
    );
}
