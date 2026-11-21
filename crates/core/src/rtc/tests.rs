// SPDX-License-Identifier: Apache-2.0
//! The clock chip's serial protocol and registers, from `docs/hw/rtc.md`. The helpers play the
//! program: they drive the chip's pins edge by edge, least significant bit first, and a test body
//! only asserts. Expected values are worked out from the note and the calendar, never read off the
//! code.

use super::*;

/// The host's time in the tests that do not care: the first second of 2000.
const EPOCH: u32 = 0;
/// A day of host time.
const DAY: u32 = 86_400;

/// The command numbers of `rtc.md`, "The command byte" (bits 4-6).
const STATUS1: u8 = 0;
const STATUS2: u8 = 4;
const DATE_TIME: u8 = 2;
const TIME: u8 = 6;
const INT1: u8 = 1;
const INT2: u8 = 5;
const ADJUST: u8 = 3;
const FREE: u8 = 7;

/// The command byte in the order the bits arrive: the fixed code, the command and the access.
const fn command(number: u8, read: bool) -> u8 {
    // The fixed code written out: bits 0-3 are 0, 1, 1, 0 as they arrive.
    0x06 | number << 4 | if read { 0x80 } else { 0 }
}

/// One bit as the program sends it: the clock falls, and rises with `bit` on the data line.
fn send_bit(chip: &mut Chip, bit: bool, host: u32) {
    chip.clock_falling();
    chip.clock_rising(bit, host);
}

/// A byte, least significant bit first.
fn send_byte(chip: &mut Chip, byte: u8, host: u32) {
    (0..8).for_each(|i| send_bit(chip, byte >> i & 1 != 0, host));
}

/// A byte most significant bit first: the order the usual command bytes are written in.
fn send_byte_msb_first(chip: &mut Chip, byte: u8, host: u32) {
    (0..8)
        .rev()
        .for_each(|i| send_bit(chip, byte >> i & 1 != 0, host));
}

/// The low `count` bits of `byte`, least significant first, and no more.
fn send_low_bits(chip: &mut Chip, byte: u8, count: u8, host: u32) {
    (0..count).for_each(|i| send_bit(chip, byte >> i & 1 != 0, host));
}

/// `count` bits of one, as a program that has lost track of the protocol clocks them.
fn send_ones(chip: &mut Chip, count: u32, host: u32) {
    (0..count).for_each(|_| send_bit(chip, true, host));
}

/// Each byte of `bytes`, least significant bit first.
fn send_bytes(chip: &mut Chip, bytes: &[u8], host: u32) {
    bytes.iter().for_each(|&byte| send_byte(chip, byte, host));
}

/// `len` bytes the chip sends, with the host's time `host` for every clock.
fn receive_bytes(chip: &mut Chip, len: usize, host: u32) -> Vec<u8> {
    (0..len).map(|_| receive_byte(chip, host)).collect()
}

/// A byte the chip sends, least significant bit first: the clock falls, the program looks at the
/// data line, and the clock rises.
fn receive_byte(chip: &mut Chip, host: u32) -> u8 {
    (0..8).fold(0, |byte, i| {
        chip.clock_falling();
        let bit = chip.data_out();
        chip.clock_rising(false, host);
        byte | u8::from(bit) << i
    })
}

/// A whole write: select, command, the parameter bytes, deselect.
fn write_register(chip: &mut Chip, number: u8, bytes: &[u8], host: u32) {
    chip.select();
    send_byte(chip, command(number, false), host);
    send_bytes(chip, bytes, host);
    chip.deselect();
}

/// A whole read of `len` bytes: select, command, the bytes, deselect.
fn read_register(chip: &mut Chip, number: u8, len: usize, host: u32) -> Vec<u8> {
    chip.select();
    send_byte(chip, command(number, true), host);
    let bytes = receive_bytes(chip, len, host);
    chip.deselect();
    bytes
}

/// The date and time the chip shows at host time `host`.
fn date_time(chip: &mut Chip, host: u32) -> Vec<u8> {
    read_register(chip, DATE_TIME, 7, host)
}

/// A chip whose clock was set to `bytes` (a date-and-time write) at host time `host`.
fn set_to(bytes: [u8; 7], host: u32) -> Chip {
    let mut chip = Chip::new();
    write_register(&mut chip, DATE_TIME, &bytes, host);
    chip
}

// ------------------------------------------------------------------------------ power-on state

#[test]
fn given_a_chip_at_power_on_when_status_1_is_read_then_it_is_24_hour_mode_with_no_flags() {
    let mut chip = Chip::new();
    assert_eq!(read_register(&mut chip, STATUS1, 1, EPOCH), vec![0x02]);
}

#[test]
fn given_a_chip_at_power_on_when_the_other_registers_are_read_then_they_are_zero() {
    let mut chip = Chip::new();
    let registers = [
        read_register(&mut chip, STATUS2, 1, EPOCH),
        read_register(&mut chip, INT1, 1, EPOCH),
        read_register(&mut chip, INT2, 3, EPOCH),
        read_register(&mut chip, ADJUST, 1, EPOCH),
        read_register(&mut chip, FREE, 1, EPOCH),
    ];
    assert_eq!(
        registers,
        [vec![0], vec![0], vec![0, 0, 0], vec![0], vec![0]]
    );
}

#[test]
fn given_a_chip_never_set_when_the_date_and_time_are_read_at_host_time_zero_then_it_is_2000_01_01()
{
    let mut chip = Chip::new();
    // 2000-01-01 was a Saturday: 5 with Monday as 0.
    assert_eq!(
        date_time(&mut chip, EPOCH),
        vec![0x00, 0x01, 0x01, 0x05, 0x00, 0x00, 0x00]
    );
}

#[test]
fn given_a_chip_never_set_when_the_host_time_is_a_day_an_hour_a_minute_and_a_second_then_it_shows_it()
 {
    let mut chip = Chip::new();
    assert_eq!(
        date_time(&mut chip, DAY + 3_600 + 60 + 1),
        vec![0x00, 0x01, 0x02, 0x06, 0x01, 0x01, 0x01]
    );
}

#[test]
fn given_a_chip_never_set_when_the_time_register_is_read_then_it_is_hour_minute_second() {
    let mut chip = Chip::new();
    assert_eq!(
        read_register(&mut chip, TIME, 3, 3_600 + 120 + 3),
        vec![0x01, 0x02, 0x03]
    );
}

// ------------------------------------------------------------------------------ setting the clock

#[test]
fn given_a_date_and_time_written_when_read_in_the_same_frame_then_it_is_what_was_written() {
    let mut chip = set_to([0x24, 0x12, 0x31, 0x03, 0x23, 0x59, 0x58], 1_000);
    // In 24-hour mode the hour byte's bit 6 reads 1 from noon on, so 23h reads 63h.
    assert_eq!(
        date_time(&mut chip, 1_000),
        vec![0x24, 0x12, 0x31, 0x03, 0x63, 0x59, 0x58]
    );
}

#[test]
fn given_a_clock_set_when_a_later_frame_has_two_more_host_seconds_then_the_year_rolls_over() {
    let mut chip = set_to([0x24, 0x12, 0x31, 0x03, 0x23, 0x59, 0x58], 1_000);
    // 23:59:58 + 2 s = 2025-01-01 00:00:00; the weekday counter steps from 3 to 4.
    assert_eq!(
        date_time(&mut chip, 1_002),
        vec![0x25, 0x01, 0x01, 0x04, 0x00, 0x00, 0x00]
    );
}

#[test]
fn given_a_leap_year_when_the_clock_passes_february_28_then_it_shows_the_29th_and_then_march_1() {
    let mut chip = set_to([0x24, 0x02, 0x28, 0x02, 0x23, 0x59, 0x59], 0);
    let shown = (date_time(&mut chip, 1), date_time(&mut chip, 1 + DAY));
    assert_eq!(
        shown,
        (
            vec![0x24, 0x02, 0x29, 0x03, 0x00, 0x00, 0x00],
            vec![0x24, 0x03, 0x01, 0x04, 0x00, 0x00, 0x00]
        )
    );
}

#[test]
fn given_a_common_year_when_the_clock_passes_february_28_then_it_shows_march_1() {
    let mut chip = set_to([0x23, 0x02, 0x28, 0x02, 0x23, 0x59, 0x59], 0);
    assert_eq!(
        date_time(&mut chip, 1),
        vec![0x23, 0x03, 0x01, 0x03, 0x00, 0x00, 0x00]
    );
}

#[test]
fn given_the_last_second_of_2099_when_the_clock_passes_it_then_it_wraps_to_2000() {
    let mut chip = set_to([0x99, 0x12, 0x31, 0x06, 0x23, 0x59, 0x59], 0);
    // The weekday is left out: the hundred years are not a whole number of weeks, so the counter
    // does not step by one across the wrap (`rtc.md`, "Unverified").
    let shown = date_time(&mut chip, 1);
    assert_eq!(
        [shown[0], shown[1], shown[2], shown[4], shown[5], shown[6]],
        [0x00, 0x01, 0x01, 0x00, 0x00, 0x00]
    );
}

#[test]
fn given_a_time_written_when_the_date_is_read_then_the_date_and_weekday_are_unchanged() {
    let mut chip = Chip::new();
    // Host day 3: 2000-01-04, weekday 5 + 3 = 8 -> 1.
    write_register(&mut chip, TIME, &[0x12, 0x34, 0x56], 3 * DAY);
    assert_eq!(
        date_time(&mut chip, 3 * DAY),
        // The hour byte's bit 6 reads 1 from noon on: 12h reads 52h.
        vec![0x00, 0x01, 0x04, 0x01, 0x52, 0x34, 0x56]
    );
}

#[test]
fn given_a_time_written_when_the_host_time_advances_then_the_time_advances_with_it() {
    let mut chip = Chip::new();
    write_register(&mut chip, TIME, &[0x23, 0x59, 0x59], 0);
    assert_eq!(
        date_time(&mut chip, 2),
        vec![0x00, 0x01, 0x02, 0x06, 0x00, 0x00, 0x01]
    );
}

/// The date-and-time payloads that are not calendar values, each of which a write must drop.
fn invalid_date_times() -> Vec<[u8; 7]> {
    vec![
        // Month 13, month 0, a BCD digit above 9, day 0, day 32, February 30, weekday 7, hour 24,
        // minute 60, second 60.
        [0x24, 0x13, 0x01, 0x00, 0x00, 0x00, 0x00],
        [0x24, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
        [0x2A, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00],
        [0x24, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00],
        [0x24, 0x01, 0x32, 0x00, 0x00, 0x00, 0x00],
        [0x24, 0x02, 0x30, 0x00, 0x00, 0x00, 0x00],
        [0x23, 0x02, 0x29, 0x00, 0x00, 0x00, 0x00],
        [0x24, 0x01, 0x01, 0x07, 0x00, 0x00, 0x00],
        [0x24, 0x01, 0x01, 0x00, 0x24, 0x00, 0x00],
        [0x24, 0x01, 0x01, 0x00, 0x00, 0x60, 0x00],
        [0x24, 0x01, 0x01, 0x00, 0x00, 0x00, 0x60],
    ]
}

/// The payloads of `rows` that changed the clock although they are not calendar values.
fn writes_that_were_taken(rows: &[[u8; 7]]) -> Vec<[u8; 7]> {
    rows.iter()
        .filter(|&&row| {
            let mut chip = Chip::new();
            let before = date_time(&mut chip, 500);
            write_register(&mut chip, DATE_TIME, &row, 500);
            date_time(&mut chip, 500) != before
        })
        .copied()
        .collect()
}

#[test]
fn given_a_date_and_time_that_is_not_a_calendar_value_when_written_then_the_clock_is_unchanged() {
    assert_eq!(
        writes_that_were_taken(&invalid_date_times()),
        Vec::<[u8; 7]>::new()
    );
}

#[test]
fn given_a_time_with_a_minute_of_60_when_written_then_the_clock_is_unchanged() {
    let mut chip = Chip::new();
    let before = date_time(&mut chip, 500);
    write_register(&mut chip, TIME, &[0x10, 0x60, 0x00], 500);
    assert_eq!(date_time(&mut chip, 500), before);
}

#[test]
fn given_unused_bits_set_in_the_date_bytes_when_written_then_they_are_ignored_and_read_zero() {
    // Month bits 5-7, day bits 6-7, weekday bits 3-7, minute and second bit 7 are not used.
    let mut chip = set_to([0x24, 0xE1, 0xC1, 0xF8, 0x80, 0x80, 0x80], 0);
    assert_eq!(
        date_time(&mut chip, 0),
        vec![0x24, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00]
    );
}

// ------------------------------------------------------------------------------- hour modes

#[test]
fn given_24_hour_mode_when_the_clock_is_past_noon_then_the_hour_is_bcd_with_the_pm_flag() {
    let mut chip = Chip::new();
    assert_eq!(
        read_register(&mut chip, TIME, 3, 13 * 3_600),
        vec![0x53, 0x00, 0x00]
    );
}

#[test]
fn given_12_hour_mode_when_the_clock_is_at_noon_and_one_pm_then_noon_is_00_with_the_pm_flag() {
    let mut chip = Chip::new();
    write_register(&mut chip, STATUS1, &[0x00], 0);
    let shown = (
        read_register(&mut chip, TIME, 3, 12 * 3_600),
        read_register(&mut chip, TIME, 3, 13 * 3_600),
        read_register(&mut chip, TIME, 3, 0),
    );
    assert_eq!(
        shown,
        (
            vec![0x40, 0x00, 0x00],
            vec![0x41, 0x00, 0x00],
            vec![0x00, 0x00, 0x00]
        )
    );
}

#[test]
fn given_a_time_set_in_12_hour_mode_when_the_mode_changes_then_the_time_is_the_same() {
    let mut chip = Chip::new();
    write_register(&mut chip, STATUS1, &[0x00], 0);
    // 5 PM in 12-hour mode.
    write_register(&mut chip, TIME, &[0x45, 0x30, 0x00], 0);
    let in_12 = read_register(&mut chip, TIME, 3, 0);
    write_register(&mut chip, STATUS1, &[0x02], 0);
    let in_24 = read_register(&mut chip, TIME, 3, 0);
    assert_eq!(
        (in_12, in_24),
        (vec![0x45, 0x30, 0x00], vec![0x57, 0x30, 0x00])
    );
}

#[test]
fn given_24_hour_mode_when_an_hour_is_written_with_the_pm_bit_then_the_bit_does_not_change_it() {
    let mut chip = Chip::new();
    // Bit 6 is read-only here: 23 stays 23, and a morning hour with the bit set stays morning.
    write_register(&mut chip, TIME, &[0x63, 0x00, 0x00], 0);
    let late = read_register(&mut chip, TIME, 3, 0);
    write_register(&mut chip, TIME, &[0x45, 0x00, 0x00], 0);
    let early = read_register(&mut chip, TIME, 3, 0);
    assert_eq!(
        (late, early),
        (vec![0x63, 0x00, 0x00], vec![0x05, 0x00, 0x00])
    );
}

#[test]
fn given_12_hour_mode_when_an_hour_of_12_is_written_then_the_clock_is_unchanged() {
    let mut chip = Chip::new();
    write_register(&mut chip, STATUS1, &[0x00], 0);
    write_register(&mut chip, TIME, &[0x12, 0x00, 0x00], 0);
    assert_eq!(read_register(&mut chip, TIME, 3, 0), vec![0x00, 0x00, 0x00]);
}

// ------------------------------------------------------------------- the other registers

#[test]
fn given_status_1_when_written_with_every_bit_then_only_the_mode_and_general_purpose_bits_stay() {
    let mut chip = Chip::new();
    write_register(&mut chip, STATUS1, &[0xFE], 0);
    assert_eq!(read_register(&mut chip, STATUS1, 1, 0), vec![0x0E]);
}

#[test]
fn given_status_2_the_adjust_and_the_free_register_when_written_then_each_reads_back_all_8_bits() {
    let mut chip = Chip::new();
    write_register(&mut chip, STATUS2, &[0xFB], 0);
    write_register(&mut chip, ADJUST, &[0x5A], 0);
    write_register(&mut chip, FREE, &[0xA5], 0);
    let registers = (
        read_register(&mut chip, STATUS2, 1, 0),
        read_register(&mut chip, ADJUST, 1, 0),
        read_register(&mut chip, FREE, 1, 0),
    );
    assert_eq!(registers, (vec![0xFB], vec![0x5A], vec![0xA5]));
}

#[test]
fn given_alarm_2_when_written_then_the_day_byte_keeps_bits_0_to_2_and_7_and_the_rest_all_8() {
    let mut chip = Chip::new();
    write_register(&mut chip, INT2, &[0xFF, 0x12, 0xC3], 0);
    assert_eq!(read_register(&mut chip, INT2, 3, 0), vec![0x87, 0x12, 0xC3]);
}

#[test]
fn given_int1_in_frequency_mode_when_written_then_one_byte_is_read_back() {
    let mut chip = Chip::new();
    write_register(&mut chip, INT1, &[0x1F], 0);
    assert_eq!(read_register(&mut chip, INT1, 1, 0), vec![0x1F]);
}

#[test]
fn given_int1_in_alarm_mode_when_written_then_three_bytes_are_read_back() {
    let mut chip = Chip::new();
    write_register(&mut chip, STATUS2, &[0x04], 0);
    write_register(&mut chip, INT1, &[0xFF, 0x82, 0x83], 0);
    assert_eq!(read_register(&mut chip, INT1, 3, 0), vec![0x87, 0x82, 0x83]);
}

#[test]
fn given_the_frequency_byte_and_alarm_1_s_minute_when_one_is_written_then_the_other_changes() {
    let mut chip = Chip::new();
    write_register(&mut chip, INT1, &[0x15], 0);
    write_register(&mut chip, STATUS2, &[0x04], 0);
    let alarm = read_register(&mut chip, INT1, 3, 0);
    write_register(&mut chip, INT1, &[0x01, 0x02, 0x33], 0);
    write_register(&mut chip, STATUS2, &[0x00], 0);
    let frequency = read_register(&mut chip, INT1, 1, 0);
    assert_eq!((alarm, frequency), (vec![0x00, 0x00, 0x15], vec![0x33]));
}

// ------------------------------------------------------------------------------------ reset

#[test]
fn given_a_chip_in_use_when_status_1_bit_0_is_written_then_every_register_and_the_clock_reset() {
    let mut chip = set_to([0x24, 0x06, 0x15, 0x02, 0x08, 0x30, 0x00], 700);
    write_register(&mut chip, STATUS2, &[0xFF], 700);
    write_register(&mut chip, FREE, &[0xFF], 700);
    write_register(&mut chip, INT2, &[0x01, 0x02, 0x03], 700);
    write_register(&mut chip, STATUS1, &[0x0F], 700);
    let after = (
        read_register(&mut chip, STATUS1, 1, 700),
        read_register(&mut chip, STATUS2, 1, 700),
        read_register(&mut chip, FREE, 1, 700),
        read_register(&mut chip, INT2, 3, 700),
        // 12-hour mode after a reset, so the hour byte is 00h at midnight.
        date_time(&mut chip, 700),
        date_time(&mut chip, 700 + DAY),
    );
    assert_eq!(
        after,
        (
            vec![0x00],
            vec![0x00],
            vec![0x00],
            vec![0, 0, 0],
            vec![0x00, 0x01, 0x01, 0x05, 0x00, 0x00, 0x00],
            vec![0x00, 0x01, 0x02, 0x06, 0x00, 0x00, 0x00]
        )
    );
}

// ------------------------------------------------------------------------- the transfer

#[test]
fn given_a_date_and_time_read_command_when_sent_most_significant_bit_first_then_the_data_is_the_same()
 {
    // The usual byte for the read, 65h, sent MSB first: the same bits on the wire as the
    // LSB-first byte `command(DATE_TIME, true)`.
    let mut chip = Chip::new();
    chip.select();
    send_byte_msb_first(&mut chip, 0x65, 0);
    let bytes = receive_bytes(&mut chip, 7, 0);
    assert_eq!(
        (command(DATE_TIME, true), bytes),
        (0xA6, vec![0x00, 0x01, 0x01, 0x05, 0x00, 0x00, 0x00])
    );
}

#[test]
fn given_a_wrong_fixed_code_when_the_command_is_sent_then_the_chip_ignores_the_session() {
    let mut chip = Chip::new();
    chip.select();
    // Bit 0 set: not the fixed code.
    send_byte(&mut chip, 0x01 | STATUS1 << 4 | READ, 0);
    let ignored = chip.session();
    // Clocks that follow change nothing, and the line stays quiet.
    let line = receive_byte(&mut chip, 0);
    assert_eq!((ignored, line), (Session::Ignored, 0));
}

#[test]
fn given_the_extended_command_bit_when_the_command_is_sent_then_the_chip_ignores_the_session() {
    let mut chip = Chip::new();
    chip.select();
    send_byte(&mut chip, command(STATUS1, true) | 0x08, 0);
    assert_eq!(chip.session(), Session::Ignored);
}

#[test]
fn given_an_ignored_session_when_the_chip_is_deselected_and_selected_then_a_command_works_again() {
    let mut chip = Chip::new();
    chip.select();
    send_byte(&mut chip, 0x00, 0);
    chip.deselect();
    assert_eq!(read_register(&mut chip, STATUS1, 1, 0), vec![0x02]);
}

#[test]
fn given_chip_select_dropping_in_the_middle_of_a_command_when_selected_again_then_a_new_command_starts()
 {
    let mut chip = Chip::new();
    chip.select();
    send_low_bits(&mut chip, command(STATUS1, true), 4, 0);
    chip.deselect();
    assert_eq!(read_register(&mut chip, STATUS1, 1, 0), vec![0x02]);
}

#[test]
fn given_a_write_cut_short_when_the_chip_is_deselected_then_nothing_was_written() {
    let mut chip = Chip::new();
    let before = date_time(&mut chip, 0);
    // Six of the seven bytes of a date and time, and half of the seventh.
    chip.select();
    send_byte(&mut chip, command(DATE_TIME, false), 0);
    send_bytes(&mut chip, &[0x24, 0x06, 0x15, 0x02, 0x08, 0x30], 0);
    send_ones(&mut chip, 4, 0);
    chip.deselect();
    assert_eq!(date_time(&mut chip, 0), before);
}

#[test]
fn given_a_register_written_when_more_clocks_follow_then_they_change_nothing() {
    let mut chip = Chip::new();
    chip.select();
    send_byte(&mut chip, command(FREE, false), 0);
    send_byte(&mut chip, 0x11, 0);
    send_byte(&mut chip, 0xFF, 0);
    chip.deselect();
    assert_eq!(read_register(&mut chip, FREE, 1, 0), vec![0x11]);
}

#[test]
fn given_a_read_when_more_clocks_follow_the_last_byte_then_the_line_reads_zero() {
    let mut chip = Chip::new();
    chip.select();
    send_byte(&mut chip, command(STATUS1, true), 0);
    let bytes = [receive_byte(&mut chip, 0), receive_byte(&mut chip, 0)];
    assert_eq!(bytes, [0x02, 0x00]);
}

#[test]
fn given_a_chip_that_is_not_selected_when_the_clock_moves_then_nothing_happens() {
    let mut chip = Chip::new();
    send_byte(&mut chip, command(FREE, false), 0);
    send_byte(&mut chip, 0xAA, 0);
    assert_eq!(
        (chip.session(), chip.free(), chip.data_out()),
        (Session::Idle, 0, false)
    );
}

#[test]
fn given_a_read_in_progress_when_the_chip_is_deselected_then_the_session_ends_and_the_line_is_quiet()
 {
    let mut chip = Chip::new();
    chip.select();
    send_byte(&mut chip, command(STATUS1, true), 0);
    // Status 1 is 02h: bit 0 is 0, bit 1 is 1, so the line is high for the second bit.
    chip.clock_falling();
    chip.clock_rising(false, 0);
    chip.clock_falling();
    let driven = chip.data_out();
    chip.deselect();
    assert_eq!(
        (driven, chip.selected(), chip.data_out()),
        (true, false, false)
    );
}

#[test]
fn given_a_date_and_time_read_when_the_host_time_changes_during_the_transfer_then_the_bytes_are_one_instant()
 {
    let mut chip = Chip::new();
    chip.select();
    send_byte(&mut chip, command(DATE_TIME, true), 59);
    // The first byte is taken at the end of the command; the later clocks run with a later time.
    let bytes: Vec<u8> = (0..7)
        .map(|_| receive_byte(&mut chip, 59 + 5 * DAY))
        .collect();
    assert_eq!(bytes, vec![0x00, 0x01, 0x01, 0x05, 0x00, 0x00, 0x59]);
}

// --------------------------------------------------------------------------- the checks

#[test]
fn given_a_new_chip_when_checked_then_it_is_well_formed() {
    assert!(Chip::new().well_formed());
}

/// The corrupt chips a savestate could hold, one field each.
fn corrupt_chips() -> Vec<Chip> {
    let broken = |edit: fn(&mut Chip)| {
        let mut chip = Chip::new();
        edit(&mut chip);
        chip
    };
    vec![
        broken(|c| c.offset = SPAN),
        broken(|c| c.weekday_base = 7),
        broken(|c| c.status1 = 0x10),
        broken(|c| c.int1[0] = 0x08),
        broken(|c| c.int2[0] = 0x40),
        broken(|c| c.session = Session::Command { byte: 0, bits: 8 }),
        broken(|c| {
            c.session = Session::Read {
                register: Register::Free,
                data: [0; 7],
                len: 8,
                position: 0,
                out: false,
            }
        }),
        broken(|c| {
            c.session = Session::Write {
                register: Register::Free,
                data: [0; 7],
                len: 1,
                position: 9,
            }
        }),
    ]
}

#[test]
fn given_chips_with_a_field_out_of_range_when_checked_then_none_is_well_formed() {
    let accepted: Vec<usize> = corrupt_chips()
        .iter()
        .enumerate()
        .filter(|(_, chip)| chip.well_formed())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(accepted, Vec::<usize>::new());
}

#[test]
fn given_every_session_a_transfer_passes_through_when_checked_then_each_is_well_formed() {
    let mut chip = Chip::new();
    chip.select();
    let command_phase = chip.well_formed();
    send_byte(&mut chip, command(DATE_TIME, true), 0);
    let reading = chip.well_formed();
    receive_bytes(&mut chip, 8, 0);
    let past_the_end = chip.well_formed();
    chip.deselect();
    chip.select();
    send_byte(&mut chip, command(FREE, false), 0);
    let writing = chip.well_formed();
    assert_eq!(
        (command_phase, reading, past_the_end, writing),
        (true, true, true, true)
    );
}

#[test]
fn given_a_chip_with_a_session_of_a_hand_made_length_when_clocked_then_it_does_not_panic() {
    let mut chip = Chip::new();
    chip.session = Session::Read {
        register: Register::Free,
        data: [0; 7],
        len: 255,
        position: 250,
        out: false,
    };
    send_ones(&mut chip, 16, 0);
    chip.session = Session::Write {
        register: Register::Free,
        data: [0; 7],
        len: 200,
        position: 250,
    };
    send_ones(&mut chip, 16, 0);
    assert_eq!(chip.free(), 0);
}
