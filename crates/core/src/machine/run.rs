// SPDX-License-Identifier: Apache-2.0
//! Run control: stopping the machine anywhere a host asks, and continuing to the same result an
//! uninterrupted run gives (`docs/hw/scheduler.md`, "Run control").
//!
//! A stop leaves the round it interrupted in `Machine::round`, a plain, saved record, so the
//! ARM7 and the events are neither run early nor late: the interleaving is the uninterrupted one,
//! and so are the frames and the savestate bytes. What the host asked for (the [`StopSet`], the
//! breakpoint to run over) lives in the call and is handed back in by the host, so the machine
//! holds nothing of a debugger's.

use super::Machine;
use super::events::Running;
use crate::input::Input;
use crate::scheduler;

/// Breakpoints one CPU takes in a [`StopSet`].
pub const MAX_BREAKPOINTS: usize = 8;

/// What ends a run besides the end of the frame. Fixed-size plain data, so building one never
/// allocates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StopSet {
    instructions: Option<u64>,
    /// The breakpoint to run over once: the one the last run stopped at.
    skip: Option<(bool, u32)>,
    arm9: [Option<u32>; MAX_BREAKPOINTS],
    arm7: [Option<u32>; MAX_BREAKPOINTS],
}

impl StopSet {
    /// Nothing but the end of the frame.
    pub const fn frame_end() -> Self {
        Self {
            instructions: None,
            skip: None,
            arm9: [None; MAX_BREAKPOINTS],
            arm7: [None; MAX_BREAKPOINTS],
        }
    }

    /// Also stop once `count` instructions have run, on either CPU, in this call.
    #[must_use]
    pub const fn with_instruction_limit(mut self, count: u64) -> Self {
        self.instructions = Some(count);
        self
    }

    /// Also stop before the instruction at `pc` runs, on the ARM9 (`arm9`) or the ARM7. A ninth
    /// breakpoint for one CPU is ignored.
    #[must_use]
    pub fn with_breakpoint(mut self, arm9: bool, pc: u32) -> Self {
        let list = if arm9 { &mut self.arm9 } else { &mut self.arm7 };
        if let Some(slot) = list.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(pc);
        }
        self
    }

    /// Continues from where the run that ended with `reason` stopped: when that was a breakpoint,
    /// the instruction there runs once instead of stopping again at once. Any other reason leaves the
    /// set as it is. A host that does not pass the last stop back in stops at the same breakpoint
    /// again.
    #[must_use]
    pub const fn resuming(mut self, reason: StopReason) -> Self {
        if let StopReason::Breakpoint { arm9, pc } = reason {
            self.skip = Some((arm9, pc));
        }
        self
    }

    fn has_breakpoint(&self, arm9: bool, pc: u32) -> bool {
        let list = if arm9 { &self.arm9 } else { &self.arm7 };
        list.contains(&Some(pc))
    }
}

/// Why a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// A frame was completed.
    FrameEnd,
    /// The instruction limit of the [`StopSet`] was reached.
    InstructionLimit,
    /// A CPU is about to run the instruction at a breakpoint.
    Breakpoint {
        /// Whether it is the ARM9 (else the ARM7).
        arm9: bool,
        /// The instruction's address.
        pc: u32,
    },
}

/// The stop conditions of the call in progress and what it has counted. Not state: it exists
/// only inside `run_until` (in [`Running`]), so it is not saved and the machine holds none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunControl {
    active: bool,
    stops: StopSet,
    executed: u64,
    resume: Option<(bool, u32)>,
    hit: Option<StopReason>,
}

impl RunControl {
    /// The conditions of a call with `stops`, the CPUs being about to run `arm9_pc` and `arm7_pc`:
    /// with only the frame end, a run asks nothing per instruction. The breakpoint to run over is
    /// kept only where its CPU is still at it, so a reason the host held on to across a step (the CPU
    /// has moved on) does not make the run skip the breakpoint's next hit.
    fn new(stops: &StopSet, arm9_pc: u32, arm7_pc: u32) -> Self {
        let at = |arm9: bool| if arm9 { arm9_pc } else { arm7_pc };
        Self {
            active: *stops != StopSet::frame_end(),
            stops: *stops,
            executed: 0,
            resume: stops.skip.filter(|&(arm9, pc)| at(arm9) == pc),
            hit: None,
        }
    }

    /// Whether any condition besides the frame end is set: with none, a run asks nothing per
    /// instruction.
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    /// Asked before each instruction of CPU `arm9` at `pc`: whether the run stops there. Counts the
    /// instruction when it does not.
    pub(crate) fn stops_before(&mut self, arm9: bool, pc: u32) -> bool {
        if self
            .stops
            .instructions
            .is_some_and(|limit| self.executed >= limit)
        {
            self.hit = Some(StopReason::InstructionLimit);
            return true;
        }
        // The breakpoint the run last stopped at runs once instead of stopping again.
        if self.resume == Some((arm9, pc)) {
            self.resume = None;
        } else if self.stops.has_breakpoint(arm9, pc) {
            self.hit = Some(StopReason::Breakpoint { arm9, pc });
            return true;
        }
        self.executed += 1;
        false
    }
}

impl Machine {
    /// Runs until a frame is complete or a condition of `stops` is met, and says which. A run that
    /// stopped before the end of the frame continues with the next call, which resumes the interrupted
    /// round: stopping and continuing gives the frames, the audio and the savestate bytes an
    /// uninterrupted [`Machine::run_frame`] gives. `input` is latched when a frame begins; a call that
    /// resumes a frame ignores it.
    ///
    /// A breakpoint stops the run before the instruction at its address runs. To go on from one, the
    /// host passes the reason back ([`StopSet::resuming`]) and the instruction there runs once.
    /// [`Machine::step_arm9`] and [`Machine::step_arm7`] first finish the interrupted round, so the
    /// step lands on a round boundary.
    pub fn run_until(&mut self, input: Input, stops: &StopSet) -> StopReason {
        let resuming = self.round.is_some();
        if !resuming {
            self.io.latch_input(input);
            self.audio.clear();
        }
        // A machine that was shut down runs nothing, as with no program: the frame only counts
        // (`touch-power.md`, "What the power register does").
        // TODO(hw-unverified): the frozen frames after shutdown are the model's (`touch-power.md`,
        // "Unverified").
        // A round in progress finishes: the frame it belongs to began before the shutdown, as an
        // uninterrupted run would have finished it.
        if !resuming && (self.header.is_none() || self.is_shut_down()) {
            self.frame_count = self.frame_count.wrapping_add(1);
            return StopReason::FrameEnd;
        }
        let started = self.frame_count;
        let control = RunControl::new(
            stops,
            self.arm9.next_instruction(),
            self.arm7.next_instruction(),
        );
        let mut running = Running {
            machine: self,
            control,
        };
        loop {
            let mut round = running
                .machine
                .round
                .take()
                .unwrap_or_else(|| scheduler::begin_round(&mut running));
            if !scheduler::continue_round(&mut running, &mut round) {
                // Only a stop condition ends a round early, and it says which.
                let hit = running.control.hit.take().unwrap_or(StopReason::FrameEnd);
                running.machine.round = Some(round);
                return hit;
            }
            if running.machine.frame_count != started {
                return StopReason::FrameEnd;
            }
        }
    }
}
