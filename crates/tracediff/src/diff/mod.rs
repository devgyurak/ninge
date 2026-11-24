// SPDX-License-Identifier: Apache-2.0
//! Comparing two instruction streams of one CPU: the first divergence, the fields that differ and
//! its class (`docs/contributing/trace-oracle.md`, "Divergence classes").
//!
//! The streams are ninge's (from a trace file) and the oracle's (a file or a live adapter). The
//! two CPUs are compared separately, so the caller hands in one CPU's records at a time.

mod boundary;

use std::fmt;

use ninge_core::trace::{Opcode, TraceRecord};

pub use boundary::{BoundaryOutcome, hit_records, walk_boundaries};

/// The CPSR's mode bits.
const MODE_MASK: u32 = 0x1F;
/// Registers compared besides the pc: r0 through r14.
const LOW_REGISTERS: usize = 15;

/// What differs between two records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The address of the instruction.
    Pc,
    /// The opcode, or its width.
    Opcode,
    /// A register, r0 through r14.
    Register(u8),
    /// The CPSR.
    Cpsr,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pc => write!(f, "pc"),
            Self::Opcode => write!(f, "opcode"),
            Self::Register(n) => write!(f, "r{n}"),
            Self::Cpsr => write!(f, "cpsr"),
        }
    }
}

/// How a divergence is to be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// An interrupt was taken at a different instruction. IRQ timing is approximate by design
    /// (`AGENTS.md` §2 rule 7), so this is not a bug by itself.
    Timing,
    /// Registers differ right after a collapsed BIOS call, where two BIOS implementations may
    /// legitimately leave scratch registers different.
    AfterBiosCall,
    /// Anything else: a pc, an opcode, a register or the CPSR.
    Mismatch,
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timing => write!(
                f,
                "timing (an interrupt was taken at a different instruction)"
            ),
            Self::AfterBiosCall => write!(f, "after a BIOS call (the two BIOSes may differ)"),
            Self::Mismatch => write!(f, "mismatch"),
        }
    }
}

/// The first place two streams of one CPU differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// How to read it.
    pub class: Class,
    /// Which of the two records' fields differ, in a fixed order: pc, opcode, r0.., cpsr.
    pub fields: [Option<Field>; LOW_REGISTERS + 3],
    /// ninge's record; its `step` is the instruction number the report names.
    pub ninge: TraceRecord,
    /// The oracle's record.
    pub oracle: TraceRecord,
}

impl Divergence {
    /// The differing fields.
    pub fn differing(&self) -> impl Iterator<Item = Field> + '_ {
        self.fields.iter().flatten().copied()
    }
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cpu = match self.ninge.cpu {
            ninge_core::trace::TraceCpu::Arm9 => 9,
            ninge_core::trace::TraceCpu::Arm7 => 7,
        };
        writeln!(
            f,
            "first divergence at instruction {} (ARM{cpu}): {}",
            self.ninge.step, self.class
        )?;
        let names: Vec<String> = self.differing().map(|field| field.to_string()).collect();
        writeln!(f, "  differs: {}", names.join(", "))?;
        writeln!(f, "  ninge : {}", self.ninge)?;
        write!(f, "  oracle: {}", self.oracle)
    }
}

/// The result of comparing two streams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every record of ninge's stream matched.
    Match {
        /// How many records were compared.
        compared: u32,
    },
    /// The streams differ.
    Diverged(Box<Divergence>),
    /// The oracle's stream ended before ninge's.
    OracleEnded {
        /// How many records matched before it ended.
        compared: u32,
    },
}

/// The fields in which `a` and `b` differ, in a fixed order.
pub fn differing_fields(a: &TraceRecord, b: &TraceRecord) -> [Option<Field>; LOW_REGISTERS + 3] {
    let mut fields = [None; LOW_REGISTERS + 3];
    let mut next = 0;
    let mut push = |field| {
        fields[next] = Some(field);
        next += 1;
    };
    if a.pc() != b.pc() {
        push(Field::Pc);
    }
    if a.opcode != b.opcode {
        push(Field::Opcode);
    }
    (0..LOW_REGISTERS)
        .filter(|&i| a.regs[i] != b.regs[i])
        .for_each(|i| push(Field::Register(i as u8)));
    if a.cpsr != b.cpsr {
        push(Field::Cpsr);
    }
    fields
}

/// Whether `opcode` is a software interrupt: the BIOS call after which registers may differ.
fn is_swi(opcode: Opcode) -> bool {
    match opcode {
        Opcode::Arm(word) => word & 0x0F00_0000 == 0x0F00_0000,
        Opcode::Thumb(half) => half >> 8 == 0xDF,
    }
}

/// How the first difference after `previous` (the last record both sides agreed on) is to be read.
fn classify(previous: Option<&TraceRecord>, ninge: &TraceRecord, oracle: &TraceRecord) -> Class {
    let pc_differs = ninge.pc() != oracle.pc();
    // An exception entry changes the mode: if either side's mode moved while the two pcs
    // parted, one side took an interrupt the other did not take yet.
    let mode_moved = previous.is_some_and(|prev| {
        let before = prev.cpsr & MODE_MASK;
        ninge.cpsr & MODE_MASK != before || oracle.cpsr & MODE_MASK != before
    });
    let after_bios = previous.is_some_and(|prev| is_swi(prev.opcode));
    match (pc_differs, mode_moved, after_bios) {
        (true, true, _) => Class::Timing,
        (false, _, true) if ninge.opcode == oracle.opcode => Class::AfterBiosCall,
        _ => Class::Mismatch,
    }
}

/// Compares ninge's records with the oracle's, record by record, and stops at the first
/// difference. The oracle's stream may fail (a stub or stream error), which ends the comparison.
///
/// # Errors
///
/// The first error the oracle's stream produces.
pub fn compare<E>(
    ninge: impl IntoIterator<Item = TraceRecord>,
    oracle: impl IntoIterator<Item = Result<TraceRecord, E>>,
) -> Result<Outcome, E> {
    let mut oracle = oracle.into_iter();
    let mut previous: Option<TraceRecord> = None;
    let mut compared = 0u32;
    for ninge in ninge {
        let Some(oracle) = oracle.next().transpose()? else {
            return Ok(Outcome::OracleEnded { compared });
        };
        let fields = differing_fields(&ninge, &oracle);
        if fields.iter().any(Option::is_some) {
            let class = classify(previous.as_ref(), &ninge, &oracle);
            return Ok(Outcome::Diverged(Box::new(Divergence {
                class,
                fields,
                ninge,
                oracle,
            })));
        }
        previous = Some(ninge);
        compared = compared.wrapping_add(1);
    }
    Ok(Outcome::Match { compared })
}

#[cfg(test)]
pub(crate) mod tests;
