// SPDX-License-Identifier: Apache-2.0
//! The read-only view of a [`Machine`] for a debugger: [`Inspect`]. A host calls
//! [`Machine::inspect`] and reads devices, CPUs, the scheduler and memory through it; nothing on
//! it changes the machine, so a panel cannot break an invariant the core keeps.
//!
//! `Inspect` hands out references to the devices themselves, not copies of their state: no
//! snapshot structs to keep in step with the devices, and no allocation (`pending` aside, which
//! is a debugger call, not the hot path). The device types are re-exported here so a host can
//! name what `Inspect` returns; their mutating methods are `pub(crate)`.
//!
//! Breakpoint and trace hooks (plan §4.1) are not here yet: the debugger is a parallel track, and
//! they join this module with its stepping and trace panels (#56, #60).

use crate::bus::{BANK_COUNT, TcmConfig};
use crate::cpu::Cpu;
use crate::machine::Machine;
use crate::machine::OAM_HALF;

pub use crate::bus::{VramBankState, VramMapping, WramShare, WramState};
pub use crate::gpu2d::{
    AffineBg, AffineFormat, EngineId, Gpu2d, Mosaic, OBJ_COUNT, ObjAttrs, TextBg,
};
pub use crate::hle::SwiRecord;
pub use crate::io::divsqrt::DivSqrt;
pub use crate::io::dma::Dma;
pub use crate::io::ipc::Ipc;
pub use crate::io::irq::InterruptController;
pub use crate::io::keypad::Keypad;
pub use crate::io::power::Power;
pub use crate::io::timers::Timers;
pub use crate::io::timing::Timing;
pub use crate::scheduler::{Event, EventKind, Scheduler};

/// A read-only view of one [`Machine`], from [`Machine::inspect`]. It borrows the machine, so the
/// machine cannot run while a view of it is alive.
#[derive(Clone, Copy, Debug)]
pub struct Inspect<'a> {
    machine: &'a Machine,
}

impl Machine {
    /// A read-only view of the machine's CPUs, devices, scheduler and memory for a debugger.
    pub fn inspect(&self) -> Inspect<'_> {
        Inspect { machine: self }
    }
}

impl<'a> Inspect<'a> {
    /// The ARM9 core.
    // The debugger's CPU panel (#56) will show it.
    pub fn arm9(&self) -> &'a Cpu<true> {
        &self.machine.arm9
    }

    /// The ARM7 core.
    // The debugger's CPU panel (#56) will show it.
    pub fn arm7(&self) -> &'a Cpu<false> {
        &self.machine.arm7
    }

    /// The ARM7's clock in master cycles (ARM9 cycles).
    // The debugger's scheduler event timeline (#59) will show it beside the master clock.
    pub fn arm7_clock(&self) -> u64 {
        self.machine.arm7_clock
    }

    /// What the HLE BIOS recorded of the ARM9's SWIs: the last number and the last one no
    /// routine implements.
    // The debugger's CPU panel (#56) will show it.
    pub fn swi_record9(&self) -> &'a SwiRecord {
        &self.machine.bios.arm9
    }

    /// What the HLE BIOS recorded of the ARM7's SWIs.
    // The debugger's CPU panel (#56) will show it.
    pub fn swi_record7(&self) -> &'a SwiRecord {
        &self.machine.bios.arm7
    }

    /// The master clock and the pending events ([`Scheduler::pending`]).
    // The debugger's scheduler event timeline (#59) will show it.
    pub fn scheduler(&self) -> &'a Scheduler {
        &self.machine.scheduler
    }

    /// VCOUNT: the current line, 0-262 (`docs/hw/display-timing.md`).
    // The debugger's IO register panel and the scheduler timeline (#59) will show it.
    pub fn vcount(&self) -> u16 {
        self.machine.io.timing().vcount()
    }

    /// DISPSTAT as the ARM9 reads it.
    // The debugger's IO register panel will show it.
    pub fn dispstat9(&self) -> u16 {
        self.machine.io.timing().dispstat::<true>()
    }

    /// DISPSTAT as the ARM7 reads it.
    // The debugger's IO register panel will show it.
    pub fn dispstat7(&self) -> u16 {
        self.machine.io.timing().dispstat::<false>()
    }

    /// VCOUNT and each CPU's DISPSTAT, with the line and flag logic behind them.
    // The debugger's IO register panel will show it.
    pub fn timing(&self) -> &'a Timing {
        self.machine.io.timing()
    }

    /// Engine A's display registers.
    // The debugger's IO register panel will show them (#58 shows the VRAM view).
    pub fn gpu2d(&self) -> &'a Gpu2d {
        self.machine.io.gpu2d()
    }

    /// OAM entry `index` (0-127) of engine `which`, decoded (`obj.md`, "The object table"); an
    /// index past 127 is taken modulo 128.
    // The debugger's OAM view (#58) shows it.
    pub fn obj(&self, which: EngineId, index: usize) -> ObjAttrs {
        let (oam_a, oam_b) = self.machine.memory.oam().split_at(OAM_HALF);
        ObjAttrs::decode(
            match which {
                EngineId::A => oam_a,
                EngineId::B => oam_b,
            },
            index,
        )
    }

    /// The ARM9's interrupt controller (IME, IE, IF).
    // The debugger's CPU panel (#56) shows it beside the ARM9's registers.
    pub fn irq9(&self) -> &'a InterruptController<true> {
        self.machine.io.irq9()
    }

    /// The ARM7's interrupt controller (IME, IE, IF).
    // The debugger's CPU panel (#56) shows it beside the ARM7's registers.
    pub fn irq7(&self) -> &'a InterruptController<false> {
        self.machine.io.irq7()
    }

    /// IPCSYNC and the two IPC FIFOs.
    // The debugger's IPC panel (#56 CPU panel until one exists) shows the sync nibbles, both
    // FIFOs' contents and the flags of both CPUs.
    pub fn ipc(&self) -> &'a Ipc {
        self.machine.io.ipc()
    }

    /// The ARM9's four timers.
    // The debugger's IO register panel (#56 CPU panel until one exists) shows each timer's
    // counter, reload and control; the scheduler timeline (#59) shows their overflow events.
    pub fn timers9(&self) -> &'a Timers<true> {
        self.machine.io.timers9()
    }

    /// The ARM7's four timers.
    // The debugger's IO register panel shows them with the ARM9's; the scheduler timeline (#59)
    // shows their overflow events.
    pub fn timers7(&self) -> &'a Timers<false> {
        self.machine.io.timers7()
    }

    /// The ARM9's four DMA channels.
    // The debugger's IO register panel (#56 CPU panel until one exists) shows each channel's
    // registers and whether a start is waiting; the scheduler timeline (#59) shows the starts.
    pub fn dma9(&self) -> &'a Dma<true> {
        self.machine.io.dma9()
    }

    /// The ARM7's four DMA channels.
    // The debugger's IO register panel shows them with the ARM9's.
    pub fn dma7(&self) -> &'a Dma<false> {
        self.machine.io.dma7()
    }

    /// The ARM9's divider and square-root unit.
    // The debugger's IO register panel shows its registers with the other devices'.
    pub fn divsqrt(&self) -> &'a DivSqrt {
        self.machine.io.divsqrt()
    }

    /// The ARM9's KEYCNT.
    // The debugger's IO register panel shows it with the other devices'; the key bits are
    // constants until input exists (`keypad.md`, #44).
    pub fn keypad9(&self) -> &'a Keypad<true> {
        self.machine.io.keypad9()
    }

    /// The ARM7's KEYCNT.
    // The debugger's IO register panel shows it with the ARM9's.
    pub fn keypad7(&self) -> &'a Keypad<false> {
        self.machine.io.keypad7()
    }

    /// The ARM7's HALTCNT.
    // The debugger's CPU panel (#56) will show whether the ARM7 asked to halt.
    pub fn power(&self) -> &'a Power {
        self.machine.io.power()
    }

    /// VRAMCNT_A as decoded.
    // The debugger's VRAM bank map (#58) will show it.
    pub fn vram_a(&self) -> VramBankState {
        self.machine.memory.vram_a()
    }

    /// All nine VRAMCNT registers as decoded, bank A first.
    // The debugger's VRAM bank map (#58) shows them with the mapping each one means.
    pub fn vram_banks(&self) -> [VramBankState; BANK_COUNT] {
        self.machine.memory.vram_banks()
    }

    /// WRAMCNT as decoded: which 16 KiB halves of the shared WRAM each CPU owns.
    // The debugger's memory viewer (#57) shows each CPU's window from it.
    pub fn wramcnt(&self) -> WramState {
        self.machine.memory.wramcnt()
    }

    /// Reads `N` bytes as the ARM9 data bus would, without side effects: IPCFIFORECV shows its
    /// next word without taking it, and nothing needs the scheduler.
    fn arm9_peek<const N: usize>(&self, addr: u32) -> [u8; N] {
        crate::bus::peek::<true, N>(
            &self.machine.memory,
            &self.machine.io,
            &TcmConfig::from_cp15(self.machine.arm9.cp15()),
            self.machine.scheduler.now(),
            addr,
        )
    }

    /// Reads `N` bytes as the ARM7 data bus would at its clock `now`, without side effects. The
    /// ARM7 has no TCM.
    fn arm7_peek<const N: usize>(&self, now: u64, addr: u32) -> [u8; N] {
        crate::bus::peek::<false, N>(
            &self.machine.memory,
            &self.machine.io,
            &TcmConfig::NONE,
            now,
            addr,
        )
    }

    /// Reads a byte as the ARM9 data bus would, without side effects.
    // The debugger's memory viewer (#57) and the runner's boot memory-dump test use it.
    pub fn arm9_peek8(&self, addr: u32) -> u8 {
        u8::from_le_bytes(self.arm9_peek(addr))
    }

    /// Reads a halfword as the ARM9 data bus would, without side effects.
    // The debugger's memory viewer (#57) and the runner's boot memory-dump test use it.
    pub fn arm9_peek16(&self, addr: u32) -> u16 {
        u16::from_le_bytes(self.arm9_peek(addr))
    }

    /// Reads a word as the ARM9 data bus would, without side effects.
    // The debugger's memory viewer (#57) and the runner's boot memory-dump test use it.
    pub fn arm9_peek32(&self, addr: u32) -> u32 {
        u32::from_le_bytes(self.arm9_peek(addr))
    }

    /// Reads a byte as the ARM7 would at its clock `now` (a timer reads as of that time),
    /// without side effects.
    // The debugger's memory viewer (#57) will show the ARM7's view.
    pub fn arm7_peek8(&self, now: u64, addr: u32) -> u8 {
        u8::from_le_bytes(self.arm7_peek(now, addr))
    }

    /// Reads a halfword as the ARM7 would at its clock `now`, without side effects.
    // The debugger's memory viewer (#57) will show the ARM7's view.
    pub fn arm7_peek16(&self, now: u64, addr: u32) -> u16 {
        u16::from_le_bytes(self.arm7_peek(now, addr))
    }

    /// Reads a word as the ARM7 would at its clock `now`, without side effects.
    // The debugger's memory viewer (#57) will show the ARM7's view.
    pub fn arm7_peek32(&self, now: u64, addr: u32) -> u32 {
        u32::from_le_bytes(self.arm7_peek(now, addr))
    }
}

#[cfg(test)]
mod tests;
