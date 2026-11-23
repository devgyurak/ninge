// SPDX-License-Identifier: Apache-2.0
//! The host's time as the machine's clock chip takes it (`docs/hw/rtc.md`, "How the model gets
//! its time"). Expected seconds are counted by hand from the calendar (days since 2000-01-01 times
//! 86400), never taken from the code.

use super::*;
use chrono::NaiveDate;

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, second))
        .expect("the test dates are real")
}

/// The rows whose date and time do not convert to the seconds beside them.
fn conversion_failures(rows: &[(NaiveDateTime, u32)]) -> Vec<(NaiveDateTime, u32)> {
    rows.iter()
        .filter(|&&(local, seconds)| wall_time(local) != seconds)
        .copied()
        .collect()
}

#[test]
fn given_dates_in_the_range_when_converted_then_the_seconds_count_from_2000_01_01() {
    let rows = [
        (at(2000, 1, 1, 0, 0, 0), 0),
        (at(2000, 1, 1, 0, 0, 1), 1),
        // 2024-02-29 is day 8825 (24 years with 6 leap days, then 31 + 28 days), at noon.
        (at(2024, 2, 29, 12, 0, 0), 762_523_200),
        (at(2099, 12, 31, 23, 59, 59), 3_155_759_999),
        (at(2100, 1, 1, 0, 0, 0), 3_155_760_000),
    ];
    assert_eq!(
        conversion_failures(&rows),
        Vec::<(NaiveDateTime, u32)>::new()
    );
}

#[test]
fn given_a_moment_before_2000_when_converted_then_it_is_the_first_second() {
    assert_eq!(wall_time(at(1999, 12, 31, 23, 59, 59)), 0);
}

#[test]
fn given_a_moment_past_what_32_bits_count_when_converted_then_it_is_the_largest_value() {
    assert_eq!(wall_time(at(2150, 1, 1, 0, 0, 0)), u32::MAX);
}

#[test]
fn given_the_host_clock_when_read_then_it_is_past_2025() {
    // The one smoke test of the adapter: any host clock set since 2025 passes, and the conversion
    // itself is pinned by the rows above.
    // 2025-01-01 is day 9132 (25 years with 7 leap days).
    assert!(local_wall_time() > 9_132 * 86_400);
}
