// SPDX-License-Identifier: Apache-2.0
//! The traced run: [`Machine::run_frame_traced`] runs a frame like [`Machine::run_frame`] and
//! hands each executed instruction to a [`Tracer`] first (`docs/contributing/trace-oracle.md`).
//!
//! The untraced loops in `events.rs` stay exactly as they are, so tracing costs the hot path
//! nothing. [`Traced`] is a second `Participants` that steps one instruction at a time and records
//! it; a test pins that both give the same frames and savestate bytes, so the duplicated loop
//! cannot drift.

use super::Machine;
use crate::bus::{Arm7View, Arm9View, TcmConfig};
use crate::cpu::{ARM7_INSTRUCTION_COST, ARM9_INSTRUCTION_COST, Cpu};
use crate::hle::SwiRecord;
use crate::input::Input;
use crate::scheduler::{Event, Participants, Scheduler};
use crate::trace::{TraceSink, Tracer};

/// A machine and a tracer, run as one set of participants.
struct Traced<'a, S> {
    machine: &'a mut Machine,
    tracer: &'a mut Tracer<S>,
}

/// [`super::arm9_instruction`] with a record of the instruction that executes. An interrupt
/// entry and a BIOS trap take an instruction's place and leave no line: no instruction of the
/// BIOS executes in ninge (`docs/hw/hle-swi.md`), and the oracle's BIOS run is collapsed to match.
fn arm9_traced<S: TraceSink>(
    arm9: &mut Cpu<true>,
    bios: &mut SwiRecord,
    view: &mut Arm9View<'_>,
    tracer: &mut Tracer<S>,
) {
    if arm9.halted() {
        if !view.io().irq9().wakes() {
            view.advance(ARM9_INSTRUCTION_COST);
            return;
        }
        arm9.set_halted(false);
    }
    let irq = view.io().irq9().line();
    let now = view.clock();
    view.begin_step(now, now.saturating_add(ARM9_INSTRUCTION_COST));
    let cost = match arm9.take_interrupt(view, irq, false) {
        Some(cost) => cost,
        None => match crate::hle::enter(arm9, view, bios) {
            Some(cost) => cost,
            None => {
                tracer.record(arm9);
                arm9.step(view)
            }
        },
    };
    view.advance(cost);
}

/// [`super::arm7_instruction`] with a record, as [`arm9_traced`].
fn arm7_traced<S: TraceSink>(
    arm7: &mut Cpu<false>,
    bios: &mut SwiRecord,
    view: &mut Arm7View<'_>,
    tracer: &mut Tracer<S>,
) {
    if arm7.halted() {
        if !view.io().irq7().wakes() {
            view.advance(ARM7_INSTRUCTION_COST);
            return;
        }
        arm7.set_halted(false);
    }
    let irq = view.io().irq7().line();
    view.begin_step(view.clock(), view.master_clock());
    let cost = match arm7.take_interrupt(view, irq, false) {
        Some(cost) => cost,
        None => match crate::hle::enter(arm7, view, bios) {
            Some(cost) => cost,
            None => {
                tracer.record(arm7);
                arm7.step(view)
            }
        },
    };
    if view.io_mut().power_mut().take_halt() {
        arm7.set_halted(true);
    }
    view.advance(cost);
}

impl<S: TraceSink> Participants for Traced<'_, S> {
    fn scheduler(&mut self) -> &mut Scheduler {
        self.machine.scheduler()
    }

    fn run_arm9_until(&mut self, target: u64) {
        let machine = &mut *self.machine;
        let mut tcm_key = machine.arm9.cp15().tcm_key();
        let mut view = Arm9View::new(
            &mut machine.memory,
            &mut machine.io,
            &mut machine.scheduler,
            &machine.rom,
            TcmConfig::from_cp15(machine.arm9.cp15()),
        );
        while view.clock() < target
            && view
                .next_event_time()
                .is_none_or(|time| view.clock() < time)
        {
            if machine.arm9.halted() && !view.io().irq9().wakes() {
                let stop = view
                    .next_event_time()
                    .map_or(target, |time| time.min(target));
                view.advance(stop - view.clock());
                break;
            }
            arm9_traced(
                &mut machine.arm9,
                &mut machine.bios.arm9,
                &mut view,
                self.tracer,
            );
            let key = machine.arm9.cp15().tcm_key();
            if key != tcm_key {
                tcm_key = key;
                view.set_tcm(TcmConfig::from_cp15(machine.arm9.cp15()));
            }
        }
    }

    fn run_arm7_until(&mut self, target: u64) {
        let machine = &mut *self.machine;
        let mut view = Arm7View::new(
            &mut machine.memory,
            &mut machine.io,
            &mut machine.scheduler,
            &machine.rom,
            machine.arm7_clock,
        );
        while view.clock() < target {
            if machine.arm7.halted() && !view.io().irq7().wakes() {
                view.advance(target - view.clock());
                break;
            }
            arm7_traced(
                &mut machine.arm7,
                &mut machine.bios.arm7,
                &mut view,
                self.tracer,
            );
        }
        machine.arm7_clock = view.clock();
    }

    fn dispatch(&mut self, event: Event) {
        self.machine.dispatch(event);
    }
}

impl Machine {
    /// Runs one frame like [`Machine::run_frame`], handing every executed instruction of the CPUs
    /// the `tracer` selects to its sink before it runs. The frames, the audio and the savestate
    /// bytes are the same as an untraced run's.
    ///
    /// Instructions the BIOS would run (an SWI, the IRQ entry and return, a wait loop) leave no
    /// record: ninge does them in Rust, and the oracle's side is collapsed to match
    /// (`docs/contributing/trace-oracle.md`, "BIOS collapsing").
    pub fn run_frame_traced<S: TraceSink>(&mut self, input: Input, tracer: &mut Tracer<S>) {
        self.io.latch_input(input);
        self.audio.clear();
        if self.header.is_none() || self.is_shut_down() {
            self.frame_count = self.frame_count.wrapping_add(1);
            return;
        }
        let started = self.frame_count;
        let mut traced = Traced {
            machine: self,
            tracer,
        };
        while traced.machine.frame_count == started {
            crate::scheduler::run_round(&mut traced);
        }
    }
}
