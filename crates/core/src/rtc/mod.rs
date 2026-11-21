// SPDX-License-Identifier: Apache-2.0
//! The real-time clock chip (`docs/hw/rtc.md`): the calendar arithmetic its registers rely on. The
//! serial transfer and the chip's registers come next (#40); the register window that reaches the
//! chip is `io::rtc`.

pub(crate) mod calendar;
