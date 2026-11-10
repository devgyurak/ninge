// SPDX-License-Identifier: Apache-2.0
//! The SWI routines the HLE BIOS implements, by number. Note: `docs/hw/hle-swi.md`.
//!
//! A routine is a function generic over the CPU and its bus: it reads and writes memory through
//! the CPU's own view, so I/O writes land, and it never allocates (AGENTS.md §2 rule 3).

use super::bios::SwiRecord;
use crate::cpu::{Bus, Cpu};

/// Runs SWI `number` for `cpu`. A number no routine implements has no effect and is recorded
/// (`hle-swi.md`, "Unimplemented number").
// TODO(phase3): the wait, copy, math and decompression routines arrive in the next #27 PRs.
pub(super) fn run<const ARM9: bool, B: Bus>(
    number: u8,
    _cpu: &mut Cpu<ARM9>,
    _bus: &mut B,
    record: &mut SwiRecord,
) {
    record.last_unimplemented = Some(number);
}
