// SPDX-License-Identifier: Apache-2.0
//! The register devices both CPUs reach through the I/O slow path: engine A's display registers,
//! VCOUNT and DISPSTAT, the two interrupt controllers, IPC and the two sets of timers. They sit
//! apart from [`crate::bus::Memory`] (the RAM buffers and the page tables) so a bus view can
//! borrow memory, devices and the scheduler at once: a register write that changes a device's
//! schedule schedules it on the spot.
//!
//! The address map lives here and nowhere else: [`decode`] turns an address into the [`Port`]
//! that owns it and an offset into that device's own register space, once per access, from the
//! per-CPU tables [`ARM9_PORTS`] and [`ARM7_PORTS`]. A test pins that no two entries overlap.
//! The device behind a port speaks one interface, [`Device`], and `Io` calls the concrete type
//! with a `match`: static dispatch, no `dyn` (`AGENTS.md` §2 rule 3).
//!
//! Every IF bit a device raises goes through [`Io::request_irq`], the one path to the two
//! controllers. A device returns what it would raise in an [`Effect`]; it never reaches a
//! controller itself (`AGENTS.md` §4, "Per-CPU devices").

use crate::bus::VRAMCNT_A_ADDR;
use crate::gpu2d::timing::{DISPSTAT_ADDR, Timing};
use crate::gpu2d::{DISPCNT_A_ADDR, Gpu2d, POWCNT1_ADDR};
use crate::ipc::{IPCFIFORECV_ADDR, IPCSYNC_ADDR, Ipc};
use crate::irq::{IME_ADDR, InterruptController, Requests};
use crate::scheduler::Scheduler;
use crate::timers::{TIMER_COUNT, TM0CNT_L_ADDR, Timers};
use serde::{Deserialize, Serialize};

/// The width of one bus access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
    /// The width of an access of `N` bytes. The bus only makes 1, 2 and 4.
    pub(crate) const fn of<const N: usize>() -> Self {
        const { assert!(N == 1 || N == 2 || N == 4) };
        match N {
            1 => Self::Byte,
            2 => Self::Half,
            _ => Self::Word,
        }
    }

    /// Bytes in the access.
    pub(crate) const fn bytes(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }

    /// The bits of a `u32` the access carries.
    pub(crate) const fn mask(self) -> u32 {
        match self {
            Self::Byte => 0xFF,
            Self::Half => 0xFFFF,
            Self::Word => u32::MAX,
        }
    }
}

/// What a register access asks of its caller: the IF bits to raise, and which of the device's
/// event slots must be replaced. Every device returns this one type, so the caller handles an
/// access the same way whatever it reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effect {
    /// The IF bits to raise.
    pub(crate) requests: Requests,
    /// Bit n: the device's event slot n changed and its scheduler event must be replaced. The
    /// four timers are slots 0-3; DMA's four channels will take the same shape (#26).
    pub(crate) reschedule: u8,
}

impl Effect {
    /// An effect that only raises `requests`.
    pub(crate) fn raising(requests: Requests) -> Self {
        Self {
            requests,
            reschedule: 0,
        }
    }

    /// Both effects in one: the requests and the slots to reschedule of each.
    pub(crate) fn merge(self, other: Self) -> Self {
        Self {
            requests: self.requests.merge(other.requests),
            reschedule: self.reschedule | other.reschedule,
        }
    }
}

/// A register device. `offset` is into the device's own register space ([`Port::base`] is
/// offset 0) and `now` is the master time of the access. `ARM9` is the CPU asking: a device that
/// serves one CPU (or two copies of one) is implemented for that CPU's value alone
/// (`AGENTS.md` §4, "Per-CPU devices").
pub(crate) trait Device<const ARM9: bool> {
    /// What a read would return, with no side effect: the debugger's and the fetch's view.
    fn peek(&self, offset: u32, width: Width, now: u64) -> u32;

    /// A CPU's read. A register that a read changes (a FIFO that pops) overrides this; the
    /// others read as they peek.
    fn read(&mut self, offset: u32, width: Width, now: u64) -> (u32, Effect) {
        (self.peek(offset, width, now), Effect::default())
    }

    /// A CPU's write of `width` at `offset`.
    fn write(&mut self, offset: u32, width: Width, value: u32, now: u64) -> Effect;
}

/// A read of `width` at `offset` built from a device that holds registers one byte at a time:
/// each byte comes from the register at its own address, so a wide read spans neighbouring
/// registers. The default helper a byte-register device opts into.
// TODO(hw-unverified): a wider read is split into bytes and each comes from the register at its
// own address (`arm9-memory-map.md`, "Unverified").
pub(crate) fn read_bytes(offset: u32, width: Width, byte: impl Fn(u32) -> u8) -> u32 {
    (0..width.bytes()).fold(0, |value, i| {
        value | u32::from(byte(offset.wrapping_add(i))) << (8 * i)
    })
}

/// A write of `width` at `offset` split into bytes, each going to the register at its own
/// address in ascending order, and the effects merged. The counterpart of [`read_bytes`].
// TODO(hw-unverified): a wider write is split into bytes and each goes to the register at its
// own address (`arm9-memory-map.md`, "Unverified").
pub(crate) fn write_bytes(
    offset: u32,
    width: Width,
    value: u32,
    mut byte: impl FnMut(u32, u8) -> Effect,
) -> Effect {
    (0..width.bytes()).fold(Effect::default(), |effect, i| {
        effect.merge(byte(offset.wrapping_add(i), (value >> (8 * i)) as u8))
    })
}

/// Who owns an address. Memory-control registers are a port so the map is complete, but their
/// state stays in `Memory` (#109): the bus view sends that port to `Memory`, not to `Io`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Port {
    /// DISPCNT of engine A and POWCNT1 (two table entries, one device).
    Gpu2d,
    /// DISPSTAT and VCOUNT.
    Timing,
    /// IME, IE and IF.
    Irq,
    /// IPCSYNC, IPCFIFOCNT and IPCFIFOSEND.
    Ipc,
    /// IPCFIFORECV: its own entry, because it lives in a different block, but the same device.
    IpcRecv,
    /// The four timers.
    Timers,
    /// VRAMCNT_A..G, WRAMCNT (ARM9) and WRAMSTAT (ARM7): owned by `Memory`.
    MemoryControl,
}

impl Port {
    /// The address that is offset 0 of the device behind this port. A device with two table
    /// entries ([`Port::Gpu2d`]) or two ports ([`Port::Ipc`], [`Port::IpcRecv`]) has one offset
    /// space across them.
    pub(crate) const fn base(self) -> u32 {
        match self {
            Self::Gpu2d => DISPCNT_A_ADDR,
            Self::Timing => DISPSTAT_ADDR,
            Self::Irq => IME_ADDR,
            Self::Ipc | Self::IpcRecv => IPCSYNC_ADDR,
            Self::Timers => TM0CNT_L_ADDR,
            Self::MemoryControl => VRAMCNT_A_ADDR,
        }
    }
}

/// One entry of the address map: the first address, the length in bytes, and the owner.
pub(crate) type PortEntry = (u32, u32, Port);

/// What the ARM9 sees, in ascending address order. Every entry is a 4-byte-aligned block whose
/// length is a multiple of 4, so one aligned access never spans two entries. Holes inside an
/// entry belong to its device, which reads them 0 and drops writes to them.
// TODO(hw-unverified): the I/O block is not mirrored (`arm9-memory-map.md`, "Unverified").
// TODO(phase3): the I/O registers of the other blocks arrive with their issues (#26).
pub(crate) const ARM9_PORTS: [PortEntry; 8] = [
    (DISPCNT_A_ADDR, 4, Port::Gpu2d),
    (DISPSTAT_ADDR, 4, Port::Timing),
    (TM0CNT_L_ADDR, 16, Port::Timers),
    // IPCSYNC, IPCFIFOCNT and IPCFIFOSEND, 4 bytes each.
    (IPCSYNC_ADDR, 12, Port::Ipc),
    // IME, a hole, IE and IF.
    (IME_ADDR, 16, Port::Irq),
    // VRAMCNT_A..D and VRAMCNT_E, F, G and WRAMCNT.
    (VRAMCNT_A_ADDR, 8, Port::MemoryControl),
    (POWCNT1_ADDR, 4, Port::Gpu2d),
    (IPCFIFORECV_ADDR, 4, Port::IpcRecv),
];

/// What the ARM7 sees: the same blocks, without engine A, and WRAMSTAT in place of the ARM9's
/// memory-control block.
pub(crate) const ARM7_PORTS: [PortEntry; 6] = [
    (DISPSTAT_ADDR, 4, Port::Timing),
    (TM0CNT_L_ADDR, 16, Port::Timers),
    (IPCSYNC_ADDR, 12, Port::Ipc),
    (IME_ADDR, 16, Port::Irq),
    // WRAMSTAT sits in the second byte.
    (VRAMCNT_A_ADDR, 4, Port::MemoryControl),
    (IPCFIFORECV_ADDR, 4, Port::IpcRecv),
];

/// The port that owns `addr` for CPU `ARM9` and the offset of `addr` in that device's register
/// space, or `None` when no device holds it (it reads 0 and ignores writes).
// TODO(hw-unverified): an undefined I/O port reads 0 (GBATEK says so); unmapped memory is
// "undefined" there and reads 0 here (`arm9-memory-map.md`, "Unverified").
pub(crate) fn decode<const ARM9: bool>(addr: u32) -> Option<(Port, u32)> {
    let ports: &[PortEntry] = if ARM9 { &ARM9_PORTS } else { &ARM7_PORTS };
    ports.iter().find_map(|&(start, len, port)| {
        let within = addr.checked_sub(start).filter(|&d| d < len)?;
        Some((port, start.wrapping_sub(port.base()).wrapping_add(within)))
    })
}

/// When an access happens: the clock of the CPU making it, and the earliest time an event it
/// schedules may fire.
#[derive(Clone, Copy, Debug)]
pub(crate) struct At {
    /// Master time of the access, as the CPU's own clock has it.
    pub(crate) now: u64,
    /// An overflow event never lies before the end of the instruction that scheduled it
    /// (`timers.md`, "Model").
    pub(crate) floor: u64,
}

/// The register devices. Plain data; the machine owns one next to its `Memory`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Io {
    // Engine A's display registers, reached through the I/O slow path.
    gpu2d: Gpu2d,
    // VCOUNT and each CPU's DISPSTAT (`display-timing.md`).
    timing: Timing,
    // Each CPU's interrupt controller: the same addresses, separate state (`interrupts.md`).
    irq9: InterruptController<true>,
    irq7: InterruptController<false>,
    // IPCSYNC and the two IPC FIFOs (`ipc.md`).
    ipc: Ipc,
    // Each CPU's four timers: the same addresses, separate state (`timers.md`).
    timers9: Timers<true>,
    timers7: Timers<false>,
}

/// Replaces the overflow event of each timer in `reschedule` (bit n = timer n) with one at its
/// next overflow, if it has one. The event never lies before `floor`: a timer an ARM7 write
/// started while it lagged the master clock can compute a time already past, and the queue's
/// invariant (`scheduler.md`, "Savestate checks") holds events at `now` or later. The timer keeps
/// its own tick grid, so only the IRQ is late (`timers.md`, "Model").
fn schedule_timers<const ARM9: bool>(
    scheduler: &mut Scheduler,
    timers: &Timers<ARM9>,
    reschedule: u8,
    floor: u64,
) {
    (0..TIMER_COUNT)
        .filter(|&index| reschedule & (1 << index) != 0)
        .for_each(|index| {
            let kind = Timers::<ARM9>::event_kind(index);
            scheduler.cancel(kind);
            if let Some(time) = timers.overflow_time(index) {
                scheduler.schedule(time.max(floor), kind);
            }
        });
}

impl Io {
    /// The devices in their power-on state.
    pub fn new() -> Self {
        Self {
            gpu2d: Gpu2d::new(),
            timing: Timing::new(),
            irq9: InterruptController::new(),
            irq7: InterruptController::new(),
            ipc: Ipc::new(),
            timers9: Timers::new(),
            timers7: Timers::new(),
        }
    }

    /// Engine A's display registers.
    // The debugger's IO register panel will show them (#58 shows the VRAM view).
    pub fn gpu2d(&self) -> &Gpu2d {
        &self.gpu2d
    }

    /// VCOUNT and each CPU's DISPSTAT.
    pub fn timing(&self) -> &Timing {
        &self.timing
    }

    pub(crate) fn timing_mut(&mut self) -> &mut Timing {
        &mut self.timing
    }

    /// The ARM9's interrupt controller.
    pub fn irq9(&self) -> &InterruptController<true> {
        &self.irq9
    }

    /// The ARM7's interrupt controller.
    pub fn irq7(&self) -> &InterruptController<false> {
        &self.irq7
    }

    /// IPCSYNC and the two IPC FIFOs, for the debugger.
    pub fn ipc(&self) -> &Ipc {
        &self.ipc
    }

    /// The ARM9's timers.
    pub fn timers9(&self) -> &Timers<true> {
        &self.timers9
    }

    /// The ARM7's timers.
    pub fn timers7(&self) -> &Timers<false> {
        &self.timers7
    }

    /// Raises the requests in the two masks on the matching controllers: IF latches them. The
    /// one path every source uses: the LCD, IPC, the timers and (later) DMA.
    pub(crate) fn request_irq(&mut self, requests: Requests) {
        self.irq9.request(requests.arm9);
        self.irq7.request(requests.arm7);
    }

    /// What CPU `ARM9` would read from `port` at master time `now`, with no side effect. A
    /// FIFO shows the word a read would take, without taking it.
    // TODO(hw-unverified): IPCFIFORECV is not mirrored (`ipc.md`, "Unverified").
    pub(crate) fn peek<const ARM9: bool>(
        &self,
        port: Port,
        offset: u32,
        width: Width,
        now: u64,
    ) -> u32 {
        match port {
            Port::Gpu2d if ARM9 => Device::<true>::peek(&self.gpu2d, offset, width, now),
            Port::Timing => Device::<ARM9>::peek(&self.timing, offset, width, now),
            Port::Irq if ARM9 => Device::<true>::peek(&self.irq9, offset, width, now),
            Port::Irq => Device::<false>::peek(&self.irq7, offset, width, now),
            Port::Ipc | Port::IpcRecv => Device::<ARM9>::peek(&self.ipc, offset, width, now),
            Port::Timers if ARM9 => Device::<true>::peek(&self.timers9, offset, width, now),
            Port::Timers => Device::<false>::peek(&self.timers7, offset, width, now),
            // The ARM7 has no engine A, and `Memory` owns the memory-control registers.
            Port::Gpu2d | Port::MemoryControl => 0,
        }
    }

    /// A CPU's read from `port`: what it returns, with the effect applied (IF bits raised, events
    /// replaced). A read that takes a word out of a FIFO can raise the sender's requests.
    pub(crate) fn read<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        at: At,
        port: Port,
        offset: u32,
        width: Width,
    ) -> u32 {
        let (value, effect) = match port {
            Port::Gpu2d if ARM9 => Device::<true>::read(&mut self.gpu2d, offset, width, at.now),
            Port::Timing => Device::<ARM9>::read(&mut self.timing, offset, width, at.now),
            Port::Irq if ARM9 => Device::<true>::read(&mut self.irq9, offset, width, at.now),
            Port::Irq => Device::<false>::read(&mut self.irq7, offset, width, at.now),
            Port::Ipc | Port::IpcRecv => Device::<ARM9>::read(&mut self.ipc, offset, width, at.now),
            Port::Timers if ARM9 => Device::<true>::read(&mut self.timers9, offset, width, at.now),
            Port::Timers => Device::<false>::read(&mut self.timers7, offset, width, at.now),
            Port::Gpu2d | Port::MemoryControl => (0, Effect::default()),
        };
        self.apply::<ARM9>(scheduler, at, port, effect);
        value
    }

    /// A CPU's write of `value` to `port`, with the effect applied. A timer write that changes a
    /// schedule replaces that overflow event in `scheduler` at once, never earlier than
    /// `at.floor` (`timers.md`, "Model").
    pub(crate) fn write<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        at: At,
        port: Port,
        offset: u32,
        width: Width,
        value: u32,
    ) {
        let now = at.now;
        let effect = match port {
            Port::Gpu2d if ARM9 => {
                Device::<true>::write(&mut self.gpu2d, offset, width, value, now)
            }
            Port::Timing => Device::<ARM9>::write(&mut self.timing, offset, width, value, now),
            Port::Irq if ARM9 => Device::<true>::write(&mut self.irq9, offset, width, value, now),
            Port::Irq => Device::<false>::write(&mut self.irq7, offset, width, value, now),
            Port::Ipc | Port::IpcRecv => {
                Device::<ARM9>::write(&mut self.ipc, offset, width, value, now)
            }
            Port::Timers if ARM9 => {
                Device::<true>::write(&mut self.timers9, offset, width, value, now)
            }
            Port::Timers => Device::<false>::write(&mut self.timers7, offset, width, value, now),
            Port::Gpu2d | Port::MemoryControl => Effect::default(),
        };
        self.apply::<ARM9>(scheduler, at, port, effect);
    }

    /// Hands an access's effect on: its IF bits to the controllers, and the timers it names to
    /// the scheduler.
    fn apply<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        at: At,
        port: Port,
        effect: Effect,
    ) {
        self.request_irq(effect.requests);
        // Only the timers have event slots today; DMA (#26) extends this.
        debug_assert!(port == Port::Timers || effect.reschedule == 0);
        if port == Port::Timers {
            match ARM9 {
                true => schedule_timers(scheduler, &self.timers9, effect.reschedule, at.floor),
                false => schedule_timers(scheduler, &self.timers7, effect.reschedule, at.floor),
            }
        }
    }

    /// A timer's overflow event fired: it reloads, its IRQ is requested, the timer it feeds ticks
    /// and its next overflow is scheduled (`timers.md`, "Counting").
    pub(crate) fn timer_overflowed(&mut self, scheduler: &mut Scheduler, arm9: bool, index: u8) {
        let index = usize::from(index);
        // Every overflow of this timer due by now is processed, not just one: an event queued late
        // (an ARM7 write while it lagged) would otherwise catch up one overflow a round with
        // the CPUs standing still. The timer is rescheduled whether or not one was due.
        let now = scheduler.now();
        if arm9 {
            let effect = self.timers9.settle_timer(index, now);
            self.request_irq(effect.requests);
            schedule_timers(
                scheduler,
                &self.timers9,
                effect.reschedule | 1 << index,
                now,
            );
        } else {
            let effect = self.timers7.settle_timer(index, now);
            self.request_irq(effect.requests);
            schedule_timers(
                scheduler,
                &self.timers7,
                effect.reschedule | 1 << index,
                now,
            );
        }
    }

    /// Whether the devices hold only values that exist. A savestate that decodes to anything
    /// else is corrupt (`savestate`).
    pub(crate) fn is_well_formed(&self) -> bool {
        self.irq9.is_well_formed()
            && self.irq7.is_well_formed()
            && self.ipc.is_well_formed()
            && self.timing.is_well_formed()
            && self.timers9.is_well_formed()
            && self.timers7.is_well_formed()
    }

    /// Replaces the IPC block, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn replace_ipc_for_test(&mut self, ipc: Ipc) {
        self.ipc = ipc;
    }
}

impl Default for Io {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu2d::timing::VCOUNT_ADDR;
    use crate::ipc::{IPCFIFOCNT_ADDR, IPCFIFOSEND_ADDR};
    use crate::irq::{IE_ADDR, IF_ADDR, InterruptController, Requests};

    /// The WRAMCNT, WRAMSTAT and VRAMCNT_G addresses, which `Memory` keeps private.
    const VRAMCNT_G: u32 = 0x0400_0246;
    const WRAMCNT: u32 = 0x0400_0247;
    const WRAMSTAT: u32 = 0x0400_0241;

    /// The pairs of entries in `table` that share an address, as (first start, second start).
    fn overlapping_pairs(table: &[PortEntry]) -> Vec<(u32, u32)> {
        table
            .iter()
            .enumerate()
            .flat_map(|(i, &(a, a_len, _))| {
                table[i + 1..]
                    .iter()
                    .filter(move |&&(b, b_len, _)| a < b + b_len && b < a + a_len)
                    .map(move |&(b, ..)| (a, b))
            })
            .collect()
    }

    #[test]
    fn given_the_io_port_tables_when_compared_pairwise_then_no_two_ports_overlap() {
        assert_eq!(
            (
                overlapping_pairs(&ARM9_PORTS),
                overlapping_pairs(&ARM7_PORTS)
            ),
            (Vec::<(u32, u32)>::new(), Vec::<(u32, u32)>::new())
        );
    }

    /// The entries of `table` that break the shape the decoder relies on: a start or a length
    /// off the 4-byte grid, an empty block, or a start below its port's base.
    fn malformed_entries(table: &[PortEntry]) -> Vec<(u32, u32)> {
        table
            .iter()
            .filter(|&&(start, len, port)| {
                start % 4 != 0 || len % 4 != 0 || len == 0 || start < port.base()
            })
            .map(|&(start, len, _)| (start, len))
            .collect()
    }

    #[test]
    fn given_the_io_port_tables_when_each_entry_is_checked_then_blocks_are_aligned_and_above_their_base()
     {
        assert_eq!(
            (
                malformed_entries(&ARM9_PORTS),
                malformed_entries(&ARM7_PORTS)
            ),
            (Vec::<(u32, u32)>::new(), Vec::<(u32, u32)>::new())
        );
    }

    /// The cases where `decode` answers something other than `expected`, as (address, got).
    fn decode_failures<const ARM9: bool>(
        expected: &[(u32, Option<(Port, u32)>)],
    ) -> Vec<(u32, Option<(Port, u32)>)> {
        expected
            .iter()
            .filter(|&&(addr, want)| decode::<ARM9>(addr) != want)
            .map(|&(addr, _)| (addr, decode::<ARM9>(addr)))
            .collect()
    }

    #[test]
    fn given_each_arm9_register_address_when_decoded_then_the_expected_port_and_offset_answer() {
        let expected = [
            (DISPCNT_A_ADDR, Some((Port::Gpu2d, 0))),
            (DISPCNT_A_ADDR + 3, Some((Port::Gpu2d, 3))),
            (
                POWCNT1_ADDR,
                Some((Port::Gpu2d, POWCNT1_ADDR - DISPCNT_A_ADDR)),
            ),
            (DISPSTAT_ADDR, Some((Port::Timing, 0))),
            (VCOUNT_ADDR, Some((Port::Timing, 2))),
            (TM0CNT_L_ADDR, Some((Port::Timers, 0))),
            (TM0CNT_L_ADDR + 15, Some((Port::Timers, 15))),
            (IPCSYNC_ADDR, Some((Port::Ipc, 0))),
            (IPCFIFOCNT_ADDR, Some((Port::Ipc, 4))),
            (IPCFIFOSEND_ADDR, Some((Port::Ipc, 8))),
            (
                IPCFIFORECV_ADDR,
                Some((Port::IpcRecv, IPCFIFORECV_ADDR - IPCSYNC_ADDR)),
            ),
            (IME_ADDR, Some((Port::Irq, 0))),
            (IE_ADDR, Some((Port::Irq, 8))),
            (IF_ADDR, Some((Port::Irq, 12))),
            (VRAMCNT_A_ADDR, Some((Port::MemoryControl, 0))),
            (VRAMCNT_G, Some((Port::MemoryControl, 6))),
            (WRAMCNT, Some((Port::MemoryControl, 7))),
        ];
        assert_eq!(decode_failures::<true>(&expected), vec![]);
    }

    #[test]
    fn given_each_arm7_register_address_when_decoded_then_the_expected_port_and_offset_answer() {
        let expected = [
            (DISPSTAT_ADDR, Some((Port::Timing, 0))),
            (VCOUNT_ADDR, Some((Port::Timing, 2))),
            (TM0CNT_L_ADDR + 4, Some((Port::Timers, 4))),
            (IPCSYNC_ADDR, Some((Port::Ipc, 0))),
            (IPCFIFOSEND_ADDR, Some((Port::Ipc, 8))),
            (
                IPCFIFORECV_ADDR,
                Some((Port::IpcRecv, IPCFIFORECV_ADDR - IPCSYNC_ADDR)),
            ),
            (IME_ADDR, Some((Port::Irq, 0))),
            (IF_ADDR, Some((Port::Irq, 12))),
            (WRAMSTAT, Some((Port::MemoryControl, 1))),
        ];
        assert_eq!(decode_failures::<false>(&expected), vec![]);
    }

    #[test]
    fn given_arm9_only_registers_when_the_arm7_decodes_them_then_no_port_owns_them() {
        let expected = [
            (DISPCNT_A_ADDR, None),
            (POWCNT1_ADDR, None),
            (WRAMCNT, None),
            (VRAMCNT_G, None),
        ];
        assert_eq!(decode_failures::<false>(&expected), vec![]);
    }

    #[test]
    fn given_addresses_next_to_the_blocks_when_decoded_then_no_port_owns_them() {
        // Just below and above IME/IE/IF (208-217), the timers (100-10F), IPCSYNC..SEND
        // (180-18B), the 2D registers and the cart-side neighbour of IPCFIFORECV.
        let expected = [
            (0x0400_0207, None),
            (0x0400_0218, None),
            (0x0400_021B, None),
            (0x0400_0200, None),
            (0x0400_0300, None),
            (TM0CNT_L_ADDR - 1, None),
            (TM0CNT_L_ADDR + 16, None),
            (IPCSYNC_ADDR - 1, None),
            (IPCFIFOSEND_ADDR + 4, None),
            (IPCFIFORECV_ADDR + 4, None),
            (0x0400_0008, None),
            (0x0000_0000, None),
        ];
        assert_eq!(
            (
                decode_failures::<true>(&expected),
                decode_failures::<false>(&expected)
            ),
            (vec![], vec![])
        );
    }

    #[test]
    fn given_every_byte_of_ime_ie_and_if_when_decoded_then_the_irq_port_owns_it() {
        let unowned: Vec<u32> = (0..4)
            .flat_map(|i| [IME_ADDR + i, IE_ADDR + i, IF_ADDR + i])
            .filter(|&addr| decode::<true>(addr).map(|(port, _)| port) != Some(Port::Irq))
            .collect();
        assert_eq!(unowned, Vec::<u32>::new());
    }

    #[test]
    fn given_a_word_at_the_second_memory_control_block_when_decoded_then_one_port_owns_all_four_bytes()
     {
        // VRAMCNT_E/F/G and WRAMCNT share one aligned word, so one access never splits.
        let ports: Vec<Option<Port>> = (0..4)
            .map(|i| decode::<true>(0x0400_0244 + i).map(|(port, _)| port))
            .collect();
        assert_eq!(ports, vec![Some(Port::MemoryControl); 4]);
    }

    #[test]
    fn given_a_word_the_width_helpers_split_when_it_is_joined_again_then_it_round_trips() {
        let split = write_bytes(0, Width::Word, 0xDDCC_BBAA, |_, _| Effect::default());
        let joined = read_bytes(0, Width::Word, |at| [0xAA, 0xBB, 0xCC, 0xDD][at as usize]);
        assert_eq!((split, joined), (Effect::default(), 0xDDCC_BBAA));
    }

    #[test]
    fn given_a_halfword_write_when_split_then_each_byte_goes_to_its_own_offset_in_order() {
        let mut seen = Vec::new();
        write_bytes(2, Width::Half, 0xBEEF, |at, byte| {
            seen.push((at, byte));
            Effect::default()
        });
        assert_eq!(seen, vec![(2, 0xEF), (3, 0xBE)]);
    }

    #[test]
    fn given_two_effects_when_merged_then_requests_and_reschedule_are_ored() {
        let merged = Effect {
            requests: Requests { arm9: 1, arm7: 0 },
            reschedule: 0b01,
        }
        .merge(Effect {
            requests: Requests { arm9: 2, arm7: 4 },
            reschedule: 0b10,
        });
        assert_eq!(
            merged,
            Effect {
                requests: Requests { arm9: 3, arm7: 4 },
                reschedule: 0b11,
            }
        );
    }

    #[test]
    fn given_power_on_devices_when_checked_then_they_are_well_formed() {
        assert!(Io::new().is_well_formed());
    }

    #[test]
    fn given_an_ie_bit_outside_the_arm9_mask_when_checked_then_the_devices_are_not_well_formed() {
        let mut io = Io::new();
        io.irq9 = InterruptController::with_unchecked_ie(1 << 7);
        assert!(!io.is_well_formed());
    }

    #[test]
    fn given_requests_for_each_cpu_when_raised_then_each_controller_latches_only_its_own() {
        let mut io = Io::new();
        io.request_irq(Requests {
            arm9: 0b01,
            arm7: 0b10,
        });
        assert_eq!((io.irq9().if_(), io.irq7().if_()), (0b01, 0b10));
    }
}
