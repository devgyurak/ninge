// SPDX-License-Identifier: Apache-2.0
//! `ninge <rom> --record PATH`: write the input each frame is run with to an input log
//! (`docs/contributing/input-log.md`), so `ninge-runner replay` can run it again.
//!
//! The [`Recorder`] takes its writer from the caller, so the logic is tested without a disk; the
//! file itself is [`create_new`], which refuses an existing path rather than clobber it. A write
//! error ends the recording, not the run: the error is kept and told to the user when the
//! program ends ([`Recorder::note`]).

use ninge_core::input_log::{LogHeader, encode_frame, encode_header, save_hash};
use ninge_core::{Input, Machine};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Writes one log line per frame, flushed as it is written, so a program that ends any way at
/// all leaves a log that is a valid prefix.
pub struct Recorder {
    /// The writer; `None` after a write failed.
    sink: Option<Box<dyn Write + Send>>,
    /// Where the log goes, for the note.
    path: PathBuf,
    /// Frame lines written so far.
    frames: u32,
    /// The write error that ended the recording.
    error: Option<io::Error>,
}

impl Recorder {
    /// Starts a log on `sink`: writes the header for the program and the save memory `machine`
    /// holds now, which must be before its first frame.
    ///
    /// # Errors
    /// The header could not be written; nothing is recorded.
    pub fn begin(
        mut sink: Box<dyn Write + Send>,
        path: PathBuf,
        machine: &Machine,
    ) -> io::Result<Self> {
        let header = LogHeader {
            rom_crc: machine
                .header()
                .expect("a machine that booted a ROM has a header")
                .crc,
            save_type: machine.save_type(),
            save_hash: save_hash(machine.save_data()),
        };
        sink.write_all(encode_header(&header).as_bytes())?;
        sink.flush()?;
        Ok(Self {
            sink: Some(sink),
            path,
            frames: 0,
            error: None,
        })
    }

    /// Writes the line for the next frame: the input the frame was run with. After a write
    /// failed this does nothing.
    pub fn record(&mut self, input: &Input) {
        let Some(sink) = &mut self.sink else {
            return;
        };
        let written = self
            .frames
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "too many frames to number"))
            .and_then(|number| {
                sink.write_all(encode_frame(number, input).as_bytes())
                    .and_then(|()| sink.flush())
                    .map(|()| number)
            });
        match written {
            Ok(number) => self.frames = number,
            Err(error) => {
                self.sink = None;
                self.error = Some(error);
            }
        }
    }

    /// How many frames are in the log.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// What to tell the user once the program ends: how much was recorded, or why it stopped.
    pub fn note(&self) -> String {
        match &self.error {
            Some(error) => format!(
                "recording to {} stopped after {} frames: {error}",
                self.path.display(),
                self.frames
            ),
            None => format!("recorded {} frames to {}", self.frames, self.path.display()),
        }
    }
}

/// Creates the log file, refusing a path that already exists.
pub fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(test)]
mod tests;
