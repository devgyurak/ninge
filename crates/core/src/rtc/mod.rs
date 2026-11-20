// SPDX-License-Identifier: Apache-2.0
//! The clock chip: its registers and the serial transfer that reaches them (`docs/hw/rtc.md`).
//!
//! The chip sees three pins, driven by the program through RTCCNT (`io::rtc`): chip select, the
//! serial clock and the data line. [`Chip`] takes the edges those pins make and answers on the data
//! line; it knows nothing of RTCCNT. The calendar arithmetic is [`calendar`].
//!
//! The chip holds no time of its own. A read is the host's time plus the difference the program last
//! wrote (`rtc.md`, "How the model gets its time"), so every call that can read or write the clock
//! is given the host's time.

pub(crate) mod calendar;

use calendar::{
    DateTime, EPOCH_WEEKDAY, SPAN, clock_seconds, from_bcd, from_seconds, hour_from_register,
    hour_register, offset_for, to_bcd, to_seconds, weekday, weekday_base,
};
use serde::{Deserialize, Serialize};

/// The command byte's fixed code, bits 0-3, in the order the bits arrive (`rtc.md`, "The command
/// byte"): 0, 1, 1, 0 from bit 0 up.
const FIXED_CODE: u8 = 0b0110;
/// The mask of the fixed code. Bit 3 set is the DSi's extended command, which this console does
/// not have.
const FIXED_CODE_MASK: u8 = 0b1111;
/// Command bits 4-6: which register.
const COMMAND_SHIFT: u32 = 4;
const COMMAND_MASK: u8 = 0b111;
/// Command bit 7: 1 = the program reads the register.
const READ: u8 = 1 << 7;

/// Status register 1 bit 0: writing 1 resets the chip.
const STATUS1_RESET: u8 = 1 << 0;
/// Status register 1 bit 1: 24-hour mode.
const STATUS1_MODE24: u8 = 1 << 1;
/// The bits of status register 1 the chip keeps: the mode and the two general-purpose bits. Bits 4-7
/// are flags the chip raises (INT1, INT2, power low, power off), and the model never raises them.
// TODO(phase5): the INT1 and INT2 flags, with the interrupt itself (`rtc.md`, "The interrupt").
const STATUS1_KEPT: u8 = 0b0000_1110;
/// Status register 2 bit 2: INT1 is alarm 1 (three parameter bytes) rather than the selected
/// frequency (one).
const STATUS2_ALARM1: u8 = 1 << 2;
/// Alarm day-of-week byte: the day in bits 0-2 and the compare enable in bit 7; bits 3-6 are zero.
const ALARM_DAY_MASK: u8 = 0b1000_0111;

/// The date-and-time register is seven bytes, the longest parameter.
const MAX_PARAMETER: usize = 7;
/// Bits in a byte.
const BYTE_BITS: u8 = 8;
/// Date-and-time month byte, bits 0-4; bits 5-7 read zero.
const MONTH_MASK: u8 = 0x1F;
/// Date-and-time day byte, bits 0-5; bits 6-7 read zero.
const DAY_MASK: u8 = 0x3F;
/// Minute and second bytes, bits 0-6; bit 7 reads zero.
const MINUTE_SECOND_MASK: u8 = 0x7F;
/// The day-of-week counter has seven values.
const WEEKDAYS: u8 = 7;

/// The registers a command can reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Register {
    /// Status register 1.
    Status1,
    /// Status register 2.
    Status2,
    /// Year, month, day, day of week, hour, minute, second.
    DateTime,
    /// Hour, minute, second.
    Time,
    /// INT1: the selected-frequency byte, or alarm 1's three bytes.
    Int1,
    /// INT2: alarm 2's three bytes.
    Int2,
    /// The clock adjustment byte.
    ClockAdjust,
    /// The free register.
    Free,
}

/// The bits of a parameter of `len` bytes.
const fn total_bits(len: u8) -> u16 {
    len as u16 * BYTE_BITS as u16
}

impl Register {
    /// The register a command (the command byte's bits 4-6) names (`rtc.md`, "The command byte").
    const fn of_command(command: u8) -> Self {
        match command & COMMAND_MASK {
            0 => Self::Status1,
            4 => Self::Status2,
            2 => Self::DateTime,
            6 => Self::Time,
            1 => Self::Int1,
            5 => Self::Int2,
            3 => Self::ClockAdjust,
            _ => Self::Free,
        }
    }
}

/// Where the chip is in a transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Session {
    /// Not selected: the clock does nothing.
    Idle,
    /// Selected, taking the command byte: `bits` of it have arrived, LSB first, in `byte`.
    Command {
        /// The command bits so far, the first to arrive in bit 0.
        byte: u8,
        /// How many bits have arrived, 0-7.
        bits: u8,
    },
    /// Selected, and the command was not one: ignored until the chip is deselected.
    // TODO(hw-unverified): a wrong fixed code leaves the session ignored (`rtc.md`, "Unverified").
    Ignored,
    /// Sending a register to the program: the bytes as of the end of the command, and how many bits
    /// have gone.
    Read {
        /// The register.
        register: Register,
        /// The register's bytes, taken when the command ended.
        data: [u8; MAX_PARAMETER],
        /// How many bytes the register has.
        len: u8,
        /// Bits sent so far, 0 up to `8 * len`.
        position: u8,
        /// The bit on the data line: set at the falling clock edge, held until the next.
        out: bool,
    },
    /// Taking a register from the program: the bits that have arrived, and how many.
    Write {
        /// The register.
        register: Register,
        /// The bytes so far.
        data: [u8; MAX_PARAMETER],
        /// How many bytes the register has.
        len: u8,
        /// Bits taken so far, 0 up to `8 * len`.
        position: u8,
    },
}

/// The clock chip. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chip {
    /// Seconds to add to the host's time, below [`SPAN`]: what the program last set the clock by.
    offset: u32,
    /// The day-of-week counter's value on the first day of the range.
    weekday_base: u8,
    /// Status register 1, the bits that are kept ([`STATUS1_KEPT`]).
    status1: u8,
    /// Status register 2.
    status2: u8,
    /// INT1 as day of week, hour, minute. The selected-frequency byte is the minute byte.
    int1: [u8; 3],
    /// INT2 as day of week, hour, minute.
    int2: [u8; 3],
    /// The clock adjustment byte.
    adjust: u8,
    /// The free register.
    free: u8,
    /// The transfer in progress.
    session: Session,
}

impl Chip {
    /// The chip at power-on: 24-hour mode, flags clear, every register zero, the clock showing the
    /// host's time.
    // TODO(hw-unverified): the power-on state is the firmware's, which GBATEK does not give
    // (`rtc.md`, "Unverified").
    pub fn new() -> Self {
        Self {
            offset: 0,
            weekday_base: EPOCH_WEEKDAY,
            status1: STATUS1_MODE24,
            status2: 0,
            int1: [0; 3],
            int2: [0; 3],
            adjust: 0,
            free: 0,
            session: Session::Idle,
        }
    }

    /// A chip in `session`, whatever it is, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn with_session_for_test(session: Session) -> Self {
        Self {
            session,
            ..Self::new()
        }
    }

    /// Seconds the clock is ahead of the host's time, below 100 years.
    pub fn offset(&self) -> u32 {
        self.offset
    }

    /// The day-of-week counter's value on 2000-01-01.
    pub fn weekday_base(&self) -> u8 {
        self.weekday_base
    }

    /// Status register 1 as stored: the mode and the general-purpose bits.
    pub fn status1(&self) -> u8 {
        self.status1
    }

    /// Status register 2.
    pub fn status2(&self) -> u8 {
        self.status2
    }

    /// INT1 as day of week, hour, minute (the selected-frequency byte is the last).
    pub fn int1(&self) -> [u8; 3] {
        self.int1
    }

    /// INT2 as day of week, hour, minute.
    pub fn int2(&self) -> [u8; 3] {
        self.int2
    }

    /// The clock adjustment byte.
    pub fn clock_adjust(&self) -> u8 {
        self.adjust
    }

    /// The free register.
    pub fn free(&self) -> u8 {
        self.free
    }

    /// The transfer in progress.
    pub fn session(&self) -> Session {
        self.session
    }

    /// Whether the chip is selected.
    pub fn selected(&self) -> bool {
        self.session != Session::Idle
    }

    /// The clock as seconds since 2000-01-01 00:00:00, when the host's time is `host`.
    pub fn seconds(&self, host: u32) -> u32 {
        clock_seconds(host, self.offset)
    }

    /// Chip select went high: a new transfer starts with a command byte.
    pub(crate) fn select(&mut self) {
        self.session = Session::Command { byte: 0, bits: 0 };
    }

    /// Chip select went low: the transfer ends. A write that has not taken its last bit changes
    /// nothing.
    // TODO(hw-unverified): a write cut short is discarded whole (`rtc.md`, "Unverified").
    pub(crate) fn deselect(&mut self) {
        self.session = Session::Idle;
    }

    /// The serial clock fell: a chip that is sending puts its next bit on the data line.
    pub(crate) fn clock_falling(&mut self) {
        if let Session::Read {
            data,
            len,
            position,
            out,
            ..
        } = &mut self.session
        {
            // Past the last byte the line is zero.
            // TODO(hw-unverified): bits beyond the register read zero (`rtc.md`, "Unverified").
            let byte = if u16::from(*position) < total_bits(*len) {
                data.get(usize::from(*position / BYTE_BITS)).copied()
            } else {
                None
            };
            *out = byte.is_some_and(|b| b >> (*position % BYTE_BITS) & 1 != 0);
        }
    }

    /// The serial clock rose: a chip that is listening takes the bit on the data line, `sio`; a
    /// chip that is sending moves on to its next bit. `host` is the host's time.
    pub(crate) fn clock_rising(&mut self, sio: bool, host: u32) {
        match &mut self.session {
            Session::Idle | Session::Ignored => {}
            Session::Command { byte, bits } => {
                // The first bit to arrive is bit 0 (`rtc.md`, "Bit order").
                *byte |= u8::from(sio).wrapping_shl(u32::from(*bits));
                *bits = bits.saturating_add(1);
                if *bits == BYTE_BITS {
                    let command = *byte;
                    self.start_parameter(command, host);
                }
            }
            Session::Read { position, len, .. } => {
                // Clocks past the last bit move nothing: the position stops at the end.
                if u16::from(*position) < total_bits(*len) {
                    *position = position.saturating_add(1);
                }
            }
            Session::Write {
                register,
                data,
                len,
                position,
            } => {
                if let Some(byte) = data.get_mut(usize::from(*position / BYTE_BITS)) {
                    *byte |= u8::from(sio) << (*position % BYTE_BITS);
                }
                *position = position.saturating_add(1);
                if u16::from(*position) == total_bits(*len) {
                    let (register, data) = (*register, *data);
                    // The parameter is complete: take it and ignore any more clocks.
                    // TODO(hw-unverified): extra clocks after the last byte are ignored (`rtc.md`,
                    // "Unverified").
                    self.session = Session::Ignored;
                    self.write_register(register, data, host);
                }
            }
        }
    }

    /// The level the chip puts on the data line: the bit it is sending, and zero when it sends none.
    // TODO(hw-unverified): an undriven data line reads zero (`rtc.md`, "Unverified").
    pub(crate) fn data_out(&self) -> bool {
        matches!(self.session, Session::Read { out: true, .. })
    }

    /// The command byte is complete: a read takes a snapshot of the register, a write waits for the
    /// parameter.
    fn start_parameter(&mut self, command: u8, host: u32) {
        if command & FIXED_CODE_MASK != FIXED_CODE {
            self.session = Session::Ignored;
            return;
        }
        let register = Register::of_command((command >> COMMAND_SHIFT) & COMMAND_MASK);
        let len = self.parameter_len(register);
        self.session = if command & READ != 0 {
            Session::Read {
                register,
                data: self.read_register(register, host),
                len,
                position: 0,
                out: false,
            }
        } else {
            Session::Write {
                register,
                data: [0; MAX_PARAMETER],
                len,
                position: 0,
            }
        };
    }

    /// How many bytes `register`'s parameter has now: INT1 is one byte (the selected frequency) or
    /// three (alarm 1) as status register 2 says.
    fn parameter_len(&self, register: Register) -> u8 {
        match register {
            Register::DateTime => 7,
            Register::Time | Register::Int2 => 3,
            Register::Int1 if self.status2 & STATUS2_ALARM1 != 0 => 3,
            Register::Status1
            | Register::Status2
            | Register::Int1
            | Register::ClockAdjust
            | Register::Free => 1,
        }
    }

    /// Whether the hour register is in 24-hour mode.
    fn mode24(&self) -> bool {
        self.status1 & STATUS1_MODE24 != 0
    }

    /// The bytes of `register` as a read sees them, with the host's time `host`.
    fn read_register(&self, register: Register, host: u32) -> [u8; MAX_PARAMETER] {
        let seconds = self.seconds(host);
        let time = from_seconds(seconds);
        let clock = [
            to_bcd(time.year),
            to_bcd(time.month),
            to_bcd(time.day),
            weekday(seconds, self.weekday_base),
            hour_register(time.hour, self.mode24()),
            to_bcd(time.minute),
            to_bcd(time.second),
        ];
        let [_, _, _, _, hour, minute, second] = clock;
        let mut data = [0; MAX_PARAMETER];
        let (head, _) = data.split_at_mut(3);
        match register {
            Register::DateTime => data = clock,
            Register::Time => head.copy_from_slice(&[hour, minute, second]),
            Register::Status1 => data[0] = self.status1,
            Register::Status2 => data[0] = self.status2,
            // The one-byte form is the minute byte of alarm 1.
            Register::Int1 if self.status2 & STATUS2_ALARM1 == 0 => data[0] = self.int1[2],
            Register::Int1 => head.copy_from_slice(&self.int1),
            Register::Int2 => head.copy_from_slice(&self.int2),
            Register::ClockAdjust => data[0] = self.adjust,
            Register::Free => data[0] = self.free,
        }
        data
    }

    /// The parameter of a write command is complete: store it.
    // TODO(hw-unverified): the status and alarm registers keep every bit the masks above allow, and
    // the test-mode bit has no effect (`rtc.md`, "Unverified").
    fn write_register(&mut self, register: Register, data: [u8; MAX_PARAMETER], host: u32) {
        match register {
            Register::Status1 => {
                self.status1 = data[0] & STATUS1_KEPT;
                if data[0] & STATUS1_RESET != 0 {
                    self.reset(host);
                }
            }
            Register::Status2 => self.status2 = data[0],
            Register::DateTime => self.set_date_time(&data, host),
            Register::Time => self.set_time(&data, host),
            Register::Int1 if self.status2 & STATUS2_ALARM1 == 0 => self.int1[2] = data[0],
            Register::Int1 => self.int1 = [data[0] & ALARM_DAY_MASK, data[1], data[2]],
            Register::Int2 => self.int2 = [data[0] & ALARM_DAY_MASK, data[1], data[2]],
            Register::ClockAdjust => self.adjust = data[0],
            Register::Free => self.free = data[0],
        }
    }

    /// A reset: every register back to zero and the clock to 2000-01-01 00:00:00, set at the host's
    /// time `host`. Status register 1 reads zero, which is 12-hour mode.
    // TODO(hw-unverified): what a reset leaves (`rtc.md`, "Unverified").
    fn reset(&mut self, host: u32) {
        *self = Self {
            offset: offset_for(0, host),
            weekday_base: EPOCH_WEEKDAY,
            status1: 0,
            status2: 0,
            int1: [0; 3],
            int2: [0; 3],
            adjust: 0,
            free: 0,
            session: self.session,
        };
    }

    /// A write of the date-and-time register: year, month, day, day of week, hour, minute, second.
    /// Not a calendar value in any field: dropped whole.
    // TODO(hw-unverified): what the chip does with a value that is not a calendar value (`rtc.md`,
    // "Unverified"). TODO(hw-unverified): the unused high bits of each written byte (month bits 5-7,
    // day bits 6-7, weekday bits 3-7, minute and second bit 7, hour bit 7) are ignored, not rejected
    // (`rtc.md`, "Unverified").
    fn set_date_time(&mut self, data: &[u8; MAX_PARAMETER], host: u32) {
        let parsed = (|| {
            let time = DateTime {
                year: from_bcd(data[0])?,
                month: from_bcd(data[1] & MONTH_MASK)?,
                day: from_bcd(data[2] & DAY_MASK)?,
                hour: hour_from_register(data[4], self.mode24())?,
                minute: from_bcd(data[5] & MINUTE_SECOND_MASK)?,
                second: from_bcd(data[6] & MINUTE_SECOND_MASK)?,
            };
            // The day-of-week counter is 0-6.
            let day_of_week = Some(data[3] & 0b111).filter(|&d| d < WEEKDAYS)?;
            Some((to_seconds(&time)?, day_of_week))
        })();
        if let Some((seconds, day_of_week)) = parsed {
            self.offset = offset_for(seconds, host);
            self.weekday_base = weekday_base(day_of_week, seconds);
        }
    }

    /// A write of the time register: hour, minute, second, with the date unchanged.
    // TODO(hw-unverified): the unused high bits of each byte are ignored, not rejected (`rtc.md`,
    // "Unverified").
    fn set_time(&mut self, data: &[u8; MAX_PARAMETER], host: u32) {
        let mut time = from_seconds(self.seconds(host));
        let parsed = (|| {
            time.hour = hour_from_register(data[0], self.mode24())?;
            time.minute = from_bcd(data[1] & MINUTE_SECOND_MASK)?;
            time.second = from_bcd(data[2] & MINUTE_SECOND_MASK)?;
            to_seconds(&time)
        })();
        // The date is the same, so the day-of-week counter is too.
        if let Some(seconds) = parsed {
            self.offset = offset_for(seconds, host);
        }
    }

    /// Whether the chip holds only values it can reach: for the check a savestate load runs.
    pub(crate) fn well_formed(&self) -> bool {
        // A session's length is a parameter's: 1 to 7 bytes.
        let length_ok = |len: u8| (1..=MAX_PARAMETER).contains(&usize::from(len));
        self.offset < SPAN
            && self.weekday_base < WEEKDAYS
            && self.status1 & !STATUS1_KEPT == 0
            && self.int1[0] & !ALARM_DAY_MASK == 0
            && self.int2[0] & !ALARM_DAY_MASK == 0
            && match self.session {
                Session::Idle | Session::Ignored => true,
                // The bits that have not arrived are zero.
                Session::Command { byte, bits } => {
                    bits < BYTE_BITS && byte.checked_shr(u32::from(bits)).unwrap_or(0) == 0
                }
                // A read that has sent every bit stays at the end.
                Session::Read { len, position, .. } => {
                    length_ok(len) && u16::from(position) <= total_bits(len)
                }
                // The last bit completes a write, so a stored write is short of it.
                Session::Write { len, position, .. } => {
                    length_ok(len) && u16::from(position) < total_bits(len)
                }
            }
    }
}

impl Default for Chip {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
