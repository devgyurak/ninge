// SPDX-License-Identifier: Apache-2.0
//! The `ninge-runner` binary: exit codes, stdout, stderr.

use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use ninge_runner::{frame_hash, testrom_path};
use std::process::{Command, Output};

fn run_cli(args: &[&str]) -> Output {
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
    assert_eq!(run_cli(&[]).status.code(), Some(2));
}

#[test]
fn given_no_arguments_when_cli_runs_then_stderr_is_not_empty() {
    assert!(!run_cli(&[]).stderr.is_empty());
}
