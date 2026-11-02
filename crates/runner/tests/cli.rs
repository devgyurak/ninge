// SPDX-License-Identifier: Apache-2.0
//! The `ninge-runner` binary: exit codes, stdout, stderr.

use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use ninge_runner::{frame_hash, testrom_path};
use std::ffi::OsStr;
use std::process::{Command, Output};

fn run_cli<S: AsRef<OsStr>>(args: &[S]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ninge-runner"))
        .args(args)
        .output()
        .expect("binary starts")
}

fn run_cli_on_smoke(extra: &[&str]) -> Output {
    let rom = testrom_path("smoke");
    let mut args = vec![rom.to_str().expect("utf-8 path")];
    args.extend_from_slice(extra);
    run_cli(&args)
}

fn expected_line(frames: u64) -> String {
    let black = frame_hash(&vec![0u32; SCREEN_WIDTH * SCREEN_HEIGHT]);
    format!("frames={frames} top={black:016x} bottom={black:016x}\n")
}

#[test]
fn given_smoke_rom_when_cli_runs_3_frames_then_exit_code_is_zero() {
    assert_eq!(run_cli_on_smoke(&["--frames", "3"]).status.code(), Some(0));
}

#[test]
fn given_smoke_rom_when_cli_runs_3_frames_then_stdout_is_exactly_the_report_line() {
    let out = run_cli_on_smoke(&["--frames", "3"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected_line(3));
}

#[test]
fn given_smoke_rom_when_cli_runs_without_frames_flag_then_stdout_reports_one_frame() {
    let out = run_cli_on_smoke(&[]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected_line(1));
}

#[test]
fn given_nonexistent_rom_path_when_cli_runs_then_exit_code_is_one() {
    let out = run_cli(&["/nonexistent/ninge/none.nds"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn given_nonexistent_rom_path_when_cli_runs_then_stderr_is_not_empty() {
    let out = run_cli(&["/nonexistent/ninge/none.nds"]);
    assert!(!out.stderr.is_empty());
}

#[test]
fn given_no_arguments_when_cli_runs_then_exit_code_is_two() {
    assert_eq!(run_cli::<&str>(&[]).status.code(), Some(2));
}

#[test]
fn given_no_arguments_when_cli_runs_then_stderr_is_not_empty() {
    assert!(!run_cli::<&str>(&[]).stderr.is_empty());
}

#[test]
fn given_extra_argument_when_cli_runs_then_exit_code_is_two() {
    let out = run_cli_on_smoke(&["--frames", "2", "extra"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn given_help_flag_when_cli_runs_then_exit_code_is_two() {
    assert_eq!(run_cli(&["--help"]).status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn given_non_utf8_missing_rom_path_when_cli_runs_then_exit_code_is_one() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::ffi::OsString::from_vec(b"/nonexistent/ninge/\xFF\xFE.nds".to_vec());
    assert_eq!(run_cli(&[path]).status.code(), Some(1));
}

fn suite_dir_with(name: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ninge-cli-suite-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir is writable");
    files
        .iter()
        .for_each(|(file, bytes)| std::fs::write(dir.join(file), bytes).expect("write"));
    dir
}

#[test]
fn given_an_empty_suite_directory_when_cli_runs_cpu_suite_then_it_prints_the_zero_total_and_exits_zero()
 {
    let dir = suite_dir_with("empty", &[]);
    let out = run_cli(&[OsStr::new("cpu-suite"), dir.as_os_str()]);
    assert_eq!(
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned()
        ),
        (Some(0), "total: 0 cases, 0 failed\n".to_string())
    );
}

#[test]
fn given_a_suite_file_with_no_cases_when_cli_runs_cpu_suite_then_the_file_has_its_own_line() {
    let mut header = Vec::new();
    header.extend_from_slice(&0xD33D_BAE0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    let dir = suite_dir_with("one", &[("a.json.bin", &header)]);
    let out = run_cli(&[OsStr::new("cpu-suite"), dir.as_os_str()]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "a.json.bin: 0 cases, 0 failed\ntotal: 0 cases, 0 failed\n"
    );
}

#[test]
fn given_a_missing_suite_directory_when_cli_runs_cpu_suite_then_exit_code_is_one() {
    let out = run_cli(&["cpu-suite", "/nonexistent/ninge/suite"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn given_cpu_suite_without_a_directory_when_cli_runs_then_exit_code_is_two() {
    let out = run_cli(&["cpu-suite"]);
    assert_eq!(out.status.code(), Some(2));
}
