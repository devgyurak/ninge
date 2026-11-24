// SPDX-License-Identifier: Apache-2.0
//! The `ninge-runner trace` binary on our own smoke ROM: exit codes, stdout, the trace file
//! (`docs/contributing/trace-oracle.md`, "The trace format").

use ninge_core::trace::{TraceCpu, parse_trace};
use ninge_runner::testrom_path;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Output};

fn run_cli<S: AsRef<OsStr>>(args: &[S]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ninge-runner"))
        .args(args)
        .output()
        .expect("binary starts")
}

/// A unique file under the temp directory, removed when dropped.
struct TempFile(PathBuf);

impl TempFile {
    fn named(name: &str) -> Self {
        Self(std::env::temp_dir().join(format!("ninge-trace-it-{}-{name}", std::process::id())))
    }

    fn path(&self) -> &str {
        self.0.to_str().expect("utf-8 path")
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs `trace` on the smoke ROM into `out`, with `extra` flags after `--frames`.
fn trace_smoke(out: &TempFile, frames: &str, extra: &[&str]) -> Output {
    let rom = testrom_path("smoke");
    let mut args = vec![
        "trace",
        rom.to_str().expect("utf-8 path"),
        "--frames",
        frames,
        "--out",
        out.path(),
    ];
    args.extend_from_slice(extra);
    run_cli(&args)
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The summary line the file itself implies: `frames=N lines=L arm9=A arm7=B`.
fn summary_of_file(out: &TempFile, frames: u64) -> String {
    let text = std::fs::read_to_string(&out.0).expect("the trace file exists");
    let records = parse_trace(&text).expect("the trace file parses");
    let of = |cpu: TraceCpu| records.iter().filter(|r| r.cpu == cpu).count();
    format!(
        "frames={frames} lines={} arm9={} arm7={}\n",
        records.len(),
        of(TraceCpu::Arm9),
        of(TraceCpu::Arm7)
    )
}

#[test]
fn given_the_smoke_rom_when_cli_traces_2_frames_then_exit_code_is_zero() {
    let out = TempFile::named("exit0.trace");
    assert_eq!(trace_smoke(&out, "2", &[]).status.code(), Some(0));
}

#[test]
fn given_the_smoke_rom_when_cli_traces_2_frames_then_stdout_is_the_single_summary_line_of_the_file()
{
    let out = TempFile::named("summary.trace");
    let output = trace_smoke(&out, "2", &[]);
    assert_eq!(stdout_of(&output), summary_of_file(&out, 2));
}

#[test]
fn given_the_smoke_rom_when_cli_traces_2_frames_then_stderr_is_empty() {
    let out = TempFile::named("quiet.trace");
    assert_eq!(stderr_of(&trace_smoke(&out, "2", &[])), "");
}

#[test]
fn given_the_smoke_rom_when_cli_traces_then_the_file_starts_with_the_header_line() {
    let out = TempFile::named("header.trace");
    trace_smoke(&out, "1", &[]);
    let text = std::fs::read_to_string(&out.0).expect("the trace file exists");
    assert!(text.starts_with("ninge-trace 1\n"));
}

#[test]
fn given_cpu_arm9_when_cli_traces_then_the_summary_reports_no_arm7_records() {
    let out = TempFile::named("arm9.trace");
    let output = trace_smoke(&out, "2", &["--cpu", "arm9"]);
    let line = stdout_of(&output);
    assert_eq!(
        (
            line.ends_with(" arm7=0\n"),
            line == summary_of_file(&out, 2)
        ),
        (true, true)
    );
}

#[test]
fn given_a_from_frame_when_cli_traces_then_the_summary_still_reports_all_frames() {
    let out = TempFile::named("from.trace");
    let output = trace_smoke(&out, "3", &["--from-frame", "2"]);
    assert_eq!(stdout_of(&output), summary_of_file(&out, 3));
}

#[test]
fn given_the_same_command_twice_when_cli_traces_then_the_two_files_are_byte_identical() {
    let (a, b) = (
        TempFile::named("det-a.trace"),
        TempFile::named("det-b.trace"),
    );
    trace_smoke(&a, "2", &[]);
    trace_smoke(&b, "2", &[]);
    let (bytes_a, bytes_b) = (
        std::fs::read(&a.0).expect("a exists"),
        std::fs::read(&b.0).expect("b exists"),
    );
    assert!(bytes_a == bytes_b, "two runs wrote different traces");
}

#[test]
fn given_trace_without_arguments_when_cli_runs_then_exit_code_is_two() {
    assert_eq!(run_cli(&["trace"]).status.code(), Some(2));
}

#[test]
fn given_trace_without_arguments_when_cli_runs_then_stderr_says_never_to_commit_or_paste_a_trace() {
    let stderr = stderr_of(&run_cli(&["trace"]));
    assert!(
        stderr.contains("not yours to share") && stderr.contains("never commit or paste it"),
        "{stderr}"
    );
}

#[test]
fn given_trace_without_out_when_cli_runs_then_exit_code_is_two_and_stdout_is_empty() {
    let rom = testrom_path("smoke");
    let output = run_cli(&["trace", rom.to_str().expect("utf-8"), "--frames", "1"]);
    assert_eq!(
        (output.status.code(), output.stdout.is_empty()),
        (Some(2), true)
    );
}

#[test]
fn given_a_file_of_junk_bytes_when_cli_traces_it_then_no_trace_file_is_left() {
    let rom = TempFile::named("junk-left.nds");
    std::fs::write(&rom.0, [1, 2, 3, 4, 5]).expect("temp dir is writable");
    let out = TempFile::named("junk-left.trace");
    let _ = run_cli(&["trace", rom.path(), "--frames", "1", "--out", out.path()]);
    assert!(!out.0.exists());
}

#[test]
fn given_a_from_frame_not_below_frames_when_cli_runs_then_exit_code_is_two_and_no_file_is_made() {
    let out = TempFile::named("badfrom.trace");
    let output = trace_smoke(&out, "2", &["--from-frame", "2"]);
    assert_eq!((output.status.code(), out.0.exists()), (Some(2), false));
}

#[test]
fn given_an_unwritable_out_path_when_cli_traces_then_exit_code_is_one() {
    let out = TempFile::named("missing-dir/out.trace");
    assert_eq!(trace_smoke(&out, "1", &[]).status.code(), Some(1));
}

#[test]
fn given_an_unwritable_out_path_when_cli_traces_then_stderr_names_the_path() {
    let out = TempFile::named("missing-dir/out.trace");
    let stderr = stderr_of(&trace_smoke(&out, "1", &[]));
    assert!(
        stderr.contains(&format!("cannot write {}", out.path())),
        "{stderr}"
    );
}

#[test]
fn given_a_missing_rom_when_cli_traces_then_exit_code_is_one() {
    let out = TempFile::named("norom.trace");
    let output = run_cli(&[
        "trace",
        "/nonexistent/ninge/none.nds",
        "--frames",
        "1",
        "--out",
        out.path(),
    ]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn given_a_file_of_junk_bytes_when_cli_traces_it_then_exit_code_is_three() {
    let rom = TempFile::named("junk.nds");
    std::fs::write(&rom.0, [1, 2, 3, 4, 5]).expect("temp dir is writable");
    let out = TempFile::named("junk.trace");
    let output = run_cli(&["trace", rom.path(), "--frames", "1", "--out", out.path()]);
    assert_eq!(
        (
            output.status.code(),
            stderr_of(&output).contains("the ROM was refused")
        ),
        (Some(3), true)
    );
}
