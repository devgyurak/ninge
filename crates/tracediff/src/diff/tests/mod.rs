// SPDX-License-Identifier: Apache-2.0
//! Tests of the diff: the comparison, the divergence classes and the boundary walk, plus the
//! done-when check of issue #60 against our own test ROM. The expected values come from
//! `docs/contributing/trace-oracle.md` ("The trace format", "Narrowing", "Divergence classes").
//!
//! The helpers here are shared with the CLI tests.

mod boundary;
mod classify;
mod compare;
mod dod;

use ninge_core::trace::{HEADER, Opcode, TraceCpu, TraceRecord};

use super::{Divergence, Outcome, compare};

/// CPSR of system mode, ARM state: the state most records are in.
pub(crate) const ARM_STATE: u32 = 0x0000_001F;
/// CPSR of system mode, Thumb state.
pub(crate) const THUMB_STATE: u32 = 0x0000_003F;

/// An ARM9 record in ARM state at `pc` with every register but r15 zero.
pub(crate) fn arm(step: u32, pc: u32, opcode: u32) -> TraceRecord {
    let mut regs = [0; 16];
    regs[15] = pc;
    TraceRecord {
        cpu: TraceCpu::Arm9,
        step,
        opcode: Opcode::Arm(opcode),
        regs,
        cpsr: ARM_STATE,
    }
}

/// An ARM9 record in Thumb state at `pc`.
pub(crate) fn thumb(step: u32, pc: u32, opcode: u16) -> TraceRecord {
    TraceRecord {
        opcode: Opcode::Thumb(opcode),
        cpsr: THUMB_STATE,
        ..arm(step, pc, 0)
    }
}

/// `record` with register `index` set.
pub(crate) fn with_reg(mut record: TraceRecord, index: usize, value: u32) -> TraceRecord {
    record.regs[index] = value;
    record
}

/// `record` with its CPSR set.
pub(crate) fn with_cpsr(record: TraceRecord, cpsr: u32) -> TraceRecord {
    TraceRecord { cpsr, ..record }
}

/// `record` as an ARM7 record.
pub(crate) fn on_arm7(record: TraceRecord) -> TraceRecord {
    TraceRecord {
        cpu: TraceCpu::Arm7,
        ..record
    }
}

/// `records` as trace text: the header, then one line per record.
pub(crate) fn trace_text(records: &[TraceRecord]) -> String {
    let lines: Vec<String> = records.iter().map(ToString::to_string).collect();
    format!("{HEADER}\n{}\n", lines.join("\n"))
}

/// A run of `count` distinct ARM records from step `first_step`, one word apart, in system mode.
pub(crate) fn run_of(first_step: u32, count: u32) -> Vec<TraceRecord> {
    (0..count)
        .map(|i| arm(first_step + i, 0x0200_0000 + 4 * i, 0xE1A0_0000))
        .collect()
}

/// Compares two in-memory streams whose oracle side cannot fail.
pub(crate) fn run(ninge: &[TraceRecord], oracle: &[TraceRecord]) -> Outcome {
    compare::<()>(ninge.iter().copied(), oracle.iter().copied().map(Ok)).expect("cannot fail")
}

/// The divergence of an outcome, `None` for any other.
pub(crate) fn divergence(outcome: Outcome) -> Option<Divergence> {
    match outcome {
        Outcome::Diverged(divergence) => Some(*divergence),
        _ => None,
    }
}
