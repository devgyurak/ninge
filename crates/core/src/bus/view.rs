// SPDX-License-Identifier: Apache-2.0
//! The two CPUs' bus views: `Memory` for RAM, `Io` for the registers and the scheduler, borrowed
//! together for one step (`docs/hw/arm9-memory-map.md`, `docs/hw/wramcnt.md`). The ARM9 view adds
//! the TCM configuration copied from CP15 before the step.

use super::Memory;
use crate::cpu::{Bus, Cp15, Tcm};
use crate::io::{At, Io, Port, Width, decode};
use crate::scheduler::{ARM9_INSTRUCTION_COST, Scheduler};

/// Both TCM regions as CP15 resolves them (`docs/hw/cp15.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcmConfig {
    /// Instruction TCM.
    pub itcm: Tcm,
    /// Data TCM.
    pub dtcm: Tcm,
}

impl TcmConfig {
    /// No TCM at all: what the ARM7 has.
    pub const NONE: Self = Self {
        itcm: Tcm {
            base: 0,
            size: 0,
            enabled: false,
            load_mode: false,
        },
        dtcm: Tcm {
            base: 0,
            size: 0,
            enabled: false,
            load_mode: false,
        },
    };

    /// Copies both regions from CP15. A CP15 write therefore takes effect from the next step.
    pub fn from_cp15(cp15: &Cp15) -> Self {
        Self {
            itcm: cp15.itcm(),
            dtcm: cp15.dtcm(),
        }
    }
}

/// What CPU `ARM9` reads at `addr` without side effects: the peek the debugger and the runner
/// use. Memory answers first; what it does not hold is read from the register devices at master
/// time `now`, and IPCFIFORECV shows its next word without taking it. Nothing here needs the
/// scheduler.
pub(crate) fn peek<const ARM9: bool, const N: usize>(
    memory: &Memory,
    io: &Io,
    tcm: &TcmConfig,
    now: u64,
    addr: u32,
) -> [u8; N] {
    let addr = addr & !(N as u32 - 1);
    memory
        .load::<ARM9, N>(tcm, addr)
        .unwrap_or_else(|| read_registers::<ARM9, N>(memory, io, now, addr))
}

/// The `N` bytes of a register access, as the one value a device takes.
fn pack<const N: usize>(bytes: [u8; N]) -> u32 {
    bytes
        .iter()
        .enumerate()
        .fold(0, |value, (i, &byte)| value | u32::from(byte) << (8 * i))
}

/// A device's answer as the `N` bytes the CPU reads.
fn unpack<const N: usize>(value: u32) -> [u8; N] {
    std::array::from_fn(|i| value.wrapping_shr(8 * i as u32) as u8)
}

/// The register devices' answer for a read of `N` bytes at `addr`, with no side effect: the
/// address is decoded once and one device (or `Memory`, for the memory-control block) answers.
// TODO(phase3): the I/O registers of the other blocks arrive with their issues.
fn read_registers<const ARM9: bool, const N: usize>(
    memory: &Memory,
    io: &Io,
    now: u64,
    addr: u32,
) -> [u8; N] {
    let width = Width::of::<N>();
    unpack(match decode::<ARM9>(addr) {
        Some((Port::MemoryControl, offset)) => memory.read_control::<ARM9>(offset, width),
        Some((port, offset)) => io.peek::<ARM9>(port, offset, width, now),
        None => 0,
    })
}

/// One CPU's access to the bus for a step: the logic both views share, generic over which CPU.
struct Access<'a, const ARM9: bool> {
    memory: &'a mut Memory,
    io: &'a mut Io,
    scheduler: &'a mut Scheduler,
    tcm: TcmConfig,
    now: u64,
}

impl<const ARM9: bool> Access<'_, ARM9> {
    fn fetch<const N: usize>(&mut self, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        self.memory
            .fetch::<ARM9, N>(&self.tcm, addr)
            .unwrap_or_else(|| read_registers::<ARM9, N>(self.memory, self.io, self.now, addr))
    }

    /// A data read as the CPU does it: a read of IPCFIFORECV takes a word out of the FIFO and can
    /// set the error flag. [`peek`] is the read without that.
    fn read<const N: usize>(&mut self, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        self.memory
            .load::<ARM9, N>(&self.tcm, addr)
            .unwrap_or_else(|| self.read_registers(addr))
    }

    fn write<const N: usize>(&mut self, addr: u32, bytes: [u8; N]) {
        let addr = addr & !(N as u32 - 1);
        if !self.memory.store::<ARM9, N>(&self.tcm, addr, bytes) {
            self.write_registers(addr, bytes);
        }
    }

    /// The slow path for a read: the address is decoded once and one device answers. An address
    /// no device holds reads 0.
    fn read_registers<const N: usize>(&mut self, addr: u32) -> [u8; N] {
        let width = Width::of::<N>();
        unpack(match decode::<ARM9>(addr) {
            Some((Port::MemoryControl, offset)) => self.memory.read_control::<ARM9>(offset, width),
            Some((port, offset)) => {
                let at = self.at();
                self.io
                    .read::<ARM9>(self.scheduler, at, port, offset, width)
            }
            None => 0,
        })
    }

    /// The slow path for a write: the address is decoded once and one device takes the access
    /// (or `Memory`, for the memory-control block); an unmapped address ignores the write.
    // TODO(hw-unverified): a byte on no register is dropped (`arm9-memory-map.md`, "Unverified").
    fn write_registers<const N: usize>(&mut self, addr: u32, bytes: [u8; N]) {
        let width = Width::of::<N>();
        let value = pack(bytes);
        match decode::<ARM9>(addr) {
            Some((Port::MemoryControl, offset)) => {
                self.memory.write_control::<ARM9>(offset, width, value);
            }
            Some((port, offset)) => {
                let at = self.at();
                self.io
                    .write::<ARM9>(self.scheduler, at, port, offset, width, value);
            }
            None => {}
        }
    }

    /// The time of this access, and the earliest an event it schedules may fire. An overflow
    /// event never lies before the end of the instruction that scheduled it: on the ARM9 that is
    /// one instruction cost past the clock; the ARM7's step leaves the master clock where it is
    /// (`timers.md`, "Model").
    // TODO(hw-unverified): an ARM7 event is floored at the master clock, which the ARM9 has
    // already carried to the round's stop (`scheduler.md`, "Unverified").
    // TODO(#115): this assumes every ARM9 access is one fixed-cost instruction; a step that
    // returns its own cost must hand the floor in instead. A DMA unit's write takes the same
    // floor (`dma.md`, "Model"), which only matters for an event it queues.
    fn at(&self) -> At {
        let floor = if ARM9 {
            self.scheduler.now().saturating_add(ARM9_INSTRUCTION_COST)
        } else {
            self.scheduler.now()
        };
        At {
            now: self.now,
            floor,
        }
    }
}

/// The ARM9's bus: a short-lived view over `Memory`, `Io` and the scheduler, with the TCM
/// configuration copied from CP15. It is not state, so it may hold borrows (AGENTS.md §2 rule 5
/// covers state only).
///
/// `now` is the ARM9's clock in master cycles, as of the access: a timer register reads the
/// counter as of it (`docs/hw/timers.md`, "Model").
pub struct Arm9View<'a> {
    memory: &'a mut Memory,
    io: &'a mut Io,
    scheduler: &'a mut Scheduler,
    tcm: TcmConfig,
    now: u64,
}

impl<'a> Arm9View<'a> {
    /// Builds the view for one step taken at the ARM9 clock `now`, in master cycles.
    pub fn new(
        memory: &'a mut Memory,
        io: &'a mut Io,
        scheduler: &'a mut Scheduler,
        tcm: TcmConfig,
        now: u64,
    ) -> Self {
        Self {
            memory,
            io,
            scheduler,
            tcm,
            now,
        }
    }

    fn access(&mut self) -> Access<'_, true> {
        Access {
            memory: self.memory,
            io: self.io,
            scheduler: self.scheduler,
            tcm: self.tcm,
            now: self.now,
        }
    }
}

impl Bus for Arm9View<'_> {
    fn fetch32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.access().fetch(addr))
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes(self.access().fetch(addr))
    }
    fn read8(&mut self, addr: u32) -> u8 {
        u8::from_le_bytes(self.access().read(addr))
    }
    fn read16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes(self.access().read(addr))
    }
    fn read32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.access().read(addr))
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.access().write(addr, value.to_le_bytes());
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.access().write(addr, value.to_le_bytes());
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.access().write(addr, value.to_le_bytes());
    }
}

/// The ARM7's bus: the same shape as the ARM9's without a TCM. It reaches main RAM, the shared
/// WRAM it owns, its own WRAM and WRAMSTAT (`docs/hw/wramcnt.md`) through the ARM7's own page
/// table, and its own registers through `Io`. Not state.
///
/// `now` is the ARM7's clock in master cycles, copied in before the step; it runs apart from the
/// scheduler's (`docs/hw/scheduler.md`).
pub struct Arm7View<'a> {
    memory: &'a mut Memory,
    io: &'a mut Io,
    scheduler: &'a mut Scheduler,
    now: u64,
}

impl<'a> Arm7View<'a> {
    /// Builds the view for one step taken at the ARM7 clock `now`, in master cycles.
    pub fn new(
        memory: &'a mut Memory,
        io: &'a mut Io,
        scheduler: &'a mut Scheduler,
        now: u64,
    ) -> Self {
        Self {
            memory,
            io,
            scheduler,
            now,
        }
    }

    fn access(&mut self) -> Access<'_, false> {
        Access {
            memory: self.memory,
            io: self.io,
            scheduler: self.scheduler,
            tcm: TcmConfig::NONE,
            now: self.now,
        }
    }
}

impl Bus for Arm7View<'_> {
    fn fetch32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.access().fetch(addr))
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes(self.access().fetch(addr))
    }
    fn read8(&mut self, addr: u32) -> u8 {
        u8::from_le_bytes(self.access().read(addr))
    }
    fn read16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes(self.access().read(addr))
    }
    fn read32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.access().read(addr))
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.access().write(addr, value.to_le_bytes());
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.access().write(addr, value.to_le_bytes());
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.access().write(addr, value.to_le_bytes());
    }
}
