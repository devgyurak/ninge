// SPDX-License-Identifier: Apache-2.0
//! `compare`: matching streams, the first difference with its fields and both records, an oracle
//! that ends early or fails, and the report text. The field order (pc, opcode, r0.., cpsr) and the
//! per-CPU step numbering are `trace-oracle.md` ("The trace format"). r15 is the pc, so it is
//! never listed as a register of its own.

use super::{ARM_STATE, arm, divergence, on_arm7, run, run_of, thumb, with_cpsr, with_reg};
use crate::diff::{Class, Field, Outcome, compare};

#[test]
fn given_identical_streams_when_compared_then_every_record_matched() {
    let stream = run_of(0, 5);
    assert_eq!(run(&stream, &stream), Outcome::Match { compared: 5 });
}

#[test]
fn given_two_empty_streams_when_compared_then_zero_records_matched() {
    assert_eq!(run(&[], &[]), Outcome::Match { compared: 0 });
}

#[test]
fn given_an_oracle_stream_longer_than_ninges_when_compared_then_only_ninges_length_is_counted() {
    let ninge = run_of(0, 3);
    let oracle = run_of(0, 6);
    assert_eq!(run(&ninge, &oracle), Outcome::Match { compared: 3 });
}

#[test]
fn given_streams_that_agree_but_for_the_step_when_compared_then_the_step_is_not_compared() {
    // The oracle numbers its records from 0 even after a from-hit alignment.
    let ninge = run_of(40, 3);
    let oracle = run_of(0, 3);
    assert_eq!(run(&ninge, &oracle), Outcome::Match { compared: 3 });
}

#[test]
fn given_a_difference_at_the_third_record_when_compared_then_the_report_is_ninges_step_of_it() {
    // Ninge's records start at step 5, so index 2 is step 7.
    let ninge = run_of(5, 5);
    let mut oracle = run_of(5, 5);
    oracle[2] = with_reg(oracle[2], 3, 1);
    let found = divergence(run(&ninge, &oracle)).map(|d| d.ninge.step);
    assert_eq!(found, Some(7));
}

#[test]
fn given_a_difference_when_compared_then_both_records_are_in_the_divergence() {
    let ninge = run_of(0, 3);
    let mut oracle = run_of(0, 3);
    oracle[1] = with_reg(oracle[1], 3, 9);
    let found = divergence(run(&ninge, &oracle)).map(|d| (d.ninge, d.oracle));
    assert_eq!(found, Some((ninge[1], oracle[1])));
}

#[test]
fn given_a_difference_in_the_first_record_when_compared_then_it_is_reported_at_step_0() {
    let ninge = run_of(0, 2);
    let mut oracle = run_of(0, 2);
    oracle[0] = with_reg(oracle[0], 0, 1);
    let found = divergence(run(&ninge, &oracle)).map(|d| d.ninge.step);
    assert_eq!(found, Some(0));
}

#[test]
fn given_a_difference_in_the_last_record_when_compared_then_it_is_reported_there() {
    let ninge = run_of(0, 4);
    let mut oracle = run_of(0, 4);
    oracle[3] = with_cpsr(oracle[3], 0x6000_001F);
    let found = divergence(run(&ninge, &oracle)).map(|d| d.ninge.step);
    assert_eq!(found, Some(3));
}

#[test]
fn given_later_differences_when_compared_then_only_the_first_is_reported() {
    let ninge = run_of(0, 6);
    let mut oracle = run_of(0, 6);
    oracle[2] = with_reg(oracle[2], 1, 1);
    oracle[4] = with_reg(oracle[4], 2, 2);
    let found = divergence(run(&ninge, &oracle)).map(|d| d.ninge.step);
    assert_eq!(found, Some(2));
}

#[test]
fn given_every_field_differing_when_compared_then_they_are_listed_in_the_fixed_order() {
    let base = arm(0, 0x0200_0000, 0xE1A0_0000);
    let other = with_cpsr(
        with_reg(with_reg(arm(0, 0x0200_0008, 0xE3A0_0001), 14, 2), 0, 1),
        0x2000_001F,
    );
    let found = divergence(run(&[base], &[other])).map(|d| d.differing().collect::<Vec<Field>>());
    assert_eq!(
        found,
        Some(vec![
            Field::Pc,
            Field::Opcode,
            Field::Register(0),
            Field::Register(14),
            Field::Cpsr,
        ])
    );
}

#[test]
fn given_a_pc_that_differs_when_compared_then_r15_is_not_also_listed_as_a_register() {
    let ninge = [arm(0, 0x0200_0000, 0xE1A0_0000)];
    let oracle = [arm(0, 0x0200_0004, 0xE1A0_0000)];
    let found = divergence(run(&ninge, &oracle)).map(|d| d.differing().collect::<Vec<Field>>());
    assert_eq!(found, Some(vec![Field::Pc]));
}

#[test]
fn given_an_arm_and_a_thumb_opcode_of_the_same_value_when_compared_then_the_opcode_differs() {
    // The width is part of the opcode column (4 digits in Thumb state, 8 in ARM state).
    let ninge = [arm(0, 0x0200_0000, 0x0000_DF05)];
    let oracle = [with_cpsr(thumb(0, 0x0200_0000, 0xDF05), ARM_STATE)];
    let found = divergence(run(&ninge, &oracle)).map(|d| d.differing().collect::<Vec<Field>>());
    assert_eq!(found, Some(vec![Field::Opcode]));
}

#[test]
fn given_two_opcodes_with_one_differing_bit_when_compared_then_only_the_opcode_differs() {
    let ninge = [arm(0, 0x0200_0000, 0xE1A0_0000)];
    let oracle = [arm(0, 0x0200_0000, 0xE1A0_0001)];
    let found = divergence(run(&ninge, &oracle)).map(|d| d.differing().collect::<Vec<Field>>());
    assert_eq!(found, Some(vec![Field::Opcode]));
}

#[test]
fn given_a_difference_in_r14_only_when_compared_then_the_last_low_register_is_compared() {
    let ninge = [arm(0, 0x0200_0000, 0xE1A0_0000)];
    let oracle = [with_reg(ninge[0], 14, 1)];
    let found = divergence(run(&ninge, &oracle)).map(|d| d.differing().collect::<Vec<Field>>());
    assert_eq!(found, Some(vec![Field::Register(14)]));
}

#[test]
fn given_a_plain_register_difference_when_compared_then_the_class_is_mismatch() {
    let ninge = run_of(0, 2);
    let mut oracle = run_of(0, 2);
    oracle[1] = with_reg(oracle[1], 3, 1);
    let found = divergence(run(&ninge, &oracle)).map(|d| d.class);
    assert_eq!(found, Some(Class::Mismatch));
}

#[test]
fn given_an_oracle_that_ends_after_two_records_when_compared_then_two_were_compared() {
    let ninge = run_of(0, 5);
    let oracle = run_of(0, 2);
    assert_eq!(run(&ninge, &oracle), Outcome::OracleEnded { compared: 2 });
}

#[test]
fn given_an_empty_oracle_when_compared_then_it_ended_before_the_first_record() {
    assert_eq!(
        run(&run_of(0, 2), &[]),
        Outcome::OracleEnded { compared: 0 }
    );
}

#[test]
fn given_a_difference_before_the_oracle_ends_when_compared_then_the_difference_wins() {
    let ninge = run_of(0, 5);
    let mut oracle = run_of(0, 3);
    oracle[2] = with_reg(oracle[2], 5, 5);
    let found = divergence(run(&ninge, &oracle)).map(|d| d.ninge.step);
    assert_eq!(found, Some(2));
}

#[test]
fn given_an_oracle_stream_that_fails_when_compared_then_the_error_is_returned() {
    let ninge = run_of(0, 4);
    let oracle = vec![Ok(ninge[0]), Ok(ninge[1]), Err("stub went away")];
    let outcome = compare(ninge, oracle);
    assert_eq!(outcome, Err("stub went away"));
}

#[test]
fn given_an_oracle_stream_that_fails_at_once_when_compared_then_the_error_is_returned() {
    let outcome = compare(run_of(0, 1), vec![Err::<_, &str>("closed")]);
    assert_eq!(outcome, Err("closed"));
}

#[test]
fn given_an_oracle_that_differs_before_it_fails_when_compared_then_the_difference_is_returned() {
    // The stream is read lazily: the error after the difference is never reached.
    let ninge = run_of(0, 3);
    let oracle = vec![Ok(with_reg(ninge[0], 0, 1)), Err("late")];
    let outcome = compare(ninge, oracle).map(|o| divergence(o).map(|d| d.ninge.step));
    assert_eq!(outcome, Ok(Some(0)));
}

#[test]
fn given_a_divergence_when_displayed_then_it_names_the_instruction_the_class_and_both_lines() {
    let ninge = run_of(0, 8);
    let mut oracle = run_of(0, 8);
    oracle[7] = with_reg(oracle[7], 3, 6);
    let text = divergence(run(&ninge, &oracle)).map(|d| d.to_string());
    // r0..r14 are fifteen words, then r15 (the pc), then the CPSR.
    let zeros = |count: usize| "00000000 ".repeat(count);
    let mismatch = format!(
        "first divergence at instruction 7 (ARM9): mismatch\n  differs: r3\n  \
         ninge : 9 00000007 0200001c e1a00000 {}0200001c 0000001f\n  \
         oracle: 9 00000007 0200001c e1a00000 {}00000006 {}0200001c 0000001f",
        zeros(15),
        zeros(3),
        zeros(11),
    );
    assert_eq!(text.as_deref(), Some(mismatch.as_str()));
}

#[test]
fn given_an_arm7_divergence_when_displayed_then_the_header_names_arm7() {
    let ninge = [on_arm7(arm(4, 0x0380_0000, 0xE1A0_0000))];
    let oracle = [on_arm7(with_reg(ninge[0], 0, 1))];
    let text = divergence(run(&ninge, &oracle)).map(|d| d.to_string());
    assert_eq!(
        text.and_then(|t| t.lines().next().map(str::to_string)),
        Some("first divergence at instruction 4 (ARM7): mismatch".to_string())
    );
}

#[test]
fn given_a_divergence_with_several_fields_when_displayed_then_they_are_comma_separated() {
    let ninge = [arm(0, 0x0200_0000, 0xE1A0_0000)];
    let oracle = [with_cpsr(with_reg(ninge[0], 1, 1), 0x8000_001F)];
    let text = divergence(run(&ninge, &oracle)).map(|d| d.to_string());
    assert_eq!(
        text.and_then(|t| t.lines().nth(1).map(str::to_string)),
        Some("  differs: r1, cpsr".to_string())
    );
}

#[test]
fn given_each_class_when_displayed_then_the_text_names_it() {
    let texts = [
        Class::Timing.to_string(),
        Class::AfterBiosCall.to_string(),
        Class::Mismatch.to_string(),
    ];
    assert_eq!(
        texts,
        [
            "timing (an interrupt was taken at a different instruction)".to_string(),
            "after a BIOS call (the two BIOSes may differ)".to_string(),
            "mismatch".to_string(),
        ]
    );
}

#[test]
fn given_each_field_when_displayed_then_it_is_the_column_name_of_the_trace() {
    let texts: Vec<String> = [
        Field::Pc,
        Field::Opcode,
        Field::Register(0),
        Field::Register(14),
        Field::Cpsr,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(texts, ["pc", "opcode", "r0", "r14", "cpsr"]);
}

#[test]
fn given_a_thumb_stream_when_compared_with_itself_then_it_matches() {
    let stream = [thumb(0, 0x0800_0000, 0x2001), thumb(1, 0x0800_0002, 0xDF05)];
    assert_eq!(run(&stream, &stream), Outcome::Match { compared: 2 });
}
