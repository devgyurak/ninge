// SPDX-License-Identifier: Apache-2.0
//! The ARM9 bus view: `Memory` plus the TCM configuration copied from CP15 before a step
//! (`docs/hw/arm9-memory-map.md`).

use super::Memory;
use crate::cpu::{Bus, Cp15, Tcm};

/// Both TCM regions as CP15 resolves them (`docs/hw/cp15.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcmConfig {
    /// Instruction TCM.
    pub itcm: Tcm,
    /// Data TCM.
    pub dtcm: Tcm,
}

impl TcmConfig {
    /// Copies both regions from CP15. A CP15 write therefore takes effect from the next step.
    pub fn from_cp15(cp15: &Cp15) -> Self {
        Self {
            itcm: cp15.itcm(),
            dtcm: cp15.dtcm(),
        }
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
        self.memory.read8(&self.tcm, addr)
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.memory.read16(&self.tcm, addr)
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.memory.read32(&self.tcm, addr)
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
