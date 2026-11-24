// SPDX-License-Identifier: Apache-2.0
//! The divergence classes (`trace-oracle.md`, "Divergence classes"): timing (an interrupt taken at
//! a different instruction: the pcs part and a side's mode moved since the last agreed record),
//! after a BIOS call (the previous record is an SWI, ARM `EF......` or Thumb `DFxx`, and pc and
//! opcode agree), and mismatch (anything else). The cases are a table run by a harness, so every
//! wrong class shows at once.

use ninge_core::trace::TraceRecord;

use super::{arm, run, thumb, with_cpsr, with_reg};
use crate::diff::{Class, Outcome};

const IRQ_MODE_CPSR: u32 = 0x0000_0092;

/// One table row: the records both sides agree on, then the first records they disagree on.
struct Case {
    name: &'static str,
    agreed: Vec<TraceRecord>,
    ninge: TraceRecord,
    oracle: TraceRecord,
    expected: Class,
}

/// The class of the first divergence, `None` when the streams did not diverge.
fn class_of(case: &Case) -> Option<Class> {
    let mut ninge = case.agreed.clone();
    ninge.push(case.ninge);
    let mut oracle = case.agreed.clone();
    oracle.push(case.oracle);
    match run(&ninge, &oracle) {
        Outcome::Diverged(divergence) => Some(divergence.class),
        _ => None,
    }
}

/// The names of the cases whose class is not the expected one.
fn wrong_classes(cases: &[Case]) -> Vec<&'static str> {
    cases
        .iter()
        .filter(|case| class_of(case) != Some(case.expected))
        .map(|case| case.name)
        .collect()
}

/// A case after one ARM instruction `before` that both sides agreed on.
fn after_arm(
    name: &'static str,
    before: u32,
    ninge: TraceRecord,
    oracle: TraceRecord,
    expected: Class,
) -> Case {
    Case {
        name,
        agreed: vec![arm(0, 0x0200_0000, before)],
        ninge,
        oracle,
        expected,
    }
}

/// A case after one Thumb instruction `before`.
fn after_thumb(
    name: &'static str,
    before: u16,
    ninge: TraceRecord,
    oracle: TraceRecord,
    expected: Class,
) -> Case {
    Case {
        name,
        agreed: vec![thumb(0, 0x0800_0000, before)],
        ninge,
        oracle,
        expected,
    }
}

const NOP: u32 = 0xE1A0_0000;

fn next() -> TraceRecord {
    arm(1, 0x0200_0004, NOP)
}

fn thumb_next() -> TraceRecord {
    thumb(1, 0x0800_0002, 0x46C0)
}

fn timing_cases() -> Vec<Case> {
    // The IRQ vector's first instruction, with the CPSR already in IRQ mode.
    let taken = with_cpsr(arm(1, 0x0000_0018, NOP), IRQ_MODE_CPSR);
    vec![
        after_arm(
            "ninge takes the interrupt",
            NOP,
            taken,
            next(),
            Class::Timing,
        ),
        after_arm(
            "the oracle takes the interrupt",
            NOP,
            next(),
            taken,
            Class::Timing,
        ),
        after_arm(
            "the interrupt is taken after a BIOS call",
            0xEF00_0005,
            taken,
            next(),
            Class::Timing,
        ),
        after_thumb(
            "an interrupt in Thumb state",
            0x46C0,
            with_cpsr(arm(1, 0x0000_0018, NOP), IRQ_MODE_CPSR),
            thumb_next(),
            Class::Timing,
        ),
    ]
}

fn bios_cases() -> Vec<Case> {
    let r0 = |record: TraceRecord| with_reg(record, 0, 0x1234);
    vec![
        after_arm(
            "r0 after an ARM SWI",
            0xEF00_0005,
            next(),
            r0(next()),
            Class::AfterBiosCall,
        ),
        after_arm(
            "an SWI number of 0",
            0xEF00_0000,
            next(),
            r0(next()),
            Class::AfterBiosCall,
        ),
        after_arm(
            "an SWI number of 24 bits",
            0xEFFF_FFFF,
            next(),
            r0(next()),
            Class::AfterBiosCall,
        ),
        after_arm(
            "r3 and r12 after an ARM SWI",
            0xEF00_0006,
            next(),
            with_reg(with_reg(next(), 3, 1), 12, 2),
            Class::AfterBiosCall,
        ),
        after_thumb(
            "r0 after a Thumb SWI",
            0xDF05,
            thumb_next(),
            r0(thumb_next()),
            Class::AfterBiosCall,
        ),
        after_thumb(
            "a Thumb SWI number of 0",
            0xDF00,
            thumb_next(),
            r0(thumb_next()),
            Class::AfterBiosCall,
        ),
        after_thumb(
            "a Thumb SWI number of 255",
            0xDFFF,
            thumb_next(),
            r0(thumb_next()),
            Class::AfterBiosCall,
        ),
    ]
}

fn mismatch_cases() -> Vec<Case> {
    let r0 = |record: TraceRecord| with_reg(record, 0, 0x1234);
    // Both sides left the user mode for IRQ mode: an IRQ return to the wrong address, not timing.
    let both_irq = |record: TraceRecord| with_cpsr(record, IRQ_MODE_CPSR);
    let other_op = arm(1, 0x0200_0004, 0xE3A0_0001);
    let taken = with_cpsr(arm(1, 0x0000_0018, NOP), IRQ_MODE_CPSR);
    vec![
        after_arm(
            "a register after an ordinary instruction",
            NOP,
            next(),
            r0(next()),
            Class::Mismatch,
        ),
        after_arm(
            "the CPSR after an ordinary instruction",
            NOP,
            next(),
            with_cpsr(next(), 0x8000_001F),
            Class::Mismatch,
        ),
        after_arm(
            "a pc that parts with no mode change",
            NOP,
            next(),
            arm(1, 0x0200_0100, NOP),
            Class::Mismatch,
        ),
        after_arm(
            "an opcode after an ordinary instruction",
            NOP,
            next(),
            other_op,
            Class::Mismatch,
        ),
        after_arm(
            "an opcode after an ARM SWI",
            0xEF00_0005,
            next(),
            other_op,
            Class::Mismatch,
        ),
        after_thumb(
            "an opcode after a Thumb SWI",
            0xDF05,
            thumb_next(),
            thumb(1, 0x0800_0002, 0x2001),
            Class::Mismatch,
        ),
        after_arm(
            "a pc after an ARM SWI with no mode change",
            0xEF00_0005,
            next(),
            arm(1, 0x0200_0100, NOP),
            Class::Mismatch,
        ),
        after_arm(
            "the CPSR flags after an ARM SWI",
            0xEF00_0006,
            next(),
            with_cpsr(next(), 0x2000_001F),
            Class::Mismatch,
        ),
        after_arm(
            "r4 after an ARM SWI",
            0xEF00_0006,
            next(),
            with_reg(next(), 4, 1),
            Class::Mismatch,
        ),
        after_arm(
            "r14 after an ARM SWI",
            0xEF00_0006,
            next(),
            with_reg(next(), 14, 1),
            Class::Mismatch,
        ),
        after_arm(
            "the pcs part while both sides are in the same mode",
            NOP,
            both_irq(next()),
            both_irq(arm(1, 0x0200_0100, NOP)),
            Class::Mismatch,
        ),
        after_arm(
            "a mode change with the same pc and opcode",
            NOP,
            next(),
            with_cpsr(next(), IRQ_MODE_CPSR),
            Class::Mismatch,
        ),
        after_arm(
            "a coprocessor opcode is no SWI",
            0xEE00_0000,
            next(),
            r0(next()),
            Class::Mismatch,
        ),
        after_arm(
            "a branch is no SWI",
            0xEA00_0000,
            next(),
            r0(next()),
            Class::Mismatch,
        ),
        after_arm(
            "DF in the low bits of an ARM word is no SWI",
            0xE000_DF00,
            next(),
            r0(next()),
            Class::Mismatch,
        ),
        after_thumb(
            "a Thumb DE opcode is no SWI",
            0xDE05,
            thumb_next(),
            r0(thumb_next()),
            Class::Mismatch,
        ),
        after_thumb(
            "a Thumb EF opcode is no SWI",
            0xEF05,
            thumb_next(),
            r0(thumb_next()),
            Class::Mismatch,
        ),
        Case {
            name: "the first record already differs in the pc and the mode",
            agreed: Vec::new(),
            ninge: arm(0, 0x0200_0000, NOP),
            oracle: taken,
            expected: Class::Mismatch,
        },
        Case {
            name: "an SWI two records back",
            agreed: vec![arm(0, 0x0200_0000, 0xEF00_0005), arm(1, 0x0200_0004, NOP)],
            ninge: arm(2, 0x0200_0008, NOP),
            oracle: r0(arm(2, 0x0200_0008, NOP)),
            expected: Class::Mismatch,
        },
        Case {
            name: "an SWI that is the first record, then an ordinary difference",
            agreed: vec![arm(0, 0x0200_0000, NOP), arm(1, 0x0200_0004, NOP)],
            ninge: arm(2, 0x0200_0008, 0xEF00_0005),
            oracle: arm(2, 0x0200_0008, 0xEF00_0006),
            expected: Class::Mismatch,
        },
    ]
}

#[test]
fn given_an_interrupt_taken_on_one_side_when_compared_then_the_class_is_timing() {
    assert_eq!(wrong_classes(&timing_cases()), Vec::<&str>::new());
}

#[test]
fn given_registers_differing_right_after_an_swi_when_compared_then_the_class_is_after_bios_call() {
    assert_eq!(wrong_classes(&bios_cases()), Vec::<&str>::new());
}

#[test]
fn given_any_other_difference_when_compared_then_the_class_is_mismatch() {
    assert_eq!(wrong_classes(&mismatch_cases()), Vec::<&str>::new());
}

#[test]
fn given_the_same_register_difference_with_and_without_a_preceding_swi_when_compared_then_only_the_swi_one_is_after_bios_call()
 {
    let after_swi = after_arm(
        "swi",
        0xEF00_0005,
        next(),
        with_reg(next(), 2, 9),
        Class::AfterBiosCall,
    );
    let plain = after_arm(
        "plain",
        NOP,
        next(),
        with_reg(next(), 2, 9),
        Class::Mismatch,
    );
    assert_eq!(
        (class_of(&after_swi), class_of(&plain)),
        (Some(Class::AfterBiosCall), Some(Class::Mismatch))
    );
}
