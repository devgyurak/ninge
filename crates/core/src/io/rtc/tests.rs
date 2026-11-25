// SPDX-License-Identifier: Apache-2.0
//! RTCCNT and the pins it drives, from `docs/hw/rtc.md`. The helpers play the program of GBATEK's
//! flowchart: they write RTCCNT to raise chip select, move the clock and put each bit on the data
//! line, so the tests go through the register and not around it. A test body only asserts.

use super::*;
use crate::input::Input;
use crate::io::device::{Lines, cx_at};
use crate::io::map::{Owner, Port, decode};
use crate::rtc::Session;

/// RTCCNT with chip select low and the clock high, every pin driven by the program: the state a
/// transfer starts from and ends in.
const IDLE: u8 = (CS_OUT | SCK_OUT | SIO_OUT) as u8 | SCK as u8 | SIO as u8;
/// The same with chip select high.
const SELECTED: u8 = IDLE | CS as u8;
/// The data pin driven by the program: set for a bit it sends. A bit it receives leaves the
/// direction bit clear.
const SEND: u8 = SIO_OUT as u8;
/// Chip select and the clock driven, the clock low.
const CLOCK_LOW: u8 = (CS_OUT | SCK_OUT | CS) as u8;
/// The same with the clock high.
const CLOCK_HIGH: u8 = CLOCK_LOW | SCK as u8;
/// Chip select dropped, the clock high: a transfer's last write.
const DESELECTED: u8 = (CS_OUT | SCK_OUT) as u8 | SCK as u8;

fn write(rtc: &mut Rtc, host_time: u32, offset: u32, width: Width, value: u32) {
    let input = Input::new().with_wall_time(host_time);
    let cx = Cx {
        now: 0,
        host: &input,
        rom: &[],
        lines: Lines {
            pen_irq_enabled: true,
            ..Lines::default()
        },
    };
    Device::<false>::write(rtc, offset, width, value, cx);
}

fn peek(rtc: &Rtc, offset: u32, width: Width) -> u32 {
    Device::<false>::peek(rtc, offset, width, cx_at(0))
}

/// One byte written to RTCCNT.
fn put(rtc: &mut Rtc, host_time: u32, value: u8) {
    write(rtc, host_time, 0, Width::Byte, u32::from(value));
}

/// RTCCNT's low byte as the program reads it.
fn get(rtc: &Rtc) -> u8 {
    peek(rtc, 0, Width::Byte) as u8
}

/// A bit sent to the chip: the clock goes low with the bit on the line, then high.
fn send_bit(rtc: &mut Rtc, host_time: u32, bit: bool) {
    put(rtc, host_time, CLOCK_LOW | SEND | u8::from(bit));
    put(rtc, host_time, CLOCK_HIGH | SEND | u8::from(bit));
}

/// A bit received from the chip: the clock goes low, then high, and the program looks at the line
/// (it leaves the data pin an input).
fn receive_bit(rtc: &mut Rtc, host_time: u32) -> bool {
    put(rtc, host_time, CLOCK_LOW);
    put(rtc, host_time, CLOCK_HIGH);
    get(rtc) & SIO as u8 != 0
}

/// A command byte, most significant bit first, as the programs write the usual bytes (65h, 61h).
fn send_command(rtc: &mut Rtc, host_time: u32, command: u8) {
    (0..8)
        .rev()
        .for_each(|i| send_bit(rtc, host_time, command >> i & 1 != 0));
}

/// A whole transfer as the programs write it: the command byte most significant bit first (the
/// usual byte, such as 65h to read the date and time), the parameter bytes to send least significant
/// bit first, and `read_len` bytes to receive.
fn transfer(
    rtc: &mut Rtc,
    host_time: u32,
    command: u8,
    parameter: &[u8],
    read_len: usize,
) -> Vec<u8> {
    put(rtc, host_time, IDLE);
    put(rtc, host_time, SELECTED);
    send_command(rtc, host_time, command);
    parameter.iter().for_each(|&byte| {
        (0..8).for_each(|i| send_bit(rtc, host_time, byte >> i & 1 != 0));
    });
    let received = (0..read_len)
        .map(|_| {
            (0..8).fold(0, |byte, i| {
                byte | u8::from(receive_bit(rtc, host_time)) << i
            })
        })
        .collect();
    put(rtc, host_time, DESELECTED);
    received
}

// ------------------------------------------------------------------------------ the register

#[test]
fn given_a_new_device_when_rtccnt_is_read_then_it_is_zero() {
    assert_eq!(peek(&Rtc::new(), 0, Width::Half), 0);
}

#[test]
fn given_every_bit_written_when_rtccnt_is_read_then_every_driven_line_reads_back() {
    let mut rtc = Rtc::new();
    write(&mut rtc, 0, 0, Width::Half, 0xFFFF);
    assert_eq!(peek(&rtc, 0, Width::Half), 0xFFFF);
}

#[test]
fn given_levels_written_without_directions_when_read_then_the_lines_read_zero() {
    // Nothing drives a line the program only reads, so the level bits written do not show.
    let mut rtc = Rtc::new();
    write(&mut rtc, 0, 0, Width::Half, 0x0F0F);
    assert_eq!(peek(&rtc, 0, Width::Half), 0x0000);
}

#[test]
fn given_directions_written_with_levels_for_only_some_lines_when_read_then_driven_levels_show() {
    let mut rtc = Rtc::new();
    // Line 3 and line 8 driven (directions in bits 7 and 12) and high; line 9 high but not driven.
    write(
        &mut rtc,
        0,
        0,
        Width::Half,
        0x1000 | 0x0080 | 0x0200 | 0x0100 | 0x0008,
    );
    assert_eq!(
        peek(&rtc, 0, Width::Half),
        0x1000 | 0x0080 | 0x0100 | 0x0008
    );
}

#[test]
fn given_the_bytes_past_rtccnt_when_read_or_written_then_they_hold_nothing() {
    let mut rtc = Rtc::new();
    write(&mut rtc, 0, 2, Width::Half, 0xFFFF);
    assert_eq!(
        (peek(&rtc, 2, Width::Half), peek(&rtc, 0, Width::Half)),
        (0, 0)
    );
}

#[test]
fn given_a_halfword_write_when_compared_with_two_byte_writes_then_the_device_is_the_same() {
    let mut halves = Rtc::new();
    write(&mut halves, 0, 0, Width::Half, u32::from(SELECTED) | 0x1000);
    let mut bytes = Rtc::new();
    write(&mut bytes, 0, 0, Width::Byte, u32::from(SELECTED));
    write(&mut bytes, 0, 1, Width::Byte, 0x10);
    assert_eq!(halves, bytes);
}

// ----------------------------------------------------------------------------- transfers

#[test]
fn given_the_usual_sequence_when_status_1_is_read_then_it_is_24_hour_mode() {
    let mut rtc = Rtc::new();
    // 61h: read status register 1.
    assert_eq!(transfer(&mut rtc, 0, 0x61, &[], 1), vec![0x02]);
}

#[test]
fn given_the_usual_sequence_when_the_date_and_time_are_read_then_they_are_the_host_s_time() {
    let mut rtc = Rtc::new();
    // 65h: read the date and time. Host time: one day, one hour, one minute, one second.
    assert_eq!(
        transfer(&mut rtc, 86_400 + 3_661, 0x65, &[], 7),
        vec![0x00, 0x01, 0x02, 0x06, 0x01, 0x01, 0x01]
    );
}

#[test]
fn given_a_date_and_time_written_when_a_later_frame_reads_it_then_the_offset_applies() {
    let mut rtc = Rtc::new();
    // 64h: write the date and time, at host time 1000; read it back 100 host seconds later.
    transfer(
        &mut rtc,
        1_000,
        0x64,
        &[0x24, 0x03, 0x15, 0x04, 0x10, 0x20, 0x30],
        0,
    );
    // 10:20:30 + 100 s = 10:22:10; hour 10 is below noon, so the byte has no PM flag.
    assert_eq!(
        transfer(&mut rtc, 1_100, 0x65, &[], 7),
        vec![0x24, 0x03, 0x15, 0x04, 0x10, 0x22, 0x10]
    );
}

#[test]
fn given_the_time_written_when_the_register_is_read_then_it_is_what_was_written() {
    let mut rtc = Rtc::new();
    // 66h writes the time and 67h reads it.
    transfer(&mut rtc, 0, 0x66, &[0x08, 0x15, 0x45], 0);
    assert_eq!(transfer(&mut rtc, 0, 0x67, &[], 3), vec![0x08, 0x15, 0x45]);
}

#[test]
fn given_status_register_2_written_and_read_when_transferred_then_it_round_trips() {
    let mut rtc = Rtc::new();
    // 62h writes status register 2 and 63h reads it.
    transfer(&mut rtc, 0, 0x62, &[0x41], 0);
    assert_eq!(transfer(&mut rtc, 0, 0x63, &[], 1), vec![0x41]);
}

#[test]
fn given_two_transfers_in_a_row_when_the_second_reads_then_the_first_does_not_disturb_it() {
    let mut rtc = Rtc::new();
    let first = transfer(&mut rtc, 0, 0x61, &[], 1);
    let second = transfer(&mut rtc, 0, 0x61, &[], 1);
    assert_eq!((first, second), (vec![0x02], vec![0x02]));
}

#[test]
fn given_the_program_selects_the_chip_when_inspected_then_a_command_is_awaited() {
    let mut rtc = Rtc::new();
    put(&mut rtc, 0, IDLE);
    let idle = rtc.chip().session();
    put(&mut rtc, 0, SELECTED);
    assert_eq!(
        (idle, rtc.chip().session()),
        (Session::Idle, Session::Command { byte: 0, bits: 0 })
    );
}

#[test]
fn given_the_clock_pin_is_an_input_when_its_level_bit_toggles_then_the_chip_sees_no_edge() {
    let mut rtc = Rtc::new();
    put(&mut rtc, 0, IDLE);
    put(&mut rtc, 0, SELECTED);
    // The clock direction bit is clear, so the program does not drive the pin.
    let level_only = (CS_OUT | CS | SIO_OUT) as u8;
    put(&mut rtc, 0, level_only);
    put(&mut rtc, 0, level_only | SCK as u8);
    put(&mut rtc, 0, level_only);
    assert_eq!(rtc.chip().session(), Session::Command { byte: 0, bits: 0 });
}

#[test]
fn given_the_clock_written_high_again_when_it_is_already_high_then_no_bit_is_taken() {
    let mut rtc = Rtc::new();
    put(&mut rtc, 0, IDLE);
    put(&mut rtc, 0, SELECTED);
    send_bit(&mut rtc, 0, true);
    put(&mut rtc, 0, CLOCK_HIGH | SEND | 1);
    put(&mut rtc, 0, CLOCK_HIGH | SEND | 1);
    assert_eq!(rtc.chip().session(), Session::Command { byte: 1, bits: 1 });
}

#[test]
fn given_chip_select_dropped_with_a_clock_edge_in_one_write_when_applied_then_the_chip_is_idle() {
    let mut rtc = Rtc::new();
    put(&mut rtc, 0, IDLE);
    put(&mut rtc, 0, SELECTED);
    put(&mut rtc, 0, CLOCK_LOW | SEND);
    // Chip select falls and the clock rises together: the select goes first, so the edge finds
    // the chip idle and no bit is taken.
    put(&mut rtc, 0, (CS_OUT | SCK_OUT) as u8 | SCK as u8 | SEND | 1);
    assert_eq!(rtc.chip().session(), Session::Idle);
}

#[test]
fn given_chip_select_and_the_clock_rising_in_one_write_when_applied_then_no_command_bit_is_taken() {
    let mut rtc = Rtc::new();
    // Everything driven and low, then select and the clock rise together with the data line high.
    put(&mut rtc, 0, (CS_OUT | SCK_OUT | SIO_OUT) as u8);
    put(&mut rtc, 0, SELECTED);
    assert_eq!(rtc.chip().session(), Session::Command { byte: 0, bits: 0 });
}

#[test]
fn given_the_chip_sending_when_the_program_drives_the_data_pin_then_the_pin_reads_the_program_s_level()
 {
    let mut rtc = Rtc::new();
    put(&mut rtc, 0, IDLE);
    put(&mut rtc, 0, SELECTED);
    // Read status 1 (61h), then let the chip put its first bit (0) and its second (1) on the line.
    send_command(&mut rtc, 0, 0x61);
    put(&mut rtc, 0, CLOCK_LOW);
    put(&mut rtc, 0, CLOCK_HIGH);
    put(&mut rtc, 0, CLOCK_LOW);
    let chip_drives = get(&rtc) & SIO as u8;
    // The program now drives the pin low: it reads its own level, not the chip's.
    put(&mut rtc, 0, CLOCK_LOW | SEND);
    let program_drives = get(&rtc) & SIO as u8;
    assert_eq!((chip_drives, program_drives), (1, 0));
}

// ------------------------------------------------------------------------------ the checks

#[test]
fn given_a_new_device_when_checked_then_it_is_well_formed() {
    assert!(Checked::well_formed(&Rtc::new(), StateTimes::at(0)));
}

#[test]
fn given_a_chip_out_of_range_when_the_device_is_checked_then_it_is_not_well_formed() {
    let mut rtc = Rtc::new();
    rtc.chip = Chip::with_session_for_test(Session::Command { byte: 0, bits: 9 });
    assert!(!Checked::well_formed(&rtc, StateTimes::at(0)));
}

// ----------------------------------------------------------------------------------- the map

#[test]
fn given_the_two_cpus_when_the_address_is_decoded_then_only_the_arm7_reaches_the_device() {
    assert_eq!(
        (decode::<true>(0x0400_0138), decode::<false>(0x0400_0138)),
        (None, Some(Owner::Io(Port::Rtc, 0)))
    );
}
