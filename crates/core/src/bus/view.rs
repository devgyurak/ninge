// SPDX-License-Identifier: Apache-2.0
//! The ARM9 bus view: `Memory` plus the TCM configuration copied from CP15 before a step
//! (`docs/hw/arm9-memory-map.md`).

use super::Memory;
use crate::cpu::{Bus, Cp15, Tcm};

/// Both TCM regions as CP15 resolves them (`docs/hw/cp15.md`), and the ARM9's clock.
///
/// The clock rides along because both are copied into the view just before a step: a timer
/// register read or write needs the time of the access (`docs/hw/timers.md`, "Model").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcmConfig {
    /// Instruction TCM.
    pub itcm: Tcm,
    /// Data TCM.
    pub dtcm: Tcm,
    /// The ARM9's clock in master cycles, as of the access; 0 until [`TcmConfig::at`].
    pub now: u64,
}

impl TcmConfig {
    /// Copies both regions from CP15. A CP15 write therefore takes effect from the next step.
    pub fn from_cp15(cp15: &Cp15) -> Self {
        Self {
            itcm: cp15.itcm(),
            dtcm: cp15.dtcm(),
            now: 0,
        }
    }

    /// This configuration with the ARM9's clock set to `now`, in master cycles.
    pub fn at(self, now: u64) -> Self {
        Self { now, ..self }
    }
}

/// A short-lived `Bus` over `Memory` and a TCM configuration. It is not state, so it may hold a
/// borrow (AGENTS.md §2 rule 5 covers state only).
pub struct Arm9View<'a> {
    memory: &'a mut Memory,
    tcm: TcmConfig,
}

impl<'a> Arm9View<'a> {
    /// Builds the view for one step.
    pub fn new(memory: &'a mut Memory, tcm: TcmConfig) -> Self {
        Self { memory, tcm }
    }
}

impl Bus for Arm9View<'_> {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.memory.fetch32(&self.tcm, addr)
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.memory.fetch16(&self.tcm, addr)
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.memory.read8_mut(&self.tcm, addr)
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.memory.read16_mut(&self.tcm, addr)
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.memory.read32_mut(&self.tcm, addr)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.memory.write8(&self.tcm, addr, value);
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.memory.write16(&self.tcm, addr, value);
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.memory.write32(&self.tcm, addr, value);
    }
}

/// The ARM7's bus: a short-lived view over `Memory`, which reaches main RAM, the shared WRAM it
/// owns, its own WRAM and WRAMSTAT (`docs/hw/wramcnt.md`). It goes through the ARM7's own page
/// table, not the ARM9's. Not state.
pub struct Arm7View<'a> {
    memory: &'a mut Memory,
    // The ARM7's clock in master cycles, copied in before the step like the ARM9's TCM
    // configuration; timer registers read it (`docs/hw/timers.md`, "Model").
    now: u64,
}

impl<'a> Arm7View<'a> {
    /// Builds the view for one step taken at the ARM7 clock `now`, in master cycles.
    pub fn new(memory: &'a mut Memory, now: u64) -> Self {
        Self { memory, now }
    }
}

impl Bus for Arm7View<'_> {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.memory.fetch32_arm7(self.now, addr)
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.memory.fetch16_arm7(self.now, addr)
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.memory.read8_arm7_mut(self.now, addr)
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.memory.read16_arm7_mut(self.now, addr)
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.memory.read32_arm7_mut(self.now, addr)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.memory.write8_arm7(self.now, addr, value);
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.memory.write16_arm7(self.now, addr, value);
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.memory.write32_arm7(self.now, addr, value);
    }
}
