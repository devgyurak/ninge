// SPDX-License-Identifier: Apache-2.0
//! The event-boundary level of narrowing (`docs/contributing/trace-oracle.md`, "Narrowing"): at
//! the k-th hit of an address breakpoint, compare both sides' registers.

use std::io::{Read, Write};

use ninge_core::trace::TraceRecord;

use super::{Field, differing_fields};
use crate::oracle::{AdapterError, OracleCpu};

/// ninge's records at `pc`, in order: the hits of a breakpoint there.
pub fn hit_records(records: &[TraceRecord], pc: u32) -> Vec<TraceRecord> {
    records.iter().filter(|r| r.pc() == pc).copied().collect()
}

/// Where the boundary walk ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryOutcome {
    /// Every hit up to `hits` agreed.
    Agree {
        /// How many hits were compared.
        hits: u32,
    },
    /// Hit number `hit` (from 1) is the first whose registers differ.
    Differs {
        /// The first differing hit, counting from 1.
        hit: u32,
        /// One of the differing fields, the first in the fixed order.
        field: Field,
        /// ninge's record at that hit.
        ninge: TraceRecord,
    },
    /// The oracle stopped hitting the breakpoint (it exited) before `hit`.
    OracleEnded {
        /// The hit that never came, counting from 1.
        hit: u32,
    },
}

/// Walks the oracle from its current state through up to `max_hits` hits of the breakpoint at
/// `pc`, comparing the registers at each with ninge's record of the same hit.
///
/// The instruction level then starts from the last agreeing hit, `hit - 1`: reset the oracle and
/// align it there (`OracleCpu::run_to_hit`).
///
/// # Errors
///
/// An [`AdapterError`] from the oracle.
pub fn walk_boundaries<T: Read + Write>(
    oracle: &mut OracleCpu<T>,
    pc: u32,
    kind: u32,
    ninge_hits: &[TraceRecord],
    max_hits: u32,
) -> Result<BoundaryOutcome, AdapterError> {
    oracle.client().set_breakpoint(pc, kind)?;
    let mut compared = 0;
    for (index, ninge) in ninge_hits.iter().take(max_hits as usize).enumerate() {
        let hit = index as u32 + 1;
        if !oracle.run_to_breakpoint(index > 0)? {
            return Ok(BoundaryOutcome::OracleEnded { hit });
        }
        let (regs, cpsr) = oracle.state()?;
        let mut theirs = *ninge;
        theirs.regs = regs;
        theirs.cpsr = cpsr;
        // Only registers and the CPSR are compared at a boundary; the opcode is the same
        // instruction by construction.
        let fields = differing_fields(ninge, &theirs);
        if let Some(field) = fields.iter().flatten().copied().next() {
            return Ok(BoundaryOutcome::Differs {
                hit,
                field,
                ninge: *ninge,
            });
        }
        compared = hit;
    }
    Ok(BoundaryOutcome::Agree { hits: compared })
}
