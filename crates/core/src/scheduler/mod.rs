// SPDX-License-Identifier: Apache-2.0
//! The event scheduler and the ARM9/ARM7 interleaving (`docs/hw/scheduler.md`).
//!
//! Events sit in a binary heap ordered by `(time, priority of the kind)`; the queue holds at most
//! one event of each kind. A round runs the ARM9, then the ARM7,
//! then the events due where the ARM9 stopped: the round's target, or earlier when the ARM9
//! queued an event before it; see [`run_round`].

use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

/// ARM9 cycles per line: 355 dots x 6 bus cycles x 2 (`display-timing.md`, "Frame layout").
pub const LINE_CYCLES: u64 = 355 * 6 * 2;
/// ARM9 cycles from the start of a line to its HBlank: 1,606 bus cycles x 2
/// (`display-timing.md`, "Flags").
// TODO(hw-unverified): one point for both CPUs (`display-timing.md`, "Unverified").
pub const HBLANK_START_CYCLES: u64 = 1_606 * 2;
/// ARM9 cycles per frame: 263 lines (`scheduler.md`, "Frame timing").
pub const FRAME_CYCLES: u64 = 263 * LINE_CYCLES;
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
/// Events the queue holds without growing: a line start, an HBlank start, eight timers and eight
/// DMA starts. Reserved up front so scheduling on the hot path does not allocate.
const EVENT_CAPACITY: usize = 24;

/// The latest time the clock or an event may hold, about 2,180 years of emulated time. A
/// savestate beyond it is refused, so line starts never reach the saturation point of `u64`.
pub const MAX_TIME: u64 = 1 << 62;

/// What an event does when it fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// A line begins: VCOUNT advances and the next line start is scheduled. The start of line 0
    /// ends the frame, which draws the screens (`display-timing.md`).
    LineStart,
    /// HBlank begins in the current line and the next one is scheduled.
    HBlankStart,
    /// A timer overflows: it reloads, the IRQ is requested, the timer it feeds ticks and the next
    /// overflow is scheduled (`timers.md`, "Model"). The two CPUs have four timers each.
    TimerOverflow {
        /// Whether it is the ARM9's timer; the ARM7's otherwise.
        arm9: bool,
        /// Which timer, 0-3.
        index: u8,
    },
    /// A DMA channel's transfer starts: the data moves and the channel repeats or ends
    /// (`dma.md`, "Model"). The two CPUs have four channels each.
    DmaStart {
        /// Whether it is the ARM9's channel; the ARM7's otherwise.
        arm9: bool,
        /// Which channel, 0-3.
        channel: u8,
    },
}

impl EventKind {
    /// Where the kind sorts among events at the same time (`scheduler.md`, "Events"): line start,
    /// HBlank start, timers, then DMA starts, the ARM9's before the ARM7's, lower index first.
    /// Explicit, not derived: `bool` sorts `false` first, which would put the ARM7 ahead.
    // TODO(hw-unverified): the order is a model choice (`scheduler.md`, "Unverified").
    fn priority(self) -> (u8, u8, u8) {
        match self {
            Self::LineStart => (0, 0, 0),
            Self::HBlankStart => (1, 0, 0),
            Self::TimerOverflow { arm9, index } => (2, u8::from(!arm9), index),
            Self::DmaStart { arm9, channel } => (3, u8::from(!arm9), channel),
        }
    }
}

/// One scheduled event, ordered by `(time, priority of the kind)`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Event {
    /// Master-clock time at which it fires.
    pub time: u64,
    /// Order in which it was scheduled in this run. Not saved and not part of the order: it only
    /// tells an event queued before a dispatch from one a handler queued during it.
    pub seq: u64,
    /// What it does.
    pub kind: EventKind,
}

// Ordered by `(time, priority)` only, so `Ord` agrees with `PartialEq`. `seq` is not part of the
// order: it is read only by the dispatch guard (`pop_due_before`).
impl Ord for Event {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time
            .cmp(&other.time)
            .then_with(|| self.kind.priority().cmp(&other.kind.priority()))
    }
}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Two events are equal when they fire at the same time and do the same thing; `seq` is
/// bookkeeping of one run, so a rebuilt queue equals the one it was rebuilt from.
impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.kind == other.kind
    }
}

impl Eq for Event {}

/// A binary heap does not implement `PartialEq`. The queue holds at most one event of each kind,
/// so two queues are equal when each event of one is in the other; no allocation.
#[derive(Clone, Debug)]
struct Queue(BinaryHeap<Reverse<Event>>);

impl Queue {
    fn new() -> Self {
        Self(BinaryHeap::with_capacity(EVENT_CAPACITY))
    }
}

impl PartialEq for Queue {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .all(|event| other.0.iter().any(|other_event| other_event == event))
    }
}

impl Eq for Queue {}

/// The master clock and the pending events (`docs/hw/scheduler.md`).
///
/// Decided in #102 (a binary heap) and #112: keyed by `(time, priority of the kind)`, and not
/// saved. Only `now` is; a load rebuilds the queue from the devices (`Machine::schedule_events`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scheduler {
    pub(crate) now: u64,
    // Runtime only: the dispatch guard's counter (see `Event::seq`).
    #[serde(skip)]
    pub(crate) next_seq: u64,
    #[serde(skip, default = "Queue::new")]
    queue: Queue,
}

/// Equal when the clock and the pending events are; `next_seq` is bookkeeping of one run, which
/// a loaded machine does not share with the one it was saved from.
impl PartialEq for Scheduler {
    fn eq(&self, other: &Self) -> bool {
        self.now == other.now && self.queue == other.queue
    }
}

impl Eq for Scheduler {}

impl Scheduler {
    /// A scheduler at time 0 with no events.
    pub fn new() -> Self {
        Self {
            now: 0,
            next_seq: 0,
            queue: Queue::new(),
        }
    }

    /// The master clock: how far the ARM9 has run, in ARM9 cycles.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Adds an event at `time`. Among events at the same time it fires in the order of its kind's
    /// priority, not of scheduling (`scheduler.md`, "Events").
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

    /// Removes every pending event of `kind`. The queue holds at most 24 events, so the scan is
    /// cheap and allocates nothing.
    pub(crate) fn cancel(&mut self, kind: EventKind) {
        self.queue.0.retain(|Reverse(event)| event.kind != kind);
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

    /// Like [`Scheduler::pop_due`], but only an event scheduled before the sequence number
    /// `before_seq`. An event a handler schedules waits for the next round.
    pub(crate) fn pop_due_before(&mut self, now: u64, before_seq: u64) -> Option<Event> {
        match self.queue.0.peek() {
            Some(Reverse(event)) if event.time <= now && event.seq < before_seq => {
                self.queue.0.pop().map(|Reverse(event)| event)
            }
            _ => None,
        }
    }

    /// Whether decoded state keeps the invariant the run loop relies on: `now` is not above
    /// `MAX_TIME` (`scheduler.md`, "Savestate checks"). The queue is not saved, so it has none.
    pub(crate) fn is_well_formed(&self) -> bool {
        self.now <= MAX_TIME
    }

    /// The first line start strictly after `now`: line starts sit on the grid
    /// `k * LINE_CYCLES` (`scheduler.md`, "Savestate checks").
    pub(crate) fn next_line_start(now: u64) -> u64 {
        Self::next_on_grid(now, 0)
    }

    /// The first HBlank start strictly after `now`, on the grid
    /// `k * LINE_CYCLES + HBLANK_START_CYCLES`.
    pub(crate) fn next_hblank_start(now: u64) -> u64 {
        Self::next_on_grid(now, HBLANK_START_CYCLES)
    }

    fn next_on_grid(now: u64, offset: u64) -> u64 {
        match now.checked_sub(offset) {
            None => offset,
            Some(since) => (since / LINE_CYCLES)
                .saturating_add(1)
                .saturating_mul(LINE_CYCLES)
                .saturating_add(offset),
        }
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
    /// Runs the ARM9 until the master clock reaches `target` or the earliest queued event,
    /// whichever is earlier, checked before every instruction. It may queue events while it runs.
    fn run_arm9_until(&mut self, target: u64);
    /// Runs the ARM7 until its clock reaches `target`: where the ARM9 stopped, not the round's
    /// original target.
    fn run_arm7_until(&mut self, target: u64);
    /// Handles a fired event.
    fn dispatch(&mut self, event: Event);
}

/// Fires every event due at `at` that was queued when this started, in `(time, seq)` order.
///
/// An event a handler schedules waits for the next round. If a clock ever saturated, an event
/// that reschedules itself into the past would otherwise be popped forever and hang the host.
fn dispatch_due<P: Participants>(participants: &mut P, at: u64) {
    let before_seq = participants.scheduler().next_seq;
    while let Some(event) = participants.scheduler().pop_due_before(at, before_seq) {
        participants.dispatch(event);
    }
}

/// One round: ARM9, then ARM7, then every event due where the ARM9 stopped (`scheduler.md`,
/// "Interleaving"). The target is the earlier of the next event and `now + SLICE`, and never
/// before `now`. The ARM9 stops at `stop`, the earlier of the target and any event it queued;
/// the ARM7 and the events use its actual clock, so a later multi-cycle cost stays right.
pub(crate) fn run_round<P: Participants>(participants: &mut P) {
    let scheduler = participants.scheduler();
    let horizon = scheduler.now.saturating_add(SLICE);
    let target = scheduler
        .next_time()
        .map_or(horizon, |time| time.min(horizon))
        .max(scheduler.now);
    participants.run_arm9_until(target);
    let stop = participants.scheduler().now;
    participants.run_arm7_until(stop);
    dispatch_due(participants, stop);
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
