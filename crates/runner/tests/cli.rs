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

/// The smoke ROM never writes DISPCNT, so engine A is display-off and shows white; POWCNT1
/// starts with the swap bit set, which puts engine A on the top screen. Engine B is powered on
/// and its DISPCNT is 0, display off, so the lower screen is white too (`docs/hw/bg-modes.md`).
fn expected_line(frames: u64) -> String {
    let white = frame_hash(&vec![0x00FF_FFFFu32; SCREEN_WIDTH * SCREEN_HEIGHT]);
    format!("frames={frames} top={white:016x} bottom={white:016x}\n")
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

/// A temp directory holding the given files; it is removed when dropped.
struct SuiteDir(std::path::PathBuf);

impl SuiteDir {
    fn with(name: &str, files: &[(&str, &[u8])]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ninge-cli-suite-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir is writable");
        files
            .iter()
            .for_each(|(file, bytes)| std::fs::write(dir.join(file), bytes).expect("write"));
        Self(dir)
    }
}

impl Drop for SuiteDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn given_an_empty_suite_directory_when_cli_runs_cpu_suite_then_it_prints_the_zero_total_and_exits_zero()
 {
    let dir = SuiteDir::with("empty", &[]);
    let out = run_cli(&[OsStr::new("cpu-suite"), dir.0.as_os_str()]);
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
    let dir = SuiteDir::with("one", &[("a.json.bin", &header)]);
    let out = run_cli(&[OsStr::new("cpu-suite"), dir.0.as_os_str()]);
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

#[test]
fn given_cpu_suite_with_a_flag_as_the_directory_when_cli_runs_then_exit_code_is_two() {
    let out = run_cli(&["cpu-suite", "--help"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn given_a_file_of_junk_bytes_when_cli_runs_it_as_a_rom_then_exit_code_is_three() {
    let dir = SuiteDir::with("junk-code", &[("junk.nds", &[1, 2, 3, 4, 5])]);
    let out = run_cli(&[dir.0.join("junk.nds")]);
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn given_a_file_of_junk_bytes_when_cli_runs_it_as_a_rom_then_stderr_says_the_rom_was_refused() {
    let dir = SuiteDir::with("junk-msg", &[("junk.nds", &[1, 2, 3, 4, 5])]);
    let out = run_cli(&[dir.0.join("junk.nds")]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("the ROM was refused"));
}

#[test]
fn given_a_file_of_junk_bytes_when_cli_runs_it_as_a_rom_then_stdout_is_empty() {
    let dir = SuiteDir::with("junk-out", &[("junk.nds", &[1, 2, 3, 4, 5])]);
    let out = run_cli(&[dir.0.join("junk.nds")]);
    assert!(out.stdout.is_empty());
}

fn save_file(name: &str, len: usize) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("ninge-cli-{}-{name}", std::process::id()));
    std::fs::write(&path, vec![0x5A; len]).expect("temp dir is writable");
    path
}

#[test]
fn given_a_save_file_of_no_chip_size_when_cli_runs_then_exit_code_is_4() {
    let path = save_file("odd.bin", 1000);
    let out = run_cli_on_smoke(&["--save", path.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(4));
}

#[test]
fn given_a_save_file_and_a_matching_type_when_cli_runs_then_it_succeeds_and_keeps_the_file() {
    let path = save_file("ok.bin", 8192);
    let out = run_cli_on_smoke(&[
        "--save",
        path.to_str().expect("utf-8 path"),
        "--save-type",
        "fram-8k",
    ]);
    assert_eq!(
        (out.status.code(), std::fs::read(&path).expect("kept")),
        (Some(0), vec![0x5A; 8192])
    );
}

#[test]
fn given_an_unknown_save_type_when_cli_runs_then_exit_code_is_2() {
    let out = run_cli_on_smoke(&["--save-type", "flash-3m"]);
    assert_eq!(out.status.code(), Some(2));
}

// ---- replay ----

/// The header CRC of the smoke ROM, which the log's header must carry.
fn smoke_crc() -> u16 {
    let mut machine = ninge_core::Machine::new();
    let rom = std::fs::read(testrom_path("smoke")).expect("smoke ROM is built");
    machine.load_rom(rom).expect("a valid header");
    machine.header().expect("a header").crc
}

/// A log file under the temp dir with `body` after a header for the smoke ROM (no save chip,
/// whose memory hashes as FNV-1a of nothing).
fn log_file(name: &str, rom_crc: u16, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("ninge-cli-{}-{name}", std::process::id()));
    let header = format!("ninge-input 1 rom={rom_crc:04x} save=none:cbf29ce484222325\n");
    std::fs::write(&path, header + body).expect("temp dir is writable");
    path
}

/// `replay` on the smoke ROM and `log`, with a save path that does not exist, so no file beside
/// the ROM takes part.
fn run_replay(log: &std::path::Path, extra: &[&str]) -> Output {
    let rom = testrom_path("smoke");
    let no_save =
        std::env::temp_dir().join(format!("ninge-cli-{}-no-such.sav", std::process::id()));
    let mut args: Vec<&OsStr> = vec![
        OsStr::new("replay"),
        rom.as_os_str(),
        log.as_os_str(),
        OsStr::new("--save"),
        no_save.as_os_str(),
    ];
    args.extend(extra.iter().map(OsStr::new));
    run_cli(&args)
}

#[test]
fn given_a_log_of_three_frames_when_cli_replays_it_then_stdout_is_the_report_of_three_frames() {
    let log = log_file("three.ninput", smoke_crc(), "f=1 b=A\nf=2\nf=3 wall=7\n");
    let out = run_replay(&log, &[]);
    assert_eq!(
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned()
        ),
        (Some(0), expected_line(3))
    );
}

#[test]
fn given_a_log_of_one_frame_when_cli_replays_it_for_four_then_four_frames_run() {
    let log = log_file("one.ninput", smoke_crc(), "f=1\n");
    let out = run_replay(&log, &["--frames", "4"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected_line(4));
}

#[test]
fn given_a_log_cut_off_mid_line_when_cli_replays_it_then_the_prefix_runs_and_stderr_says_so() {
    let log = log_file("cut.ninput", smoke_crc(), "f=1\nf=2 b=");
    let out = run_replay(&log, &[]);
    assert_eq!(
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).contains("partial line")
        ),
        (Some(0), expected_line(1), true)
    );
}

#[test]
fn given_a_malformed_log_when_cli_replays_it_then_exit_code_is_5_and_stderr_names_the_line() {
    let log = log_file("bad.ninput", smoke_crc(), "f=1\nf=9\n");
    let out = run_replay(&log, &[]);
    assert_eq!(
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).contains("line 3"),
            out.stdout.is_empty()
        ),
        (Some(5), true, true)
    );
}

#[test]
fn given_a_log_for_another_rom_when_cli_replays_it_then_exit_code_is_5_and_no_frame_runs() {
    let log = log_file("other.ninput", smoke_crc() ^ 1, "f=1\n");
    let out = run_replay(&log, &[]);
    assert_eq!((out.status.code(), out.stdout.is_empty()), (Some(5), true));
}

#[test]
fn given_a_log_that_does_not_exist_when_cli_replays_it_then_exit_code_is_1() {
    let out = run_replay(std::path::Path::new("/nonexistent/ninge/none.ninput"), &[]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn given_replay_without_a_log_when_cli_runs_then_exit_code_is_2() {
    let rom = testrom_path("smoke");
    let out = run_cli(&[OsStr::new("replay"), rom.as_os_str()]);
    assert_eq!(out.status.code(), Some(2));
}
