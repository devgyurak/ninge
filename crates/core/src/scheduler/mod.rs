// SPDX-License-Identifier: Apache-2.0
//! The event scheduler and the ARM9/ARM7 interleaving (`docs/hw/scheduler.md`).
//!
//! Events sit in a binary heap ordered by `(time, seq)`. A round runs the ARM9, then the ARM7,
//! then the events due at the target; see [`run_round`].

use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// ARM9 cycles per frame: 263 lines x 355 dots x 6 bus cycles x 2 (`scheduler.md`, "Frame timing").
pub const FRAME_CYCLES: u64 = 263 * 355 * 6 * 2;
/// Longest stretch one CPU runs before the other catches up, in master cycles (`scheduler.md`,
/// "Interleaving"). A model choice, not a hardware fact.
// TODO(hw-unverified): see `scheduler.md`, "Unverified". Decided in #102.
pub const SLICE: u64 = 64;
/// Master cycles one ARM9 instruction costs (`scheduler.md`, "Cost per instruction").
// TODO(hw-unverified): ignores waitstates and multi-cycle instructions (`scheduler.md`, "Unverified").
pub const ARM9_INSTRUCTION_COST: u64 = 1;
/// Master cycles one ARM7 instruction costs (`scheduler.md`, "Cost per instruction").
// TODO(hw-unverified): ignores waitstates and multi-cycle instructions (`scheduler.md`, "Unverified").
pub const ARM7_INSTRUCTION_COST: u64 = 2;
/// Events the queue holds without growing; reserved up front so scheduling on the hot path
/// does not allocate.
const EVENT_CAPACITY: usize = 16;

/// What an event does when it fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventKind {
    /// The end of a frame: the screens are drawn and the next frame end is scheduled.
    FrameEnd,
}

/// One scheduled event, ordered by `(time, seq)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Event {
    /// Master-clock time at which it fires.
    pub time: u64,
    /// Order in which it was scheduled; breaks ties between equal times.
    pub seq: u64,
    /// What it does.
    pub kind: EventKind,
}

/// A binary heap does not implement `PartialEq`; two queues are equal when their slots are,
/// which holds for equal histories (the machine is deterministic).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Queue(BinaryHeap<Reverse<Event>>);

impl PartialEq for Queue {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_slice() == other.0.as_slice()
    }
}

impl Eq for Queue {}

/// The master clock and the pending events (`docs/hw/scheduler.md`).
///
/// Decided in #102: a binary heap keyed by `(time, seq)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheduler {
    pub(crate) now: u64,
    pub(crate) next_seq: u64,
    queue: Queue,
}

impl Scheduler {
    /// A scheduler at time 0 with no events.
    pub fn new() -> Self {
        Self {
            now: 0,
            next_seq: 0,
            queue: Queue(BinaryHeap::with_capacity(EVENT_CAPACITY)),
        }
    }

    /// The master clock: how far the ARM9 has run, in ARM9 cycles.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Adds an event at `time`. It fires after every event already scheduled for the same time.
    pub fn schedule(&mut self, time: u64, kind: EventKind) {
        let seq = self.next_seq;
        // A u64 counter cannot realistically wrap; wrapping keeps a hand-made state from panicking.
        self.next_seq = seq.wrapping_add(1);
        self.queue.0.push(Reverse(Event { time, seq, kind }));
    }

    /// The time of the earliest pending event.
    pub fn next_time(&self) -> Option<u64> {
        self.queue.0.peek().map(|Reverse(event)| event.time)
    }

    /// Removes and returns the earliest event whose time is not after `now`.
    pub fn pop_due(&mut self, now: u64) -> Option<Event> {
        match self.next_time() {
            Some(time) if time <= now => self.queue.0.pop().map(|Reverse(event)| event),
            _ => None,
        }
    }

    /// The pending events in firing order.
    // The debugger's scheduler event timeline (#59) will show them.
    pub fn pending(&self) -> Vec<Event> {
        let mut events: Vec<Event> = self.queue.0.iter().map(|&Reverse(event)| event).collect();
        events.sort();
        events
    }

    /// Advances the master clock by `cycles`. Saturates: time never runs backwards.
    pub(crate) fn advance(&mut self, cycles: u64) {
        self.now = self.now.saturating_add(cycles);
    }

    /// The times of the queued events of `kind`, in no particular order.
    pub(crate) fn times_of(&self, kind: EventKind) -> Vec<u64> {
        self.queue
            .0
            .iter()
            .filter(|Reverse(event)| event.kind == kind)
            .map(|Reverse(event)| event.time)
            .collect()
    }

    /// Restores the queue's reserved capacity after a savestate load decoded it at its length.
    pub(crate) fn reserve(&mut self) {
        let spare = EVENT_CAPACITY.saturating_sub(self.queue.0.len());
        self.queue.0.reserve(spare);
    }

    /// Whether decoded state keeps the invariants the run loop relies on: every queued event is
    /// not before `now`, and `next_seq` is above every queued sequence number (`scheduler.md`,
    /// "Savestate checks"). A savestate that decodes to anything else is corrupt.
    pub(crate) fn is_well_formed(&self) -> bool {
        self.queue
            .0
            .iter()
            .all(|Reverse(event)| event.time >= self.now && event.seq < self.next_seq)
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// What one round of the run loop drives. `Machine` implements it; a test implements it with a
/// recorder to pin the order of a round. Static dispatch only (AGENTS.md §2 rule 3).
pub(crate) trait Participants {
    fn scheduler(&mut self) -> &mut Scheduler;
    /// Runs the ARM9 until the master clock reaches `target`.
    fn run_arm9_until(&mut self, target: u64);
    /// Runs the ARM7 until its clock reaches `target`.
    fn run_arm7_until(&mut self, target: u64);
    /// Handles a fired event.
    fn dispatch(&mut self, event: Event);
}

/// Fires every event due at `at`, in `(time, seq)` order, including ones a handler schedules.
fn dispatch_due<P: Participants>(participants: &mut P, at: u64) {
    while let Some(event) = participants.scheduler().pop_due(at) {
        participants.dispatch(event);
    }
}

/// One round: ARM9, then ARM7, then every event due at the target (`scheduler.md`,
/// "Interleaving"). The target is the earlier of the next event and `now + SLICE`, and never
/// before `now`.
pub(crate) fn run_round<P: Participants>(participants: &mut P) {
    let scheduler = participants.scheduler();
    let horizon = scheduler.now.saturating_add(SLICE);
    let target = scheduler
        .next_time()
        .map_or(horizon, |time| time.min(horizon))
        .max(scheduler.now);
    participants.run_arm9_until(target);
    participants.run_arm7_until(target);
    dispatch_due(participants, target);
}

/// Brings the ARM7 up to the ARM9 clock and fires what is due, after a lone ARM9 step
/// (`scheduler.md`, "Debugger single-step").
pub(crate) fn settle<P: Participants>(participants: &mut P) {
    let now = participants.scheduler().now;
    participants.run_arm7_until(now);
    dispatch_due(participants, now);
}

#[cfg(test)]
mod tests;
