// SPDX-License-Identifier: Apache-2.0
//! `run_live`: ninge's trace against a live oracle, instruction by instruction, from the start or
//! from the K-th hit of a breakpoint (`trace-oracle.md`, "Narrowing", level 3: from the last
//! agreeing boundary both sides step until the first divergence). The oracle is the fake stub
//! scripted with the oracle's records.

use ninge_core::trace::{TraceCpu, TraceRecord};

use super::{live_args, parts};
use crate::cli::{CliError, LiveArgs, run_live};
use crate::diff::tests::{arm, on_arm7, run_of, trace_text, with_reg};
use crate::oracle::tests::fake::{Fake, Mode, SharedLog, requests, script_of};

const A: u32 = 0x0200_0000;
const B: u32 = 0x0200_0004;
const C: u32 = 0x0200_0008;
const D: u32 = 0x0200_000C;
const E: u32 = 0x0200_0010;

/// Records at `pcs`, numbered from step 0.
fn seq(pcs: &[u32]) -> Vec<TraceRecord> {
    pcs.iter()
        .enumerate()
        .map(|(step, pc)| arm(step as u32, *pc, 0xE1A0_0000))
        .collect()
}

fn live(
    ninge: &[TraceRecord],
    oracle: &[TraceRecord],
    args: &LiveArgs,
) -> (Result<crate::cli::Report, CliError>, SharedLog) {
    let (fake, log) = Fake::new(script_of(oracle));
    (run_live(fake, &trace_text(ninge), args), log)
}

fn report(ninge: &[TraceRecord], oracle: &[TraceRecord], args: &LiveArgs) -> (String, bool) {
    parts(live(ninge, oracle, args).0.expect("the run finishes"))
}

fn with_breakpoint(pc: u32, from_hit: u32) -> LiveArgs {
    LiveArgs {
        breakpoint: Some(pc),
        from_hit,
        ..live_args(TraceCpu::Arm9)
    }
}

// ---- from the start ----

#[test]
fn given_matching_streams_when_run_live_then_the_report_counts_them() {
    let records = run_of(0, 5);
    assert_eq!(
        report(&records, &records, &live_args(TraceCpu::Arm9)),
        ("ARM9: 5 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_register_that_differs_at_instruction_3_when_run_live_then_the_report_names_it() {
    let ninge = run_of(0, 6);
    let mut oracle = run_of(0, 6);
    oracle[3] = with_reg(oracle[3], 1, 5);
    let (text, diverged) = report(&ninge, &oracle, &live_args(TraceCpu::Arm9));
    assert_eq!(
        (diverged, text.lines().next().map(str::to_string)),
        (
            true,
            Some("first divergence at instruction 3 (ARM9): mismatch".to_string())
        )
    );
}

#[test]
fn given_an_oracle_that_ends_early_when_run_live_then_the_report_says_how_far_it_got() {
    let ninge = run_of(0, 5);
    let oracle = run_of(0, 3);
    assert_eq!(
        report(&ninge, &oracle, &live_args(TraceCpu::Arm9)),
        (
            "ARM9: the oracle ended after 3 matching instructions".to_string(),
            true
        )
    );
}

#[test]
fn given_a_limit_before_the_difference_when_run_live_then_the_streams_match() {
    let ninge = run_of(0, 10);
    let mut oracle = run_of(0, 10);
    oracle[7] = with_reg(oracle[7], 1, 5);
    let args = LiveArgs {
        max: Some(5),
        ..live_args(TraceCpu::Arm9)
    };
    assert_eq!(
        report(&ninge, &oracle, &args),
        ("ARM9: 5 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_limit_past_the_difference_when_run_live_then_the_difference_is_found() {
    let ninge = run_of(0, 10);
    let mut oracle = run_of(0, 10);
    oracle[7] = with_reg(oracle[7], 1, 5);
    let args = LiveArgs {
        max: Some(8),
        ..live_args(TraceCpu::Arm9)
    };
    let (text, diverged) = report(&ninge, &oracle, &args);
    assert_eq!(
        (
            diverged,
            text.starts_with("first divergence at instruction 7 ")
        ),
        (true, true)
    );
}

#[test]
fn given_a_limit_of_exactly_the_difference_when_run_live_then_it_is_not_compared() {
    // The limit counts instructions compared: 7 means steps 0..=6, so step 7 is out of reach.
    let ninge = run_of(0, 10);
    let mut oracle = run_of(0, 10);
    oracle[7] = with_reg(oracle[7], 1, 5);
    let args = LiveArgs {
        max: Some(7),
        ..live_args(TraceCpu::Arm9)
    };
    assert_eq!(
        report(&ninge, &oracle, &args),
        ("ARM9: 7 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_limit_of_0_when_run_live_then_nothing_is_compared() {
    let ninge = run_of(0, 3);
    let args = LiveArgs {
        max: Some(0),
        ..live_args(TraceCpu::Arm9)
    };
    assert_eq!(
        report(&ninge, &[], &args),
        ("ARM9: 0 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_breakpoint_but_no_from_hit_when_run_live_then_the_oracle_is_not_aligned() {
    let records = seq(&[A, B, C]);
    let (result, log) = live(&records, &records, &with_breakpoint(B, 0));
    assert_eq!(
        (parts(result.unwrap()).0, requests(&log, 'Z')),
        ("ARM9: 3 instructions match".to_string(), Vec::new())
    );
}

#[test]
fn given_an_arm7_trace_when_run_live_for_the_arm7_then_the_report_names_arm7() {
    let records: Vec<TraceRecord> = run_of(0, 3).into_iter().map(on_arm7).collect();
    assert_eq!(
        report(&records, &records, &live_args(TraceCpu::Arm7)),
        ("ARM7: 3 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_trace_of_both_cpus_when_run_live_then_only_the_requested_cpus_records_are_compared() {
    let nine = run_of(0, 3);
    let seven: Vec<TraceRecord> = run_of(0, 4).into_iter().map(on_arm7).collect();
    let ninge = [seven.clone(), nine.clone()].concat();
    assert_eq!(
        report(&ninge, &nine, &live_args(TraceCpu::Arm9)),
        ("ARM9: 3 instructions match".to_string(), false)
    );
}

// ---- from the K-th hit ----

#[test]
fn given_a_second_hit_when_run_live_from_it_then_the_streams_are_compared_from_that_hit() {
    // B is at steps 1, 3 and 5; from hit 2 the ninge slice is steps 3..=6, four records.
    let records = seq(&[A, B, C, B, D, B, E]);
    assert_eq!(
        report(&records, &records, &with_breakpoint(B, 2)),
        ("ARM9: 4 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_first_hit_when_run_live_from_it_then_the_slice_starts_at_the_first_record_at_the_pc() {
    let records = seq(&[A, B, C, B, D, B, E]);
    assert_eq!(
        report(&records, &records, &with_breakpoint(B, 1)),
        ("ARM9: 6 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_difference_after_the_hit_when_run_live_from_it_then_ninges_step_numbers_the_report() {
    // The oracle's own count restarts at the hit; the report uses ninge's step (5, not 2).
    let ninge = seq(&[A, B, C, B, D, B, E]);
    let mut oracle = ninge.clone();
    oracle[5] = with_reg(oracle[5], 2, 9);
    let (text, diverged) = report(&ninge, &oracle, &with_breakpoint(B, 2));
    assert_eq!(
        (diverged, text.lines().next().map(str::to_string)),
        (
            true,
            Some("first divergence at instruction 5 (ARM9): mismatch".to_string())
        )
    );
}

#[test]
fn given_a_difference_before_the_hit_when_run_live_from_it_then_it_is_not_seen() {
    let ninge = seq(&[A, B, C, B, D, B, E]);
    let mut oracle = ninge.clone();
    oracle[2] = with_reg(oracle[2], 2, 9);
    assert_eq!(
        report(&ninge, &oracle, &with_breakpoint(B, 2)),
        ("ARM9: 4 instructions match".to_string(), false)
    );
}

#[test]
fn given_a_from_hit_when_run_live_then_the_breakpoint_is_set_and_removed_with_the_arm_kind() {
    let records = seq(&[A, B, C, B, D]);
    let (_, log) = live(&records, &records, &with_breakpoint(B, 2));
    assert_eq!(
        (requests(&log, 'Z'), requests(&log, 'z')),
        (
            vec!["Z0,2000004,4".to_string()],
            vec!["z0,2000004,4".to_string()]
        )
    );
}

#[test]
fn given_a_from_hit_when_run_live_then_the_oracle_runs_to_each_hit_in_turn() {
    let records = seq(&[A, B, C, B, D]);
    let (_, log) = live(&records, &records, &with_breakpoint(B, 2));
    assert_eq!(requests(&log, 'c').len(), 2);
}

#[test]
fn given_a_hit_the_trace_does_not_have_when_run_live_then_there_is_no_such_hit() {
    let records = seq(&[A, B, C, B, D]);
    let (result, log) = live(&records, &records, &with_breakpoint(B, 3));
    assert_eq!(
        (
            matches!(result, Err(CliError::NoSuchHit(3))),
            log.borrow().packets.len()
        ),
        (true, 0)
    );
}

#[test]
fn given_a_hit_the_oracle_does_not_reach_when_run_live_then_there_is_no_such_hit() {
    // The trace has three hits of B; the stub's script ends at its second.
    let ninge = seq(&[A, B, C, B, D, B]);
    let oracle = seq(&[A, B, C, B]);
    let (result, _) = live(&ninge, &oracle, &with_breakpoint(B, 3));
    assert!(matches!(result, Err(CliError::NoSuchHit(3))));
}

#[test]
fn given_a_hit_of_another_cpu_only_when_run_live_then_there_is_no_such_hit() {
    // The only record at B is the ARM7's; the ARM9 never hits it.
    let ninge = [seq(&[A, C]), vec![on_arm7(arm(0, B, 0xE1A0_0000))]].concat();
    let (result, _) = live(&ninge, &seq(&[A, C]), &with_breakpoint(B, 1));
    assert!(matches!(result, Err(CliError::NoSuchHit(1))));
}

#[test]
fn given_a_no_such_hit_error_when_displayed_then_it_names_the_hit() {
    assert_eq!(
        CliError::NoSuchHit(3).to_string(),
        "no hit 3 of the breakpoint on one of the sides"
    );
}

#[test]
fn given_a_boundary_hint_when_run_live_from_hit_minus_1_then_the_instruction_after_it_is_found() {
    // The oracle's second B differs; `boundaries` says hit 2, so `live --from-hit 1` starts at the
    // last agreeing boundary and finds the instruction (here the hit itself, step 3).
    let ninge = seq(&[A, B, C, B, D]);
    let mut oracle = ninge.clone();
    oracle[3] = with_reg(oracle[3], 0, 1);
    let (text, diverged) = report(&ninge, &oracle, &with_breakpoint(B, 1));
    assert_eq!(
        (diverged, text.lines().next().map(str::to_string)),
        (
            true,
            Some("first divergence at instruction 3 (ARM9): mismatch".to_string())
        )
    );
}

// ---- errors ----

#[test]
fn given_a_bad_trace_when_run_live_then_the_error_names_the_ninge_side_and_nothing_is_sent() {
    let (fake, log) = Fake::new(script_of(&run_of(0, 1)));
    let result = run_live(fake, "garbage\n", &live_args(TraceCpu::Arm9));
    assert_eq!(
        (
            matches!(result, Err(CliError::Trace { which: "ninge", .. })),
            log.borrow().packets.len()
        ),
        (true, 0)
    );
}

#[test]
fn given_a_stub_that_answers_with_errors_when_run_live_then_the_oracle_failed() {
    let records = run_of(0, 2);
    let (fake, _) = Fake::new(script_of(&records));
    let result = run_live(
        fake.mode(Mode::Error),
        &trace_text(&records),
        &live_args(TraceCpu::Arm9),
    );
    assert!(matches!(result, Err(CliError::Oracle(_))));
}

#[test]
fn given_an_oracle_error_when_displayed_then_it_is_prefixed_with_the_oracle() {
    let records = run_of(0, 1);
    let (fake, _) = Fake::new(script_of(&records));
    let error = run_live(
        fake.mode(Mode::Error),
        &trace_text(&records),
        &live_args(TraceCpu::Arm9),
    )
    .unwrap_err();
    assert!(error.to_string().starts_with("the oracle: "));
}
