// SPDX-License-Identifier: Apache-2.0
//! ninge-runner: headless runner for tests, traces, and golden frames.
//!
//! All file IO lives here; ninge-core only ever sees bytes. Usage:
//!
//! ```text
//! ninge-runner <rom> [--frames N]    # N defaults to 1
//! ```
//!
//! A ROM path that starts with `-` is read as a flag and refused; pass `./-x.nds` instead.
//!
//! The binary prints one line, `frames=N top=<016x> bottom=<016x>`, where each hash is
//! [`frame_hash`] of a screen. It exits 0 on success, 1 on an IO error, 2 on a usage error.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use ninge_core::{Machine, Screen};

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
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message}"),
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Usage(_) => None,
            Self::Io { source, .. } => Some(source),
        }
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

const USAGE: &str = "usage: ninge-runner <rom> [--frames N]";

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
pub fn run(rom: Vec<u8>, frames: u64) -> RunReport {
    let mut machine = Machine::new();
    machine.load_rom(rom);
    for _ in 0..frames {
        machine.run_frame();
    }
    RunReport {
        frames: machine.frame_count(),
        top_hash: frame_hash(machine.frame(Screen::Top)),
        bottom_hash: frame_hash(machine.frame(Screen::Bottom)),
    }
}

/// FNV-1a 64 over each pixel's little-endian bytes, in row order.
///
/// The hash is documented and used for golden frames by #4.
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
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn parse(args: &[&str]) -> Result<Args, RunnerError> {
        parse_args(args.iter().map(OsString::from))
    }

    fn missing_rom_error() -> RunnerError {
        load_rom_file(Path::new("/nonexistent/ninge/none.nds")).expect_err("path does not exist")
    }

    fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ninge-runner-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).expect("temp dir is writable");
        path
    }

    #[test]
    fn given_only_a_rom_path_when_parsing_args_then_frames_default_to_one() {
        let args = parse(&["game.nds"]).expect("valid args");
        assert_eq!(
            (args.rom().to_path_buf(), args.frames()),
            (PathBuf::from("game.nds"), 1)
        );
    }

    #[test]
    fn given_frames_flag_when_parsing_args_then_frames_is_the_given_count() {
        let args = parse(&["game.nds", "--frames", "5"]).expect("valid args");
        assert_eq!(
            (args.rom().to_path_buf(), args.frames()),
            (PathBuf::from("game.nds"), 5)
        );
    }

    #[test]
    fn given_no_arguments_when_parsing_args_then_usage_error() {
        assert!(matches!(parse(&[]), Err(RunnerError::Usage { .. })));
    }

    #[test]
    fn given_non_numeric_frames_when_parsing_args_then_usage_error() {
        let result = parse(&["game.nds", "--frames", "many"]);
        assert!(matches!(result, Err(RunnerError::Usage { .. })));
    }

    #[test]
    fn given_frames_flag_without_value_when_parsing_args_then_usage_error() {
        let result = parse(&["game.nds", "--frames"]);
        assert!(matches!(result, Err(RunnerError::Usage { .. })));
    }

    #[test]
    fn given_extra_argument_after_frames_when_parsing_args_then_usage_error() {
        let result = parse(&["rom", "--frames", "2", "extra"]);
        assert!(matches!(result, Err(RunnerError::Usage { .. })));
    }

    #[test]
    fn given_repeated_frames_flag_when_parsing_args_then_usage_error() {
        let result = parse(&["rom", "--frames", "2", "--frames", "3"]);
        assert!(matches!(result, Err(RunnerError::Usage { .. })));
    }

    #[test]
    fn given_flag_in_rom_position_when_parsing_args_then_usage_error() {
        let result = parse(&["--help"]);
        assert!(matches!(result, Err(RunnerError::Usage { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn given_non_utf8_rom_path_when_parsing_args_then_rom_path_is_kept() {
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(vec![b'r', 0xFF, 0xFE, b'.', b'n', b'd', b's']);
        let args = parse_args([raw.clone()]).expect("valid args");
        assert_eq!(args.rom(), Path::new(&raw));
    }

    #[test]
    fn given_no_pixels_when_hashing_then_hash_is_the_fnv1a_offset_basis() {
        assert_eq!(frame_hash(&[]), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn given_one_pixel_when_hashing_then_hash_is_fnv1a_over_its_little_endian_bytes() {
        // FNV-1a 64 over bytes 01 02 03 04 (pixel 0x0403_0201, little-endian):
        // h = 0xcbf29ce484222325; for each byte b: h ^= b; h = h.wrapping_mul(0x100000001b3).
        // Computed independently with a python3 snippet (big-int arithmetic masked to 64 bits).
        assert_eq!(frame_hash(&[0x0403_0201]), 0xbe7a_5e77_5165_785d);
    }

    #[test]
    fn given_one_black_pixel_when_hashing_then_hash_is_fnv1a_over_four_zero_bytes() {
        // Same python3 computation over 00 00 00 00.
        assert_eq!(frame_hash(&[0]), 0x4d25_767f_9dce_13f5);
    }

    #[test]
    fn given_a_run_report_when_displayed_then_it_is_one_key_value_line() {
        let report = RunReport {
            frames: 3,
            top_hash: 0xab,
            bottom_hash: 0x1234_5678_9abc_def0,
        };
        assert_eq!(
            report.to_string(),
            "frames=3 top=00000000000000ab bottom=123456789abcdef0"
        );
    }

    #[test]
    fn given_usage_error_when_displayed_then_message_is_not_empty() {
        let err = parse(&[]).expect_err("no rom path");
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn given_io_error_when_displayed_then_message_names_the_path() {
        assert!(
            missing_rom_error()
                .to_string()
                .contains("/nonexistent/ninge/none.nds")
        );
    }

    #[test]
    fn given_io_error_when_asking_for_source_then_underlying_error_is_present() {
        assert!(missing_rom_error().source().is_some());
    }

    #[test]
    fn given_usage_error_when_asking_for_source_then_there_is_none() {
        let err = parse(&[]).expect_err("no rom path");
        assert!(err.source().is_none());
    }

    #[test]
    fn given_missing_file_when_loading_rom_then_io_error() {
        assert!(matches!(missing_rom_error(), RunnerError::Io { .. }));
    }

    #[test]
    fn given_existing_file_when_loading_rom_then_bytes_are_returned_verbatim() {
        let path = write_temp("verbatim.bin", &[9, 8, 7]);
        let loaded = load_rom_file(&path).expect("file exists");
        std::fs::remove_file(&path).expect("cleanup");
        assert_eq!(loaded, vec![9, 8, 7]);
    }
}
