// SPDX-License-Identifier: Apache-2.0
//! The register devices both CPUs reach through the I/O slow path: engine A's display registers,
//! VCOUNT and DISPSTAT, the two interrupt controllers, IPC and the two sets of timers and of DMA
//! channels. They sit apart from [`crate::bus::Memory`] (the RAM buffers and the page tables) so a
//! bus view can borrow memory, devices and the scheduler at once: a register write that changes a
//! device's schedule schedules it on the spot.
//!
//! `map` decodes an address to the `Owner` that holds it and `device` defines the one interface a
//! device speaks, `Device`. `Io`, here, owns the devices and calls the concrete type behind a
//! `Port` through one `match` that lives in `device_call!`: static dispatch, no `dyn`
//! (`AGENTS.md` §2 rule 3). A device depends on `device` and never on `Io`.
//!
//! Every IF bit a device raises goes through `Io::request_irq`, the one path to the two
//! controllers. A device returns what it would raise in an `Effect`; it never reaches a
//! controller itself (`AGENTS.md` §4, "Per-CPU devices").

#[macro_use]
mod registry;
pub(crate) mod device;
pub mod divsqrt;
pub mod dma;
pub mod ipc;
pub mod irq;
pub mod keypad;
pub(crate) mod map;
pub mod power;
#[cfg(test)]
mod tests;
pub mod timers;
pub mod timing;

use crate::gpu2d::Gpu2d;
use crate::scheduler::{EventKind, Scheduler};
use device::{Checked, Device, Effect, Fired, SlotMask, Width};
use divsqrt::DivSqrt;
use dma::{Dma, End, Transfer, Trigger};
use ipc::Ipc;
use irq::{InterruptController, Requests};
use keypad::Keypad;
use map::Port;
use power::Power;
use serde::{Deserialize, Serialize};
use timers::Timers;
use timing::Timing;

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

registry! {
    $
    shared {
        /// VCOUNT and each CPU's DISPSTAT.
        timing / timing_mut: Timing = Timing::new() => [Port::Timing],
        /// IPCSYNC and the two IPC FIFOs (`ipc.md`), for the debugger.
        ipc: Ipc = Ipc::new() => [Port::Ipc, Port::IpcRecv],
    }
    arm9 {
        /// Both 2D engines' display registers and POWCNT1.
        gpu2d / gpu2d_mut: Gpu2d = Gpu2d::new() => [Port::Gpu2d],
        /// The ARM9's divider and square-root unit (`divsqrt.md`).
        divsqrt: DivSqrt = DivSqrt::new() => [Port::DivSqrt],
    }
    arm7 {
        /// The ARM7's HALTCNT (`interrupts.md`, "HALTCNT").
        power / power_mut: Power = Power::new() => [Port::Power],
    }
    each {
        /// Each CPU's interrupt controller: the same addresses, separate state (`interrupts.md`).
        irq9 / irq7: InterruptController = InterruptController::new() => [Port::Irq],
        /// Each CPU's four timers: the same addresses, separate state (`timers.md`).
        timers9 / timers7: Timers = Timers::new() => [Port::Timers],
        /// Each CPU's four DMA channels: the same addresses, separate state (`dma.md`).
        dma9 / dma7: Dma = Dma::new() => [Port::Dma, Port::DmaFill],
        /// Each CPU's KEYINPUT and KEYCNT, and the ARM7's EXTKEYIN (`keypad.md`).
        keypad9 / keypad7: Keypad = Keypad::new() => [Port::Keypad],
    }
}

impl Io {
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
    /// "Model").
    fn schedule_slots<const ARM9: bool>(
        &self,
        scheduler: &mut Scheduler,
        port: Port,
        mask: SlotMask,
        floor: u64,
    ) {
        (0..SlotMask::BITS as u8)
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
