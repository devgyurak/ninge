// SPDX-License-Identifier: Apache-2.0
//! Tests of `ninge-runner trace` (`docs/contributing/trace-oracle.md`, "The trace format").

use super::*;
use crate::{Command, parse_command, testrom_path};
use ninge_core::trace::{Opcode, parse_trace};

// ---- harness ---------------------------------------------------------------------------------

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn parse(args: &[&str]) -> Result<TraceArgs, RunnerError> {
    parse_trace_args(os(args))
}

/// The names of the cases that did not come back as a usage error that carries the usage text.
fn non_usage_failures(cases: &[(&str, &[&str])]) -> Vec<String> {
    cases
        .iter()
        .filter(|(_, args)| {
            !matches!(parse(args), Err(RunnerError::Usage(message)) if message.contains(TRACE_USAGE))
        })
        .map(|(name, _)| (*name).to_string())
        .collect()
}

const EXPECTED_PATH: &str = "never commit or paste it";

fn smoke_rom() -> Vec<u8> {
    load_rom_file(&testrom_path("smoke")).expect("the smoke test ROM is built")
}

fn args_for(frames: &str, from_frame: &str, cpu: &str) -> TraceArgs {
    parse(&[
        "smoke.nds",
        "--frames",
        frames,
        "--from-frame",
        from_frame,
        "--cpu",
        cpu,
        "--out",
        "unused.trace",
    ])
    .expect("valid args")
}

/// Runs the smoke ROM into memory; returns the text and the summary.
fn trace_text(frames: &str, from_frame: &str, cpu: &str) -> (String, TraceSummary) {
    let (bytes, summary) =
        run_trace(smoke_rom(), &args_for(frames, from_frame, cpu), Vec::new()).expect("runs");
    (String::from_utf8(bytes).expect("a trace is UTF-8"), summary)
}

fn records(text: &str) -> Vec<TraceRecord> {
    parse_trace(text).expect("the trace parses")
}

/// A record of `cpu` at `step`, ARM state, SVC mode, every register 0 but r15 = `pc`.
fn record(cpu: TraceCpu, step: u32, pc: u32) -> TraceRecord {
    let mut regs = [0; 16];
    regs[15] = pc;
    TraceRecord {
        cpu,
        step,
        opcode: Opcode::Arm(0xE1A0_0000),
        regs,
        cpsr: 0x0000_00D3,
    }
}

/// The line of `record` written out by hand from the format of `trace-oracle.md`.
fn line(cpu: char, step: u32, pc: u32) -> String {
    let zeros = " 00000000".repeat(15);
    format!("{cpu} {step:08x} {pc:08x} e1a00000{zeros} {pc:08x} 000000d3\n")
}

/// A writer that accepts `limit` bytes and then fails every write, counting the failures.
#[derive(Debug)]
struct FailingWriter {
    limit: usize,
    accepted: usize,
    failed_writes: u32,
}

impl FailingWriter {
    fn accepting(limit: usize) -> Self {
        Self {
            limit,
            accepted: 0,
            failed_writes: 0,
        }
    }
}

impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.accepted + buf.len() > self.limit {
            self.failed_writes += 1;
            return Err(io::Error::other("disk full"));
        }
        self.accepted += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A writer that accepts everything and fails to flush.
struct FlushFails;

impl Write for FlushFails {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("flush refused"))
    }
}

/// Per CPU, the steps that are not the consecutive count from 0: `(cpu, expected, found)`.
fn step_gaps(records: &[TraceRecord]) -> Vec<(TraceCpu, u32, u32)> {
    let mut next = [0u32; 2];
    let mut gaps = Vec::new();
    records.iter().for_each(|r| {
        let slot = r.cpu as usize;
        if r.step != next[slot] {
            gaps.push((r.cpu, next[slot], r.step));
        }
        next[slot] = r.step.wrapping_add(1);
    });
    gaps
}

fn count_of(records: &[TraceRecord], cpu: TraceCpu) -> u32 {
    u32::try_from(records.iter().filter(|r| r.cpu == cpu).count()).expect("fits")
}

/// How many records of `part` differ from the last records of `full`, ignoring `step`.
fn suffix_mismatches(full: &[TraceRecord], part: &[TraceRecord]) -> usize {
    let tail = &full[full.len() - part.len()..];
    tail.iter()
        .zip(part)
        .filter(|(a, b)| TraceRecord { step: 0, ..**a } != TraceRecord { step: 0, ..**b })
        .count()
}

fn sink_output(records: &[TraceRecord]) -> (String, u64) {
    let mut sink = WriterSink::new(Vec::new()).expect("a Vec accepts the header");
    records.iter().for_each(|r| sink.record(r));
    let (bytes, lines) = sink.finish().expect("a Vec accepts every line");
    (String::from_utf8(bytes).expect("UTF-8"), lines)
}

fn record_many<W: Write>(sink: &mut WriterSink<W>, count: u32) {
    (0..count).for_each(|step| sink.record(&record(TraceCpu::Arm9, step, 0x0200_0000)));
}

// ---- argument parsing ------------------------------------------------------------------------

#[test]
fn given_a_full_trace_line_when_parsing_then_every_field_is_taken() {
    let parsed = parse(&[
        "game.nds",
        "--frames",
        "10",
        "--from-frame",
        "4",
        "--cpu",
        "arm7",
        "--out",
        "t.trace",
    ])
    .expect("valid");
    assert_eq!(
        (
            parsed.rom().to_path_buf(),
            parsed.frames(),
            parsed.from_frame(),
            parsed.cpus(),
            parsed.out().to_path_buf()
        ),
        (
            PathBuf::from("game.nds"),
            10,
            4,
            TraceCpus::Arm7,
            PathBuf::from("t.trace")
        )
    );
}

#[test]
fn given_flags_in_another_order_when_parsing_then_the_result_is_the_same() {
    let a = parse(&["g.nds", "--frames", "3", "--out", "o", "--cpu", "arm9"]).expect("valid");
    let b = parse(&["g.nds", "--cpu", "arm9", "--out", "o", "--frames", "3"]).expect("valid");
    assert_eq!(a, b);
}

#[test]
fn given_only_frames_and_out_when_parsing_then_from_frame_is_0_and_cpu_is_both() {
    let parsed = parse(&["g.nds", "--frames", "2", "--out", "o"]).expect("valid");
    assert_eq!((parsed.from_frame(), parsed.cpus()), (0, TraceCpus::Both));
}

#[test]
fn given_each_cpu_word_when_parsing_then_it_selects_that_cpu_set() {
    let pick = |word: &str| {
        parse(&["g", "--frames", "2", "--cpu", word, "--out", "o"])
            .expect("valid")
            .cpus()
    };
    assert_eq!(
        [pick("arm9"), pick("arm7"), pick("both")],
        [TraceCpus::Arm9, TraceCpus::Arm7, TraceCpus::Both]
    );
}

#[test]
fn given_a_from_frame_one_below_frames_when_parsing_then_it_is_accepted() {
    let parsed = parse(&["g", "--frames", "5", "--from-frame", "4", "--out", "o"]).expect("valid");
    assert_eq!(parsed.from_frame(), 4);
}

#[test]
fn given_a_malformed_trace_line_when_parsing_then_each_case_is_a_usage_error_with_the_usage_text() {
    let cases: &[(&str, &[&str])] = &[
        ("no arguments", &[]),
        ("rom is a flag", &["--frames", "3", "--out", "o"]),
        (
            "rom starts with dash",
            &["-rom.nds", "--frames", "3", "--out", "o"],
        ),
        ("frames missing", &["g", "--out", "o"]),
        ("out missing", &["g", "--frames", "3"]),
        (
            "frames not a number",
            &["g", "--frames", "ten", "--out", "o"],
        ),
        ("frames negative", &["g", "--frames", "-1", "--out", "o"]),
        ("frames empty", &["g", "--frames", "", "--out", "o"]),
        (
            "from-frame not a number",
            &["g", "--frames", "3", "--from-frame", "x", "--out", "o"],
        ),
        (
            "from-frame equals frames",
            &["g", "--frames", "3", "--from-frame", "3", "--out", "o"],
        ),
        (
            "from-frame above frames",
            &["g", "--frames", "3", "--from-frame", "9", "--out", "o"],
        ),
        ("frames zero", &["g", "--frames", "0", "--out", "o"]),
        (
            "cpu unknown word",
            &["g", "--frames", "3", "--cpu", "arm", "--out", "o"],
        ),
        (
            "cpu wrong case",
            &["g", "--frames", "3", "--cpu", "ARM9", "--out", "o"],
        ),
        (
            "frames repeated",
            &["g", "--frames", "3", "--frames", "4", "--out", "o"],
        ),
        (
            "out repeated",
            &["g", "--frames", "3", "--out", "o", "--out", "p"],
        ),
        (
            "cpu repeated",
            &[
                "g", "--frames", "3", "--cpu", "arm9", "--cpu", "arm9", "--out", "o",
            ],
        ),
        (
            "from-frame repeated",
            &[
                "g",
                "--frames",
                "3",
                "--from-frame",
                "0",
                "--from-frame",
                "1",
                "--out",
                "o",
            ],
        ),
        (
            "unknown flag",
            &["g", "--frames", "3", "--bogus", "1", "--out", "o"],
        ),
        (
            "extra positional",
            &["g", "--frames", "3", "--out", "o", "extra"],
        ),
        ("frames without value", &["g", "--frames"]),
        ("out without value", &["g", "--frames", "3", "--out"]),
        (
            "cpu without value",
            &["g", "--frames", "3", "--out", "o", "--cpu"],
        ),
        (
            "from-frame without value",
            &["g", "--frames", "3", "--out", "o", "--from-frame"],
        ),
    ];
    assert_eq!(non_usage_failures(cases), Vec::<String>::new());
}

#[test]
fn given_a_usage_error_when_formatted_then_the_message_forbids_committing_or_pasting_a_trace() {
    let error = parse(&[]).expect_err("no ROM");
    assert!(error.to_string().contains(EXPECTED_PATH));
}

#[cfg(unix)]
#[test]
fn given_a_non_utf8_flag_when_parsing_then_it_is_a_usage_error() {
    use std::os::unix::ffi::OsStringExt;
    let args = vec![
        OsString::from("g"),
        OsString::from_vec(vec![b'-', b'-', 0xFF]),
        OsString::from("1"),
    ];
    assert!(matches!(parse_trace_args(args), Err(RunnerError::Usage(_))));
}

#[test]
fn given_trace_as_the_first_word_when_routing_a_command_then_it_is_a_trace_command() {
    let command = parse_command(os(&["trace", "g.nds", "--frames", "2", "--out", "o"]));
    assert!(matches!(command, Ok(Command::Trace(_))));
}

#[test]
fn given_trace_with_no_arguments_when_routing_a_command_then_it_is_a_usage_error() {
    let command = parse_command(os(&["trace"]));
    assert!(matches!(command, Err(RunnerError::Usage(_))));
}

#[test]
fn given_roms_named_like_the_subcommand_when_routing_then_only_the_exact_word_routes_to_trace() {
    let routes = |name: &str| matches!(parse_command(os(&[name])), Ok(Command::Run(_)));
    assert_eq!(
        [routes("trace.nds"), routes("./trace"), routes("traces")],
        [true, true, true]
    );
}

#[test]
fn given_a_plain_rom_run_when_routing_a_command_then_it_is_still_a_run_command() {
    let command = parse_command(os(&["game.nds", "--frames", "2"]));
    assert!(matches!(command, Ok(Command::Run(_))));
}

#[test]
fn given_cpu_suite_when_routing_a_command_then_it_is_still_a_cpu_suite_command() {
    let command = parse_command(os(&["cpu-suite", "dir"]));
    assert!(matches!(command, Ok(Command::CpuSuite(_))));
}

// ---- WriterSink ------------------------------------------------------------------------------

#[test]
fn given_no_records_when_a_sink_finishes_then_the_output_is_only_the_header_line() {
    assert_eq!(sink_output(&[]), ("ninge-trace 1\n".to_string(), 0));
}

#[test]
fn given_records_when_a_sink_finishes_then_the_header_comes_first_then_one_line_each_in_order() {
    let given = [
        record(TraceCpu::Arm9, 0, 0x0200_0000),
        record(TraceCpu::Arm7, 0, 0x0380_0000),
        record(TraceCpu::Arm9, 1, 0x0200_0004),
    ];
    let expected = format!(
        "ninge-trace 1\n{}{}{}",
        line('9', 0, 0x0200_0000),
        line('7', 0, 0x0380_0000),
        line('9', 1, 0x0200_0004)
    );
    assert_eq!(sink_output(&given), (expected, 3));
}

#[test]
fn given_a_writer_that_fails_on_the_first_record_when_finishing_then_the_error_is_returned() {
    let mut writer = FailingWriter::accepting(HEADER.len() + 1);
    let mut sink = WriterSink::new(&mut writer).expect("the header fits");
    sink.record(&record(TraceCpu::Arm9, 0, 0x0200_0000));
    let error = sink.finish().expect_err("the write failed");
    assert_eq!(error.to_string(), "disk full");
}

#[test]
fn given_a_writer_that_failed_when_more_records_arrive_then_none_of_them_is_written() {
    let mut writer = FailingWriter::accepting(HEADER.len() + 1);
    let mut sink = WriterSink::new(&mut writer).expect("the header fits");
    record_many(&mut sink, 5);
    let _ = sink.finish();
    assert_eq!(
        (writer.failed_writes, writer.accepted),
        (1, HEADER.len() + 1)
    );
}

#[test]
fn given_a_writer_that_fails_on_the_second_record_when_finishing_then_the_first_stays_written() {
    let one_line = line('9', 0, 0x0200_0000).len();
    let mut writer = FailingWriter::accepting(HEADER.len() + 1 + one_line);
    let mut sink = WriterSink::new(&mut writer).expect("the header fits");
    sink.record(&record(TraceCpu::Arm9, 0, 0x0200_0000));
    sink.record(&record(TraceCpu::Arm9, 1, 0x0200_0004));
    let finished = sink.finish();
    assert_eq!(
        (finished.is_err(), writer.accepted),
        (true, HEADER.len() + 1 + one_line)
    );
}

#[test]
fn given_a_writer_that_rejects_the_header_when_creating_a_sink_then_new_returns_the_error() {
    let mut writer = FailingWriter::accepting(0);
    let created = WriterSink::new(&mut writer);
    assert_eq!(
        created.err().map(|e| e.to_string()),
        Some("disk full".to_string())
    );
}

#[test]
fn given_a_writer_whose_flush_fails_when_finishing_then_the_flush_error_is_returned() {
    let sink = WriterSink::new(FlushFails).expect("writes are accepted");
    let error = sink.finish().err().expect("flush fails");
    assert_eq!(error.to_string(), "flush refused");
}

// ---- run_trace -------------------------------------------------------------------------------

#[test]
fn given_the_smoke_rom_when_traced_then_the_output_starts_with_the_header_line() {
    let (text, _) = trace_text("2", "0", "both");
    assert!(text.starts_with("ninge-trace 1\n"));
}

#[test]
fn given_the_smoke_rom_when_traced_then_the_whole_output_parses_as_a_trace() {
    let (text, _) = trace_text("2", "0", "both");
    assert_eq!(parse_trace(&text).err(), None);
}

#[test]
fn given_the_smoke_rom_when_traced_then_each_cpus_steps_count_up_from_0_without_gaps() {
    let (text, _) = trace_text("2", "0", "both");
    assert_eq!(step_gaps(&records(&text)), Vec::new());
}

#[test]
fn given_the_smoke_rom_when_traced_then_both_cpus_appear() {
    let (text, _) = trace_text("2", "0", "both");
    let all = records(&text);
    assert_eq!(
        (
            count_of(&all, TraceCpu::Arm9) > 0,
            count_of(&all, TraceCpu::Arm7) > 0
        ),
        (true, true)
    );
}

#[test]
fn given_the_smoke_rom_when_traced_then_the_summary_line_count_equals_the_parsed_record_count() {
    let (text, summary) = trace_text("2", "0", "both");
    assert_eq!(
        u64::try_from(records(&text).len()).expect("fits"),
        summary.lines
    );
}

#[test]
fn given_the_smoke_rom_when_traced_then_the_summary_per_cpu_counts_equal_the_parsed_ones() {
    let (text, summary) = trace_text("2", "0", "both");
    let all = records(&text);
    assert_eq!(
        summary.steps,
        [
            count_of(&all, TraceCpu::Arm9),
            count_of(&all, TraceCpu::Arm7)
        ]
    );
}

#[test]
fn given_the_smoke_rom_when_traced_then_the_summary_frames_is_the_frames_asked_for() {
    let (_, summary) = trace_text("3", "0", "arm9");
    assert_eq!(summary.frames, 3);
}

#[test]
fn given_cpu_arm9_when_traced_then_every_record_is_cpu_9_and_there_are_some() {
    let (text, summary) = trace_text("2", "0", "arm9");
    let all = records(&text);
    assert_eq!(
        (
            count_of(&all, TraceCpu::Arm7),
            count_of(&all, TraceCpu::Arm9) > 0,
            summary.steps[1]
        ),
        (0, true, 0)
    );
}

#[test]
fn given_cpu_arm7_when_traced_then_every_record_is_cpu_7_and_there_are_some() {
    let (text, summary) = trace_text("2", "0", "arm7");
    let all = records(&text);
    assert_eq!(
        (
            count_of(&all, TraceCpu::Arm9),
            count_of(&all, TraceCpu::Arm7) > 0,
            summary.steps[0]
        ),
        (0, true, 0)
    );
}

#[test]
fn given_the_smoke_rom_when_traced_twice_then_the_two_outputs_are_byte_identical() {
    let first = trace_text("2", "0", "both").0;
    let second = trace_text("2", "0", "both").0;
    assert!(first == second, "two runs of the same ROM differ");
}

#[test]
fn given_a_from_frame_when_traced_then_steps_restart_at_0_and_the_machine_still_runs_all_frames() {
    let (text, summary) = trace_text("3", "2", "both");
    assert_eq!(
        (step_gaps(&records(&text)), summary.frames),
        (Vec::new(), 3)
    );
}

#[test]
fn given_a_from_frame_when_traced_then_the_trace_is_shorter_than_the_full_trace() {
    let full = trace_text("3", "0", "both").1.lines;
    let late = trace_text("3", "2", "both").1.lines;
    assert!(late < full, "late={late} full={full}");
}

#[test]
fn given_a_from_frame_when_traced_then_the_records_equal_the_tail_of_the_full_trace_but_for_steps()
{
    let full = records(&trace_text("3", "0", "both").0);
    let late = records(&trace_text("3", "2", "both").0);
    assert_eq!(suffix_mismatches(&full, &late), 0);
}

#[test]
fn given_a_from_frame_when_traced_then_the_first_record_of_each_cpu_has_step_0() {
    let late = records(&trace_text("3", "1", "both").0);
    let first_steps: Vec<u32> = [TraceCpu::Arm9, TraceCpu::Arm7]
        .iter()
        .filter_map(|cpu| late.iter().find(|r| r.cpu == *cpu).map(|r| r.step))
        .collect();
    assert_eq!(first_steps, vec![0, 0]);
}

#[test]
fn given_garbage_bytes_when_traced_then_the_rom_is_refused() {
    let result = run_trace(vec![1, 2, 3, 4, 5], &args_for("2", "0", "both"), Vec::new());
    assert!(matches!(result, Err(RunnerError::Rom(_))));
}

#[test]
fn given_a_failing_writer_when_traced_then_the_error_is_a_write_error_naming_the_out_path() {
    let result = run_trace(
        smoke_rom(),
        &args_for("1", "0", "arm9"),
        FailingWriter::accepting(HEADER.len() + 1),
    );
    let message = result.err().map(|e| e.to_string());
    assert_eq!(
        message.map(|m| m.starts_with("cannot write unused.trace: ")),
        Some(true)
    );
}

#[test]
fn given_a_writer_that_rejects_the_header_when_traced_then_the_error_is_a_write_error() {
    let result = run_trace(
        smoke_rom(),
        &args_for("1", "0", "arm9"),
        FailingWriter::accepting(0),
    );
    assert!(matches!(result, Err(RunnerError::Write { .. })));
}

#[test]
fn given_a_missing_rom_file_when_writing_a_trace_file_then_the_error_is_a_read_error() {
    let args = parse(&[
        "/nonexistent/ninge/none.nds",
        "--frames",
        "1",
        "--out",
        "/nonexistent/ninge/out.trace",
    ])
    .expect("valid");
    assert!(matches!(
        write_trace_file(&args),
        Err(RunnerError::Io { .. })
    ));
}

#[test]
fn given_an_unwritable_out_path_when_writing_a_trace_file_then_the_write_error_names_it() {
    let rom = testrom_path("smoke");
    let out = "/nonexistent/ninge/dir/out.trace";
    let args =
        parse(&[rom.to_str().expect("utf-8"), "--frames", "1", "--out", out]).expect("valid");
    let message = write_trace_file(&args).err().map(|e| e.to_string());
    assert_eq!(
        message.map(|m| m.starts_with(&format!("cannot write {out}: "))),
        Some(true)
    );
}
