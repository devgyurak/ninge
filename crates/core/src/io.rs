// SPDX-License-Identifier: Apache-2.0
//! The register devices both CPUs reach through the I/O slow path: engine A's display registers,
//! VCOUNT and DISPSTAT, the two interrupt controllers, IPC and the two sets of timers and of DMA
//! channels. They sit
//! apart from [`crate::bus::Memory`] (the RAM buffers and the page tables) so a bus view can
//! borrow memory, devices and the scheduler at once: a register write that changes a device's
//! schedule schedules it on the spot.
//!
//! The address map lives here and nowhere else: [`decode`] turns an address into the [`Port`]
//! that owns it and an offset into that device's own register space, once per access, from the
//! per-CPU tables [`ARM9_PORTS`] and [`ARM7_PORTS`]. A test pins that no two entries overlap.
//! The device behind a port speaks one interface, [`Device`], and `Io` calls the concrete type
//! through one `match` that lives in `device_call!`: static dispatch, no `dyn` (`AGENTS.md` §2
//! rule 3).
//!
//! A device whose registers own scheduler events (the timers, the DMA channels) names them through
//! the same interface: a slot is one event of the device, [`Device::slot_kind`] gives its
//! [`EventKind`], [`Device::slot_due`] the time it is next due and [`Device::slot_fire`] what
//! happens when it fires. `Io` replaces the events of the slots an [`Effect`] names and fires them
//! ([`Io::fire`]) without knowing which device it is.
//!
//! Every IF bit a device raises goes through [`Io::request_irq`], the one path to the two
//! controllers. A device returns what it would raise in an [`Effect`]; it never reaches a
//! controller itself (`AGENTS.md` §4, "Per-CPU devices").

use crate::bus::VRAMCNT_A_ADDR;
use crate::dma::{
    CHANNELS_BYTES, DMA0FILL_ADDR, DMA0SAD_ADDR, Dma, End, FILL_BYTES, Transfer, Trigger,
};
use crate::gpu2d::timing::{DISPSTAT_ADDR, Timing};
use crate::gpu2d::{DISPCNT_A_ADDR, Gpu2d, POWCNT1_ADDR};
use crate::ipc::{IPCFIFORECV_ADDR, IPCSYNC_ADDR, Ipc};
use crate::irq::{IME_ADDR, InterruptController, Requests};
use crate::scheduler::{EventKind, Scheduler};
use crate::timers::{TM0CNT_L_ADDR, Timers};
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
    /// four timers are slots 0-3 of their device, and so are the four DMA channels.
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

    /// The scheduler event that carries event slot `slot`, or `None` when the device has no such
    /// slot (a device with no events has none).
    fn slot_kind(&self, _slot: u8) -> Option<EventKind> {
        None
    }

    /// The master time slot `slot`'s event is next due, or `None` when it has none. `Io` never
    /// queues it before the earliest time the access allows ([`At::floor`]), so a time already
    /// past, or 0, means "as soon as allowed".
    fn slot_due(&self, _slot: u8) -> Option<u64> {
        None
    }

    /// Slot `slot`'s event fired at master time `now`. Whatever it changes comes back in the
    /// [`Fired`], the slot's own event included: `Io` does not assume it is replaced.
    fn slot_fire(&mut self, _slot: u8, _now: u64) -> Fired {
        Fired::default()
    }
}

/// What a fired event asks of its caller: an [`Effect`], like a register access, and for DMA the
/// transfer the machine must now move through the owning CPU's bus, which a device cannot reach.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fired {
    /// The IF bits to raise and the slots to reschedule.
    pub(crate) effect: Effect,
    /// A transfer to run, from a DMA start.
    pub(crate) transfer: Option<Transfer>,
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
    /// The four DMA channels' address, count and control registers.
    Dma,
    /// The ARM9's DMA fill registers: their own entry, because they live in a different block,
    /// but the same device as [`Port::Dma`].
    DmaFill,
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
            Self::Dma | Self::DmaFill => DMA0SAD_ADDR,
            Self::MemoryControl => VRAMCNT_A_ADDR,
        }
    }
}

/// `&$place` for `ref` and `&mut $place` for `mut`: lets [`device_call`] borrow a device either way.
macro_rules! borrow {
    (ref $place:expr) => {
        &$place
    };
    (mut $place:expr) => {
        &mut $place
    };
}

/// The one place a [`Port`] meets its device for one CPU: calls `Device::<cpu>::$method` on the
/// field `$io` keeps for `$port`, borrowing it as `$b` says (`ref` or `mut`), and evaluates to
/// the call's result. `$arm9` is the const generic naming the CPU that asks. A port the CPU's
/// `Io` does not own (the memory-control block, or engine A seen from the ARM7) is a bug in the
/// caller: it asserts in a debug build and evaluates to `$absent`.
///
/// A macro and not a function because each arm calls a different concrete device type. It is
/// still static dispatch: every arm is a monomorphized call, no `dyn` (`AGENTS.md` §2 rule 3).
/// A new device adds its arms here once and every access path picks it up.
macro_rules! device_call {
    ($io:expr, $arm9:ident, $port:expr, $b:tt, $method:ident($($arg:expr),*), $absent:expr) => {
        match $port {
            Port::Gpu2d if $arm9 => Device::<true>::$method(borrow!($b $io.gpu2d) $(, $arg)*),
            Port::Timing => Device::<$arm9>::$method(borrow!($b $io.timing) $(, $arg)*),
            Port::Irq if $arm9 => Device::<true>::$method(borrow!($b $io.irq9) $(, $arg)*),
            Port::Irq => Device::<false>::$method(borrow!($b $io.irq7) $(, $arg)*),
            Port::Ipc | Port::IpcRecv => {
                Device::<$arm9>::$method(borrow!($b $io.ipc) $(, $arg)*)
            }
            Port::Timers if $arm9 => Device::<true>::$method(borrow!($b $io.timers9) $(, $arg)*),
            Port::Timers => Device::<false>::$method(borrow!($b $io.timers7) $(, $arg)*),
            Port::Dma | Port::DmaFill if $arm9 => {
                Device::<true>::$method(borrow!($b $io.dma9) $(, $arg)*)
            }
            Port::Dma | Port::DmaFill => Device::<false>::$method(borrow!($b $io.dma7) $(, $arg)*),
            Port::Gpu2d | Port::MemoryControl => {
                debug_assert!(false, "port not reachable through Io");
                $absent
            }
        }
    };
}

/// One entry of the address map: the first address, the length in bytes, and the owner.
pub(crate) type PortEntry = (u32, u32, Port);

/// What the ARM9 sees, in ascending address order. Every entry is a 4-byte-aligned block whose
/// length is a multiple of 4, so one aligned access never spans two entries. Holes inside an
/// entry belong to its device, which reads them 0 and drops writes to them.
// TODO(hw-unverified): the I/O block is not mirrored (`arm9-memory-map.md`, "Unverified").
// TODO(phase3): the I/O registers of the other blocks arrive with their issues.
pub(crate) const ARM9_PORTS: [PortEntry; 10] = [
    (DISPCNT_A_ADDR, 4, Port::Gpu2d),
    (DISPSTAT_ADDR, 4, Port::Timing),
    // DMA0SAD..DMA3CNT, twelve bytes a channel, then the fill registers.
    (DMA0SAD_ADDR, CHANNELS_BYTES, Port::Dma),
    (DMA0FILL_ADDR, FILL_BYTES, Port::DmaFill),
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
pub(crate) const ARM7_PORTS: [PortEntry; 7] = [
    (DISPSTAT_ADDR, 4, Port::Timing),
    (DMA0SAD_ADDR, CHANNELS_BYTES, Port::Dma),
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
    /// An event never lies before the end of the instruction that scheduled it (`timers.md`,
    /// "Model"; `dma.md`, "Model").
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
    // Each CPU's four DMA channels: the same addresses, separate state (`dma.md`).
    dma9: Dma<true>,
    dma7: Dma<false>,
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
            dma9: Dma::new(),
            dma7: Dma::new(),
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

    /// The ARM9's DMA channels.
    // The debugger's IO register panel (#56 CPU panel until one exists) shows each channel's
    // registers and whether a start is waiting; the scheduler timeline (#59) shows the starts.
    pub fn dma9(&self) -> &Dma<true> {
        &self.dma9
    }

    /// The ARM7's DMA channels.
    // The debugger's IO register panel shows them with the ARM9's.
    pub fn dma7(&self) -> &Dma<false> {
        &self.dma7
    }

    /// Raises the requests in the two masks on the matching controllers: IF latches them. The
    /// one path every source uses: the LCD, IPC, the timers and DMA.
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
        device_call!(self, ARM9, port, ref, peek(offset, width, now), 0)
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
        let (value, effect) = device_call!(
            self,
            ARM9,
            port,
            mut,
            read(offset, width, at.now),
            (0, Effect::default())
        );
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
        let effect = device_call!(
            self,
            ARM9,
            port,
            mut,
            write(offset, width, value, at.now),
            Effect::default()
        );
        self.apply::<ARM9>(scheduler, at, port, effect);
    }

    /// Hands an access's effect on: its IF bits to the controllers, and the event slots it names
    /// to the scheduler.
    fn apply<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        at: At,
        port: Port,
        effect: Effect,
    ) {
        self.request_irq(effect.requests);
        if effect.reschedule != 0 {
            self.schedule_slots::<ARM9>(scheduler, port, effect.reschedule, at.floor);
        }
    }

    /// Replaces the event of each slot of `port`'s device in `mask` (bit n = slot n) with one at
    /// the time the slot is next due, if it has one. The event never lies before `floor`: a
    /// timer an ARM7 write started while it lagged the master clock can compute a time already
    /// past, and the queue's invariant (`scheduler.md`, "Savestate checks") holds events at `now`
    /// or later. The timer keeps its own tick grid, so only the IRQ is late (`timers.md`,
    /// "Model"). Slots are replaced in ascending order, so equal times fire in that order.
    fn schedule_slots<const ARM9: bool>(
        &self,
        scheduler: &mut Scheduler,
        port: Port,
        mask: u8,
        floor: u64,
    ) {
        (0..u8::BITS as u8)
            .filter(|slot| mask & (1 << slot) != 0)
            .for_each(|slot| {
                let kind = device_call!(self, ARM9, port, ref, slot_kind(slot), None);
                if let Some(kind) = kind {
                    scheduler.cancel(kind);
                    let due = device_call!(self, ARM9, port, ref, slot_due(slot), None);
                    if let Some(time) = due {
                        scheduler.schedule(time.max(floor), kind);
                    }
                }
            });
    }

    /// A device's event fired: it does what the device says ([`Device::slot_fire`]), its IF bits
    /// are raised and its slots rescheduled from the master clock. A DMA start comes back as the
    /// transfer the machine must run. An event no device owns (a line or HBlank start) does
    /// nothing here.
    pub(crate) fn fire(&mut self, scheduler: &mut Scheduler, kind: EventKind) -> Option<Transfer> {
        let (arm9, port, slot) = match kind {
            EventKind::TimerOverflow { arm9, index } => (arm9, Port::Timers, index),
            EventKind::DmaStart { arm9, channel } => (arm9, Port::Dma, channel),
            EventKind::LineStart | EventKind::HBlankStart => return None,
        };
        // The one place the event's runtime CPU becomes the const generic.
        if arm9 {
            self.fire_slot::<true>(scheduler, port, slot)
        } else {
            self.fire_slot::<false>(scheduler, port, slot)
        }
    }

    fn fire_slot<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        port: Port,
        slot: u8,
    ) -> Option<Transfer> {
        let now = scheduler.now();
        let fired = device_call!(
            self,
            ARM9,
            port,
            mut,
            slot_fire(slot, now),
            Fired::default()
        );
        self.apply::<ARM9>(scheduler, At { now, floor: now }, port, fired.effect);
        fired.transfer
    }

    /// `trigger` happened at master time `time`: every channel of both CPUs that waits for it is
    /// queued to start, the ARM9's first, each in channel order. The event is never before the
    /// master clock (`scheduler.md`, "Savestate checks").
    // TODO(hw-unverified): the ARM9's channels go before the ARM7's (`dma.md`, "Unverified").
    pub(crate) fn trigger_dma(&mut self, scheduler: &mut Scheduler, trigger: Trigger, time: u64) {
        let floor = time.max(scheduler.now());
        let arm9 = self.dma9.trigger(trigger);
        let arm7 = self.dma7.trigger(trigger);
        self.schedule_slots::<true>(scheduler, Port::Dma, arm9, floor);
        self.schedule_slots::<false>(scheduler, Port::Dma, arm7, floor);
    }

    /// The transfer of a CPU's channel ended at `end`: the channel repeats or is disabled, and
    /// its IRQ is raised (`dma.md`, "The end of a transfer").
    pub(crate) fn dma_finished(&mut self, transfer: &Transfer, end: End) {
        let effect = if transfer.arm9 {
            self.dma9.finish(transfer, end)
        } else {
            self.dma7.finish(transfer, end)
        };
        self.request_irq(effect.requests);
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
            && self.dma9.is_well_formed()
            && self.dma7.is_well_formed()
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
            // DMA0SAD..DMA3CNT, 12 bytes a channel, then DMA0FILL..DMA3FILL (`dma.md`).
            (DMA0SAD_ADDR, Some((Port::Dma, 0))),
            (0x0400_00B8, Some((Port::Dma, 8))),
            (0x0400_00DF, Some((Port::Dma, 0x2F))),
            (DMA0FILL_ADDR, Some((Port::DmaFill, 0x30))),
            (0x0400_00EF, Some((Port::DmaFill, 0x3F))),
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
            // The ARM7 has the DMA channels but no fill registers (`dma.md`, "Registers").
            (DMA0SAD_ADDR, Some((Port::Dma, 0))),
            (0x0400_00DF, Some((Port::Dma, 0x2F))),
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
            (0x0400_00E0, None),
            (0x0400_00EF, None),
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
            // Just below DMA0SAD and just above the fill block, which ends at 0400_00EFh.
            (0x0400_00AF, None),
            (0x0400_00F0, None),
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
    fn given_line_and_hblank_events_when_fired_through_io_then_no_transfer_comes_back() {
        let mut io = Io::new();
        let mut scheduler = Scheduler::new();
        assert_eq!(
            (
                io.fire(&mut scheduler, EventKind::LineStart),
                io.fire(&mut scheduler, EventKind::HBlankStart)
            ),
            (None, None)
        );
    }

    #[test]
    fn given_dma_start_events_for_channels_with_no_start_waiting_when_fired_then_no_transfer_comes_back()
     {
        // A start that was cancelled leaves nothing to run (`dma.md`, "Enabling").
        let mut io = Io::new();
        let mut scheduler = Scheduler::new();
        let kind = |arm9, channel| EventKind::DmaStart { arm9, channel };
        assert_eq!(
            (
                io.fire(&mut scheduler, kind(true, 0)),
                io.fire(&mut scheduler, kind(false, 3))
            ),
            (None, None)
        );
    }

    /// The times of the ARM9 channel 0 start event after the channel was set to wait for VBlank
    /// and VBlank was raised at `time`, with the master clock at `now`.
    fn vblank_start_times(now: u64, time: u64) -> Vec<u64> {
        const ENABLE: u32 = 1 << 31;
        const ARM9_VBLANK: u32 = 1 << 27;
        let mut io = Io::new();
        let mut scheduler = Scheduler::new();
        scheduler.advance(now);
        let at = At {
            now,
            floor: now + 1,
        };
        io.write::<true>(
            &mut scheduler,
            at,
            Port::Dma,
            8,
            Width::Word,
            ENABLE | ARM9_VBLANK | 1,
        );
        io.trigger_dma(&mut scheduler, Trigger::VBlank, time);
        scheduler.times_of(EventKind::DmaStart {
            arm9: true,
            channel: 0,
        })
    }

    #[test]
    fn given_a_vblank_raised_before_the_master_clock_when_triggered_then_the_start_is_queued_at_the_clock()
     {
        // An event is never before `now` (`scheduler.md`, "Savestate checks").
        assert_eq!(vblank_start_times(100, 40), vec![100]);
    }

    #[test]
    fn given_a_vblank_raised_after_the_master_clock_when_triggered_then_the_start_is_queued_at_that_time()
     {
        assert_eq!(vblank_start_times(100, 150), vec![150]);
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
