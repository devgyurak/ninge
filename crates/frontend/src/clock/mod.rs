// SPDX-License-Identifier: Apache-2.0
//! The host's wall-clock time for the machine's clock chip (`docs/hw/rtc.md`, "How the model gets
//! its time"). ninge-core never reads a clock; the frontend reads the host's local time once per
//! frame and hands it over as [`Input::wall_time`](ninge_core::Input::wall_time): whole seconds
//! since 2000-01-01 00:00:00 in local time.
//!
//! The conversion ([`wall_time`]) is a pure function of a date and time, so it is tested without a
//! clock; [`local_wall_time`] is the thin adapter that reads one.

use chrono::{Local, NaiveDateTime};

/// Seconds from 1970-01-01 to 2000-01-01, both at 00:00:00: 30 years with 7 leap days.
const UNIX_TO_2000: i64 = (30 * 365 + 7) * 86_400;

/// The seconds from 2000-01-01 00:00:00 to `local`, a local date and time. A moment before 2000
/// is 0, and one that does not fit 32 bits (past the year 2136) is the largest value; the machine
/// wraps anything beyond 2099 itself.
// A moment that the local time zone repeats (the hour a clock is turned back) maps to the same
// seconds twice, so the machine's clock steps back by that hour with the host's, as it would for
// a clock that was set.
pub fn wall_time(local: NaiveDateTime) -> u32 {
    let seconds = local.and_utc().timestamp() - UNIX_TO_2000;
    u32::try_from(seconds.max(0)).unwrap_or(u32::MAX)
}

/// The host's local time now, as [`wall_time`] gives it.
pub fn local_wall_time() -> u32 {
    wall_time(Local::now().naive_local())
}

#[cfg(test)]
mod tests;
