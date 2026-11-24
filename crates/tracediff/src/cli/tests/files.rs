// SPDX-License-Identifier: Apache-2.0
//! `run_files`: two trace texts compared per CPU (`trace-oracle.md`: the CPUs are compared
//! separately, since ARM9/ARM7 interleaving differs between emulators by design).

use ninge_core::trace::{TraceCpu, TraceRecord};

use super::parts;
use crate::cli::{CliError, run_files};
use crate::diff::tests::{arm, on_arm7, run_of, trace_text, with_reg};

fn nine(count: u32) -> Vec<TraceRecord> {
    run_of(0, count)
}

fn seven(count: u32) -> Vec<TraceRecord> {
    run_of(0, count).into_iter().map(on_arm7).collect()
}

/// ARM9 records and ARM7 records, ARM9's first.
fn both(nine_count: u32, seven_count: u32) -> Vec<TraceRecord> {
    [nine(nine_count), seven(seven_count)].concat()
}

/// ARM7 records first, then ARM9's: the other interleaving of the same two streams.
fn both_reversed(nine_count: u32, seven_count: u32) -> Vec<TraceRecord> {
    [seven(seven_count), nine(nine_count)].concat()
}

#[test]
fn given_identical_traces_when_compared_then_each_cpu_reports_its_match_count() {
    let text = trace_text(&both(3, 2));
    let report = run_files(&text, &text, None).unwrap();
    assert_eq!(
        parts(report),
        (
            "ARM9: 3 instructions match\nARM7: 2 instructions match".to_string(),
            false
        )
    );
}

#[test]
fn given_the_same_streams_interleaved_differently_when_compared_then_they_match() {
    let ninge = trace_text(&both(3, 2));
    let oracle = trace_text(&both_reversed(3, 2));
    let report = run_files(&ninge, &oracle, None).unwrap();
    assert!(!parts(report).1);
}

#[test]
fn given_a_difference_on_the_arm7_when_compared_then_it_names_arm7_and_the_arm9_still_matches() {
    let ninge = trace_text(&both(3, 4));
    let mut changed = both(3, 4);
    changed[3 + 2] = with_reg(changed[3 + 2], 3, 7);
    let report = run_files(&ninge, &trace_text(&changed), None).unwrap();
    let (text, diverged) = parts(report);
    assert_eq!(
        (
            diverged,
            text.starts_with(
                "ARM9: 3 instructions match\nfirst divergence at instruction 2 (ARM7): mismatch\n"
            )
        ),
        (true, true)
    );
}

#[test]
fn given_differences_on_both_cpus_when_compared_then_both_are_reported() {
    let ninge = trace_text(&both(3, 3));
    let mut changed = both(3, 3);
    changed[1] = with_reg(changed[1], 0, 1);
    changed[4] = with_reg(changed[4], 0, 1);
    let (text, diverged) = parts(run_files(&ninge, &trace_text(&changed), None).unwrap());
    assert_eq!(
        (
            diverged,
            text.matches("first divergence at instruction 1 (ARM9)")
                .count(),
            text.matches("first divergence at instruction 1 (ARM7)")
                .count(),
        ),
        (true, 1, 1)
    );
}

#[test]
fn given_the_cpu_filter_when_only_the_other_cpu_differs_then_the_report_is_clean() {
    let ninge = trace_text(&both(3, 3));
    let mut changed = both(3, 3);
    changed[4] = with_reg(changed[4], 0, 1);
    let report = run_files(&ninge, &trace_text(&changed), Some(TraceCpu::Arm9)).unwrap();
    assert_eq!(
        parts(report),
        ("ARM9: 3 instructions match".to_string(), false)
    );
}

#[test]
fn given_the_cpu_filter_when_that_cpu_differs_then_only_it_is_reported() {
    let ninge = trace_text(&both(3, 3));
    let mut changed = both(3, 3);
    changed[4] = with_reg(changed[4], 0, 1);
    let (text, diverged) =
        parts(run_files(&ninge, &trace_text(&changed), Some(TraceCpu::Arm7)).unwrap());
    assert_eq!(
        (diverged, text.contains("ARM9"), text.contains("(ARM7)")),
        (true, false, true)
    );
}

#[test]
fn given_a_cpu_with_records_only_on_the_oracle_side_when_compared_then_it_is_a_divergence() {
    let ninge = trace_text(&nine(2));
    let oracle = trace_text(&both(2, 5));
    let report = run_files(&ninge, &oracle, None).unwrap();
    assert_eq!(
        parts(report),
        (
            "ARM9: 2 instructions match\nARM7: no ninge records, but 5 oracle records".to_string(),
            true
        )
    );
}

#[test]
fn given_a_filter_for_a_cpu_with_no_ninge_records_when_compared_then_it_is_noted() {
    let text = trace_text(&nine(2));
    let report = run_files(&text, &text, Some(TraceCpu::Arm7)).unwrap();
    assert_eq!(parts(report), ("ARM7: no ninge records".to_string(), false));
}

#[test]
fn given_an_oracle_trace_that_ends_early_when_compared_then_the_report_says_how_far_it_got() {
    let ninge = trace_text(&nine(5));
    let oracle = trace_text(&nine(2));
    let report = run_files(&ninge, &oracle, None).unwrap();
    assert_eq!(
        parts(report),
        (
            "ARM9: the oracle ended after 2 matching instructions\nARM7: no ninge records"
                .to_string(),
            true
        )
    );
}

#[test]
fn given_an_oracle_trace_with_no_records_for_a_cpu_when_compared_then_the_oracle_ended_at_0() {
    let ninge = trace_text(&both(1, 2));
    let oracle = trace_text(&nine(1));
    let (text, diverged) = parts(run_files(&ninge, &oracle, None).unwrap());
    assert_eq!(
        (
            diverged,
            text.ends_with("ARM7: the oracle ended after 0 matching instructions")
        ),
        (true, true)
    );
}

#[test]
fn given_an_oracle_trace_that_is_longer_when_compared_then_ninges_length_decides() {
    let report = run_files(
        &trace_text(&nine(2)),
        &trace_text(&nine(6)),
        Some(TraceCpu::Arm9),
    );
    assert_eq!(
        parts(report.unwrap()),
        ("ARM9: 2 instructions match".to_string(), false)
    );
}

#[test]
fn given_comments_and_blank_lines_in_the_traces_when_compared_then_they_are_skipped() {
    let plain = trace_text(&nine(2));
    let noisy = plain.replacen('\n', "\n# a comment\n\n", 1);
    let report = run_files(&noisy, &plain, Some(TraceCpu::Arm9)).unwrap();
    assert!(!parts(report).1);
}

#[test]
fn given_a_bad_ninge_trace_when_compared_then_the_error_names_the_ninge_side() {
    let error = run_files("not a trace\n", &trace_text(&nine(1)), None).unwrap_err();
    assert!(matches!(error, CliError::Trace { which: "ninge", .. }));
}

#[test]
fn given_a_bad_oracle_trace_when_compared_then_the_error_names_the_oracle_side() {
    let broken = format!("{}9 not-a-line\n", trace_text(&nine(1)));
    let error = run_files(&trace_text(&nine(1)), &broken, None).unwrap_err();
    assert!(matches!(
        error,
        CliError::Trace {
            which: "oracle",
            ..
        }
    ));
}

#[test]
fn given_a_bad_trace_when_the_error_is_displayed_then_it_names_the_side_and_the_line() {
    let broken = format!("{}9 not-a-line\n", trace_text(&nine(1)));
    let error = run_files(&trace_text(&nine(1)), &broken, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the oracle trace: line 3: expected 21 columns"
    );
}

#[test]
fn given_a_trace_with_a_wrong_header_when_compared_then_it_is_refused() {
    let error = run_files("ninge-trace 2\n", "ninge-trace 2\n", None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the ninge trace: the first line is not `ninge-trace 1`"
    );
}

#[test]
fn given_a_trace_with_a_pc_that_is_not_r15_when_parsed_then_the_trace_error_is_returned() {
    let mut record = arm(0, 0x0200_0000, 0);
    record.regs[15] = 0x0200_0004;
    let broken = trace_text(&[record]).replacen("02000004", "02000000", 1);
    let error = run_files(&broken, &broken, None).unwrap_err();
    assert!(matches!(error, CliError::Trace { which: "ninge", .. }));
}

#[test]
fn given_empty_traces_with_the_header_when_compared_then_neither_cpu_has_records() {
    let header = "ninge-trace 1\n";
    let report = run_files(header, header, None).unwrap();
    assert_eq!(
        parts(report),
        (
            "ARM9: no ninge records\nARM7: no ninge records".to_string(),
            false
        )
    );
}

#[test]
fn given_a_cpu_with_no_records_on_either_side_when_compared_then_it_is_not_a_divergence() {
    let text = trace_text(&nine(2));
    let report = run_files(&text, &text, None).unwrap();
    assert_eq!(
        parts(report),
        (
            "ARM9: 2 instructions match\nARM7: no ninge records".to_string(),
            false
        )
    );
}
