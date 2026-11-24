// SPDX-License-Identifier: Apache-2.0
//! `ninge-runner replay <rom> <log> [--frames N] [--save PATH]`: run a ROM on a recorded input
//! log (`docs/contributing/input-log.md`).
//!
//! The log's header says which ROM and which save memory it was recorded against; a replay
//! against another is refused before any frame runs. The run is a reproduction, so it never
//! writes the save file: the file must hold what it held when the recording began.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use ninge_core::cart::resolve_save;
use ninge_core::input_log::{self, ParsedLog};
use ninge_core::{Machine, Screen};

use crate::golden::InputLog;
use crate::{RunReport, RunnerError, frame_hash};

/// What `replay` was asked for.
#[derive(Debug)]
pub struct ReplayArgs {
    rom: PathBuf,
    log: PathBuf,
    frames: Option<u64>,
    save: Option<PathBuf>,
}

impl ReplayArgs {
    /// The ROM file to load.
    pub fn rom(&self) -> &Path {
        &self.rom
    }

    /// The input log file to read.
    pub fn log(&self) -> &Path {
        &self.log
    }

    /// How many frames to run: `None` runs the log's frames.
    pub fn frames(&self) -> Option<u64> {
        self.frames
    }

    /// The save file the recording began with, or `None` for the one beside the ROM.
    pub fn save(&self) -> Option<&Path> {
        self.save.as_deref()
    }
}

const USAGE: &str = "usage: ninge-runner replay <rom> <log> [--frames N] [--save PATH]";

/// Parses the arguments after `replay`.
///
/// A path that starts with `-` is read as a flag and refused; pass `./-x` instead.
pub fn parse_replay_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<ReplayArgs, RunnerError> {
    let mut args = args.into_iter();
    let mut path = |what: &str| -> Result<PathBuf, RunnerError> {
        let value = args
            .next()
            .ok_or_else(|| RunnerError::Usage(format!("missing {what} path\n{USAGE}")))?;
        match value.as_encoded_bytes().first() {
            Some(b'-') => Err(RunnerError::Usage(format!(
                "expected a {what} path, found {}\n{USAGE}",
                value.to_string_lossy()
            ))),
            _ => Ok(PathBuf::from(value)),
        }
    };
    let rom = path("ROM")?;
    let log = path("log")?;
    let mut frames = None;
    let mut save = None;
    while let Some(flag) = args.next() {
        let value = args.next();
        let needs = |what: &str| RunnerError::Usage(format!("{what}\n{USAGE}"));
        if flag == "--frames" && frames.is_none() {
            frames = Some(
                value
                    .as_deref()
                    .and_then(OsStr::to_str)
                    .and_then(|text| text.parse().ok())
                    .ok_or_else(|| needs("--frames needs a number"))?,
            );
        } else if flag == "--save" && save.is_none() {
            save = Some(PathBuf::from(
                value.ok_or_else(|| needs("--save needs a path"))?,
            ));
        } else {
            return Err(RunnerError::Usage(format!(
                "unexpected argument {}\n{USAGE}",
                flag.to_string_lossy()
            )));
        }
    }
    Ok(ReplayArgs {
        rom,
        log,
        frames,
        save,
    })
}

/// What a replay produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayOutcome {
    /// The usual report: frames run and both screens' hashes after the last one.
    pub report: RunReport,
    /// Whether the log's last line had no newline and was dropped (a log cut off mid-write).
    pub dropped_partial_line: bool,
}

/// A machine booted on a ROM and save that an input log fits, ready to run the log's frames.
pub struct Replay {
    machine: Machine,
    log: InputLog,
    log_frames: u64,
    dropped_partial_line: bool,
}

impl Replay {
    /// Boots `rom` and checks the log in `log_text` against it.
    ///
    /// The log's header must match the ROM's header CRC and the save memory the machine holds
    /// before frame 1: `save_file` is the save file's bytes, if there is one, fitted to the chip
    /// the log names.
    ///
    /// # Errors
    /// [`RunnerError::Log`] for a log that is malformed, [`RunnerError::LogMismatch`] for one
    /// recorded for another ROM or save, and the ROM and save errors of [`crate::run_with_save`].
    pub fn begin(
        rom: Vec<u8>,
        log_text: &str,
        save_file: Option<Vec<u8>>,
    ) -> Result<Self, RunnerError> {
        let ParsedLog {
            header,
            frames,
            dropped_partial_line,
        } = input_log::parse(log_text)?;
        let mut machine = Machine::new();
        machine.load_rom(rom)?;
        let (kind, data) = resolve_save(Some(header.save_type), save_file)?;
        machine.insert_save(kind, data)?;
        let rom_crc = machine
            .header()
            .expect("load_rom succeeded, so the header is parsed")
            .crc;
        header.check(
            rom_crc,
            machine.save_type(),
            input_log::save_hash(machine.save_data()),
        )?;
        Ok(Self {
            machine,
            log_frames: frames.len() as u64,
            log: InputLog::new(frames),
            dropped_partial_line,
        })
    }

    /// How many frames the log holds.
    pub fn log_frames(&self) -> u64 {
        self.log_frames
    }

    /// The machine, for a test or a debugger to look into after a run.
    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// Runs `frames` more frames, or the log's frames when `None`; frames past the end of the log
    /// run with no input, as [`InputLog::input_for`] gives.
    pub fn run(&mut self, frames: Option<u64>) -> ReplayOutcome {
        let count = frames.unwrap_or(self.log_frames);
        (0..count).for_each(|_| {
            self.machine
                .run_frame(self.log.input_for(self.machine.frame_count()));
        });
        ReplayOutcome {
            report: RunReport {
                frames: self.machine.frame_count(),
                top_hash: frame_hash(self.machine.frame(Screen::Top)),
                bottom_hash: frame_hash(self.machine.frame(Screen::Bottom)),
                audio_hash: None,
            },
            dropped_partial_line: self.dropped_partial_line,
        }
    }
}

/// Boots `rom` on the log in `log_text` and runs it: `frames` of `None` runs the log's frames.
///
/// # Errors
/// As [`Replay::begin`].
pub fn replay(
    rom: Vec<u8>,
    log_text: &str,
    frames: Option<u64>,
    save_file: Option<Vec<u8>>,
) -> Result<ReplayOutcome, RunnerError> {
    Replay::begin(rom, log_text, save_file).map(|mut replay| replay.run(frames))
}

#[cfg(test)]
mod tests;
