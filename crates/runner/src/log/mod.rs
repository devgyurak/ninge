// SPDX-License-Identifier: Apache-2.0
//! The host's input for each frame of a run, in memory and as text (plan §7.2, a replay).
//!
//! An [`InputLog`] is the list the golden helpers and [`crate::replay`] feed the machine, one
//! [`Input`] per frame. It reads and writes the version 1 text format of
//! `docs/contributing/input-log.md` through `ninge_core::input_log`, so a log a frontend recorded
//! replays here, and a log built here can be written for a replay later.

use ninge_core::Input;
use ninge_core::input_log::{self, InputLogError, LogHeader};

/// The host's input for each frame of a run: entry `n` is set before the run's frame `n + 1`
/// (counting from 1) and a frame past the end gets [`Input::default`] (nothing held, hinge open).
/// One [`Input`] per frame is a complete input log: the same ROM and the same log give the same
/// frames and savestate bytes (`docs/hw/keypad.md`, "Where the bits come from").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputLog(Vec<Input>);

/// A log read from text: what it belongs to, its frames, and whether a cut-off last line was
/// dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadLog {
    /// The program and save memory the log was recorded for.
    pub header: LogHeader,
    /// The log's frames.
    pub log: InputLog,
    /// Whether the last line had no newline and was dropped (a log cut off mid-write).
    pub dropped_partial_line: bool,
}

impl InputLog {
    /// A log of `inputs`, one for each frame from the first.
    pub fn new(inputs: Vec<Input>) -> Self {
        Self(inputs)
    }

    /// The input for the frame with 0-based index `frame`.
    pub fn input_for(&self, frame: u64) -> Input {
        usize::try_from(frame)
            .ok()
            .and_then(|index| self.0.get(index))
            .copied()
            .unwrap_or_default()
    }

    /// How many frames the log holds.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the log holds no frame.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Reads a log from its text form.
    ///
    /// # Errors
    /// The first line the format refuses, or a text with no header.
    pub fn from_text(text: &str) -> Result<ReadLog, InputLogError> {
        let parsed = input_log::parse(text)?;
        Ok(ReadLog {
            header: parsed.header,
            log: Self(parsed.frames),
            dropped_partial_line: parsed.dropped_partial_line,
        })
    }

    /// The text form of the log under `header`: the header line, then one line for each frame,
    /// numbered from 1.
    pub fn to_text(&self, header: &LogHeader) -> String {
        std::iter::once(input_log::encode_header(header))
            .chain(
                self.0
                    .iter()
                    .zip(1u32..)
                    .map(|(input, number)| input_log::encode_frame(number, input)),
            )
            .collect()
    }
}

#[cfg(test)]
mod tests;
