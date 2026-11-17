// SPDX-License-Identifier: Apache-2.0
//! ninge-runner: headless runner for tests, traces, and golden frames.
//!
//! All file IO lives here; ninge-core only ever sees bytes. Usage:
//!
//! ```text
//! ninge-runner <rom> [--frames N] [--save PATH] [--save-type TYPE]    # N defaults to 1
//! ninge-runner cpu-suite <dir>       # run the ARM7TDMI JSON suite files in <dir>
//! ```
//!
//! A ROM path that starts with `-` is read as a flag and refused; pass `./-x.nds` instead. A
//! ROM literally named `cpu-suite` is passed as `./cpu-suite`. The same rule applies to the
//! `cpu-suite` directory: pass `./-x` for a directory whose name starts with `-`.
//!
//! The binary prints one line, `frames=N top=<016x> bottom=<016x>`, where each hash is
//! [`frame_hash`] of a screen. It exits 0 on success, 1 on an IO error, 2 on a usage error, 3 when
//! the machine refuses the ROM (an invalid header), 4 when the save file or type is refused.
//!
//! The save chip's file is read at the start when it exists and written at the end when the run
//! changed it, as raw bytes. `--save` names the file (default: the ROM's path with the extension
//! `.sav`); `--save-type` names the chip (`auto` by default: a file's size names it, else the chip
//! is detected from its first commands). See `docs/hw/auxspi-saves.md`.
//!
//! `cpu-suite` prints one `<file>: <cases> cases, <failed> failed` line per `*.json.bin` file
//! and a final `total:` line. Failing cases are expected until the CPU executes instructions
//! (#8–#13), so they do not change the exit code; see [`cpu_suite`].

pub mod cpu_suite;
pub mod golden;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use ninge_core::cart::{SaveError, SaveType, resolve_save};
use ninge_core::{Input, Machine, RomError, Screen};

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
    /// A file could not be written.
    Write {
        /// The file that failed.
        path: PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
    /// The machine refused the ROM: its header is invalid.
    Rom(RomError),
    /// The save file does not fit the save type, or no type has its size.
    Save(SaveError),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message}"),
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Suite(error) => write!(f, "{error}"),
            Self::Write { path, source } => write!(f, "cannot write {}: {source}", path.display()),
            Self::Rom(error) => write!(f, "the ROM was refused: {error}"),
            Self::Save(error) => write!(f, "the save was refused: {error}"),
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Usage(_) => None,
            Self::Io { source, .. } | Self::Write { source, .. } => Some(source),
            Self::Suite(error) => Some(error),
            Self::Rom(error) => Some(error),
            Self::Save(error) => Some(error),
        }
    }
}

impl From<RomError> for RunnerError {
    fn from(error: RomError) -> Self {
        Self::Rom(error)
    }
}

impl From<SaveError> for RunnerError {
    fn from(error: SaveError) -> Self {
        Self::Save(error)
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

/// Parsed command line: `<rom> [--frames N] [--save PATH] [--save-type TYPE]`.
#[derive(Debug)]
pub struct Args {
    rom: PathBuf,
    frames: u64,
    save: Option<PathBuf>,
    save_type: Option<SaveType>,
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

    /// The save file and chip the command line asks for.
    pub fn save_setup(&self) -> SaveSetup {
        SaveSetup {
            path: self
                .save
                .clone()
                .unwrap_or_else(|| default_save_path(&self.rom)),
            kind: self.save_type,
        }
    }
}

const USAGE: &str = "usage: ninge-runner <rom> [--frames N] [--save PATH] [--save-type TYPE]\n       ninge-runner cpu-suite <dir>";

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
    let mut frames = None;
    let mut save = None;
    let mut save_type = None;
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
        } else if flag == "--save-type" && save_type.is_none() {
            let text = value.ok_or_else(|| needs("--save-type needs a name"))?;
            save_type = Some(
                text.to_str()
                    .and_then(SaveType::from_name)
                    .ok_or_else(|| needs(&unknown_save_type(&text)))?,
            );
        } else {
            return Err(unexpected(&flag));
        }
    }
    Ok(Args {
        rom: PathBuf::from(rom),
        frames: frames.unwrap_or(1),
        save,
        save_type,
    })
}

fn unknown_save_type(name: &OsStr) -> String {
    format!(
        "unknown save type {}; one of {}",
        name.to_string_lossy(),
        SaveType::names().collect::<Vec<_>>().join(", ")
    )
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

/// Where the save chip's file is and which chip the user named, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveSetup {
    /// The save file: read at the start when it exists, written at the end when the run changed it.
    pub path: PathBuf,
    /// The chip asked for with `--save-type`; `None` leaves it to the file's size and detection.
    pub kind: Option<SaveType>,
}

/// The save file beside a ROM: its path with the extension `.sav`.
pub fn default_save_path(rom: &Path) -> PathBuf {
    rom.with_extension("sav")
}

/// Reads a save file; a file that is not there is `None`, not an error.
pub fn read_save_file(path: &Path) -> Result<Option<Vec<u8>>, RunnerError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(RunnerError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Writes a save file atomically: the bytes go to `<path>.tmp` and are renamed over `path`, so a
/// crash leaves the old file or the new one, never half of one.
pub fn write_save_file(path: &Path, bytes: &[u8]) -> Result<(), RunnerError> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let written = std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, path));
    written.map_err(|source| {
        // Best effort: a failed write should not leave its temp file behind.
        let _ = std::fs::remove_file(&temp);
        RunnerError::Write {
            path: path.to_path_buf(),
            source,
        }
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
        machine.run_frame(Input::new());
    }
    Ok(RunReport {
        frames: machine.frame_count(),
        top_hash: frame_hash(machine.frame(Screen::Top)),
        bottom_hash: frame_hash(machine.frame(Screen::Bottom)),
    })
}

/// Writes the save memory to `path` when its generation is not `baseline`, the one the host saw at
/// its last write; returns whether it wrote. A chip with no memory yet (`auto`, undecided) has
/// nothing to write.
pub fn flush_save(machine: &Machine, baseline: u64, path: &Path) -> Result<bool, RunnerError> {
    let moved = machine.save_generation() != baseline && !machine.save_data().is_empty();
    if moved {
        write_save_file(path, machine.save_data())?;
    }
    Ok(moved)
}

/// What the run did with the save chip, for the host to tell the user: a wrong guess by detection
/// is visible this way (`docs/hw/auxspi-saves.md`, "Wrong detection and the way out").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveOutcome {
    /// The chip at the end of the run; `Auto` when detection never decided.
    pub kind: SaveType,
    /// Whether the save file was written.
    pub written: bool,
}

impl fmt::Display for SaveOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let written = if self.written { ", file written" } else { "" };
        write!(f, "save chip {}{written}", self.kind.name())
    }
}

/// Like [`run`], with the save chip: the file is read first when it exists and fitted by
/// [`resolve_save`], and written after the last frame when the run changed the save memory.
pub fn run_with_save(
    rom: Vec<u8>,
    frames: u64,
    save: &SaveSetup,
) -> Result<(RunReport, SaveOutcome), RunnerError> {
    let mut machine = Machine::new();
    machine.load_rom(rom)?;
    let (kind, data) = resolve_save(save.kind, read_save_file(&save.path)?)?;
    machine.insert_save(kind, data)?;
    let baseline = machine.save_generation();
    for _ in 0..frames {
        machine.run_frame(Input::new());
    }
    let written = flush_save(&machine, baseline, &save.path)?;
    let report = RunReport {
        frames: machine.frame_count(),
        top_hash: frame_hash(machine.frame(Screen::Top)),
        bottom_hash: frame_hash(machine.frame(Screen::Bottom)),
    };
    let outcome = SaveOutcome {
        kind: machine.save_type(),
        written,
    };
    Ok((report, outcome))
}

/// Like [`run`], on a machine whose cartridge has a blank save chip of `kind` (every byte `FFh`),
/// for the golden tests of a ROM that talks to its chip. The image is made here, never read from a
/// file, so no save file takes part (`tests/golden/README.md`).
pub fn run_with_blank_save(
    rom: Vec<u8>,
    frames: u64,
    kind: SaveType,
) -> Result<RunReport, RunnerError> {
    let mut machine = Machine::new();
    machine.load_rom(rom)?;
    let (kind, data) = resolve_save(Some(kind), None)?;
    machine.insert_save(kind, data)?;
    for _ in 0..frames {
        machine.run_frame(Input::new());
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
