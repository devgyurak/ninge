// SPDX-License-Identifier: Apache-2.0
//! `run_boundaries`: the event-boundary level of narrowing (`trace-oracle.md`, "Narrowing", level
//! 2). At the k-th hit of the breakpoint both sides' registers are compared; the first boundary
//! that differs is reported with how to rerun the instruction level from the last agreeing one
//! (`live --breakpoint ADDR --from-hit hit-1`).

use ninge_core::trace::{TraceCpu, TraceRecord};

use super::{live_args, parts};
use crate::cli::{CliError, LiveArgs, Report, run_boundaries};
use crate::diff::tests::{arm, on_arm7, trace_text, with_reg};
use crate::oracle::tests::fake::{Fake, Mode, script_of};

const A: u32 = 0x0200_0000;
const B: u32 = 0x0200_0004;
const C: u32 = 0x0200_0008;
const D: u32 = 0x0200_000C;

fn seq(pcs: &[u32]) -> Vec<TraceRecord> {
    pcs.iter()
        .enumerate()
        .map(|(step, pc)| arm(step as u32, *pc, 0xE1A0_0000))
        .collect()
}

fn args(hits: u32) -> LiveArgs {
    LiveArgs {
        breakpoint: Some(B),
        hits,
        ..live_args(TraceCpu::Arm9)
    }
}

fn boundaries(
    ninge: &[TraceRecord],
    oracle: &[TraceRecord],
    args: &LiveArgs,
) -> Result<Report, CliError> {
    let (fake, _) = Fake::new(script_of(oracle));
    run_boundaries(fake, &trace_text(ninge), args)
}

fn report(ninge: &[TraceRecord], oracle: &[TraceRecord], args: &LiveArgs) -> (String, bool) {
    parts(boundaries(ninge, oracle, args).expect("the walk finishes"))
}

#[test]
fn given_boundaries_that_all_agree_when_walked_then_the_report_counts_them() {
    let records = seq(&[A, B, C, B, D, B]);
    assert_eq!(
        report(&records, &records, &args(u32::MAX)),
        (
            "ARM9: 3 boundary hit(s) at 02000004 agree".to_string(),
            false
        )
    );
}

#[test]
fn given_a_second_hit_that_differs_when_walked_then_it_is_named_with_the_rerun_hint() {
    let ninge = seq(&[A, B, C, B, D, B]);
    let mut oracle = ninge.clone();
    oracle[3] = with_reg(oracle[3], 0, 9);
    assert_eq!(
        report(&ninge, &oracle, &args(u32::MAX)),
        (
            "ARM9: hit 2 of 02000004 is the first boundary that differs (r0; ninge step 3)\n\
             reset the oracle, then run `live` with --breakpoint 2000004 --from-hit 1 \
             to find the instruction"
                .to_string(),
            true
        )
    );
}

#[test]
fn given_a_first_hit_that_differs_when_walked_then_the_hint_restarts_from_hit_0() {
    let ninge = seq(&[A, B, C, B]);
    let mut oracle = ninge.clone();
    oracle[1] = with_reg(oracle[1], 4, 1);
    let (text, diverged) = report(&ninge, &oracle, &args(u32::MAX));
    assert_eq!(
        (
            diverged,
            text.starts_with(
                "ARM9: hit 1 of 02000004 is the first boundary that differs (r4; ninge step 1)\n"
            ),
            text.ends_with("--from-hit 0 to find the instruction"),
        ),
        (true, true, true)
    );
}

#[test]
fn given_a_differing_cpsr_when_walked_then_the_field_named_is_cpsr() {
    let ninge = seq(&[A, B]);
    let mut oracle = ninge.clone();
    oracle[1].cpsr = 0x2000_001F;
    let (text, _) = report(&ninge, &oracle, &args(u32::MAX));
    assert!(text.contains("differs (cpsr; ninge step 1)"));
}

#[test]
fn given_an_oracle_that_exits_early_when_walked_then_the_report_names_the_hit_it_never_reached() {
    let ninge = seq(&[A, B, C, B, D, B]);
    let oracle = seq(&[A, B]);
    assert_eq!(
        report(&ninge, &oracle, &args(u32::MAX)),
        (
            "ARM9: the oracle never reached hit 2 of 02000004".to_string(),
            true
        )
    );
}

#[test]
fn given_a_limit_before_the_difference_when_walked_then_only_that_many_boundaries_are_compared() {
    let ninge = seq(&[A, B, C, B, D, B]);
    let mut oracle = ninge.clone();
    oracle[5] = with_reg(oracle[5], 0, 9);
    assert_eq!(
        report(&ninge, &oracle, &args(2)),
        (
            "ARM9: 2 boundary hit(s) at 02000004 agree".to_string(),
            false
        )
    );
}

#[test]
fn given_an_arm7_walk_when_reported_then_the_text_names_arm7() {
    let records: Vec<TraceRecord> = seq(&[A, B, C]).into_iter().map(on_arm7).collect();
    let arm7 = LiveArgs {
        cpu: TraceCpu::Arm7,
        ..args(u32::MAX)
    };
    assert_eq!(
        report(&records, &records, &arm7),
        (
            "ARM7: 1 boundary hit(s) at 02000004 agree".to_string(),
            false
        )
    );
}

#[test]
fn given_a_hit_of_the_other_cpu_when_walked_then_it_is_not_a_boundary() {
    let nine = seq(&[A, B, C]);
    let ninge = [nine.clone(), vec![on_arm7(arm(0, B, 0xE1A0_0000))]].concat();
    assert_eq!(
        report(&ninge, &nine, &args(u32::MAX)),
        (
            "ARM9: 1 boundary hit(s) at 02000004 agree".to_string(),
            false
        )
    );
}

#[test]
fn given_no_breakpoint_when_walked_then_it_is_a_usage_error() {
    let records = seq(&[A, B]);
    let result = boundaries(&records, &records, &live_args(TraceCpu::Arm9));
    assert!(matches!(result, Err(CliError::Usage(_))));
}

#[test]
fn given_a_bad_trace_when_walked_then_the_error_names_the_ninge_side() {
    let (fake, _) = Fake::new(script_of(&seq(&[A, B])));
    let result = run_boundaries(fake, "not a trace\n", &args(u32::MAX));
    assert!(matches!(
        result,
        Err(CliError::Trace { which: "ninge", .. })
    ));
}

#[test]
fn given_a_stub_without_breakpoints_when_walked_then_the_oracle_failed() {
    let records = seq(&[A, B]);
    let (fake, _) = Fake::new(script_of(&records));
    let result = run_boundaries(
        fake.mode(Mode::Empty),
        &trace_text(&records),
        &args(u32::MAX),
    );
    assert!(matches!(result, Err(CliError::Oracle(_))));
}

#[test]
fn given_a_breakpoint_ninge_never_reaches_when_walked_then_it_is_no_such_hit() {
    let records = seq(&[A, B]);
    let (fake, _) = Fake::new(script_of(&records));
    let result = run_boundaries(
        fake,
        &trace_text(&records),
        &LiveArgs {
            breakpoint: Some(0x0BAD_0000),
            ..args(u32::MAX)
        },
    );
    assert!(matches!(result, Err(CliError::NoSuchHit(1))));
}
