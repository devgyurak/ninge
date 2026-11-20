// SPDX-License-Identifier: Apache-2.0
//! The clock chip's calendar as pure functions: seconds, calendar fields, BCD, the hour register
//! and the day-of-week counter (`docs/hw/rtc.md`, "The date and time registers" and "How the
//! model gets its time").
//!
//! A time is whole seconds since 2000-01-01 00:00:00, below [`SPAN`]: the chip's year register has
//! two digits, so the hundred years 2000-2099 are the whole range and a time past the end wraps to
//! the start. Everything here is integer arithmetic and total: a value that is not a calendar
//! value comes back as `None`, never as a panic.

// The chip that uses these arrives in the next PR of #40; until then only the tests do.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the chip of the next PR of #40 uses the calendar")
)]

/// Seconds in a day.
pub(crate) const SECONDS_PER_DAY: u32 = 86_400;

/// Days in four years that start with a leap year (2000, 2004, ...): 366 + 365 + 365 + 365.
const DAYS_PER_FOUR_YEARS: u32 = 1461;
/// Days in a leap year, the first of each four.
const DAYS_IN_LEAP_YEAR: u32 = 366;
/// Days in a common year.
const DAYS_IN_YEAR: u32 = 365;

/// Days in 2000-2099: 25 leap years (2000, 2004, ..., 2096) and 75 common ones. The year 2100, which
/// is not a leap year, is outside the range, so every fourth year is leap (`rtc.md`).
const DAYS_IN_RANGE: u32 = 36_525;

/// Seconds in the whole range, 2000-01-01 00:00:00 up to and not including 2100-01-01 00:00:00.
/// A time at or past it wraps to the start (`rtc.md`, "Unverified").
// TODO(hw-unverified): the wrap at the end of the range (`rtc.md`, "Unverified").
pub(crate) const SPAN: u32 = DAYS_IN_RANGE * SECONDS_PER_DAY;

/// The weekday of 2000-01-01 (a Saturday) when Monday is 0: the day-of-week counter's value for
/// the first day of the range.
// TODO(hw-unverified): GBATEK leaves the counter's assignment to the program; the model's default
// is Monday = 0 (`rtc.md`, "Unverified").
pub(crate) const EPOCH_WEEKDAY: u8 = 5;

/// Days in a week: the day-of-week register is a septenary counter.
const WEEKDAYS: u32 = 7;

/// Days before the first of each month in a common year.
const DAYS_BEFORE_MONTH: [u16; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

/// Hour register bit 6: AM/PM (0 = AM, 1 = PM).
pub(crate) const HOUR_PM: u8 = 1 << 6;
/// Hour register bits 0-5: the BCD hour.
const HOUR_DIGITS: u8 = 0x3F;

/// A calendar date and time as the chip's registers hold it, in plain numbers (not BCD). `year` is
/// the two-digit year, 0 for 2000.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DateTime {
    /// 0-99, for 2000-2099.
    pub(crate) year: u8,
    /// 1-12.
    pub(crate) month: u8,
    /// 1 up to the month's length.
    pub(crate) day: u8,
    /// 0-23.
    pub(crate) hour: u8,
    /// 0-59.
    pub(crate) minute: u8,
    /// 0-59.
    pub(crate) second: u8,
}

/// Whether the two-digit `year` is a leap year. Inside 2000-2099 every fourth year is: 2000 is a
/// leap year, and 2100, the century that is not, is outside the range.
pub(crate) const fn is_leap_year(year: u8) -> bool {
    year.is_multiple_of(4)
}

/// The days in `month` (1-12) of the two-digit `year`; 0 for a month that does not exist, so a day
/// is never within it.
pub(crate) const fn days_in_month(year: u8, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The calendar fields of `seconds` since the start of the range; a value at or past [`SPAN`] wraps.
pub(crate) fn from_seconds(seconds: u32) -> DateTime {
    let seconds = seconds % SPAN;
    let days = seconds / SECONDS_PER_DAY;
    let rest = seconds % SECONDS_PER_DAY;

    // Each four years open with a leap year, so the year inside the cycle is 0 for the leap year's
    // 366 days and then one per 365.
    let cycle = days / DAYS_PER_FOUR_YEARS;
    let in_cycle = days % DAYS_PER_FOUR_YEARS;
    let (year_in_cycle, mut day) = match in_cycle.checked_sub(DAYS_IN_LEAP_YEAR) {
        None => (0, in_cycle),
        Some(after) => (1 + after / DAYS_IN_YEAR, after % DAYS_IN_YEAR),
    };
    // The range holds 25 cycles, so the year is at most 99: the narrowing cast cannot truncate.
    let year = (cycle * 4 + year_in_cycle) as u8;

    let mut month = 1;
    while month < 12 && day >= u32::from(days_in_month(year, month)) {
        day -= u32::from(days_in_month(year, month));
        month += 1;
    }

    DateTime {
        year,
        month,
        // `day` is below a month's length and `rest` below a day, so each fits its field.
        day: day as u8 + 1,
        hour: (rest / 3600) as u8,
        minute: (rest % 3600 / 60) as u8,
        second: (rest % 60) as u8,
    }
}

/// Seconds since the start of the range of `time`, or `None` when a field is not a calendar value
/// (`rtc.md`, "Edge cases").
pub(crate) fn to_seconds(time: &DateTime) -> Option<u32> {
    let valid = time.year <= 99
        && (1..=12).contains(&time.month)
        && (1..=days_in_month(time.year, time.month)).contains(&time.day)
        && time.hour <= 23
        && time.minute <= 59
        && time.second <= 59;
    valid.then(|| {
        let year = u32::from(time.year);
        // The leap days before this year: the years 0, 4, 8, ... below it.
        let leap_days_before = year.div_ceil(4);
        let month_start = DAYS_BEFORE_MONTH
            .get(usize::from(time.month - 1))
            .map_or(0, |&days| u32::from(days));
        let leap_day = u32::from(time.month > 2 && is_leap_year(time.year));
        let days = year * DAYS_IN_YEAR
            + leap_days_before
            + month_start
            + leap_day
            + u32::from(time.day - 1);
        days * SECONDS_PER_DAY
            + u32::from(time.hour) * 3600
            + u32::from(time.minute) * 60
            + u32::from(time.second)
    })
}

/// The BCD byte of `value`, 0-99. A larger value is not a BCD byte; its tens digit is cut to four
/// bits.
pub(crate) const fn to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

/// The value of a BCD byte, or `None` when a digit is above 9.
pub(crate) const fn from_bcd(byte: u8) -> Option<u8> {
    let (tens, ones) = (byte >> 4, byte & 0x0F);
    if tens > 9 || ones > 9 {
        None
    } else {
        Some(tens * 10 + ones)
    }
}

/// The hour register for `hour` (0-23): in 24-hour mode the BCD hour, in 12-hour mode the BCD hour
/// on a 12-hour clock where 12 o'clock is `00h`; bit 6 is 1 from noon on in both modes.
pub(crate) fn hour_register(hour: u8, mode24: bool) -> u8 {
    let shown = if mode24 { hour } else { hour % 12 };
    to_bcd(shown) | if hour >= 12 { HOUR_PM } else { 0 }
}

/// The hour (0-23) a written hour register means, or `None` when it is not an hour in the mode:
/// in 24-hour mode bit 6 is read-only and ignored; in 12-hour mode the BCD hour is 0-11 and bit 6
/// says PM.
pub(crate) fn hour_from_register(byte: u8, mode24: bool) -> Option<u8> {
    let shown = from_bcd(byte & HOUR_DIGITS)?;
    if mode24 {
        (shown <= 23).then_some(shown)
    } else {
        (shown <= 11).then_some(shown + if byte & HOUR_PM != 0 { 12 } else { 0 })
    }
}

/// The day-of-week counter's value at `seconds` when the counter is `base` on the first day of the
/// range: it steps once a day and wraps at 7. `base` is 0-6.
pub(crate) fn weekday(seconds: u32, base: u8) -> u8 {
    // The remainder of a division by 7 is below 7, so it fits a `u8`.
    ((seconds % SPAN / SECONDS_PER_DAY + u32::from(base)) % WEEKDAYS) as u8
}

/// The `base` that makes the counter read `weekday` (0-6) at `seconds`: the inverse of [`weekday`].
pub(crate) fn weekday_base(weekday: u8, seconds: u32) -> u8 {
    let day = seconds % SPAN / SECONDS_PER_DAY % WEEKDAYS;
    // The remainder of a division by 7 is below 7, so it fits a `u8`.
    ((u32::from(weekday) + WEEKDAYS - day) % WEEKDAYS) as u8
}

/// The chip's time when the host says it is `host` seconds into the range and the program last set
/// the clock `offset` seconds ahead of the host: a read is the host's time plus the difference,
/// wrapped to the range.
pub(crate) fn clock_seconds(host: u32, offset: u32) -> u32 {
    // The remainder of a division by `SPAN` is below `SPAN`, which fits a `u32`.
    ((u64::from(host) + u64::from(offset)) % u64::from(SPAN)) as u32
}

/// The difference to keep when the program sets the clock to `written` (below [`SPAN`]) while the
/// host's time is `host`: the inverse of [`clock_seconds`]. A guest write to the clock is only this.
pub(crate) fn offset_for(written: u32, host: u32) -> u32 {
    // The remainder of a division by `SPAN` is below `SPAN`, which fits a `u32`.
    ((u64::from(written % SPAN) + u64::from(SPAN) - u64::from(host % SPAN)) % u64::from(SPAN))
        as u32
}

#[cfg(test)]
mod tests;
