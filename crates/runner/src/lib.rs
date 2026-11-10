// SPDX-License-Identifier: Apache-2.0
//! ninge-runner: headless runner for tests, traces, and golden frames.
//!
//! All file IO lives here; ninge-core only ever sees bytes. Usage:
//!
//! ```text
//! ninge-runner <rom> [--frames N]    # N defaults to 1
//! ninge-runner cpu-suite <dir>       # run the ARM7TDMI JSON suite files in <dir>
//! ```
//!
//! A ROM path that starts with `-` is read as a flag and refused; pass `./-x.nds` instead. A
//! ROM literally named `cpu-suite` is passed as `./cpu-suite`. The same rule applies to the
//! `cpu-suite` directory: pass `./-x` for a directory whose name starts with `-`.
//!
//! The binary prints one line, `frames=N top=<016x> bottom=<016x>`, where each hash is
//! [`frame_hash`] of a screen. It exits 0 on success, 1 on an IO error, 2 on a usage error, 3 when
//! the machine refuses the ROM (an invalid header).
//!
//! `cpu-suite` prints one `<file>: <cases> cases, <failed> failed` line per `*.json.bin` file
//! and a final `total:` line. Failing cases are expected until the CPU executes instructions
//! (#8–#13), so they do not change the exit code; see [`cpu_suite`].

pub mod cpu_suite;
pub mod golden;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use ninge_core::{Machine, RomError, Screen};

/// Why the runner could not do what was asked.
#[derive(Debug)]
pub enum RunnerError {
    /// The command line was malformed; the message says how.
    Usage(String),
    /// A file could not be read.
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
    /// A CPU suite file could not be read or decoded.
    Suite(cpu_suite::SuiteError),
    /// The machine refused the ROM: its header is invalid.
    Rom(RomError),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message}"),
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Suite(error) => write!(f, "{error}"),
            Self::Rom(error) => write!(f, "the ROM was refused: {error}"),
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Usage(_) => None,
            Self::Io { source, .. } => Some(source),
            Self::Suite(error) => Some(error),
            Self::Rom(error) => Some(error),
        }
    }
}

impl From<RomError> for RunnerError {
    fn from(error: RomError) -> Self {
        Self::Rom(error)
    }
}

impl From<cpu_suite::SuiteError> for RunnerError {
    fn from(error: cpu_suite::SuiteError) -> Self {
        Self::Suite(error)
    }
}

/// What the command line asks for.
#[derive(Debug)]
pub enum Command {
    /// Run a ROM for some frames and print its screen hashes.
    Run(Args),
    /// Run the ARM7TDMI JSON suite files in a directory.
    CpuSuite(PathBuf),
}

/// Parses the arguments after the program name into a [`Command`].
///
/// `cpu-suite <dir>` selects the suite; anything else is a ROM run as in [`parse_args`].
pub fn parse_command(args: impl IntoIterator<Item = OsString>) -> Result<Command, RunnerError> {
    let mut args = args.into_iter().peekable();
    if args.peek().is_none_or(|first| first != "cpu-suite") {
        return parse_args(args).map(Command::Run);
    }
    args.next();
    let dir = args
        .next()
        .ok_or_else(|| RunnerError::Usage(format!("missing suite directory\n{USAGE}")))?;
    if dir.as_encoded_bytes().first() == Some(&b'-') {
        return Err(RunnerError::Usage(format!(
            "expected a directory, found {}\n{USAGE}",
            dir.to_string_lossy()
        )));
    }
    match args.next() {
        Some(extra) => Err(unexpected(&extra)),
        None => Ok(Command::CpuSuite(PathBuf::from(dir))),
    }
}

/// Parsed command line: `<rom> [--frames N]`.
#[derive(Debug)]
pub struct Args {
    rom: PathBuf,
    frames: u64,
}

impl Args {
    /// The ROM file to load.
    pub fn rom(&self) -> &Path {
        &self.rom
    }

    /// How many frames to run.
    pub fn frames(&self) -> u64 {
        self.frames
    }
}

const USAGE: &str = "usage: ninge-runner <rom> [--frames N]\n       ninge-runner cpu-suite <dir>";

/// Parses the arguments after the program name.
///
/// The ROM path stays an [`OsString`], so a path that is not UTF-8 still works.
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, RunnerError> {
    let mut args = args.into_iter();
    let rom = args
        .next()
        .ok_or_else(|| RunnerError::Usage(format!("missing ROM path\n{USAGE}")))?;
    if rom.as_encoded_bytes().first() == Some(&b'-') {
        return Err(RunnerError::Usage(format!(
            "expected a ROM path, found {}\n{USAGE}",
            rom.to_string_lossy()
        )));
    }
    let frames = match args.next() {
        None => 1,
        Some(flag) if flag == "--frames" => args
            .next()
            .and_then(|value| value.to_str().and_then(|text| text.parse().ok()))
            .ok_or_else(|| RunnerError::Usage(format!("--frames needs a number\n{USAGE}")))?,
        Some(other) => return Err(unexpected(&other)),
    };
    if let Some(extra) = args.next() {
        return Err(unexpected(&extra));
    }
    Ok(Args {
        rom: PathBuf::from(rom),
        frames,
    })
}

fn unexpected(argument: &OsStr) -> RunnerError {
    RunnerError::Usage(format!(
        "unexpected argument {}\n{USAGE}",
        argument.to_string_lossy()
    ))
}

/// Reads a ROM file into memory; the bytes go to the core unparsed.
pub fn load_rom_file(path: &Path) -> Result<Vec<u8>, RunnerError> {
    std::fs::read(path).map_err(|source| RunnerError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Result of a headless run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunReport {
    /// Frames the machine ran.
    pub frames: u64,
    /// [`frame_hash`] of the top screen after the last frame.
    pub top_hash: u64,
    /// [`frame_hash`] of the bottom screen after the last frame.
    pub bottom_hash: u64,
}

impl fmt::Display for RunReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frames={} top={:016x} bottom={:016x}",
            self.frames, self.top_hash, self.bottom_hash
        )
    }
}

/// Loads `rom` into a fresh machine, runs `frames` frames, and hashes both screens.
///
/// A ROM the machine refuses (an invalid header) is an error, not a blank run.
pub fn run(rom: Vec<u8>, frames: u64) -> Result<RunReport, RunnerError> {
    let mut machine = Machine::new();
    machine.load_rom(rom)?;
    for _ in 0..frames {
        machine.run_frame();
    }
    Ok(RunReport {
        frames: machine.frame_count(),
        top_hash: frame_hash(machine.frame(Screen::Top)),
        bottom_hash: frame_hash(machine.frame(Screen::Bottom)),
    })
}

/// FNV-1a 64 over each pixel's little-endian bytes, in row order.
///
/// Decided in #68; defined in `tests/golden/README.md` and used by [`golden`].
pub fn frame_hash(pixels: &[u32]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    pixels
        .iter()
        .flat_map(|pixel| pixel.to_le_bytes())
        .fold(OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(PRIME)
        })
}

/// Path of a built test ROM, `testroms/build/<name>.nds` (built by `build_testroms.sh`).
///
/// For in-repo tests only: it resolves into this source checkout.
pub fn testrom_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testroms/build")
        .join(format!("{name}.nds"))
}

#[cfg(test)]
mod tests;
