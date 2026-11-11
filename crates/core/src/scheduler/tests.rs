// SPDX-License-Identifier: Apache-2.0
//! The event queue and the round order. Expected values come from `docs/hw/scheduler.md`:
//! "Events" (order is (timestamp, priority of the kind): LineStart, HBlankStart, timer overflows
//! ARM9 0-3 then ARM7 0-3, DMA starts ARM9 0-3 then ARM7 0-3; never the scheduling order; a
//! handler's event waits for the next round), "Interleaving" (target
//! is the smaller of the next event and now + SLICE, `now` when an event is already due; the ARM9
//! stops early at an event it queues before the target, and the ARM7 and the dispatch then use
//! that stop; ARM9, then ARM7, then events), "Debugger single-step" and "Savestate checks" (the
//! clock is at most MAX_TIME; the queue is rebuilt on load, so it is not checked), plus
//! its "Edge cases".

use super::*;

/// What a recording participant saw, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Arm9(u64),
    Arm7(u64),
    /// A dispatched event: (time, kind).
    Dispatch(u64, EventKind),
}

/// Participants that log each call. The ARM9 moves the master clock to the target, as the
/// real ARM9 does (it runs until `now` reaches the target).
struct Recorder {
    scheduler: Scheduler,
    log: Vec<Step>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            scheduler: Scheduler::new(),
            log: Vec::new(),
        }
    }

    /// A recorder whose clock is at `now`.
    fn at(now: u64) -> Self {
        let mut recorder = Self::new();
        recorder.scheduler.now = now;
        recorder
    }

    fn with_events(now: u64, times: &[u64]) -> Self {
        let mut recorder = Self::at(now);
        schedule_all(&mut recorder.scheduler, times);
        recorder
    }
}

impl Participants for Recorder {
    fn scheduler(&mut self) -> &mut Scheduler {
        &mut self.scheduler
    }
    fn run_arm9_until(&mut self, target: u64) {
        self.log.push(Step::Arm9(target));
        self.scheduler.now = self.scheduler.now.max(target);
    }
    fn run_arm7_until(&mut self, target: u64) {
        self.log.push(Step::Arm7(target));
    }
    fn dispatch(&mut self, event: Event) {
        self.log.push(Step::Dispatch(event.time, event.kind));
    }
}

fn schedule_all(scheduler: &mut Scheduler, times: &[u64]) {
    times
        .iter()
        .for_each(|&time| scheduler.schedule(time, EventKind::LineStart));
}

/// Pops everything due at `now`, as `(time, kind)`.
fn drain(scheduler: &mut Scheduler, now: u64) -> Vec<(u64, EventKind)> {
    std::iter::from_fn(|| scheduler.pop_due(now))
        .map(|event| (event.time, event.kind))
        .collect()
}

// ------------------------------------------------------------------- the queue

#[test]
fn given_events_out_of_order_when_popped_then_they_come_in_timestamp_order() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[50, 10, 30, 20, 40]);
    let times: Vec<u64> = drain(&mut scheduler, 100)
        .into_iter()
        .map(|(time, _)| time)
        .collect();
    assert_eq!(times, vec![10, 20, 30, 40, 50]);
}

#[test]
fn given_event_in_future_when_popping_due_at_now_then_none_is_returned() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[100]);
    let popped = scheduler.pop_due(99);
    assert_eq!((popped, scheduler.pending().len()), (None, 1));
}

#[test]
fn given_event_exactly_at_now_when_popping_due_then_it_is_returned() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[100]);
    let popped = scheduler.pop_due(100).map(|event| event.time);
    assert_eq!(popped, Some(100));
}

#[test]
fn given_one_due_and_one_future_event_when_popping_due_twice_then_only_the_due_one_comes_out() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[200, 10]);
    let first = scheduler.pop_due(100).map(|event| event.time);
    let second = scheduler.pop_due(100);
    assert_eq!((first, second), (Some(10), None));
}

#[test]
fn given_empty_queue_when_popping_due_then_none_is_returned() {
    let mut scheduler = Scheduler::new();
    assert_eq!(scheduler.pop_due(u64::MAX), None);
}

#[test]
fn given_popped_event_when_popping_again_then_it_does_not_come_back() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[10]);
    let first = scheduler.pop_due(10).is_some();
    let second = scheduler.pop_due(10);
    assert_eq!((first, second), (true, None));
}

#[test]
fn given_an_event_when_scheduled_then_it_keeps_its_time_and_kind() {
    let mut scheduler = Scheduler::new();
    scheduler.schedule(77, EventKind::LineStart);
    let event = scheduler.pop_due(77).expect("due");
    assert_eq!((event.time, event.kind), (77, EventKind::LineStart));
}

// ------------------------------------------------------- tie order is the kind's priority

const fn dma(arm9: bool, channel: u8) -> EventKind {
    EventKind::DmaStart { arm9, channel }
}
const fn timer(arm9: bool, index: u8) -> EventKind {
    EventKind::TimerOverflow { arm9, index }
}

/// Every kind in the order `scheduler.md` ("Events") fires it when several share one time:
/// line start, HBlank start, ARM9 timers 0-3, ARM7 timers 0-3, ARM9 DMA 0-3, ARM7 DMA 0-3.
const PRIORITY_ORDER: [EventKind; 18] = [
    EventKind::LineStart,
    EventKind::HBlankStart,
    timer(true, 0),
    timer(true, 1),
    timer(true, 2),
    timer(true, 3),
    timer(false, 0),
    timer(false, 1),
    timer(false, 2),
    timer(false, 3),
    dma(true, 0),
    dma(true, 1),
    dma(true, 2),
    dma(true, 3),
    dma(false, 0),
    dma(false, 1),
    dma(false, 2),
    dma(false, 3),
];

/// The order the kinds in `scheduled` pop at one shared time when queued in that order.
fn popped_at_one_time(scheduled: &[EventKind]) -> Vec<EventKind> {
    let mut scheduler = Scheduler::new();
    scheduled
        .iter()
        .for_each(|&kind| scheduler.schedule(10, kind));
    drain(&mut scheduler, 10)
        .into_iter()
        .map(|(_, kind)| kind)
        .collect()
}

/// `PRIORITY_ORDER` visited with a stride coprime to 18, a fixed stand-in for a shuffle.
fn strided(stride: usize) -> Vec<EventKind> {
    (0..PRIORITY_ORDER.len())
        .map(|i| PRIORITY_ORDER[(i * stride) % PRIORITY_ORDER.len()])
        .collect()
}

/// Every schedule order tried (in order, reversed, three strides, rotated) whose pop order is
/// not `PRIORITY_ORDER`.
fn schedule_orders_that_break_priority() -> Vec<Vec<EventKind>> {
    let mut reversed = PRIORITY_ORDER.to_vec();
    reversed.reverse();
    let mut rotated = PRIORITY_ORDER.to_vec();
    rotated.rotate_left(7);
    [
        PRIORITY_ORDER.to_vec(),
        reversed,
        rotated,
        strided(5),
        strided(7),
        strided(11),
    ]
    .into_iter()
    .filter(|order| popped_at_one_time(order) != PRIORITY_ORDER)
    .collect()
}

#[test]
fn given_every_kind_at_one_time_when_scheduled_in_many_orders_then_each_pops_in_priority_order() {
    assert_eq!(
        schedule_orders_that_break_priority(),
        Vec::<Vec<EventKind>>::new()
    );
}

/// A scheduler holding every kind at time 10, queued from the lowest priority to the highest.
fn scheduled_in_reverse_priority() -> Scheduler {
    let mut scheduler = Scheduler::new();
    PRIORITY_ORDER
        .iter()
        .rev()
        .for_each(|&kind| scheduler.schedule(10, kind));
    scheduler
}

#[test]
fn given_every_kind_at_one_time_when_scheduled_in_reverse_and_listed_then_pending_is_in_priority_order()
 {
    let scheduler = scheduled_in_reverse_priority();
    let listed: Vec<EventKind> = scheduler.pending().iter().map(|e| e.kind).collect();
    assert_eq!(listed, PRIORITY_ORDER.to_vec());
}

#[test]
fn given_the_arm7_timer_queued_first_when_the_arm9_timer_of_the_same_index_ties_then_the_arm9_pops_first()
 {
    assert_eq!(
        popped_at_one_time(&[timer(false, 1), timer(true, 1)]),
        vec![timer(true, 1), timer(false, 1)]
    );
}

#[test]
fn given_an_arm7_timer_0_and_an_arm9_timer_3_when_they_tie_then_the_arm9_timer_pops_first() {
    // The ARM9 group precedes the ARM7 group whatever the indices.
    assert_eq!(
        popped_at_one_time(&[timer(false, 0), timer(true, 3)]),
        vec![timer(true, 3), timer(false, 0)]
    );
}

#[test]
fn given_a_dma_start_queued_before_a_timer_overflow_when_they_tie_then_the_timer_pops_first() {
    assert_eq!(
        popped_at_one_time(&[dma(true, 0), timer(false, 3)]),
        vec![timer(false, 3), dma(true, 0)]
    );
}

#[test]
fn given_an_arm7_dma_start_queued_before_an_arm9_one_when_they_tie_then_the_arm9_one_pops_first() {
    assert_eq!(
        popped_at_one_time(&[dma(false, 0), dma(true, 3)]),
        vec![dma(true, 3), dma(false, 0)]
    );
}

#[test]
fn given_dma_channels_queued_in_reverse_when_they_tie_then_they_pop_in_channel_order() {
    assert_eq!(
        popped_at_one_time(&[dma(true, 2), dma(true, 0), dma(true, 1)]),
        vec![dma(true, 0), dma(true, 1), dma(true, 2)]
    );
}

#[test]
fn given_a_low_priority_event_at_an_earlier_time_when_popped_then_it_still_comes_before_a_later_line_start()
 {
    let mut scheduler = Scheduler::new();
    scheduler.schedule(11, EventKind::LineStart);
    scheduler.schedule(10, dma(false, 3));
    assert_eq!(
        drain(&mut scheduler, 11),
        vec![(10, dma(false, 3)), (11, EventKind::LineStart)]
    );
}

// ------------------------------------------------------- next_time and pending

#[test]
fn given_no_events_when_asking_next_time_then_it_is_none() {
    assert_eq!(Scheduler::new().next_time(), None);
}

#[test]
fn given_events_out_of_order_when_asking_next_time_then_it_is_the_earliest() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[30, 10, 20]);
    assert_eq!(scheduler.next_time(), Some(10));
}

#[test]
fn given_earliest_event_popped_when_asking_next_time_then_it_is_the_next_earliest() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[30, 10, 20]);
    scheduler.pop_due(10);
    assert_eq!(scheduler.next_time(), Some(20));
}

#[test]
fn given_events_out_of_order_when_listing_pending_then_they_are_in_firing_order() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[30, 10, 20, 10]);
    let order: Vec<u64> = scheduler.pending().iter().map(|event| event.time).collect();
    assert_eq!(order, vec![10, 10, 20, 30]);
}

#[test]
fn given_pending_listed_twice_when_compared_then_listing_did_not_consume_the_queue() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[10, 20]);
    let first = scheduler.pending();
    let second = scheduler.pending();
    assert_eq!((first.len(), first == second), (2, true));
}

// --------------------------------------------------------------- the clock

#[test]
fn given_new_scheduler_when_reading_it_then_now_is_zero_and_nothing_is_pending() {
    let scheduler = Scheduler::new();
    assert_eq!((scheduler.now(), scheduler.pending().len()), (0, 0));
}

#[test]
fn given_the_clock_when_advanced_twice_then_it_is_the_sum() {
    let mut scheduler = Scheduler::new();
    scheduler.advance(3);
    scheduler.advance(4);
    assert_eq!(scheduler.now(), 7);
}

#[test]
fn given_frame_cycles_when_computed_from_the_display_timing_then_it_is_1120380() {
    // 263 lines x 355 dots x 6 bus cycles x 2 (scheduler.md, "Frame timing").
    assert_eq!(FRAME_CYCLES, 1_120_380);
}

#[test]
fn given_the_slice_constant_when_read_then_it_is_64() {
    assert_eq!(SLICE, 64);
}

// -------------------------------------------------------- savestate validity

#[test]
fn given_new_scheduler_when_checked_then_it_is_well_formed() {
    assert!(Scheduler::new().is_well_formed());
}

#[test]
fn given_events_scheduled_through_the_api_when_checked_then_it_is_well_formed() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[10, 20, 10]);
    assert!(scheduler.is_well_formed());
}

#[test]
fn given_clock_at_max_time_with_no_events_when_checked_then_it_is_accepted() {
    let mut scheduler = Scheduler::new();
    scheduler.now = MAX_TIME;
    assert!(scheduler.is_well_formed());
}

#[test]
fn given_clock_past_max_time_with_no_events_when_checked_then_it_is_refused() {
    let mut scheduler = Scheduler::new();
    scheduler.now = MAX_TIME + 1;
    assert!(!scheduler.is_well_formed());
}

// ------------------------------------------------------------- the round

#[test]
fn given_event_due_exactly_at_the_target_when_running_a_round_then_arm9_then_arm7_then_the_event() {
    let mut recorder = Recorder::with_events(0, &[10]);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(10),
            Step::Arm7(10),
            Step::Dispatch(10, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_no_event_nearer_than_a_slice_when_running_a_round_then_target_is_now_plus_slice() {
    let mut recorder = Recorder::with_events(100, &[100 + SLICE + 1]);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm9(100 + SLICE), Step::Arm7(100 + SLICE)]
    );
}

#[test]
fn given_no_events_at_all_when_running_a_round_then_target_is_now_plus_slice() {
    let mut recorder = Recorder::at(1000);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm9(1000 + SLICE), Step::Arm7(1000 + SLICE)]
    );
}

#[test]
fn given_event_exactly_one_slice_away_when_running_a_round_then_it_is_dispatched_at_the_target() {
    let mut recorder = Recorder::with_events(0, &[SLICE]);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(SLICE),
            Step::Arm7(SLICE),
            Step::Dispatch(SLICE, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_event_nearer_than_a_slice_when_running_a_round_then_target_is_the_event_time() {
    let mut recorder = Recorder::with_events(100, &[120]);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(120),
            Step::Arm7(120),
            Step::Dispatch(120, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_event_at_now_when_running_a_round_then_target_is_now_and_the_event_is_dispatched() {
    let mut recorder = Recorder::with_events(100, &[100]);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(100),
            Step::Arm7(100),
            Step::Dispatch(100, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_event_already_overdue_when_running_a_round_then_target_is_now_and_the_event_is_dispatched()
{
    let mut recorder = Recorder::with_events(100, &[50]);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(100),
            Step::Arm7(100),
            Step::Dispatch(50, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_events_scheduled_in_reverse_priority_at_the_target_when_running_a_round_then_they_dispatch_in_priority_order()
 {
    // `scheduler.md`, "Events": a timer overflow, then HBlank, then the line start were queued
    // (the reverse of their priority); the line start fires first, the timer last.
    let mut recorder = Recorder::at(0);
    recorder.scheduler.schedule(30, ARM9_TIMER0);
    recorder.scheduler.schedule(30, EventKind::HBlankStart);
    recorder.scheduler.schedule(30, EventKind::LineStart);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(30),
            Step::Arm7(30),
            Step::Dispatch(30, EventKind::LineStart),
            Step::Dispatch(30, EventKind::HBlankStart),
            Step::Dispatch(30, ARM9_TIMER0)
        ]
    );
}

#[test]
fn given_an_event_past_the_target_when_running_a_round_then_it_stays_queued() {
    let mut recorder = Recorder::with_events(0, &[SLICE + 1]);
    run_round(&mut recorder);
    assert_eq!(recorder.scheduler.pending().len(), 1);
}

#[test]
fn given_a_dispatched_event_when_running_a_round_then_it_is_no_longer_queued() {
    let mut recorder = Recorder::with_events(0, &[10]);
    run_round(&mut recorder);
    assert_eq!(recorder.scheduler.pending().len(), 0);
}

#[test]
fn given_events_at_two_times_when_running_two_rounds_then_each_round_dispatches_its_own() {
    let mut recorder = Recorder::at(0);
    schedule_all(&mut recorder.scheduler, &[10, 20]);
    run_round(&mut recorder);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(10),
            Step::Arm7(10),
            Step::Dispatch(10, EventKind::LineStart),
            Step::Arm9(20),
            Step::Arm7(20),
            Step::Dispatch(20, EventKind::LineStart)
        ]
    );
}

/// Participants whose dispatch schedules another event at the time it just fired, as a
/// saturated clock would make a self-rescheduling handler do.
struct SelfScheduling {
    inner: Recorder,
}

impl Participants for SelfScheduling {
    fn scheduler(&mut self) -> &mut Scheduler {
        self.inner.scheduler()
    }
    fn run_arm9_until(&mut self, target: u64) {
        self.inner.run_arm9_until(target);
    }
    fn run_arm7_until(&mut self, target: u64) {
        self.inner.run_arm7_until(target);
    }
    fn dispatch(&mut self, event: Event) {
        self.inner.dispatch(event);
        self.inner.scheduler.schedule(event.time, event.kind);
    }
}

#[test]
fn given_a_handler_scheduling_a_due_event_when_running_a_round_then_it_waits_for_the_next_round() {
    let mut participants = SelfScheduling {
        inner: Recorder::with_events(0, &[10]),
    };
    run_round(&mut participants);
    let dispatched = participants
        .inner
        .log
        .iter()
        .filter(|step| matches!(step, Step::Dispatch(..)))
        .count();
    assert_eq!(
        (dispatched, participants.inner.scheduler.pending().len()),
        (1, 1)
    );
}

#[test]
fn given_a_handler_scheduling_a_due_event_when_running_two_rounds_then_each_round_dispatches_one() {
    let mut participants = SelfScheduling {
        inner: Recorder::with_events(0, &[10]),
    };
    run_round(&mut participants);
    run_round(&mut participants);
    let dispatched = participants
        .inner
        .log
        .iter()
        .filter(|step| matches!(step, Step::Dispatch(..)))
        .count();
    assert_eq!(dispatched, 2);
}

/// Participants whose ARM9 queues an event at `queued_at` while it runs and stops there, as the
/// real ARM9 does when it must not run past an event it queued.
struct Arm9Queues {
    inner: Recorder,
    queued_at: u64,
}

impl Participants for Arm9Queues {
    fn scheduler(&mut self) -> &mut Scheduler {
        self.inner.scheduler()
    }
    fn run_arm9_until(&mut self, target: u64) {
        self.inner.log.push(Step::Arm9(target));
        self.inner
            .scheduler
            .schedule(self.queued_at, EventKind::LineStart);
        self.inner.scheduler.now = self.queued_at;
    }
    fn run_arm7_until(&mut self, target: u64) {
        self.inner.run_arm7_until(target);
    }
    fn dispatch(&mut self, event: Event) {
        self.inner.dispatch(event);
    }
}

#[test]
fn given_the_arm9_queues_an_event_before_the_target_when_running_a_round_then_arm7_and_dispatch_use_the_stop()
 {
    // `scheduler.md`, "Interleaving": the ARM9 stops at the event it queued (20), so the ARM7
    // runs to 20 and the event fires at 20, not at the old target (now + SLICE).
    let mut participants = Arm9Queues {
        inner: Recorder::at(0),
        queued_at: 20,
    };
    run_round(&mut participants);
    assert_eq!(
        participants.inner.log,
        vec![
            Step::Arm9(SLICE),
            Step::Arm7(20),
            Step::Dispatch(20, EventKind::LineStart)
        ]
    );
}

// ------------------------------------------------------------------- settle

#[test]
fn given_event_due_at_now_when_settling_then_arm7_runs_to_now_and_then_the_event_dispatches() {
    let mut recorder = Recorder::with_events(40, &[40]);
    settle(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm7(40), Step::Dispatch(40, EventKind::LineStart)]
    );
}

#[test]
fn given_event_in_the_future_when_settling_then_only_the_arm7_runs() {
    let mut recorder = Recorder::with_events(40, &[41]);
    settle(&mut recorder);
    assert_eq!(recorder.log, vec![Step::Arm7(40)]);
}

#[test]
fn given_event_overdue_when_settling_then_it_is_dispatched_after_the_arm7() {
    let mut recorder = Recorder::with_events(40, &[30]);
    settle(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm7(40), Step::Dispatch(30, EventKind::LineStart)]
    );
}

#[test]
fn given_no_events_when_settling_then_the_arm9_is_not_run() {
    let mut recorder = Recorder::at(40);
    settle(&mut recorder);
    assert_eq!(recorder.log, vec![Step::Arm7(40)]);
}

#[test]
fn given_events_scheduled_in_reverse_priority_when_settling_then_they_dispatch_in_priority_order() {
    let mut recorder = Recorder::at(40);
    recorder.scheduler.schedule(40, ARM7_TIMER0);
    recorder.scheduler.schedule(40, ARM9_TIMER0);
    recorder.scheduler.schedule(40, EventKind::LineStart);
    settle(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm7(40),
            Step::Dispatch(40, EventKind::LineStart),
            Step::Dispatch(40, ARM9_TIMER0),
            Step::Dispatch(40, ARM7_TIMER0)
        ]
    );
}

// ------------------------------------------------------------------------- cancel
// `cancel` is the scheduler half of "a write to TMnCNT_H replaces that timer's event"
// (`docs/hw/timers.md`, "Model"): it drops every pending event of one kind and nothing else.

const ARM9_TIMER0: EventKind = EventKind::TimerOverflow {
    arm9: true,
    index: 0,
};
const ARM9_TIMER1: EventKind = EventKind::TimerOverflow {
    arm9: true,
    index: 1,
};
const ARM7_TIMER0: EventKind = EventKind::TimerOverflow {
    arm9: false,
    index: 0,
};

/// The `(time, kind)` of every pending event, in firing order.
fn pending_pairs(scheduler: &Scheduler) -> Vec<(u64, EventKind)> {
    scheduler
        .pending()
        .iter()
        .map(|event| (event.time, event.kind))
        .collect()
}

#[test]
fn given_one_timer_event_when_cancelled_then_nothing_is_pending() {
    let mut scheduler = Scheduler::new();
    scheduler.schedule(100, ARM9_TIMER0);
    scheduler.cancel(ARM9_TIMER0);
    assert_eq!(pending_pairs(&scheduler), vec![]);
}

#[test]
fn given_two_events_of_one_kind_when_cancelled_then_both_are_removed() {
    let mut scheduler = Scheduler::new();
    scheduler.schedule(100, ARM9_TIMER0);
    scheduler.schedule(200, ARM9_TIMER0);
    scheduler.cancel(ARM9_TIMER0);
    assert_eq!(pending_pairs(&scheduler), vec![]);
}

#[test]
fn given_events_of_other_kinds_when_one_kind_is_cancelled_then_the_others_stay_in_order() {
    let mut scheduler = Scheduler::new();
    scheduler.schedule(30, EventKind::LineStart);
    scheduler.schedule(10, ARM9_TIMER0);
    scheduler.schedule(20, ARM9_TIMER1);
    scheduler.schedule(25, ARM7_TIMER0);
    scheduler.schedule(40, EventKind::HBlankStart);
    scheduler.cancel(ARM9_TIMER0);
    assert_eq!(
        pending_pairs(&scheduler),
        vec![
            (20, ARM9_TIMER1),
            (25, ARM7_TIMER0),
            (30, EventKind::LineStart),
            (40, EventKind::HBlankStart)
        ]
    );
}

#[test]
fn given_the_same_timer_index_on_both_cpus_when_the_arm9_one_is_cancelled_then_the_arm7_one_stays()
{
    let mut scheduler = Scheduler::new();
    scheduler.schedule(10, ARM9_TIMER0);
    scheduler.schedule(10, ARM7_TIMER0);
    scheduler.cancel(ARM9_TIMER0);
    assert_eq!(pending_pairs(&scheduler), vec![(10, ARM7_TIMER0)]);
}

#[test]
fn given_no_event_of_the_kind_when_cancelled_then_the_queue_is_unchanged() {
    let mut scheduler = Scheduler::new();
    scheduler.schedule(10, EventKind::LineStart);
    let before = scheduler.pending();
    scheduler.cancel(ARM9_TIMER0);
    assert_eq!(scheduler.pending(), before);
}

#[test]
fn given_an_empty_queue_when_cancelled_then_it_is_still_empty() {
    let mut scheduler = Scheduler::new();
    scheduler.cancel(ARM9_TIMER0);
    assert_eq!(pending_pairs(&scheduler), vec![]);
}

#[test]
fn given_a_cancelled_event_when_popping_due_then_it_does_not_fire() {
    let mut scheduler = Scheduler::new();
    scheduler.schedule(10, ARM9_TIMER0);
    scheduler.schedule(10, EventKind::LineStart);
    scheduler.cancel(ARM9_TIMER0);
    let fired: Vec<EventKind> = std::iter::from_fn(|| scheduler.pop_due(10))
        .map(|event| event.kind)
        .collect();
    assert_eq!(fired, vec![EventKind::LineStart]);
}

#[test]
fn given_a_cancel_and_a_reschedule_when_listing_pending_then_exactly_one_event_of_the_kind_remains()
{
    let mut scheduler = Scheduler::new();
    scheduler.schedule(10, ARM9_TIMER0);
    scheduler.cancel(ARM9_TIMER0);
    scheduler.schedule(50, ARM9_TIMER0);
    assert_eq!(pending_pairs(&scheduler), vec![(50, ARM9_TIMER0)]);
}

#[test]
fn given_a_cancel_when_checked_then_the_queue_is_still_well_formed() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[10, 20]);
    scheduler.schedule(15, ARM9_TIMER0);
    scheduler.cancel(ARM9_TIMER0);
    assert!(scheduler.is_well_formed());
}

// ------------------------------------------------------------------ the line and HBlank grid

/// The grid points `next_line_start` and `next_hblank_start` return for `now` values on and
/// around a grid point, where they differ from "the first point strictly after `now`".
fn grid_failures() -> Vec<(u64, u64, u64)> {
    let line = LINE_CYCLES;
    let hblank = HBLANK_START_CYCLES;
    let cases = [
        // (now, next line start, next HBlank start)
        (0, line, hblank),
        (hblank - 1, line, hblank),
        (hblank, line, line + hblank),
        (line - 1, line, line + hblank),
        (line, 2 * line, line + hblank),
        (line + hblank - 1, 2 * line, line + hblank),
        (line + hblank, 2 * line, 2 * line + hblank),
    ];
    cases
        .into_iter()
        .filter(|&(now, next_line, next_hblank)| {
            (
                Scheduler::next_line_start(now),
                Scheduler::next_hblank_start(now),
            ) != (next_line, next_hblank)
        })
        .collect()
}

#[test]
fn given_clocks_on_and_beside_grid_points_when_the_next_line_events_are_asked_then_each_is_strictly_after_now()
 {
    assert_eq!(grid_failures(), vec![]);
}
