// SPDX-License-Identifier: Apache-2.0
//! The `ninge-tracediff` binary end to end: exit 0 for no divergence, 1 for a divergence, 2 for a
//! usage error, 3 for any other error (an unreadable file, a bad trace, no oracle to connect to).
//!
//! The traces are made from our own test ROM (`testroms/smoke`, built by `verify.sh`, never
//! committed) with the public API of ninge-core, and written to temporary files that are removed
//! when the test ends. No oracle is needed: the `files` command compares two traces.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use ninge_core::trace::{HEADER, TraceCpus, TraceRecord, TraceSink, Tracer};
use ninge_core::{Input, Machine};

/// How many records of the smoke ROM's ARM9 the traces hold.
const WINDOW: usize = 400;
/// The register edited in the second trace.
const EDITED: usize = 3;

/// A sink that keeps the first [`WINDOW`] records.
struct Head(Vec<TraceRecord>);

impl TraceSink for Head {
    fn record(&mut self, record: &TraceRecord) {
        if self.0.len() < WINDOW {
            self.0.push(*record);
        }
    }
}

/// The first records of the ARM9 running the smoke ROM for one frame.
fn smoke_records() -> Vec<TraceRecord> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testroms/build/smoke.nds"
    );
    let rom = std::fs::read(path).expect("the smoke ROM is built by verify.sh");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("the smoke ROM loads");
    let mut tracer = Tracer::new(Head(Vec::new()), TraceCpus::Arm9);
    machine.run_frame_traced(Input::default(), &mut tracer);
    tracer.into_sink().0
}

/// `records` as trace text.
fn text_of(records: &[TraceRecord]) -> String {
    let lines: Vec<String> = records.iter().map(ToString::to_string).collect();
    format!("{HEADER}\n{}\n", lines.join("\n"))
}

/// `records` with register 3 changed in the record at `index` only.
fn edited(records: &[TraceRecord], index: usize) -> Vec<TraceRecord> {
    let mut records = records.to_vec();
    records[index].regs[EDITED] = records[index].regs[EDITED].wrapping_add(1);
    records
}

/// A file under the system temp directory with a unique name, removed when dropped.
struct TempFile(PathBuf);

impl TempFile {
    fn with(contents: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "ninge-tracediff-test-{}-{}.trace",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, contents).expect("the temp file is written");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        // Best effort: a file that is already gone is not a failure of the test.
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run(args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ninge-tracediff"))
        .args(args)
        .output()
        .expect("the binary starts")
}

fn run_files(ninge: &Path, oracle: &Path) -> Output {
    run(&["files".as_ref(), ninge.as_os_str(), oracle.as_os_str()])
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn given_two_traces_of_the_same_run_when_compared_then_the_exit_code_is_0() {
    let text = text_of(&smoke_records());
    let (a, b) = (TempFile::with(&text), TempFile::with(&text));
    let output = run_files(a.path(), b.path());
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn given_two_traces_of_the_same_run_when_compared_then_stdout_counts_the_arm9_records() {
    let records = smoke_records();
    let text = text_of(&records);
    let (a, b) = (TempFile::with(&text), TempFile::with(&text));
    let output = run_files(a.path(), b.path());
    assert_eq!(
        stdout(&output),
        format!(
            "ARM9: {} instructions match\nARM7: no ninge records\n",
            records.len()
        )
    );
}

#[test]
fn given_one_register_edited_in_the_second_trace_when_compared_then_the_exit_code_is_1() {
    let records = smoke_records();
    let a = TempFile::with(&text_of(&records));
    let b = TempFile::with(&text_of(&edited(&records, 57)));
    let output = run_files(a.path(), b.path());
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn given_one_register_edited_at_instruction_57_when_compared_then_stdout_names_instruction_57() {
    let records = smoke_records();
    let a = TempFile::with(&text_of(&records));
    let b = TempFile::with(&text_of(&edited(&records, 57)));
    let output = run_files(a.path(), b.path());
    assert!(
        stdout(&output).contains("first divergence at instruction 57 (ARM9): mismatch"),
        "stdout was {:?}",
        stdout(&output)
    );
}

#[test]
fn given_one_register_edited_when_compared_then_stdout_names_the_register_and_shows_both_lines() {
    let records = smoke_records();
    let changed = edited(&records, 0);
    let a = TempFile::with(&text_of(&records));
    let b = TempFile::with(&text_of(&changed));
    let text = stdout(&run_files(a.path(), b.path()));
    assert_eq!(
        text,
        format!(
            "first divergence at instruction 0 (ARM9): mismatch\n  differs: r3\n  ninge : {}\n  oracle: {}\nARM7: no ninge records\n",
            records[0], changed[0]
        )
    );
}

#[test]
fn given_the_cpu_filter_when_compared_then_only_that_cpu_is_in_the_output() {
    let records = smoke_records();
    let text = text_of(&records);
    let (a, b) = (TempFile::with(&text), TempFile::with(&text));
    let output = run(&[
        "files".as_ref(),
        a.path().as_os_str(),
        b.path().as_os_str(),
        "--cpu".as_ref(),
        "9".as_ref(),
    ]);
    assert_eq!(
        stdout(&output),
        format!("ARM9: {} instructions match\n", records.len())
    );
}

#[test]
fn given_no_arguments_when_run_then_the_exit_code_is_2() {
    assert_eq!(run(&[]).status.code(), Some(2));
}

#[test]
fn given_no_arguments_when_run_then_stderr_carries_the_user_rom_warning() {
    let output = run(&[]);
    assert!(
        stderr(&output).contains("not yours to share"),
        "stderr was {:?}",
        stderr(&output)
    );
}

#[test]
fn given_no_arguments_when_run_then_stdout_is_empty() {
    assert_eq!(stdout(&run(&[])), "");
}

#[test]
fn given_an_unknown_command_when_run_then_the_exit_code_is_2() {
    let output = run(&["compare".as_ref(), "a".as_ref(), "b".as_ref()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn given_a_missing_file_when_compared_then_the_exit_code_is_3_and_stderr_names_the_file() {
    let present = TempFile::with(&text_of(&smoke_records()));
    let missing = std::env::temp_dir().join("ninge-tracediff-test-does-not-exist.trace");
    let output = run_files(present.path(), &missing);
    assert_eq!(
        (
            output.status.code(),
            stderr(&output).contains("does-not-exist.trace")
        ),
        (Some(3), true)
    );
}

#[test]
fn given_a_file_that_is_not_a_trace_when_compared_then_the_exit_code_is_3() {
    let good = TempFile::with(&text_of(&smoke_records()));
    let bad = TempFile::with("this is not a trace\n");
    let output = run_files(bad.path(), good.path());
    assert_eq!(
        (
            output.status.code(),
            stderr(&output).contains("the ninge trace")
        ),
        (Some(3), true)
    );
}

#[test]
fn given_a_live_command_when_nothing_listens_on_the_port_then_the_exit_code_is_3() {
    // Bind to learn a free loopback port, then close it so nothing answers there.
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("a loopback port is free")
        .local_addr()
        .expect("the listener has an address")
        .port()
        .to_string();
    let trace = TempFile::with(&text_of(&smoke_records()));
    let output = run(&[
        "live".as_ref(),
        trace.path().as_os_str(),
        "--port".as_ref(),
        port.as_ref(),
        "--cpu".as_ref(),
        "9".as_ref(),
    ]);
    assert_eq!(
        (
            output.status.code(),
            stderr(&output).contains("cannot connect to 127.0.0.1")
        ),
        (Some(3), true)
    );
}
