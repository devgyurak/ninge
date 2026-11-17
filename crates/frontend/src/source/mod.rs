// SPDX-License-Identifier: Apache-2.0
//! Where the frames come from, and the errors that stop the program before the terminal
//! is touched.

use crate::cli::{Options, UsageError};
use crate::savefile::{SaveFile, default_path, read_optional};
use ninge_core::cart::{SaveError, resolve_save};
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
    /// The save file does not fit the save type, or no type has its size (exit 4).
    Save(SaveError),
    /// The terminal loop failed (exit 1).
    Run(String),
}

impl StartError {
    /// The process exit code, the same as `ninge-runner`'s.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Rom(_) => 3,
            Self::Save(_) => 4,
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
            Self::Save(e) => write!(f, "the save was refused: {e}"),
            Self::Run(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StartError {}

/// Reads and boots the ROM at `path`, and fits the save chip from `options`: the save file when it
/// exists, as [`resolve_save`] reads it. These are the frontend's file reads.
pub fn load(path: PathBuf, options: &Options) -> Result<(Source, SaveFile), StartError> {
    let rom = std::fs::read(&path).map_err(|source| StartError::Io {
        path: path.clone(),
        source,
    })?;
    let mut machine = boot(rom).map_err(StartError::Rom)?;
    let save_path = options.save.clone().unwrap_or_else(|| default_path(&path));
    let file = read_optional(&save_path).map_err(|source| StartError::Io {
        path: save_path.clone(),
        source,
    })?;
    let (kind, data) = resolve_save(options.save_type, file).map_err(StartError::Save)?;
    machine.insert_save(kind, data).map_err(StartError::Save)?;
    let save = SaveFile::new(save_path, &machine);
    Ok((Source::Machine(Box::new(machine)), save))
}

/// What the command line asks to run: the ROM's machine with its save file, or the dummy pattern.
pub fn start(options: &Options) -> Result<(Source, Option<SaveFile>), StartError> {
    match &options.rom {
        Some(path) => load(path.clone(), options).map(|(source, save)| (source, Some(save))),
        None => Ok((Source::Pattern, None)),
    }
}

#[cfg(test)]
mod tests;
