// SPDX-License-Identifier: Apache-2.0
//! The event queue after a savestate load (#112). The queue is not saved; a load rebuilds it from
//! the device state and `now`, so a loaded machine must hold the same events, in the same firing
//! order, as the machine that was never interrupted, and must stay identical to it afterwards.
//! Expected values come from `docs/hw/scheduler.md` ("Events": ties go by the kind's priority,
//! line start before timer overflow, ARM9 timer 0-3 before ARM7 timer 0-3, never by scheduling
//! order; "Savestate checks": the queue is derived, a waiting DMA start is due at `now`),
//! `docs/hw/timers.md` ("Clock": one F/1 tick is 2 master cycles) and `docs/hw/dma.md`
//! ("Starting": VBlank at line 192, which begins at `192 * LINE_CYCLES`), never from running the
//! code. The programs are `b .` spins.

use crate::cart::test_image::{Image, words};
use crate::cpu::Bus;
use crate::machine::*;
use crate::scheduler::{EventKind, LINE_CYCLES, run_round};

const B_SELF: u32 = 0xEAFF_FFFE;
const TM0CNT_L: u32 = 0x0400_0100;
const START: u16 = 0x80;
/// Reload `FFF0h`: 16 ticks of 2 master cycles to the first overflow.
const RELOAD_16_TICKS: u16 = 0xFFF0;
/// Reload `EF5Ch`: 4,260 ticks (`10000h - EF5Ch`), so an overflow 8,520 master cycles after the
/// start, exactly two lines (`LINE_CYCLES` is 4,260).
const RELOAD_TWO_LINES: u16 = 0xEF5C;

const SRC: u32 = 0x0220_0000;
const DST: u32 = 0x0230_0000;
const DMA0SAD: u32 = 0x0400_00B0;
const DMA0DAD: u32 = 0x0400_00B4;
const DMA0CNT: u32 = 0x0400_00B8;
/// Enable, word-sized, ARM9 VBlank start (bits 27-29 = 1), 4 units.
const VBLANK_COPY_4_WORDS: u32 = (1 << 31) | (1 << 26) | (1 << 27) | 4;

fn machine() -> Machine {
    let mut image = Image::valid();
    image.arm9_code = words(&[B_SELF]);
    image.arm7_code = words(&[B_SELF]);
    let mut m = Machine::new();
    m.load_rom(image.build()).expect("synthetic image");
    m
}

fn step(m: &mut Machine, steps: u64) {
    (0..steps).for_each(|_| m.step_arm9());
}

/// Starts ARM9 timer `index` at F/1 with `reload`, at the current master time.
fn start_timer9(m: &mut Machine, index: u32, reload: u16) {
    m.bus9().write16(TM0CNT_L + 4 * index, reload);
    m.bus9().write16(TM0CNT_L + 4 * index + 2, START);
}

fn rounds(m: &mut Machine, count: u32) {
    (0..count).for_each(|_| run_round(m));
}

/// Whole rounds until the machine is at the start of `line` in frame 0.
fn run_until_line(m: &mut Machine, line: u16) {
    (0..200_000).for_each(|_| {
        if m.frame_count() != 0 || m.vcount() != line {
            run_round(m);
        }
    });
}

/// The pending events as `(time, kind)`, in firing order.
fn pending_pairs(m: &Machine) -> Vec<(u64, EventKind)> {
    m.scheduler()
        .pending()
        .iter()
        .map(|event| (event.time, event.kind))
        .collect()
}

/// What a save, a load onto a fresh machine with the same ROM, and a shared continuation show.
#[derive(Debug, PartialEq, Eq)]
struct Resumed {
    /// The loaded machine's queue right after the load.
    loaded_pending: Vec<(u64, EventKind)>,
    /// The uninterrupted machine's queue at the save point.
    original_pending: Vec<(u64, EventKind)>,
    /// Whether the savestate bytes are equal after both ran `more`.
    bytes_equal_after: bool,
}

/// Saves `original`, loads it onto a fresh machine holding the same ROM, then runs `more` on the
/// loaded machine and on a clone of `original`.
fn resume(original: &Machine, more: impl Fn(&mut Machine)) -> Resumed {
    let mut loaded = Machine::new();
    loaded
        .load_rom(original.rom().to_vec())
        .expect("the ROM loaded before");
    loaded
        .load_state(&original.save_state())
        .expect("a state saved by the same build loads");
    let loaded_pending = pending_pairs(&loaded);
    let mut uninterrupted = original.clone();
    more(&mut loaded);
    more(&mut uninterrupted);
    Resumed {
        loaded_pending,
        original_pending: pending_pairs(original),
        bytes_equal_after: loaded.save_state() == uninterrupted.save_state(),
    }
}

fn timer9(index: u8) -> EventKind {
    EventKind::TimerOverflow { arm9: true, index }
}

// ------------------------------------------------- (a) two timers tied, set in reverse order

/// Timers 1 then 0 started at the same time with the same period, both overflowed once at 32
/// and rescheduled for 64; the clock is at 33.
fn two_timers_started_in_reverse() -> Machine {
    let mut m = machine();
    start_timer9(&mut m, 1, RELOAD_16_TICKS);
    start_timer9(&mut m, 0, RELOAD_16_TICKS);
    step(&mut m, 33);
    m
}

#[test]
fn given_two_tied_timers_started_in_reverse_index_order_when_saved_and_loaded_then_the_queue_is_in_index_order_and_equals_the_uninterrupted_one()
 {
    let r = resume(&two_timers_started_in_reverse(), |m| step(m, 200));
    let expected = vec![(64, timer9(0)), (64, timer9(1))];
    assert_eq!(
        (
            r.loaded_pending.iter().filter(|(t, _)| *t == 64).count(),
            r.loaded_pending[..2].to_vec(),
            r.original_pending[..2].to_vec(),
            r.bytes_equal_after
        ),
        (2, expected.clone(), expected, true)
    );
}

#[test]
fn given_two_timers_started_in_opposite_orders_at_the_same_time_when_saved_then_the_bytes_are_identical()
 {
    // The device state is the same and the queue is rebuilt, so the scheduling history cannot
    // show in the savestate (`scheduler.md`, "Savestate checks").
    let mut index_order = machine();
    start_timer9(&mut index_order, 0, RELOAD_16_TICKS);
    start_timer9(&mut index_order, 1, RELOAD_16_TICKS);
    let mut reverse_order = machine();
    start_timer9(&mut reverse_order, 1, RELOAD_16_TICKS);
    start_timer9(&mut reverse_order, 0, RELOAD_16_TICKS);
    assert!(index_order.save_state() == reverse_order.save_state());
}

// ------------------------------------------- (b) a timer and the line start due at one time

/// Timer 0 started at time 0 overflows at 8,520, the same time as the second line start after
/// it, which is queued later (at 4,260) than the timer's event; the clock is at 8,519.
fn timer_tied_with_a_line_start() -> Machine {
    let mut m = machine();
    start_timer9(&mut m, 0, RELOAD_TWO_LINES);
    step(&mut m, 2 * LINE_CYCLES - 1);
    m
}

#[test]
fn given_a_timer_and_a_line_start_due_at_the_same_time_when_saved_and_loaded_then_the_line_start_is_first_and_the_queue_equals_the_uninterrupted_one()
 {
    let r = resume(&timer_tied_with_a_line_start(), |m| step(m, 5000));
    let at_tie = |pending: &[(u64, EventKind)]| {
        pending
            .iter()
            .filter(|(time, _)| *time == 2 * LINE_CYCLES)
            .map(|&(_, kind)| kind)
            .collect::<Vec<_>>()
    };
    let expected = vec![EventKind::LineStart, timer9(0)];
    assert_eq!(
        (
            at_tie(&r.loaded_pending),
            at_tie(&r.original_pending),
            r.bytes_equal_after
        ),
        (expected.clone(), expected, true)
    );
}

// ------------------------------------------- (c) a handler-scheduled DMA start waiting at now

/// An ARM9 VBlank channel after line 192 began: the line start's handler queued the transfer's
/// start, due at the line start, which is `now`, and it has not run yet.
fn dma_start_waiting_at_now() -> Machine {
    let mut m = machine();
    (0..4).for_each(|i| m.bus9().write32(SRC + 4 * i, i + 1));
    m.bus9().write32(DMA0SAD, SRC);
    m.bus9().write32(DMA0DAD, DST);
    m.bus9().write32(DMA0CNT, VBLANK_COPY_4_WORDS);
    run_until_line(&mut m, 192);
    m
}

#[test]
fn given_a_vblank_dma_start_queued_by_the_line_handler_when_saved_and_loaded_then_the_start_is_due_at_now_and_the_run_equals_the_uninterrupted_one()
 {
    let original = dma_start_waiting_at_now();
    let now = 192 * LINE_CYCLES;
    let r = resume(&original, |m| rounds(m, 3));
    let dma_start = EventKind::DmaStart {
        arm9: true,
        channel: 0,
    };
    let is_dma = |pending: &[(u64, EventKind)]| {
        pending
            .iter()
            .filter(|(_, kind)| *kind == dma_start)
            .map(|&(time, _)| time)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        (
            original.scheduler().now(),
            is_dma(&r.loaded_pending),
            is_dma(&r.original_pending),
            r.bytes_equal_after
        ),
        (now, vec![now], vec![now], true)
    );
}

#[test]
fn given_a_vblank_dma_start_waiting_at_now_when_loaded_and_a_round_runs_then_the_words_are_copied()
{
    // The rebuilt start must actually fire: the copy is the observable (`dma.md`, "Starting").
    let original = dma_start_waiting_at_now();
    let mut loaded = machine();
    loaded.load_state(&original.save_state()).expect("loads");
    run_round(&mut loaded);
    let copied: Vec<u32> = (0..4).map(|i| loaded.arm9_peek32(DST + 4 * i)).collect();
    assert_eq!(copied, vec![1, 2, 3, 4]);
}
