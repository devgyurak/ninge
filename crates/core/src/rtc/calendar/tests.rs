// SPDX-License-Identifier: Apache-2.0
//! The calendar, from `docs/hw/rtc.md`: seconds <-> fields, BCD, the hour register, the weekday
//! counter and the guest-set offset. Expected values are worked out by hand from the calendar
//! (days since 2000-01-01 times 86400), never taken from the code.

use super::*;
use proptest::prelude::*;

const fn dt(year: u8, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> DateTime {
    DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// The rows whose seconds do not split into the fields, or whose fields do not join into the
/// seconds.
fn conversion_failures(rows: &[(DateTime, u32)]) -> Vec<(DateTime, u32)> {
    rows.iter()
        .filter(|&&(time, seconds)| {
            from_seconds(seconds) != time || to_seconds(&time) != Some(seconds)
        })
        .copied()
        .collect()
}

/// The rows `to_seconds` accepts although a field is not a calendar value.
fn wrongly_accepted(rows: &[DateTime]) -> Vec<DateTime> {
    rows.iter()
        .filter(|time| to_seconds(time).is_some())
        .copied()
        .collect()
}

// ------------------------------------------------------------------------- seconds and fields

#[test]
fn given_month_ends_and_leap_days_when_converted_then_seconds_and_fields_agree() {
    let rows = [
        (dt(0, 1, 1, 0, 0, 0), 0),
        (dt(0, 1, 1, 0, 0, 1), 1),
        (dt(0, 1, 1, 23, 59, 59), 86_399),
        (dt(0, 1, 31, 23, 59, 59), 2_678_399),
        (dt(0, 2, 1, 0, 0, 0), 2_678_400),
        // 2000 is a leap year: February has 29 days.
        (dt(0, 2, 29, 0, 0, 0), 5_097_600),
        (dt(0, 2, 29, 23, 59, 59), 5_183_999),
        (dt(0, 3, 1, 0, 0, 0), 5_184_000),
        (dt(0, 12, 31, 23, 59, 59), 31_622_399),
        (dt(1, 1, 1, 0, 0, 0), 31_622_400),
        // 2001 is not: February 28 is followed by March 1.
        (dt(1, 2, 28, 23, 59, 59), 36_719_999),
        (dt(1, 3, 1, 0, 0, 0), 36_720_000),
        (dt(4, 1, 1, 0, 0, 0), 126_230_400),
        (dt(4, 2, 29, 0, 0, 0), 131_328_000),
        (dt(96, 2, 29, 0, 0, 0), 3_034_627_200),
        // 2099 is the last year and not a leap year.
        (dt(99, 3, 1, 0, 0, 0), 3_129_321_600),
        (dt(99, 12, 31, 0, 0, 0), 3_155_673_600),
        (dt(99, 12, 31, 23, 59, 59), 3_155_759_999),
    ];
    assert_eq!(conversion_failures(&rows), Vec::<(DateTime, u32)>::new());
}

#[test]
fn given_the_span_when_the_range_ends_then_it_is_a_hundred_years_of_25_leap_years() {
    // 100 years of 365 days, and a 366th day in each of the 25 leap years 2000, 2004, ..., 2096.
    assert_eq!(SPAN, (100 * 365 + 25) * 86_400);
}

#[test]
fn given_a_time_past_the_last_second_of_2099_when_split_then_it_wraps_to_the_start() {
    let rows = [
        (SPAN, dt(0, 1, 1, 0, 0, 0)),
        (SPAN + 90_061, dt(0, 1, 2, 1, 1, 1)),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(seconds, time)| from_seconds(seconds) != time)
        .collect();
    assert_eq!(failures, Vec::<&(u32, DateTime)>::new());
}

#[test]
fn given_fields_that_are_not_a_calendar_value_when_joined_then_none_is_returned() {
    let rows = [
        dt(100, 1, 1, 0, 0, 0),
        dt(0, 0, 1, 0, 0, 0),
        dt(0, 13, 1, 0, 0, 0),
        dt(0, 1, 0, 0, 0, 0),
        dt(0, 1, 32, 0, 0, 0),
        dt(0, 4, 31, 0, 0, 0),
        dt(0, 2, 30, 0, 0, 0),
        dt(1, 2, 29, 0, 0, 0),
        dt(99, 2, 29, 0, 0, 0),
        dt(0, 1, 1, 24, 0, 0),
        dt(0, 1, 1, 0, 60, 0),
        dt(0, 1, 1, 0, 0, 60),
        dt(255, 255, 255, 255, 255, 255),
    ];
    assert_eq!(wrongly_accepted(&rows), Vec::<DateTime>::new());
}

#[test]
fn given_leap_years_in_the_range_when_asked_then_every_fourth_year_is_one() {
    let rows = [
        (0, true),
        (1, false),
        (2, false),
        (3, false),
        (4, true),
        (96, true),
        (97, false),
        (99, false),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(year, leap)| is_leap_year(year) != leap)
        .collect();
    assert_eq!(failures, Vec::<&(u8, bool)>::new());
}

#[test]
fn given_months_of_leap_and_common_years_when_counted_then_the_lengths_are_the_calendar_s() {
    let rows = [
        (0, 2, 29),
        (1, 2, 28),
        (4, 2, 29),
        (99, 2, 28),
        (0, 1, 31),
        (0, 4, 30),
        (0, 6, 30),
        (0, 8, 31),
        (0, 9, 30),
        (0, 11, 30),
        (0, 12, 31),
        (0, 0, 0),
        (0, 13, 0),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(year, month, days)| days_in_month(year, month) != days)
        .collect();
    assert_eq!(failures, Vec::<&(u8, u8, u8)>::new());
}

proptest! {
    #[test]
    fn given_any_second_in_the_range_when_split_and_joined_then_it_is_unchanged(
        seconds in 0..SPAN,
    ) {
        prop_assert_eq!(to_seconds(&from_seconds(seconds)), Some(seconds));
    }

    #[test]
    fn given_any_valid_fields_when_joined_and_split_then_they_are_unchanged(
        year in 0u8..=99,
        month in 1u8..=12,
        day in 1u8..=28,
        hour in 0u8..=23,
        minute in 0u8..=59,
        second in 0u8..=59,
    ) {
        let time = dt(year, month, day, hour, minute, second);
        prop_assert_eq!(to_seconds(&time).map(from_seconds), Some(time));
    }
}

// ------------------------------------------------------------------------------------- BCD

#[test]
fn given_values_when_turned_into_bcd_then_the_digits_are_the_decimal_ones() {
    let rows = [(0, 0x00), (9, 0x09), (10, 0x10), (59, 0x59), (99, 0x99)];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(value, bcd)| to_bcd(value) != bcd)
        .collect();
    assert_eq!(failures, Vec::<&(u8, u8)>::new());
}

#[test]
fn given_bytes_with_a_digit_above_nine_when_read_as_bcd_then_none_is_returned() {
    let rows = [0x0A, 0x0F, 0xA0, 0x1A, 0xFF];
    let accepted: Vec<_> = rows.iter().filter(|&&b| from_bcd(b).is_some()).collect();
    assert_eq!(accepted, Vec::<&u8>::new());
}

proptest! {
    #[test]
    fn given_any_value_to_99_when_turned_into_bcd_and_back_then_it_is_unchanged(value in 0u8..=99) {
        prop_assert_eq!(from_bcd(to_bcd(value)), Some(value));
    }
}

// --------------------------------------------------------------------------- the hour register

#[test]
fn given_hours_when_shown_in_each_mode_then_the_register_is_bcd_with_the_pm_flag() {
    let rows = [
        // (hour, 24-hour mode, register)
        (0, true, 0x00),
        (11, true, 0x11),
        (12, true, 0x52),
        (23, true, 0x63),
        // 12-hour mode: 12 o'clock is 00h, and the flag still says PM from noon on.
        (0, false, 0x00),
        (1, false, 0x01),
        (11, false, 0x11),
        (12, false, 0x40),
        (13, false, 0x41),
        (23, false, 0x51),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(hour, mode24, register)| hour_register(hour, mode24) != register)
        .collect();
    assert_eq!(failures, Vec::<&(u8, bool, u8)>::new());
}

#[test]
fn given_hour_registers_when_written_in_each_mode_then_the_hour_is_decoded_or_refused() {
    let rows = [
        // (register, 24-hour mode, hour)
        (0x00, true, Some(0)),
        (0x23, true, Some(23)),
        (0x24, true, None),
        (0x1A, true, None),
        // Bit 6 is read-only in 24-hour mode, so it does not change the hour.
        (0x63, true, Some(23)),
        (0x12, true, Some(12)),
        (0x00, false, Some(0)),
        (0x40, false, Some(12)),
        (0x11, false, Some(11)),
        (0x51, false, Some(23)),
        // 12 is not a 12-hour value: 12 o'clock is 00h.
        (0x12, false, None),
        (0x52, false, None),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(register, mode24, hour)| hour_from_register(register, mode24) != hour)
        .collect();
    assert_eq!(failures, Vec::<&(u8, bool, Option<u8>)>::new());
}

proptest! {
    #[test]
    fn given_any_hour_and_mode_when_shown_and_written_back_then_it_is_unchanged(
        hour in 0u8..=23,
        mode24 in any::<bool>(),
    ) {
        prop_assert_eq!(hour_from_register(hour_register(hour, mode24), mode24), Some(hour));
    }
}

// ----------------------------------------------------------------------------- the day of week

#[test]
fn given_the_first_days_of_the_range_when_the_weekday_is_read_then_it_steps_once_a_day() {
    let day = SECONDS_PER_DAY;
    let rows = [
        (0, 5),
        (day - 1, 5),
        (day, 6),
        (2 * day, 0),
        (7 * day, 5),
        (8 * day, 6),
        // 2099-12-31 is day 36524: 36524 mod 7 = 5, and 5 + 5 = 10 -> 3 (a Thursday).
        (SPAN - 1, 3),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(seconds, weekday_now)| weekday(seconds, EPOCH_WEEKDAY) != weekday_now)
        .collect();
    assert_eq!(failures, Vec::<&(u32, u8)>::new());
}

proptest! {
    #[test]
    fn given_any_second_and_weekday_when_the_base_is_set_then_the_counter_reads_it_there(
        seconds in 0..SPAN,
        wanted in 0u8..7,
    ) {
        prop_assert_eq!(weekday(seconds, weekday_base(wanted, seconds)), wanted);
    }
}

// ------------------------------------------------------------------------------ the offset

#[test]
fn given_a_written_time_and_the_host_time_when_read_at_the_same_host_time_then_it_is_the_written_time()
 {
    let rows = [
        (0, 0),
        (1_000, 5),
        (5, 1_000),
        (SPAN - 1, 0),
        (0, SPAN - 1),
        (123, u32::MAX),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(written, host)| clock_seconds(host, offset_for(written, host)) != written)
        .collect();
    assert_eq!(failures, Vec::<&(u32, u32)>::new());
}

#[test]
fn given_a_clock_set_when_the_host_time_advances_then_the_clock_advances_with_it_and_wraps() {
    let offset = offset_for(SPAN - 5, 100);
    assert_eq!(
        (
            clock_seconds(103, offset),
            clock_seconds(105, offset),
            clock_seconds(110, offset)
        ),
        (SPAN - 2, 0, 5)
    );
}

#[test]
fn given_a_clock_never_set_when_read_then_it_is_the_host_time() {
    assert_eq!(clock_seconds(90_061, 0), 90_061);
}

proptest! {
    #[test]
    fn given_any_written_and_host_time_when_the_clock_is_set_then_a_read_returns_the_written_time(
        written in 0..SPAN,
        host in any::<u32>(),
    ) {
        prop_assert_eq!(clock_seconds(host, offset_for(written, host)), written);
    }

    #[test]
    fn given_any_clock_set_when_the_host_moves_on_then_the_clock_moves_by_the_same_seconds(
        written in 0..SPAN,
        host in 0..SPAN,
        elapsed in 0u32..1_000_000,
    ) {
        let offset = offset_for(written, host);
        prop_assert_eq!(
            clock_seconds(host + elapsed, offset),
            (written + elapsed) % SPAN
        );
    }
}
