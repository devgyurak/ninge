// SPDX-License-Identifier: Apache-2.0
//! Test fixture: bus access on a whole `Machine`, the way a CPU step reaches it. The views split-borrow
//! the machine's fields, so a register write schedules its timers like a real instruction's does.

use crate::bus::{Arm7View, Arm9View, TcmConfig};
use crate::cpu::ARM9_INSTRUCTION_COST;
use crate::machine::Machine;

impl Machine {
    /// The ARM9's bus at the master clock, with the TCM as CP15 has it.
    pub(crate) fn bus9(&mut self) -> Arm9View<'_> {
        let tcm = TcmConfig::from_cp15(self.arm9.cp15());
        let now = self.scheduler.now();
        Arm9View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            tcm,
            now,
            now.saturating_add(ARM9_INSTRUCTION_COST),
        )
    }

    /// The ARM7's bus at its own clock `now`.
    pub(crate) fn bus7(&mut self, now: u64) -> Arm7View<'_> {
        let floor = self.scheduler.now();
        Arm7View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            now,
            floor,
        )
    }

    /// A byte as the ARM7 would read it at its clock `now`, without side effects.
    pub(crate) fn arm7_peek8(&self, now: u64, addr: u32) -> u8 {
        u8::from_le_bytes(self.arm7_peek(now, addr))
    }

    /// A halfword as the ARM7 would read it at its clock `now`, without side effects.
    pub(crate) fn arm7_peek16(&self, now: u64, addr: u32) -> u16 {
        u16::from_le_bytes(self.arm7_peek(now, addr))
    }

    /// A word as the ARM7 would read it at its clock `now`, without side effects.
    pub(crate) fn arm7_peek32(&self, now: u64, addr: u32) -> u32 {
        u32::from_le_bytes(self.arm7_peek(now, addr))
    }

    fn arm7_peek<const N: usize>(&self, now: u64, addr: u32) -> [u8; N] {
        crate::bus::peek::<false, N>(&self.memory, &self.io, &TcmConfig::NONE, now, addr)
    }
}
