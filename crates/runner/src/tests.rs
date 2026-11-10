// SPDX-License-Identifier: Apache-2.0
//! Tests of `the crate root`.

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

fn command(args: &[&str]) -> Result<Command, RunnerError> {
    parse_command(args.iter().map(OsString::from))
}

/// A short description of a parsed command or of why it was refused.
fn describe(parsed: Result<Command, RunnerError>) -> String {
    match parsed {
        Ok(Command::Run(args)) => format!("run {} {}", args.rom().display(), args.frames()),
        Ok(Command::CpuSuite(dir)) => format!("cpu-suite {}", dir.display()),
        Err(RunnerError::Usage(message)) => {
            format!("usage: {}", message.lines().next().unwrap_or(""))
        }
        Err(other) => format!("other: {other}"),
    }
}

#[test]
fn given_cpu_suite_and_a_directory_when_parsing_a_command_then_it_selects_the_suite() {
    assert_eq!(
        describe(command(&["cpu-suite", "some/dir"])),
        "cpu-suite some/dir"
    );
}

#[test]
fn given_a_rom_path_when_parsing_a_command_then_it_selects_a_run() {
    assert_eq!(
        describe(command(&["game.nds", "--frames", "4"])),
        "run game.nds 4"
    );
}

#[test]
fn given_cpu_suite_without_a_directory_when_parsing_a_command_then_it_is_a_usage_error() {
    assert_eq!(
        describe(command(&["cpu-suite"])),
        "usage: missing suite directory"
    );
}

#[test]
fn given_cpu_suite_with_an_extra_argument_when_parsing_a_command_then_it_is_a_usage_error() {
    assert_eq!(
        describe(command(&["cpu-suite", "dir", "more"])),
        "usage: unexpected argument more"
    );
}

#[test]
fn given_no_arguments_when_parsing_a_command_then_it_is_the_missing_rom_usage_error() {
    assert_eq!(describe(command(&[])), "usage: missing ROM path");
}

#[test]
fn given_cpu_suite_with_a_flag_as_the_directory_when_parsing_a_command_then_it_is_a_usage_error() {
    assert_eq!(
        describe(command(&["cpu-suite", "--help"])),
        "usage: expected a directory, found --help"
    );
}

#[test]
fn given_a_rom_named_with_a_dot_slash_cpu_suite_when_parsing_a_command_then_it_is_a_rom() {
    assert_eq!(describe(command(&["./cpu-suite"])), "run ./cpu-suite 1");
}

#[test]
fn given_a_suite_error_when_converted_then_it_displays_and_exposes_its_source() {
    let error = RunnerError::from(cpu_suite::SuiteError::Format {
        offset: 3,
        reason: "bad".to_string(),
    });
    assert_eq!(
        (error.to_string(), error.source().is_some()),
        ("bad suite data at byte 3: bad".to_string(), true)
    );
}
