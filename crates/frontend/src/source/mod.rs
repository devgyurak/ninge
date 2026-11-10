// SPDX-License-Identifier: Apache-2.0
//! Where the frames come from, and the errors that stop the program before the terminal
//! is touched.

use crate::cli::UsageError;
use ninge_core::{Machine, RomError};
use std::fmt;
use std::path::PathBuf;

/// What produces each frame.
pub enum Source {
    /// The moving dummy pattern, shown when no ROM is given.
    Pattern,
    /// A machine that has loaded a ROM; every tick runs one frame of it.
    Machine(Box<Machine>),
}

/// Builds a machine from ROM bytes, or returns why the header was refused.
pub fn boot(rom: Vec<u8>) -> Result<Machine, RomError> {
    let mut machine = Machine::new();
    machine.load_rom(rom)?;
    Ok(machine)
}

/// Why the program stops before the main loop.
#[derive(Debug)]
pub enum StartError {
    /// Bad arguments (exit 2).
    Usage(UsageError),
    /// The ROM was read but refused (exit 3).
    Rom(RomError),
    /// The ROM file could not be read (exit 1).
    Io {
        /// The path that was asked for.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// The terminal loop failed (exit 1).
    Run(String),
}

impl StartError {
    /// The process exit code, the same as `ninge-runner`'s.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Rom(_) => 3,
            Self::Io { .. } | Self::Run(_) => 1,
        }
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(e) => write!(f, "{e}"),
            Self::Rom(e) => write!(f, "ROM refused: {e}"),
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Run(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StartError {}

/// Reads and boots the ROM at `path`. This is the frontend's only file IO.
pub fn load(path: PathBuf) -> Result<Source, StartError> {
    let rom = std::fs::read(&path).map_err(|source| StartError::Io { path, source })?;
    boot(rom)
        .map(|m| Source::Machine(Box::new(m)))
        .map_err(StartError::Rom)
}

#[cfg(test)]
mod tests;
