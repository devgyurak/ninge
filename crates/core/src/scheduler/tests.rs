// SPDX-License-Identifier: Apache-2.0
//! The event queue and the round order. Expected values come from `docs/hw/scheduler.md`:
//! "Events" (order is (timestamp, sequence), the counter only goes up), "Interleaving" (target
//! is the smaller of the next event and now + SLICE, `now` when an event is already due; ARM9,
//! then ARM7, then events), "Debugger single-step" and "Savestate checks", plus its "Edge
//! cases".

use super::*;

/// What a recording participant saw, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Arm9(u64),
    Arm7(u64),
    /// A dispatched event: (time, seq).
    Dispatch(u64, u64),
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
        self.log.push(Step::Dispatch(event.time, event.seq));
    }
}

fn schedule_all(scheduler: &mut Scheduler, times: &[u64]) {
    times
        .iter()
        .for_each(|&time| scheduler.schedule(time, EventKind::LineStart));
}

/// Pops everything due at `now`, as `(time, seq)`.
fn drain(scheduler: &mut Scheduler, now: u64) -> Vec<(u64, u64)> {
    std::iter::from_fn(|| scheduler.pop_due(now))
        .map(|event| (event.time, event.seq))
        .collect()
}

/// The sequence numbers `count` consecutive `schedule` calls will get, given the counter now.
fn next_seqs(scheduler: &Scheduler, count: u64) -> Vec<u64> {
    (0..count)
        .map(|offset| scheduler.next_seq.wrapping_add(offset))
        .collect()
}

// ------------------------------------------------------------------- the queue

#[test]
fn given_two_events_at_same_time_when_popped_then_earlier_scheduled_comes_first() {
    let mut scheduler = Scheduler::new();
    let seqs = next_seqs(&scheduler, 2);
    schedule_all(&mut scheduler, &[10, 10]);
    let popped: Vec<u64> = drain(&mut scheduler, 10)
        .into_iter()
        .map(|(_, seq)| seq)
        .collect();
    assert_eq!(popped, seqs);
}

#[test]
fn given_five_events_at_same_time_interleaved_with_others_when_popped_then_they_come_in_scheduling_order()
 {
    let mut scheduler = Scheduler::new();
    let seqs = next_seqs(&scheduler, 8);
    // Same-time events are the 1st, 2nd, 4th, 6th and 7th scheduled.
    schedule_all(&mut scheduler, &[20, 20, 5, 20, 30, 20, 20, 1]);
    let at_20: Vec<u64> = drain(&mut scheduler, 20)
        .into_iter()
        .filter(|&(time, _)| time == 20)
        .map(|(_, seq)| seq)
        .collect();
    assert_eq!(at_20, vec![seqs[0], seqs[1], seqs[3], seqs[5], seqs[6]]);
}

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

#[test]
fn given_events_scheduled_when_popped_then_sequence_numbers_come_from_a_counter_that_goes_up() {
    let mut scheduler = Scheduler::new();
    let before = scheduler.next_seq;
    schedule_all(&mut scheduler, &[5, 5, 5]);
    assert_eq!(scheduler.next_seq, before.wrapping_add(3));
}

#[test]
fn given_sequence_counter_at_u64_max_when_scheduling_then_it_wraps_without_panic() {
    let mut scheduler = Scheduler::new();
    scheduler.next_seq = u64::MAX;
    schedule_all(&mut scheduler, &[5]);
    assert_eq!(scheduler.next_seq, 0);
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
fn given_two_events_at_same_time_when_listing_pending_then_the_earlier_scheduled_is_first() {
    let mut scheduler = Scheduler::new();
    let seqs = next_seqs(&scheduler, 2);
    schedule_all(&mut scheduler, &[10, 10]);
    let listed: Vec<u64> = scheduler.pending().iter().map(|event| event.seq).collect();
    assert_eq!(listed, seqs);
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
fn given_the_cost_constants_when_read_then_arm9_costs_1_arm7_costs_2_and_slice_is_64() {
    assert_eq!(
        (ARM9_INSTRUCTION_COST, ARM7_INSTRUCTION_COST, SLICE),
        (1, 2, 64)
    );
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
fn given_sequence_counter_equal_to_a_queued_seq_when_checked_then_it_is_refused() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[10, 20]);
    let highest = scheduler
        .pending()
        .iter()
        .map(|e| e.seq)
        .max()
        .expect("two");
    scheduler.next_seq = highest;
    assert!(!scheduler.is_well_formed());
}

#[test]
fn given_sequence_counter_below_a_queued_seq_when_checked_then_it_is_refused() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[10, 20, 30]);
    let highest = scheduler
        .pending()
        .iter()
        .map(|e| e.seq)
        .max()
        .expect("three");
    scheduler.next_seq = highest.wrapping_sub(1);
    assert!(!scheduler.is_well_formed());
}

#[test]
fn given_sequence_counter_just_above_every_queued_seq_when_checked_then_it_is_accepted() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[10, 20]);
    let highest = scheduler
        .pending()
        .iter()
        .map(|e| e.seq)
        .max()
        .expect("two");
    scheduler.next_seq = highest + 1;
    assert!(scheduler.is_well_formed());
}

#[test]
fn given_event_time_before_now_when_checked_then_it_is_refused() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[50]);
    scheduler.now = 51;
    assert!(!scheduler.is_well_formed());
}

#[test]
fn given_event_time_equal_to_now_when_checked_then_it_is_accepted() {
    let mut scheduler = Scheduler::new();
    schedule_all(&mut scheduler, &[50]);
    scheduler.now = 50;
    assert!(scheduler.is_well_formed());
}

#[test]
fn given_clock_at_max_time_with_event_at_max_time_when_checked_then_it_is_accepted() {
    let mut scheduler = Scheduler::new();
    scheduler.now = MAX_TIME;
    schedule_all(&mut scheduler, &[MAX_TIME]);
    assert!(scheduler.is_well_formed());
}

#[test]
fn given_clock_at_max_time_with_event_past_max_time_when_checked_then_it_is_refused() {
    let mut scheduler = Scheduler::new();
    scheduler.now = MAX_TIME;
    schedule_all(&mut scheduler, &[MAX_TIME + 5]);
    assert!(!scheduler.is_well_formed());
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
    let seq = recorder.scheduler.pending()[0].seq;
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm9(10), Step::Arm7(10), Step::Dispatch(10, seq)]
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
    let seq = recorder.scheduler.pending()[0].seq;
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(SLICE),
            Step::Arm7(SLICE),
            Step::Dispatch(SLICE, seq)
        ]
    );
}

#[test]
fn given_event_nearer_than_a_slice_when_running_a_round_then_target_is_the_event_time() {
    let mut recorder = Recorder::with_events(100, &[120]);
    let seq = recorder.scheduler.pending()[0].seq;
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm9(120), Step::Arm7(120), Step::Dispatch(120, seq)]
    );
}

#[test]
fn given_event_at_now_when_running_a_round_then_target_is_now_and_the_event_is_dispatched() {
    let mut recorder = Recorder::with_events(100, &[100]);
    let seq = recorder.scheduler.pending()[0].seq;
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm9(100), Step::Arm7(100), Step::Dispatch(100, seq)]
    );
}

#[test]
fn given_event_already_overdue_when_running_a_round_then_target_is_now_and_the_event_is_dispatched()
{
    let mut recorder = Recorder::with_events(100, &[50]);
    let seq = recorder.scheduler.pending()[0].seq;
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![Step::Arm9(100), Step::Arm7(100), Step::Dispatch(50, seq)]
    );
}

#[test]
fn given_two_events_at_the_target_when_running_a_round_then_they_dispatch_in_scheduling_order() {
    let mut recorder = Recorder::at(0);
    let seqs = next_seqs(&recorder.scheduler, 2);
    schedule_all(&mut recorder.scheduler, &[30, 30]);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(30),
            Step::Arm7(30),
            Step::Dispatch(30, seqs[0]),
            Step::Dispatch(30, seqs[1])
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
    let seqs = next_seqs(&recorder.scheduler, 2);
    schedule_all(&mut recorder.scheduler, &[10, 20]);
    run_round(&mut recorder);
    run_round(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm9(10),
            Step::Arm7(10),
            Step::Dispatch(10, seqs[0]),
            Step::Arm9(20),
            Step::Arm7(20),
            Step::Dispatch(20, seqs[1])
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

// ------------------------------------------------------------------- settle

#[test]
fn given_event_due_at_now_when_settling_then_arm7_runs_to_now_and_then_the_event_dispatches() {
    let mut recorder = Recorder::with_events(40, &[40]);
    let seq = recorder.scheduler.pending()[0].seq;
    settle(&mut recorder);
    assert_eq!(recorder.log, vec![Step::Arm7(40), Step::Dispatch(40, seq)]);
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
    let seq = recorder.scheduler.pending()[0].seq;
    settle(&mut recorder);
    assert_eq!(recorder.log, vec![Step::Arm7(40), Step::Dispatch(30, seq)]);
}

#[test]
fn given_no_events_when_settling_then_the_arm9_is_not_run() {
    let mut recorder = Recorder::at(40);
    settle(&mut recorder);
    assert_eq!(recorder.log, vec![Step::Arm7(40)]);
}

#[test]
fn given_two_events_due_when_settling_then_they_dispatch_in_scheduling_order() {
    let mut recorder = Recorder::at(40);
    let seqs = next_seqs(&recorder.scheduler, 2);
    schedule_all(&mut recorder.scheduler, &[40, 40]);
    settle(&mut recorder);
    assert_eq!(
        recorder.log,
        vec![
            Step::Arm7(40),
            Step::Dispatch(40, seqs[0]),
            Step::Dispatch(40, seqs[1])
        ]
    );
}
