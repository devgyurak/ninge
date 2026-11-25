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
pub mod cart;
pub(crate) mod device;
pub mod divsqrt;
pub mod dma;
pub mod ipc;
pub mod irq;
pub mod keypad;
pub(crate) mod map;
pub mod power;
pub mod rtc;
pub mod spi;
#[cfg(test)]
mod tests;
pub mod timers;
pub mod timing;

use crate::gpu2d::Gpu2d;
use crate::gpu3d::Gpu3d;
use crate::input::Input;
use crate::scheduler::{EventKind, Scheduler};
use crate::spu::Spu;
use cart::Cart;
use device::{
    Checked, Cx, Device, Effect, Fired, Lines, Signal, Signals, SlotMask, Width, Written,
};
use divsqrt::DivSqrt;
use dma::{Dma, End, Transfer, Trigger};
use ipc::Ipc;
use irq::{InterruptController, Requests};
use keypad::Keypad;
use map::Port;
use power::Power;
use rtc::Rtc;
use serde::{Deserialize, Serialize};
use spi::Spi;
use timers::Timers;
use timing::Timing;

/// When an access happens and what it reaches: the clock of the CPU making it, the earliest time an
/// event it schedules may fire, and the cartridge image (`device::Cx::rom`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct At<'a> {
    /// Master time of the access, as the CPU's own clock has it.
    pub(crate) now: u64,
    /// An event never lies before the end of the instruction that scheduled it (`timers.md`,
    /// "Model"; `dma.md`, "Model").
    pub(crate) floor: u64,
    /// The cartridge image the machine was loaded with, empty before a ROM is loaded.
    pub(crate) rom: &'a [u8],
}

registry! {
    $
    shared {
        /// VCOUNT and each CPU's DISPSTAT.
        timing / timing_mut: Timing = Timing::new() => [Port::Timing],
        /// IPCSYNC and the two IPC FIFOs (`ipc.md`), for the debugger.
        ipc: Ipc = Ipc::new() => [Port::Ipc, Port::IpcRecv],
        /// The cartridge slot's ROM interface (`cart-protocol.md`).
        cart / cart_mut: Cart = Cart::new() => [Port::Cart, Port::CartExmem, Port::CartData],
    }
    arm9 {
        /// Both 2D engines' display registers and POWCNT1.
        gpu2d / gpu2d_mut: Gpu2d = Gpu2d::new() => [Port::Gpu2d],
        /// The ARM9's divider and square-root unit (`divsqrt.md`).
        divsqrt: DivSqrt = DivSqrt::new() => [Port::DivSqrt],
        /// The ARM9's 3D engine: DISP3DCNT, the geometry engine with its FIFO, matrices and
        /// polygon RAM, GXSTAT and the rendering registers (`geometry-engine.md`).
        gpu3d: Gpu3d = Gpu3d::new() => [Port::Gpu3d],
    }
    arm7 {
        /// The ARM7's HALTCNT (`interrupts.md`, "HALTCNT").
        power: Power = Power::new() => [Port::Power],
        /// The ARM7's SPI bus and the firmware chip on it (`spi.md`).
        spi: Spi = Spi::new() => [Port::Spi],
        /// The ARM7's RTCCNT and the clock chip on its pins (`rtc.md`).
        rtc: Rtc = Rtc::new() => [Port::Rtc],
        /// The ARM7's sound and capture registers (`sound-registers.md`).
        spu / spu_mut: Spu = Spu::new() => [Port::Spu],
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
    /// Latches `input` as a frame begins (`keypad.md`, "Where the bits come from"). The one copy
    /// of the host's data: every device reads it through the access's [`Cx`].
    ///
    /// The keypad IRQ and the hinge IRQ are checked here, against the input latched before this one
    /// (`keypad.md`, "The keypad IRQ"); a request raised outside a CPU access takes the same
    /// [`Io::request_irq`] path as the events.
    pub(crate) fn latch_input(&mut self, input: Input) {
        let previous = std::mem::replace(&mut self.input, input);
        let requests = self
            .keypad9
            .latch_requests(&previous, &input)
            .merge(self.keypad7.latch_requests(&previous, &input));
        self.request_irq(requests);
    }

    /// What a load does to the devices once the state has been checked: each device rebuilds what it
    /// derives ([`Checked::restore`]), and the save chip's generation goes one past `previous`, the
    /// count the machine held before the load, since the save memory may differ from what a host last
    /// wrote (`docs/hw/auxspi-saves.md`, "Save files").
    pub(crate) fn after_load(&mut self, previous_generation: u64, now: u64) {
        self.restore(now);
        self.cart_mut()
            .set_save_generation(previous_generation.wrapping_add(1));
    }

    /// The wires between devices as they stand now (`device::Lines`).
    fn lines(&self) -> Lines {
        Lines {
            pen_irq_enabled: self.spi.pen_irq_enabled(),
            geometry_powered: self.gpu2d.geometry_engine_on(),
            rendering_powered: self.gpu2d.rendering_engine_on(),
        }
    }

    /// The input latched when the frame began, for the debugger.
    pub(crate) fn input(&self) -> Input {
        self.input
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
        rom: &[u8],
    ) -> u32 {
        let cx = Cx {
            now,
            host: &self.input,
            rom,
            lines: self.lines(),
        };
        device_call!(self, ARM9, port, ref, peek(offset, width, cx), 0)
    }

    /// A CPU's read from `port`: what it returns, with the effect applied (IF bits raised, events
    /// replaced). A read that takes a word out of a FIFO can raise the sender's requests.
    pub(crate) fn read<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        at: At<'_>,
        port: Port,
        offset: u32,
        width: Width,
    ) -> u32 {
        let cx = Cx {
            now: at.now,
            host: &self.input,
            rom: at.rom,
            lines: self.lines(),
        };
        let (value, effect) = device_call!(
            self,
            ARM9,
            port,
            mut,
            read(offset, width, cx),
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
        at: At<'_>,
        port: Port,
        offset: u32,
        width: Width,
        value: u32,
    ) -> Written {
        let cx = Cx {
            now: at.now,
            host: &self.input,
            rom: at.rom,
            lines: self.lines(),
        };
        let effect = device_call!(
            self,
            ARM9,
            port,
            mut,
            write(offset, width, value, cx),
            Effect::default()
        );
        self.apply::<ARM9>(scheduler, at, port, effect)
    }

    /// Hands an access's effect on: its IF bits to the controllers, the event slots it names to
    /// the scheduler, then its signals, and, when it asks to recheck, what the devices hold up as a
    /// level (signals to the devices that wait for them, IF bits to the controllers), never earlier
    /// than `at.floor`.
    fn apply<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        at: At<'_>,
        port: Port,
        effect: Effect,
    ) -> Written {
        self.request_irq(effect.requests);
        if effect.reschedule != 0 {
            self.schedule_slots::<ARM9>(scheduler, port, effect.reschedule, at.floor);
        }
        self.route(scheduler, effect.signals, at.floor);
        // A signal that is a level: the device that waits for it may have been enabled while it
        // already holds, and takes it now. One already queued is not queued twice.
        if effect.recheck_held_signals {
            self.route(scheduler, self.held_signals::<ARM9>(), at.floor);
        }
        // An IF bit that is a level (the FIFO IRQ): an acknowledge cleared it while its condition
        // holds, so it is raised again. A bit that is already set stays set.
        if effect.recheck_held_requests {
            self.request_irq(self.held_requests::<ARM9>());
        }
        Written {
            stall: effect.stall,
            halt: effect.halt,
        }
    }

    /// Delivers `signals` at master time `time`, one signal at a time in the order of
    /// [`Signal::ALL`] so that the result does not depend on how they were collected. This is the
    /// one table of who listens to what: a device raises a signal and never names its listener.
    // TODO(hw-unverified): whether the other CPU's channels ever see a card request
    // (`cart-protocol.md`, "Unverified"). The channel order is in `start_dma`.
    fn route(&mut self, scheduler: &mut Scheduler, signals: Signals, time: u64) {
        Signal::ALL
            .into_iter()
            .filter(|&signal| signals.contains(signal))
            .for_each(|signal| {
                // A word of the cartridge slot goes to the DMA of the CPU that owns it.
                let (arm9, arm7) = match signal {
                    Signal::CardWordArm9 => (Some(Trigger::CardWord { arm9: true }), None),
                    Signal::CardWordArm7 => (None, Some(Trigger::CardWord { arm9: false })),
                    // The geometry engine is the ARM9's, so only its channels see the FIFO.
                    Signal::GeometryFifoHalf => (Some(Trigger::GeometryFifo), None),
                };
                self.start_dma(scheduler, arm9, arm7, time);
            });
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
        let device = DeviceId::of(port);
        (0..device.slots())
            .filter(|slot| mask & (1 << slot) != 0)
            .for_each(|slot| {
                let kind = EventKind::Slot {
                    device,
                    arm9: ARM9 || device.is_shared(),
                    slot,
                };
                scheduler.cancel(kind);
                let due = device_call!(self, ARM9, port, ref, slot_due(slot), None);
                if let Some(time) = due {
                    scheduler.schedule(time.max(floor), kind);
                }
            });
    }

    /// A device's event fired: it does what the device says ([`Device::slot_fire`]), its IF bits
    /// are raised and its slots rescheduled from the master clock. A DMA start comes back as the
    /// transfer the machine must run. An event no device owns (a line or HBlank start) does
    /// nothing here.
    ///
    /// No device has an arm of its own: a DMA start needs no special event because its transfer
    /// already comes back through the generic [`Fired::transfer`], and the machine runs it.
    pub(crate) fn fire(&mut self, scheduler: &mut Scheduler, kind: EventKind) -> Option<Transfer> {
        let EventKind::Slot { device, arm9, slot } = kind else {
            return None;
        };
        let port = device.first_port();
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
        self.apply::<ARM9>(
            scheduler,
            At {
                now,
                floor: now,
                // The ROM is read when a word is taken, never when an event fires; a slot that
                // needs it at fire time must be handed it here (`cart-protocol.md`, "Savestate").
                rom: &[],
            },
            port,
            fired.effect,
        );
        fired.transfer
    }

    /// `trigger` happened at master time `time`: every channel of both CPUs that waits for it is
    /// queued to start, the ARM9's first, each in channel order. The event is never before the
    /// master clock (`scheduler.md`, "Savestate checks"). The line events (VBlank, HBlank, the
    /// start of a frame, the main-memory display) call this directly: they come from the
    /// scheduler's own line events, not from a device, so they are not a [`Signal`].
    pub(crate) fn trigger_dma(&mut self, scheduler: &mut Scheduler, trigger: Trigger, time: u64) {
        self.start_dma(scheduler, Some(trigger), Some(trigger), time);
    }

    /// Queues the channels of each CPU that wait for its trigger, if it has one, the ARM9's first.
    // TODO(hw-unverified): the ARM9's channels go before the ARM7's (`dma.md`, "Unverified").
    fn start_dma(
        &mut self,
        scheduler: &mut Scheduler,
        arm9: Option<Trigger>,
        arm7: Option<Trigger>,
        time: u64,
    ) {
        let floor = time.max(scheduler.now());
        let queued9 = arm9.map_or(0, |trigger| self.dma9.trigger(trigger, time));
        let queued7 = arm7.map_or(0, |trigger| self.dma7.trigger(trigger, time));
        self.schedule_slots::<true>(scheduler, Port::Dma, queued9, floor);
        self.schedule_slots::<false>(scheduler, Port::Dma, queued7, floor);
    }

    /// The transfer of a CPU's channel ended at `end`: the channel repeats or is disabled, and
    /// its IRQ is raised (`dma.md`, "The end of a transfer").
    pub(crate) fn dma_finished(
        &mut self,
        scheduler: &mut Scheduler,
        transfer: &Transfer,
        end: End,
        arm9_held_until: u64,
    ) {
        let effect = if transfer.arm9 {
            self.dma9.finish(transfer, end)
        } else {
            self.dma7.finish(transfer, end)
        };
        self.request_irq(effect.requests);
        // A burst into the geometry FIFO ended with the FIFO still less than half full: the channel
        // goes on, once the burst's words have gone by at a bus cycle each. The pause keeps a channel
        // that never fills the FIFO from restarting at the very same time for ever.
        // TODO(hw-unverified): one word a bus cycle (`geometry-engine.md`, "Unverified").
        if transfer.is_geometry_burst()
            && self
                .held_signals::<true>()
                .contains(Signal::GeometryFifoHalf)
        {
            // The next burst waits for its words to have gone by and for the ARM9 hold the burst
            // may have caused (a DMA that stalled paused the ARM9 with it).
            let time = scheduler
                .now()
                .saturating_add(transfer.duration())
                .max(arm9_held_until);
            self.route(scheduler, Signals::of(Signal::GeometryFifoHalf), time);
        }
    }

    /// Replaces the SPI bus, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn replace_spi_for_test(&mut self, spi: Spi) {
        self.spi = spi;
    }

    /// The 3D engine, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn gpu3d_for_test(&mut self) -> &mut Gpu3d {
        &mut self.gpu3d
    }

    /// Replaces the RTC, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn replace_rtc_for_test(&mut self, rtc: Rtc) {
        self.rtc = rtc;
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
