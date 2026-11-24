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
//! **Unstable until 1.0.** The names and structure of what is re-exported here may change in a
//! minor release, [`Inspect`] and [`Machine::inspect`] included. The debugger panels (#56-#59)
//! decide what becomes stable.
//!
//! Breakpoint and trace hooks (plan §4.1) are not here yet: the debugger is a parallel track, and
//! they join this module with its stepping and trace panels (#56, #60).

use crate::bus::{BANK_COUNT, TcmConfig};
use crate::cpu::Cpu;
use crate::input::Input;
use crate::machine::Machine;

pub use crate::bus::{ExtPaletteBanks, Textures, VramBankState, VramMapping, WramShare, WramState};
pub use crate::gpu2d::{
    AffineBg, AffineFormat, BrightnessMode, EffectMode, Effects, EngineId, Gpu2d, LayerSet,
    MasterBrightness, Mosaic, OBJ_COUNT, ObjAttrs, Targets, TextBg, WindowRect, Windows,
};
pub use crate::gpu3d::Gpu3d;
pub use crate::hle::SwiRecord;
pub use crate::io::DeviceId;
pub use crate::io::cart::Cart;
pub use crate::io::divsqrt::DivSqrt;
pub use crate::io::dma::Dma;
pub use crate::io::ipc::Ipc;
pub use crate::io::irq::InterruptController;
pub use crate::io::keypad::Keypad;
pub use crate::io::power::Power;
pub use crate::io::rtc::Rtc;
pub use crate::io::spi::Spi;
pub use crate::io::timers::Timers;
pub use crate::io::timing::Timing;
pub use crate::rtc::{Chip, Register, Session};
pub use crate::scheduler::{Event, EventKind, LineEvent, Scheduler};
pub use crate::spu::{Capture, Channel, Spu};

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
        ObjAttrs::decode(self.machine.memory.oam_of(which), index)
    }

    /// The input the keypad registers read: what `Machine::run_frame` was given, as latched when
    /// the last frame began.
    // The debugger's IO register panel will show it with the keypad registers.
    pub fn input(&self) -> Input {
        self.machine.io.input()
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

    /// The cartridge slot: EXMEMCNT, AUXSPICNT, ROMCTRL, the command and the transfer in progress.
    // The debugger's IO register panel (#56 CPU panel until one exists) shows ROMCTRL, AUXSPICNT,
    // the current command, address and remaining bytes, and which CPU owns the slot; #57 adds the
    // memory view that the data comes through.
    pub fn cart(&self) -> &'a Cart {
        self.machine.io.cart()
    }

    /// The ARM7's SPI bus: SPICNT, the selected device, busy and the firmware chip's command,
    /// address and status.
    // The debugger's IO register panel (#56 CPU panel until one exists) shows SPICNT, the selected
    // device, whether a unit is on the wire (`done_at`) and the firmware chip's command, address
    // and status; #57's memory view shows the firmware image through it.
    pub fn spi(&self) -> &'a Spi {
        self.machine.io.spi()
    }

    /// The ARM7's RTCCNT and the clock chip behind its pins.
    // The debugger's IO register panel (#56 CPU panel until one exists) shows RTCCNT, the chip's
    // transfer in progress, its status and alarm registers, and the clock as the host's time plus
    // the program's offset.
    pub fn rtc(&self) -> &'a Rtc {
        self.machine.io.rtc()
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
    // The debugger's IO register panel shows it with the other devices'; the key bits come from the
    // latched input (`Inspect::input`).
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

    /// The ARM7's sound registers: every channel's registers and end time, SOUNDCNT, SOUNDBIAS and
    /// the capture units.
    // No panel shows them yet: the debugger panels (the Parallel track) will add a sound panel.
    pub fn spu(&self) -> &'a Spu {
        self.machine.io.spu()
    }

    /// The 3D engine stub: DISP3DCNT, the matrix mode, the three stack pointers, the error flag and
    /// the command decoder's state.
    // No panel shows them yet: the debugger's 3D panels come with the geometry engine in Phase 7
    // (#53).
    pub fn gpu3d(&self) -> &'a Gpu3d {
        self.machine.io.gpu3d()
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

    /// Which bank backs each extended-palette slot of engine `which` (`extended-palettes.md`, "Which
    /// bank backs a slot"), from the VRAMCNT registers as they are now.
    // The debugger's VRAM bank map (#58) shows it beside the banks.
    pub fn ext_palette_banks(&self, which: EngineId) -> ExtPaletteBanks {
        self.machine.memory.ext_palette_banks(which == EngineId::B)
    }

    /// The texture image and palette slots as the rasterizer reads them (`vram-banks.md`), from the
    /// VRAMCNT registers as they are now.
    // The debugger's texture viewer (#58) reads them; the rasterizer (#50) reads them from `Memory`.
    pub fn textures(&self) -> Textures<'_> {
        self.machine.memory.textures()
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
            &self.machine.rom,
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
            &self.machine.rom,
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
