// SPDX-License-Identifier: Apache-2.0
//! The machine's event handlers: what the scheduler's rounds call back into. `Participants` is the
//! scheduler's view of the machine (run a CPU to a time, dispatch a due event); the handlers below
//! it are the line, HBlank and DMA starts those events lead to (`docs/hw/scheduler.md`).

use super::{Machine, SCREEN_HEIGHT};
use crate::bus::{Arm7View, Arm9View, TcmConfig};
use crate::io::dma::Trigger;
use crate::io::timing::FIRST_VBLANK_LINE;
use crate::scheduler::{Event, EventKind, LINE_CYCLES, Participants, Scheduler};

impl Participants for Machine {
    fn scheduler(&mut self) -> &mut Scheduler {
        &mut self.scheduler
    }

    fn run_arm9_until(&mut self, target: u64) {
        // An instruction can queue an event before `target` (a timer write, #109); stop at it so
        // it sees every earlier instruction and none after (`scheduler.md`, "Interleaving").
        // See #118. `next_time` is a heap peek: O(1), no allocation.
        while self.scheduler.now() < target
            && self
                .scheduler
                .next_time()
                .is_none_or(|time| self.scheduler.now() < time)
        {
            // A halted ARM9 runs nothing, so its clock skips in one step to `target` or to an event
            // queued before it (by the instruction that halted it, say a timer write), whichever is
            // earlier: the event fires on time and the wake is tested after it
            // (`docs/hw/scheduler.md`, "Interleaving").
            if self.arm9.halted() && !self.io.irq9().wakes() {
                let stop = self
                    .scheduler
                    .next_time()
                    .map_or(target, |time| time.min(target));
                self.scheduler.advance(stop - self.scheduler.now());
                break;
            }
            self.step_arm9_instruction();
        }
    }

    fn run_arm7_until(&mut self, target: u64) {
        while self.arm7_clock < target {
            if self.arm7.halted() && !self.io.irq7().wakes() {
                self.arm7_clock = target;
                break;
            }
            self.step_arm7_instruction();
        }
    }

    fn dispatch(&mut self, event: Event) {
        match event.kind {
            EventKind::LineStart => self.line_started(event.time),
            EventKind::HBlankStart => self.hblank_started(event.time),
            EventKind::TimerOverflow { .. } | EventKind::DmaStart { .. } => {
                if let Some(transfer) = self.io.fire(&mut self.scheduler, event.kind) {
                    self.run_dma(transfer);
                }
            }
        }
    }
}

impl Machine {
    /// Runs a DMA transfer through the owning CPU's bus view with the TCM switched off: DMA
    /// cannot reach it (`docs/hw/dma.md`, "Transferring"). The view is the CPU's own, so the
    /// units reach RAM, VRAM and the I/O registers (the DMA registers too) as an instruction's
    /// accesses would. The channel then repeats or ends and raises its IRQ.
    ///
    /// The transfer is atomic: all units move at the event's master time and neither CPU stalls.
    /// An ARM7 start comes late by what the ARM7 lags the master clock, under one slice.
    // TODO(hw-unverified): the transfer takes no cycles, holds no CPU and cannot be pre-empted
    // by a lower channel, and an ARM7 start can be late (`docs/hw/dma.md`, "Unverified");
    // revisit with the Phase 8 timing work.
    fn run_dma(&mut self, transfer: crate::io::dma::Transfer) {
        let now = self.scheduler.now();
        let end = if transfer.arm9 {
            let mut view = Arm9View::new(
                &mut self.memory,
                &mut self.io,
                &mut self.scheduler,
                // TODO(hw-unverified): DMA sees the address behind a TCM window, not a hole
                // (`docs/hw/dma.md`, "Unverified").
                TcmConfig::NONE,
                now,
                // A DMA unit's write is not an instruction, so its floor is `now`: an event it
                // queues waits for the next round and fires at `now`, which is where a load
                // rebuilds it (`dma.md`, "Model").
                now,
            );
            transfer.run(&mut view)
        } else {
            let mut view = Arm7View::new(
                &mut self.memory,
                &mut self.io,
                &mut self.scheduler,
                self.arm7_clock,
                now,
            );
            transfer.run(&mut view)
        };
        // A DMA unit may write HALTCNT: the ARM7 halts at once, not after its next instruction.
        if self.io.power_mut().take_halt() {
            self.arm7.set_halted(true);
        }
        self.io.dma_finished(&transfer, end);
    }

    /// A line begins at `time` (`docs/hw/display-timing.md`). VBlank starts here too, when the
    /// line is the first blank one; this is the one place for it.
    ///
    /// The start of each visible line draws that line (`docs/hw/bg-text.md`, "Lines"); the start
    /// of the first blank line presents the finished frame, and the start of line 0 ends the frame:
    /// it is counted.
    ///
    /// The first blank line also starts the VBlank-start DMA channels (`docs/hw/dma.md`,
    /// "Starting").
    fn line_started(&mut self, time: u64) {
        let requests = self.io.timing_mut().line_started();
        self.io.request_irq(requests);
        if self.io.timing().vcount() == FIRST_VBLANK_LINE {
            self.io
                .trigger_dma(&mut self.scheduler, Trigger::VBlank, time);
        }
        let line = usize::from(self.io.timing().vcount());
        // The last visible line is drawn: the frame is whole and becomes the one callers read.
        if line == SCREEN_HEIGHT {
            self.present_frame();
        }
        self.render_line(line);
        // The start of line 0 ends the frame; its own line 0 belongs to the next one.
        if line == 0 {
            self.frame_count = self.frame_count.wrapping_add(1);
        }
        // From the event's own time, not `now`, so a line's length never drifts.
        self.scheduler
            .schedule(time.saturating_add(LINE_CYCLES), EventKind::LineStart);
    }

    /// HBlank begins at `time` in the current line (`docs/hw/display-timing.md`); the one place
    /// for it. In a visible line it starts the HBlank-start DMA channels (`docs/hw/dma.md`,
    /// "Starting").
    // TODO(hw-unverified): no HBlank-start DMA in the blank lines (`docs/hw/dma.md`, "Unverified").
    fn hblank_started(&mut self, time: u64) {
        let requests = self.io.timing_mut().hblank_started();
        self.io.request_irq(requests);
        if self.io.timing().vcount() < FIRST_VBLANK_LINE {
            self.io
                .trigger_dma(&mut self.scheduler, Trigger::HBlank, time);
        }
        self.scheduler
            .schedule(time.saturating_add(LINE_CYCLES), EventKind::HBlankStart);
    }
}
