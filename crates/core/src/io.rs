// SPDX-License-Identifier: Apache-2.0
//! The register devices both CPUs reach through the I/O slow path: engine A's display registers,
//! VCOUNT and DISPSTAT, the two interrupt controllers, IPC and the two sets of timers. They sit
//! apart from [`crate::bus::Memory`] (the RAM buffers and the page tables) so a bus view can
//! borrow memory, devices and the scheduler at once: a register write that changes a device's
//! schedule schedules it on the spot.
//!
//! Every IF bit a device raises goes through [`Io::request_irq`], the one path to the two
//! controllers.

use crate::gpu2d::Gpu2d;
use crate::gpu2d::timing::Timing;
use crate::ipc::{IPCFIFORECV_ADDR, IPCFIFOSEND_ADDR, Ipc};
use crate::irq::InterruptController;
use crate::scheduler::Scheduler;
use crate::timers::{TIMER_COUNT, Timers};
use serde::{Deserialize, Serialize};

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

/// Which byte of IPCFIFORECV `addr` is, if it is one.
fn recv_offset(addr: u32) -> Option<usize> {
    addr.checked_sub(IPCFIFORECV_ADDR)
        .filter(|&offset| offset < 4)
        .map(|offset| offset as usize)
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
    pub(crate) fn request_irq(&mut self, arm9: u32, arm7: u32) {
        self.irq9.request(arm9);
        self.irq7.request(arm7);
    }

    /// The byte of the register at `addr` as CPU `ARM9` reads it at master time `now`, if a
    /// device holds it. A byte of IPCFIFORECV shows the word a read would take, without taking
    /// it. Each byte of a wide read comes from the register at its own address.
    // TODO(hw-unverified): IPCFIFORECV is not mirrored (`ipc.md`, "Unverified").
    // TODO(phase3): the ARM7's other I/O registers arrive with their issues (#26).
    pub(crate) fn read_byte<const ARM9: bool>(&self, addr: u32, now: u64) -> Option<u8> {
        let recv =
            || recv_offset(addr).map(|offset| self.ipc.peek_recv::<ARM9>().to_le_bytes()[offset]);
        if ARM9 {
            self.irq9
                .read_byte(addr)
                .or_else(|| self.ipc.read_byte::<true>(addr))
                .or_else(recv)
                .or_else(|| self.gpu2d.read_byte(addr))
                .or_else(|| self.timing.read_byte::<true>(addr))
                .or_else(|| self.timers9.read_byte(addr, now))
        } else {
            self.irq7
                .read_byte(addr)
                .or_else(|| self.ipc.read_byte::<false>(addr))
                .or_else(recv)
                .or_else(|| self.timing.read_byte::<false>(addr))
                .or_else(|| self.timers7.read_byte(addr, now))
        }
    }

    /// A CPU's read of `N` bytes at `addr` when it lands on IPCFIFORECV: one word is taken from
    /// the FIFO whatever the width, and the access sees the bytes at its own address.
    // TODO(hw-unverified): a byte or halfword read takes a whole word (`ipc.md`, "Unverified").
    pub(crate) fn receive_bytes<const ARM9: bool, const N: usize>(
        &mut self,
        addr: u32,
    ) -> Option<[u8; N]> {
        let offset = recv_offset(addr)?;
        let (word, requests) = self.ipc.receive::<ARM9>();
        self.request_irq(requests.irq9, requests.irq7);
        let bytes = word.to_le_bytes();
        // `addr` is aligned to `N` and the register is 4 bytes, so the access stays inside it.
        Some(std::array::from_fn(|i| bytes[(offset + i) & 3]))
    }

    /// A CPU's write of `N` bytes at the aligned `addr` when it lands on IPCFIFOSEND: a word
    /// write sends one word. Returns whether the address was IPCFIFOSEND.
    // TODO(hw-unverified): a byte or halfword write is ignored (`ipc.md`, "Unverified").
    pub(crate) fn send_word<const ARM9: bool, const N: usize>(
        &mut self,
        addr: u32,
        bytes: [u8; N],
    ) -> bool {
        if addr & !3 != IPCFIFOSEND_ADDR {
            return false;
        }
        if let Ok(word) = <[u8; 4]>::try_from(bytes.as_slice()) {
            let requests = self.ipc.send::<ARM9>(u32::from_le_bytes(word));
            self.request_irq(requests.irq9, requests.irq7);
        }
        true
    }

    /// Writes one byte of a register as CPU `ARM9` at master time `now`; a byte no device holds
    /// is dropped. A timer write that changes a schedule replaces that overflow event in
    /// `scheduler` at once, never earlier than `floor` (`timers.md`, "Model").
    // TODO(hw-unverified): a wider write is split into bytes and each goes to the register at its
    // own address (`arm9-memory-map.md`, "Unverified").
    pub(crate) fn write_byte<const ARM9: bool>(
        &mut self,
        scheduler: &mut Scheduler,
        now: u64,
        floor: u64,
        addr: u32,
        value: u8,
    ) {
        if ARM9 {
            self.irq9.write_byte(addr, value);
            let requests = self.ipc.write_byte::<true>(addr, value);
            self.request_irq(requests.irq9, requests.irq7);
            self.gpu2d.write_byte(addr, value);
            self.timing.write_byte::<true>(addr, value);
            let outcome = self.timers9.write_byte(addr, value, now);
            self.request_irq(outcome.requests, 0);
            schedule_timers(scheduler, &self.timers9, outcome.reschedule, floor);
        } else {
            self.irq7.write_byte(addr, value);
            let requests = self.ipc.write_byte::<false>(addr, value);
            self.request_irq(requests.irq9, requests.irq7);
            self.timing.write_byte::<false>(addr, value);
            let outcome = self.timers7.write_byte(addr, value, now);
            self.request_irq(0, outcome.requests);
            schedule_timers(scheduler, &self.timers7, outcome.reschedule, floor);
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
            let outcome = self.timers9.settle_timer(index, now);
            self.request_irq(outcome.requests, 0);
            schedule_timers(
                scheduler,
                &self.timers9,
                outcome.reschedule | 1 << index,
                now,
            );
        } else {
            let outcome = self.timers7.settle_timer(index, now);
            self.request_irq(0, outcome.requests);
            schedule_timers(
                scheduler,
                &self.timers7,
                outcome.reschedule | 1 << index,
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
    use crate::irq::InterruptController;

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
        io.request_irq(0b01, 0b10);
        assert_eq!((io.irq9().if_(), io.irq7().if_()), (0b01, 0b10));
    }
}
