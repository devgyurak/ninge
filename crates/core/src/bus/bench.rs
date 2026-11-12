// SPDX-License-Identifier: Apache-2.0
//! Test fixture: a `Memory`, an `Io` and a `Scheduler` bundled the way the machine owns them, with
//! the two bus views built on demand. The accessors keep the shape the bus tests were written
//! against (`read32(&tcm, addr)` for the peek, `write32_arm7(now, addr, value)` through the
//! ARM7's view), so each case stays the scenario it was.

use super::{Arm7View, Arm9View, Memory, Region, TcmConfig, VramBankState, WramState};
use crate::cpu::{ARM9_INSTRUCTION_COST, Bus};
use crate::io::Io;
use crate::io::irq::InterruptController;
use crate::scheduler::Scheduler;

/// The three owners a bus view borrows.
pub(crate) struct Bench {
    pub(crate) memory: Memory,
    pub(crate) io: Io,
    pub(crate) scheduler: Scheduler,
}

impl Bench {
    /// Everything in its power-on state, the clock at 0.
    pub(crate) fn new() -> Self {
        Self {
            memory: Memory::new(),
            io: Io::new(),
            scheduler: Scheduler::new(),
        }
    }

    /// The ARM9's view with `tcm`, at the master clock.
    pub(crate) fn arm9(&mut self, tcm: TcmConfig) -> Arm9View<'_> {
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

    /// The ARM7's view at its own clock `now`.
    pub(crate) fn arm7(&mut self, now: u64) -> Arm7View<'_> {
        let floor = self.scheduler.now();
        Arm7View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            now,
            floor,
        )
    }

    fn peek9<const N: usize>(&self, tcm: &TcmConfig, addr: u32) -> [u8; N] {
        super::peek::<true, N>(&self.memory, &self.io, tcm, self.scheduler.now(), addr)
    }

    fn peek7<const N: usize>(&self, now: u64, addr: u32) -> [u8; N] {
        super::peek::<false, N>(&self.memory, &self.io, &TcmConfig::NONE, now, addr)
    }

    pub(crate) fn read8(&self, tcm: &TcmConfig, addr: u32) -> u8 {
        u8::from_le_bytes(self.peek9(tcm, addr))
    }
    pub(crate) fn read16(&self, tcm: &TcmConfig, addr: u32) -> u16 {
        u16::from_le_bytes(self.peek9(tcm, addr))
    }
    pub(crate) fn read32(&self, tcm: &TcmConfig, addr: u32) -> u32 {
        u32::from_le_bytes(self.peek9(tcm, addr))
    }
    pub(crate) fn read8_arm7(&self, now: u64, addr: u32) -> u8 {
        u8::from_le_bytes(self.peek7(now, addr))
    }
    pub(crate) fn read16_arm7(&self, now: u64, addr: u32) -> u16 {
        u16::from_le_bytes(self.peek7(now, addr))
    }
    pub(crate) fn read32_arm7(&self, now: u64, addr: u32) -> u32 {
        u32::from_le_bytes(self.peek7(now, addr))
    }

    pub(crate) fn fetch16(&mut self, tcm: &TcmConfig, addr: u32) -> u16 {
        self.arm9(*tcm).fetch16(addr)
    }
    pub(crate) fn fetch32(&mut self, tcm: &TcmConfig, addr: u32) -> u32 {
        self.arm9(*tcm).fetch32(addr)
    }
    pub(crate) fn fetch16_arm7(&mut self, now: u64, addr: u32) -> u16 {
        self.arm7(now).fetch16(addr)
    }
    pub(crate) fn fetch32_arm7(&mut self, now: u64, addr: u32) -> u32 {
        self.arm7(now).fetch32(addr)
    }

    pub(crate) fn write8(&mut self, tcm: &TcmConfig, addr: u32, value: u8) {
        self.arm9(*tcm).write8(addr, value);
    }
    pub(crate) fn write16(&mut self, tcm: &TcmConfig, addr: u32, value: u16) {
        self.arm9(*tcm).write16(addr, value);
    }
    pub(crate) fn write32(&mut self, tcm: &TcmConfig, addr: u32, value: u32) {
        self.arm9(*tcm).write32(addr, value);
    }
    pub(crate) fn write8_arm7(&mut self, now: u64, addr: u32, value: u8) {
        self.arm7(now).write8(addr, value);
    }
    pub(crate) fn write16_arm7(&mut self, now: u64, addr: u32, value: u16) {
        self.arm7(now).write16(addr, value);
    }
    pub(crate) fn write32_arm7(&mut self, now: u64, addr: u32, value: u32) {
        self.arm7(now).write32(addr, value);
    }

    pub(crate) fn region(&self, addr: u32) -> Region {
        self.memory.region(addr)
    }
    pub(crate) fn region7(&self, addr: u32) -> Region {
        self.memory.region7(addr)
    }
    pub(crate) fn vram_a(&self) -> VramBankState {
        self.memory.vram_a()
    }
    pub(crate) fn vram_banks(&self) -> [VramBankState; 9] {
        self.memory.vram_banks()
    }
    pub(crate) fn wramcnt(&self) -> WramState {
        self.memory.wramcnt()
    }
    pub(crate) fn irq9(&self) -> &InterruptController<true> {
        self.io.irq9()
    }
    pub(crate) fn irq7(&self) -> &InterruptController<false> {
        self.io.irq7()
    }
}
