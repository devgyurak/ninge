// SPDX-License-Identifier: Apache-2.0
//! Command-line arguments: an optional ROM path and the save chip's options. Pure, so it is tested
//! without a process.

use ninge_core::cart::SaveType;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// The usage line shown with every [`UsageError`].
pub const USAGE: &str = "usage: ninge [--no-audio] [<rom> [--save PATH] [--save-type TYPE]]";

/// Arguments the frontend does not accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{USAGE}", self.0)
    }
}

impl std::error::Error for UsageError {}

/// What the command line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// The ROM to run; none shows the dummy pattern.
    pub rom: Option<PathBuf>,
    /// The save file; the default is the ROM's path with the extension `.sav`.
    pub save: Option<PathBuf>,
    /// The save chip, which `auto` (or leaving it out) leaves to the file's size and detection.
    pub save_type: Option<SaveType>,
    /// `--no-audio`: open no sound device and keep time by the frame period.
    pub no_audio: bool,
}

/// Parses the arguments after the program name: none shows the dummy pattern, one path is the ROM,
/// `--save PATH` and `--save-type TYPE` (which need a ROM) name the save file and chip, and
/// `--no-audio` (with or without a ROM) opens no sound device. More than one path, or any other
/// option, is a usage error.
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Options, UsageError> {
    let mut args = args.into_iter();
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--no-audio" if !options.no_audio => options.no_audio = true,
            "--save" if options.save.is_none() => {
                let value = args
                    .next()
                    .ok_or_else(|| UsageError("--save needs a path".to_string()))?;
                options.save = Some(PathBuf::from(value));
            }
            "--save-type" if options.save_type.is_none() => {
                let value = args
                    .next()
                    .ok_or_else(|| UsageError("--save-type needs a name".to_string()))?;
                options.save_type = Some(
                    value
                        .to_str()
                        .and_then(SaveType::from_name)
                        .ok_or_else(|| unknown_save_type(&value.to_string_lossy()))?,
                );
            }
            _ if text.starts_with('-') => {
                return Err(UsageError(format!("unknown option {text}")));
            }
            _ if options.rom.is_some() => {
                return Err(UsageError("expected at most one ROM path".to_string()));
            }
            _ => options.rom = Some(PathBuf::from(&arg)),
        }
    }
    if options.rom.is_none() && (options.save.is_some() || options.save_type.is_some()) {
        return Err(UsageError("--save and --save-type need a ROM".to_string()));
    }
    Ok(options)
}

fn unknown_save_type(name: &str) -> UsageError {
    UsageError(format!(
        "unknown save type {name}; one of {}",
        SaveType::names().collect::<Vec<_>>().join(", ")
    ))
}

#[cfg(test)]
mod tests;
