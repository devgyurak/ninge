// SPDX-License-Identifier: Apache-2.0
//! The machine's event handlers: what the scheduler's rounds call back into. `Participants` is the
//! scheduler's view of the machine (run a CPU to a time, dispatch a due event); the handlers below
//! it are the line, HBlank and DMA starts those events lead to (`docs/hw/scheduler.md`).

use super::AUDIO_CAPACITY;
use super::run::RunControl;
use super::{Machine, SCREEN_HEIGHT, arm7_instruction, arm9_instruction};
use crate::bus::{Arm9View, TcmConfig, peek};
use crate::io::dma::Trigger;
use crate::io::timing::{FIRST_VBLANK_LINE, LINES, RENDER_START_LINE};
use crate::scheduler::{Event, EventKind, LINE_CYCLES, LineEvent, Participants, Scheduler};
use crate::spu::{CAPTURES, CaptureWrites, OUTPUT_SAMPLE_CYCLES};

/// The scheduler's view of a machine with no host stop conditions: what `step_arm7` and the tests
/// that run a round drive.
impl Participants for Machine {
    fn scheduler(&mut self) -> &mut Scheduler {
        &mut self.scheduler
    }

    fn run_arm9_until(&mut self, target: u64) -> bool {
        self.arm9_until(target, &mut RunControl::default())
    }

    fn run_arm7_until(&mut self, target: u64) -> bool {
        self.arm7_until(target, &mut RunControl::default())
    }

    fn dispatch(&mut self, event: Event) {
        self.dispatch_event(event);
    }
}

/// A machine run under a host's stop conditions: what [`Machine::run_until`] hands the scheduler.
/// The conditions live and die with the call, so the machine holds none of them.
pub(crate) struct Running<'a> {
    pub(crate) machine: &'a mut Machine,
    pub(crate) control: RunControl,
}

impl Participants for Running<'_> {
    fn scheduler(&mut self) -> &mut Scheduler {
        &mut self.machine.scheduler
    }

    fn run_arm9_until(&mut self, target: u64) -> bool {
        self.machine.arm9_until(target, &mut self.control)
    }

    fn run_arm7_until(&mut self, target: u64) -> bool {
        self.machine.arm7_until(target, &mut self.control)
    }

    fn dispatch(&mut self, event: Event) {
        self.machine.dispatch_event(event);
    }
}

impl Machine {
    /// Runs the ARM9 until the master clock reaches `target` or the earliest queued event; false
    /// when a stop condition of `control` ended it first.
    fn arm9_until(&mut self, target: u64, control: &mut RunControl) -> bool {
        // An instruction can queue an event before `target` (a timer write, #109); stop at it so
        // it sees every earlier instruction and none after (`scheduler.md`, "Interleaving").
        // See #118. `next_event_time` is a heap peek: O(1), no allocation.
        //
        // One view serves the whole batch; only a CP15 write that moves a TCM mapping
        // refreshes it, and it takes effect from the next instruction
        // (`docs/hw/arm9-memory-map.md`).
        let mut tcm_key = self.arm9.cp15().tcm_key();
        let mut view = Arm9View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            &self.rom,
            TcmConfig::from_cp15(self.arm9.cp15()),
        )
        .with_hold(self.arm9_hold_until);
        let mut completed = true;
        while view.clock() < target
            && view
                .next_event_time()
                .is_none_or(|time| view.clock() < time)
        {
            // A held ARM9 (a full geometry FIFO) runs nothing either: its clock skips to the hold's
            // end, to `target` or to an event, whichever is first, so the events keep firing.
            if view.hold_until() > view.clock() {
                let stop = view
                    .next_event_time()
                    .map_or(target, |time| time.min(target))
                    .min(view.hold_until());
                view.advance(stop - view.clock());
                continue;
            }
            // A halted ARM9 runs nothing, so its clock skips in one step to `target` or to an event
            // queued before it (by the instruction that halted it, say a timer write), whichever is
            // earlier: the event fires on time and the wake is tested after it
            // (`docs/hw/scheduler.md`, "Interleaving").
            if self.arm9.halted() && !view.io().irq9().wakes() {
                let stop = view
                    .next_event_time()
                    .map_or(target, |time| time.min(target));
                view.advance(stop - view.clock());
                break;
            }
            if control.is_active() && control.stops_before(true, self.arm9.next_instruction()) {
                completed = false;
                break;
            }
            arm9_instruction(&mut self.arm9, &mut self.bios.arm9, &mut view);
            let key = self.arm9.cp15().tcm_key();
            if key != tcm_key {
                tcm_key = key;
                view.set_tcm(TcmConfig::from_cp15(self.arm9.cp15()));
            }
        }
        self.arm9_hold_until = view.hold_until();
        completed
    }

    /// Runs the ARM7 until its clock reaches `target`; false when a stop condition of `control`
    /// ended it first.
    fn arm7_until(&mut self, target: u64, control: &mut RunControl) -> bool {
        self.with_arm7_bus(|arm7, bios, view| {
            while view.clock() < target {
                if arm7.halted() && !view.io().irq7().wakes() {
                    view.advance(target - view.clock());
                    break;
                }
                if control.is_active() && control.stops_before(false, arm7.next_instruction()) {
                    return false;
                }
                arm7_instruction(arm7, bios, view);
            }
            true
        })
    }

    /// Handles a fired event.
    fn dispatch_event(&mut self, event: Event) {
        match event.kind {
            EventKind::Line(LineEvent::Start) => self.line_started(event.time),
            EventKind::Line(LineEvent::HBlank) => self.hblank_started(event.time),
            EventKind::Sample => self.sample_due(event.time),
            EventKind::Slot { .. } => {
                if let Some(transfer) = self.io.fire(&mut self.scheduler, event.kind) {
                    self.run_dma(transfer);
                }
            }
        }
    }
}

impl Machine {
    /// An output sample is due at `time` (`docs/hw/spu.md`, "Output samples"): every channel's
    /// sample is read from the ARM7's view of memory, and the result joins the frame's audio. The
    /// next sample is scheduled from the event's own time, so the grid never drifts.
    ///
    /// No allocation: the frame's buffer is reserved once and a sample past its bound is dropped
    /// (a debugger stepping for many frames without `run_frame`), since the audio is output and not
    /// state.
    fn sample_due(&mut self, time: u64) {
        let (memory, io, rom) = (&self.memory, &self.io, &self.rom[..]);
        let read = |addr: u32| peek::<false, 1>(memory, io, &TcmConfig::NONE, rom, time, addr)[0];
        // The gates outside the unit: the speakers and the sound amplifier (`spu.md`, "The gates").
        let audible = io.power().speakers_on() && io.spi().power().amplifier_audible();
        let tick = io.spu().tick(time, audible, read);
        self.io.spu_mut().set_decoders(tick.decoders);
        if self.audio.len() < AUDIO_CAPACITY {
            self.audio.extend(tick.output);
        }
        // The capture writes come after the mix, so the one-sample delay of a capture played back by
        // channel 1 or 3 is the model (`docs/hw/sound-capture.md`, "Model").
        self.write_captures(tick.captures);
        self.scheduler
            .schedule(time.saturating_add(OUTPUT_SAMPLE_CYCLES), EventKind::Sample);
    }

    /// Writes what the capture units captured, through the ARM7's bus view as a DMA write would
    /// go (`docs/hw/sound-capture.md`, "Model"): the shared WRAM split, the mirrors and the I/O
    /// window behave as for a DMA. Nothing is built when no capture runs.
    fn write_captures(&mut self, captures: [Option<CaptureWrites>; CAPTURES]) {
        if captures.iter().all(Option::is_none) {
            return;
        }
        // As for a DMA unit, a write may reach HALTCNT: the ARM7 halts at once (`with_arm7_bus`).
        self.with_arm7_bus(|_, _, view| {
            captures
                .iter()
                .flatten()
                .for_each(|writes| writes.run(view));
        });
    }

    /// Runs a DMA transfer through the owning CPU's bus view with the TCM switched off: DMA
    /// cannot reach it (`docs/hw/dma.md`, "Transferring"). The view is the CPU's own, so the
    /// units reach RAM, VRAM and the I/O registers (the DMA registers too) as an instruction's
    /// accesses would. The channel then repeats or ends and raises its IRQ.
    ///
    /// The transfer is atomic: all units move at the event's master time and take no cycles of
    /// their own. The one thing that holds a CPU is an ARM9 word that finds the geometry FIFO
    /// full: it holds the ARM9 for the engine's wait (`geometry-engine.md`, "The FIFO IRQ, the DMA
    /// start and the stall"). An ARM7 start comes late by what the ARM7 lags the master clock, under
    /// one slice.
    // TODO(hw-unverified): the transfer takes no cycles, holds no CPU but through a full geometry
    // FIFO and cannot be pre-empted by a lower channel, and an ARM7 start can be late
    // (`docs/hw/dma.md`, "Unverified"); revisit with the Phase 8 timing work.
    fn run_dma(&mut self, transfer: crate::io::dma::Transfer) {
        let end = if transfer.arm9 {
            let mut view = Arm9View::new(
                &mut self.memory,
                &mut self.io,
                &mut self.scheduler,
                &self.rom,
                // TODO(hw-unverified): DMA sees the address behind a TCM window, not a hole
                // (`docs/hw/dma.md`, "Unverified").
                TcmConfig::NONE,
            );
            // A DMA unit's write is not an instruction, so the view's floor stays at `now`: an
            // event it queues waits for the next round and fires at `now`, which is where a load
            // rebuilds it (`dma.md`, "Model").
            let end = transfer.run(&mut view);
            // A word that found the geometry FIFO full paused the DMA and the ARM9 together
            // (GBATEK, "GXFIFO DMA Overkill"): the ARM9 is held for the same time.
            let stall = view.take_stall();
            self.arm9_hold_until = self
                .arm9_hold_until
                .max(self.scheduler.now().saturating_add(stall));
            end
        } else {
            // A DMA unit may write HALTCNT: the ARM7 halts at once, not after its next instruction
            // (`with_arm7_bus`).
            self.with_arm7_bus(|_, _, view| transfer.run(view))
        };
        self.io
            .dma_finished(&mut self.scheduler, &transfer, end, self.arm9_hold_until);
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
            // The affine layers' reference points come from their registers again
            // (`docs/hw/bg-modes.md`, "Affine layers").
            self.io.gpu2d_mut().vblank_started();
        }
        let line = usize::from(self.io.timing().vcount());
        self.start_display_dma(line, time);
        // The last visible line is drawn: the frame is whole and becomes the one callers read.
        if line == SCREEN_HEIGHT {
            self.present_frame();
        }
        self.render_line(line);
        // The rendering engine starts the next frame's lines here, from the scene SWAP_BUFFERS
        // handed on at line 192 and the registers as the VBlank left them (`rasterizer.md`, "When
        // the hardware draws").
        if line == usize::from(RENDER_START_LINE) {
            self.render_3d();
        }
        // The start of line 0 ends the frame; its own line 0 belongs to the next one.
        if line == 0 {
            self.frame_count = self.frame_count.wrapping_add(1);
        }
        // From the event's own time, not `now`, so a line's length never drifts.
        self.scheduler.schedule(
            time.saturating_add(LINE_CYCLES),
            EventKind::Line(LineEvent::Start),
        );
    }

    /// The ARM9 DMA starts a line begins: the start of display at line 0, and the main-memory
    /// display's requests for the next visible line, raised from the last line of the previous
    /// frame so line 0 finds its pixels (`docs/hw/display-capture.md`, "ARM9 DMA start modes 3 and
    /// 4"). The line itself is drawn right after, from what the FIFO holds now.
    // TODO(hw-unverified): the start of display fires once per frame at line 0, and the line's
    // requests are all raised at the start of the line before it (`display-capture.md`,
    // "Unverified").
    fn start_display_dma(&mut self, line: usize, time: u64) {
        if line == 0 {
            self.io
                .trigger_dma(&mut self.scheduler, Trigger::StartOfDisplay, time);
        }
        let frame_start = line == usize::from(LINES) - 1;
        if frame_start || line + 1 < SCREEN_HEIGHT {
            self.io.trigger_dma(
                &mut self.scheduler,
                Trigger::MainMemory { frame_start },
                time,
            );
        }
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
        self.scheduler.schedule(
            time.saturating_add(LINE_CYCLES),
            EventKind::Line(LineEvent::HBlank),
        );
    }
}
