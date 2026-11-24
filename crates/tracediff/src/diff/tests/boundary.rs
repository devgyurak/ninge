// SPDX-License-Identifier: Apache-2.0
//! The event-boundary level of narrowing (`trace-oracle.md`, "Narrowing", level 2): at the k-th hit
//! of an address breakpoint both sides' registers are compared; the first boundary that differs is
//! found. The oracle is the fake stub, whose `c` runs to the next scripted state at a breakpoint.

use ninge_core::trace::TraceRecord;

use super::{arm, with_cpsr, with_reg};
use crate::diff::{BoundaryOutcome, Field, hit_records, walk_boundaries};
use crate::oracle::tests::fake::{Fake, Mode, SharedLog, requests, script_of};
use crate::oracle::{AdapterError, OracleCpu, RegisterLayout};

const BP: u32 = 0x0200_0100;
const OTHER: u32 = 0x0200_0000;
const NOP: u32 = 0xE1A0_0000;

/// A record at the breakpoint with r0 = `marker`.
fn hit(marker: u32) -> TraceRecord {
    with_reg(arm(0, BP, NOP), 0, marker)
}

fn elsewhere(marker: u32) -> TraceRecord {
    with_reg(arm(0, OTHER, NOP), 0, marker)
}

fn oracle(records: &[TraceRecord]) -> (OracleCpu<Fake>, SharedLog) {
    let (fake, log) = Fake::new(script_of(records));
    (
        OracleCpu::connect(
            fake,
            ninge_core::trace::TraceCpu::Arm9,
            RegisterLayout::default(),
        ),
        log,
    )
}

fn walk(
    ninge: &[TraceRecord],
    oracle_records: &[TraceRecord],
    max_hits: u32,
) -> (Result<BoundaryOutcome, AdapterError>, SharedLog) {
    let (mut cpu, log) = oracle(oracle_records);
    (walk_boundaries(&mut cpu, BP, 4, ninge, max_hits), log)
}

fn outcome(
    ninge: &[TraceRecord],
    oracle_records: &[TraceRecord],
    max_hits: u32,
) -> BoundaryOutcome {
    walk(ninge, oracle_records, max_hits)
        .0
        .expect("the walk runs")
}

// ---- hit_records ----

#[test]
fn given_records_when_hits_are_taken_then_those_at_the_pc_come_back_in_order() {
    let records = [
        arm(0, OTHER, NOP),
        arm(1, BP, NOP),
        arm(2, OTHER, NOP),
        arm(3, BP, NOP),
        arm(4, BP, NOP),
    ];
    let steps: Vec<u32> = hit_records(&records, BP).iter().map(|r| r.step).collect();
    assert_eq!(steps, vec![1, 3, 4]);
}

#[test]
fn given_a_register_that_holds_the_address_when_hits_are_taken_then_only_the_pc_counts() {
    let records = [with_reg(arm(0, OTHER, NOP), 1, BP), arm(1, BP, NOP)];
    let steps: Vec<u32> = hit_records(&records, BP).iter().map(|r| r.step).collect();
    assert_eq!(steps, vec![1]);
}

#[test]
fn given_no_record_at_the_pc_when_hits_are_taken_then_there_are_none() {
    let records = [arm(0, OTHER, NOP), arm(1, OTHER + 4, NOP)];
    assert_eq!(hit_records(&records, BP), Vec::<TraceRecord>::new());
}

#[test]
fn given_no_records_when_hits_are_taken_then_there_are_none() {
    assert_eq!(hit_records(&[], BP), Vec::<TraceRecord>::new());
}

// ---- walk_boundaries ----

#[test]
fn given_two_agreeing_hits_when_walked_then_both_agree() {
    let hits = [hit(1), hit(2)];
    let script = [elsewhere(0), hit(1), elsewhere(5), hit(2)];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::Agree { hits: 2 }
    );
}

#[test]
fn given_no_hits_when_walked_then_nothing_is_compared_and_no_resume_is_sent() {
    let (result, log) = walk(&[], &[elsewhere(0), hit(1)], 10);
    assert_eq!(
        (result.unwrap(), requests(&log, 'c').len()),
        (BoundaryOutcome::Agree { hits: 0 }, 0)
    );
}

#[test]
fn given_a_second_hit_that_differs_when_walked_then_it_is_hit_2_with_its_register() {
    let hits = [hit(1), hit(2)];
    let script = [elsewhere(0), hit(1), elsewhere(5), hit(9)];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::Differs {
            hit: 2,
            field: Field::Register(0),
            ninge: hits[1],
        }
    );
}

#[test]
fn given_a_first_hit_that_differs_when_walked_then_it_is_hit_1() {
    let hits = [hit(1), hit(2)];
    let script = [elsewhere(0), hit(7), elsewhere(5), hit(2)];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::Differs {
            hit: 1,
            field: Field::Register(0),
            ninge: hits[0],
        }
    );
}

#[test]
fn given_several_differing_registers_when_walked_then_the_first_in_the_fixed_order_is_named() {
    let hits = [with_reg(with_reg(hit(0), 5, 1), 2, 1)];
    let script = [
        elsewhere(0),
        with_cpsr(with_reg(with_reg(hit(0), 5, 9), 2, 9), 0x2000_001F),
    ];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::Differs {
            hit: 1,
            field: Field::Register(2),
            ninge: hits[0],
        }
    );
}

#[test]
fn given_only_the_cpsr_differing_when_walked_then_the_field_is_cpsr() {
    let hits = [hit(1)];
    let script = [elsewhere(0), with_cpsr(hit(1), 0x8000_001F)];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::Differs {
            hit: 1,
            field: Field::Cpsr,
            ninge: hits[0],
        }
    );
}

#[test]
fn given_a_different_opcode_at_the_same_registers_when_walked_then_the_hits_agree() {
    // Only registers and the CPSR are compared at a boundary.
    let hits = [hit(1)];
    let script = [
        elsewhere(0),
        TraceRecord {
            opcode: ninge_core::trace::Opcode::Arm(0xE3A0_0001),
            ..hit(1)
        },
    ];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::Agree { hits: 1 }
    );
}

#[test]
fn given_an_oracle_that_stops_elsewhere_when_walked_then_the_pc_differs() {
    // No breakpoint state ahead: the stub stops at the last state, which is not the breakpoint.
    let hits = [hit(1)];
    let script = [elsewhere(0), elsewhere(1)];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::Differs {
            hit: 1,
            field: Field::Pc,
            ninge: hits[0],
        }
    );
}

#[test]
fn given_an_oracle_that_exits_after_hit_1_when_walked_then_it_ended_at_hit_2() {
    // The breakpoint is the last state, so stepping off it exits.
    let hits = [hit(1), hit(2)];
    let script = [elsewhere(0), hit(1)];
    assert_eq!(
        outcome(&hits, &script, 10),
        BoundaryOutcome::OracleEnded { hit: 2 }
    );
}

#[test]
fn given_an_oracle_that_exits_on_the_first_resume_when_walked_then_it_ended_at_hit_1() {
    let (fake, _) = Fake::new(script_of(&[elsewhere(0), hit(1)]));
    let mut cpu = OracleCpu::connect(
        fake.reply(b'c', b"W00"),
        ninge_core::trace::TraceCpu::Arm9,
        RegisterLayout::default(),
    );
    let result = walk_boundaries(&mut cpu, BP, 4, &[hit(1)], 10).unwrap();
    assert_eq!(result, BoundaryOutcome::OracleEnded { hit: 1 });
}

#[test]
fn given_a_difference_past_max_hits_when_walked_then_it_is_not_reached() {
    let hits = [hit(1), hit(2), hit(3)];
    let script = [
        elsewhere(0),
        hit(1),
        elsewhere(5),
        hit(2),
        elsewhere(6),
        hit(99),
    ];
    assert_eq!(
        outcome(&hits, &script, 2),
        BoundaryOutcome::Agree { hits: 2 }
    );
}

#[test]
fn given_a_difference_at_max_hits_when_walked_then_it_is_found() {
    let hits = [hit(1), hit(2), hit(3)];
    let script = [
        elsewhere(0),
        hit(1),
        elsewhere(5),
        hit(2),
        elsewhere(6),
        hit(99),
    ];
    assert_eq!(
        outcome(&hits, &script, 3),
        BoundaryOutcome::Differs {
            hit: 3,
            field: Field::Register(0),
            ninge: hits[2],
        }
    );
}

#[test]
fn given_max_hits_of_1_when_walked_then_the_oracle_is_resumed_once() {
    let hits = [hit(1), hit(2), hit(3)];
    let script = [
        elsewhere(0),
        hit(1),
        elsewhere(5),
        hit(2),
        elsewhere(6),
        hit(3),
    ];
    let (result, log) = walk(&hits, &script, 1);
    assert_eq!(
        (result.unwrap(), requests(&log, 'c').len()),
        (BoundaryOutcome::Agree { hits: 1 }, 1)
    );
}

#[test]
fn given_max_hits_of_0_when_walked_then_nothing_is_compared() {
    let (result, log) = walk(&[hit(1)], &[elsewhere(0), hit(2)], 0);
    assert_eq!(
        (result.unwrap(), requests(&log, 'c').len()),
        (BoundaryOutcome::Agree { hits: 0 }, 0)
    );
}

#[test]
fn given_a_walk_when_it_starts_then_the_breakpoint_is_set_with_the_kind_and_the_hex_address() {
    let (_, log) = walk(&[hit(1)], &[elsewhere(0), hit(1)], 10);
    assert_eq!(requests(&log, 'Z'), vec!["Z0,2000100,4".to_string()]);
}

#[test]
fn given_the_second_hit_when_walked_then_the_oracle_steps_off_the_first_before_resuming() {
    let hits = [hit(1), hit(2)];
    let script = [elsewhere(0), hit(1), elsewhere(5), hit(2)];
    let (_, log) = walk(&hits, &script, 10);
    let order: Vec<char> = log
        .borrow()
        .packets
        .iter()
        .filter_map(|p| p.chars().next())
        .filter(|c| matches!(c, 'c' | 's'))
        .collect();
    assert_eq!(order, vec!['c', 's', 'c']);
}

#[test]
fn given_a_stub_without_breakpoints_when_walked_then_the_error_is_returned() {
    let (fake, _) = Fake::new(script_of(&[hit(1)]));
    let mut cpu = OracleCpu::connect(
        fake.mode(Mode::Empty),
        ninge_core::trace::TraceCpu::Arm9,
        RegisterLayout::default(),
    );
    let result = walk_boundaries(&mut cpu, BP, 4, &[hit(1)], 10);
    assert!(matches!(result, Err(AdapterError::Rsp(_))));
}

#[test]
fn given_a_register_reply_that_is_too_short_when_walked_then_the_error_is_returned() {
    let (fake, _) = Fake::new(script_of(&[elsewhere(0), hit(1)]));
    let mut cpu = OracleCpu::connect(
        fake.reply(b'g', "00".repeat(64).as_bytes()),
        ninge_core::trace::TraceCpu::Arm9,
        RegisterLayout::default(),
    );
    let result = walk_boundaries(&mut cpu, BP, 4, &[hit(1)], 10);
    assert!(matches!(result, Err(AdapterError::RegisterCount { .. })));
}
