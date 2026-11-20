// SPDX-License-Identifier: Apache-2.0
//! The clock chip through `Machine` (`docs/hw/rtc.md`): RTCCNT on the ARM7's bus only, the host's
//! time reaching the chip through the frame's `Input`, the offset a program sets, and the chip in a
//! savestate. The programs' bits are sent by hand, one RTCCNT write at a time. Expected values come
//! from the note and the calendar (days since 2000-01-01), never from running the code.

use super::dma::machine;
use super::*;
use crate::cpu::Bus;
use crate::input::Input;

const RTCCNT: u32 = 0x0400_0138;

/// RTCCNT with chip select low and the clock high, every pin driven: where a transfer starts and
/// ends.
const IDLE: u8 = 0b0111_0010 | 1;
/// The same with chip select high.
const SELECTED: u8 = IDLE | 0b100;
/// The clock low, chip select high, the clock and select pins driven.
const CLOCK_LOW: u8 = 0b0110_0100;
/// The same with the clock high.
const CLOCK_HIGH: u8 = CLOCK_LOW | 0b10;
/// The data pin driven by the program (direction bit 4).
const SEND: u8 = 0b1_0000;
/// Chip select dropped with the clock high.
const DESELECTED: u8 = 0b0110_0010;
/// The data line's level bit.
const SIO: u8 = 1;

/// The ARM7 clock now, for an access made through its bus.
fn now(m: &Machine) -> u64 {
    m.inspect().arm7_clock()
}

fn put(m: &mut Machine, value: u8) {
    let at = now(m);
    m.bus7(at).write8(RTCCNT, value);
}

fn get(m: &mut Machine) -> u8 {
    let at = now(m);
    m.bus7(at).read8(RTCCNT)
}

fn send_bit(m: &mut Machine, bit: bool) {
    put(m, CLOCK_LOW | SEND | u8::from(bit));
    put(m, CLOCK_HIGH | SEND | u8::from(bit));
}

fn receive_bit(m: &mut Machine) -> bool {
    put(m, CLOCK_LOW);
    put(m, CLOCK_HIGH);
    get(m) & SIO != 0
}

/// A command byte, most significant bit first (the usual bytes, 65h and the like).
fn send_command(m: &mut Machine, command: u8) {
    (0..8)
        .rev()
        .for_each(|i| send_bit(m, command >> i & 1 != 0));
}

fn send_byte(m: &mut Machine, byte: u8) {
    (0..8).for_each(|i| send_bit(m, byte >> i & 1 != 0));
}

fn receive_bytes(m: &mut Machine, len: usize) -> Vec<u8> {
    (0..len)
        .map(|_| (0..8).fold(0, |byte, i| byte | u8::from(receive_bit(m)) << i))
        .collect()
}

/// A whole transfer: the command, the bytes to send, then `read_len` bytes to receive.
fn transfer(m: &mut Machine, command: u8, parameter: &[u8], read_len: usize) -> Vec<u8> {
    put(m, IDLE);
    put(m, SELECTED);
    send_command(m, command);
    parameter.iter().for_each(|&byte| send_byte(m, byte));
    let received = receive_bytes(m, read_len);
    put(m, DESELECTED);
    received
}

/// A frame run with the host's time at `wall_time`.
fn frame_at(m: &mut Machine, wall_time: u32) {
    m.run_frame(Input::new().with_wall_time(wall_time));
}

/// The date-and-time read: 65h.
fn read_date_time(m: &mut Machine) -> Vec<u8> {
    transfer(m, 0x65, &[], 7)
}

/// 2001-01-01 00:00:00 is 366 days (2000 is a leap year) after the start of the range.
const NEW_YEAR_2001: u32 = 366 * 86_400;

#[test]
fn given_the_arm9_when_it_writes_and_reads_rtccnt_then_it_reads_zero_and_the_arm7_is_unaffected() {
    let mut m = machine();
    m.bus9().write16(RTCCNT, 0xFFFF);
    let arm9 = m.bus9().read16(RTCCNT);
    let arm7 = get(&mut m);
    assert_eq!((arm9, arm7), (0, 0));
}

#[test]
fn given_the_arm7_when_it_writes_rtccnt_then_it_reads_back_and_the_arm9_still_reads_zero() {
    let mut m = machine();
    let at = now(&m);
    m.bus7(at).write16(RTCCNT, 0xFFFF);
    let arm7 = m.bus7(at).read16(RTCCNT);
    let arm9 = m.bus9().read16(RTCCNT);
    assert_eq!((arm7, arm9), (0xFFFF, 0));
}

#[test]
fn given_a_machine_at_power_on_when_inspected_then_the_chip_is_in_24_hour_mode_and_unset() {
    let m = Machine::new();
    let chip = m.inspect().rtc().chip();
    assert_eq!(
        (chip.status1(), chip.offset(), chip.selected()),
        (0x02, 0, false)
    );
}

#[test]
fn given_a_frame_with_a_host_time_when_the_arm7_reads_the_date_and_time_then_it_shows_that_time() {
    let mut m = machine();
    frame_at(&mut m, NEW_YEAR_2001 + 3_600 + 120 + 3);
    // Day 366: 366 mod 7 = 2, and 5 + 2 -> weekday 0.
    assert_eq!(
        read_date_time(&mut m),
        vec![0x01, 0x01, 0x01, 0x00, 0x01, 0x02, 0x03]
    );
}

#[test]
fn given_no_frame_run_when_the_arm7_reads_the_date_and_time_then_it_is_the_start_of_2000() {
    let mut m = machine();
    assert_eq!(
        read_date_time(&mut m),
        vec![0x00, 0x01, 0x01, 0x05, 0x00, 0x00, 0x00]
    );
}

#[test]
fn given_a_clock_set_when_a_later_frame_has_more_host_time_then_the_clock_advanced_by_it() {
    let mut m = machine();
    frame_at(&mut m, 1_000);
    // 64h: write the date and time (2024-03-15, weekday 4, 10:20:30).
    transfer(&mut m, 0x64, &[0x24, 0x03, 0x15, 0x04, 0x10, 0x20, 0x30], 0);
    let same_frame = read_date_time(&mut m);
    frame_at(&mut m, 1_000 + 90);
    let next_frame = read_date_time(&mut m);
    assert_eq!(
        (same_frame, next_frame),
        (
            vec![0x24, 0x03, 0x15, 0x04, 0x10, 0x20, 0x30],
            // 10:20:30 + 90 s = 10:22:00.
            vec![0x24, 0x03, 0x15, 0x04, 0x10, 0x22, 0x00]
        )
    );
}

#[test]
fn given_a_clock_set_when_the_host_time_jumps_back_then_the_clock_goes_back_with_it() {
    let mut m = machine();
    frame_at(&mut m, 5_000);
    transfer(&mut m, 0x66, &[0x01, 0x00, 0x00], 0);
    frame_at(&mut m, 4_000);
    // 01:00:00 - 1000 s = 00:43:20.
    assert_eq!(transfer(&mut m, 0x67, &[], 3), vec![0x00, 0x43, 0x20]);
}

#[test]
fn given_the_same_input_log_when_two_machines_run_it_then_their_savestates_are_identical() {
    let run = || {
        let mut m = machine();
        frame_at(&mut m, 700);
        transfer(&mut m, 0x66, &[0x12, 0x00, 0x00], 0);
        frame_at(&mut m, 760);
        m.save_state()
    };
    assert_eq!(run(), run());
}

#[test]
fn given_a_transfer_interrupted_by_a_savestate_when_it_finishes_then_it_matches_an_uninterrupted_one()
 {
    // Select the chip, send the read command for the date and time and receive two bytes.
    let start = |m: &mut Machine| {
        frame_at(m, NEW_YEAR_2001);
        put(m, IDLE);
        put(m, SELECTED);
        send_command(m, 0x65);
        receive_bytes(m, 2)
    };
    let mut whole = machine();
    let mut whole_read = start(&mut whole);
    whole_read.extend(receive_bytes(&mut whole, 5));
    put(&mut whole, DESELECTED);

    let mut saved = machine();
    start(&mut saved);
    let state = saved.save_state();
    let mut loaded = machine();
    let loaded_ok = loaded.load_state(&state);
    let mut second_half = receive_bytes(&mut loaded, 5);
    put(&mut loaded, DESELECTED);
    let mut resumed = vec![0x01, 0x01];
    resumed.append(&mut second_half);
    assert_eq!((loaded_ok, resumed), (Ok(()), whole_read));
}

#[test]
fn given_a_clock_set_when_the_state_is_saved_and_loaded_then_the_offset_survives() {
    let mut m = machine();
    frame_at(&mut m, 200);
    transfer(&mut m, 0x66, &[0x23, 0x59, 0x00], 0);
    let state = m.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    frame_at(&mut loaded, 200 + 60);
    // 23:59:00 + 60 s = 00:00:00 the next day (2000-01-02), and the weekday steps from 5 to 6.
    assert_eq!(
        (result, read_date_time(&mut loaded)),
        (Ok(()), vec![0x00, 0x01, 0x02, 0x06, 0x00, 0x00, 0x00])
    );
}
