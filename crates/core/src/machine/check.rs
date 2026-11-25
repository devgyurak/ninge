// SPDX-License-Identifier: Apache-2.0
//! The check a savestate load runs on a decoded machine: whether it is a machine a run could
//! reach (`docs/contributing/state-contents.md`). Each device checks itself through the registry
//! (`Checked::well_formed`); what spans devices, the CPUs and the clocks is checked here, so the
//! savestate module handles the format alone.

use super::Machine;
use crate::cpu::ARM7_INSTRUCTION_COST;
use crate::io::device::StateTimes;
use crate::scheduler::MAX_TIME;

impl Machine {
    /// Whether this machine, decoded from a savestate, is one a run could reach on `rom`, the ROM
    /// of the machine that loads it. A booted program has a ROM and an unbooted machine has none
    /// (`load_rom`); a state that says otherwise would run on a header alone, or hold a ROM it
    /// never booted. Memory buffers of the wrong size would read as zeros rather than panic, but
    /// are not a machine.
    pub(crate) fn is_well_formed(&self, rom: &[u8]) -> bool {
        let now = self.scheduler.now();
        // The header is derived from the ROM, so it must be what the ROM parses to.
        self.header == crate::cart::parse(rom).ok()
            && self.memory.is_well_formed()
            && self.screens_well_formed()
            && self.frame3d_well_formed()
            && self.scheduler.is_well_formed()
            // A round in progress belongs to a booted program and to times a run can reach.
            && self.round.is_none_or(|round| {
                self.header.is_some() && round.well_formed(now, self.arm7_clock)
            })
            // Each device at the times the state holds: VCOUNT and the HBlank flag against the
            // time the line events have reached, which a machine with no program does not have.
            && self.io.is_well_formed(StateTimes {
                now,
                lines: self.header.is_some().then(|| self.events_from()),
            })
            && self.arm9.is_well_formed()
            && self.arm7.is_well_formed()
            // A latched touch is on the screen: the latch clamps it (`Touch::MAX_Y`).
            && self.io.input().touch.is_none_or(|touch| touch == touch.clamped())
            // A booted machine between rounds is settled: the ARM7 is at `now` or one master cycle
            // past it. One that lags would spend the first round catching up. In a round a stop
            // interrupted the ARM7 has not caught up yet, and lags by up to a slice.
            && !(self.header.is_some() && self.round.is_none() && self.arm7_clock < now)
            // TODO(phase8): once an instruction's cost varies, this bound is the largest cost.
            && self.arm7_clock < now.saturating_add(ARM7_INSTRUCTION_COST)
            && self.arm9_hold_until <= MAX_TIME
    }
}
