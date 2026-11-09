// SPDX-License-Identifier: Apache-2.0
//! The timers on their own. Expected values come from `docs/hw/timers.md`: "Clock" (one F/1 tick
//! is 2 master cycles; F/64, F/256, F/1024 are 128, 512, 2048), "Counting" (a timer overflows every
//! `10000h - reload` ticks; start loads the reload; a reload write changes only the reload;
//! stop freezes the counter), "Count-up", "Writing a 32-bit word", "Edge cases", "Model" (the
//! counter is computed from time; the first tick is one full period after the start) and
//! "Unverified" (tests named `..._unverified` pin a modeled choice, not a hardware fact).
//!
//! Master time is in ARM9 cycles. A timer started at `T` with reload `R` and a prescaler of
//! `c` master cycles per tick first overflows at `T + (0x10000 - R) * c`.

use super::*;

type Arm9Timers = Timers<true>;

/// Master cycles per tick for the prescaler settings 0 to 3 (`timers.md`, "Clock").
const CYCLES_PER_TICK: [u64; 4] = [2, 128, 512, 2048];

/// TMnCNT_H: start, and the IRQ-enable bit.
const START: u16 = 0x80;
const IRQ: u16 = 0x40;
const COUNT_UP: u16 = 0x04;

/// Timer *n*'s TMnCNT_L and TMnCNT_H (`timers.md`, "Registers").
const fn reload_addr(index: u32) -> u32 {
    TM0CNT_L_ADDR + 4 * index
}
const fn control_addr(index: u32) -> u32 {
    TM0CNT_L_ADDR + 4 * index + 2
}

/// A halfword write: bytes in ascending address order, as the bus splits it. Returns the mask of
/// timers whose overflow event the write made stale.
fn write16(timers: &mut Arm9Timers, addr: u32, value: u16, now: u64) -> u8 {
    let low = timers.write_byte(addr, value as u8, now);
    let high = timers.write_byte(addr + 1, (value >> 8) as u8, now);
    low.reschedule | high.reschedule
}

/// A word write: four bytes in ascending address order (`timers.md`, "Writing a 32-bit word").
/// Returns the mask of timers whose overflow event the write made stale.
fn write32(timers: &mut Arm9Timers, addr: u32, value: u32, now: u64) -> u8 {
    value
        .to_le_bytes()
        .iter()
        .enumerate()
        .fold(0, |mask, (i, &byte)| {
            mask | timers.write_byte(addr + i as u32, byte, now).reschedule
        })
}

/// Writes timer `index`'s reload, then its control, at `now`. Returns the reschedule mask.
fn start(timers: &mut Arm9Timers, index: u32, reload: u16, control: u16, now: u64) -> u8 {
    write16(timers, reload_addr(index), reload, now)
        | write16(timers, control_addr(index), control, now)
}

/// One timer 0, started at `now`.
fn timer0(reload: u16, control: u16, now: u64) -> Arm9Timers {
    let mut timers = Arm9Timers::new();
    start(&mut timers, 0, reload, control, now);
    timers
}

/// Timer 0 plus timers 1 to 3 chained to it by count-up; `irq[n]` is timer n's IRQ enable and
/// `reloads[n]` its reload. All four are started at time 0.
fn chain(irq: [bool; 4], reloads: [u16; 4]) -> Arm9Timers {
    let mut timers = Arm9Timers::new();
    (0..4u32).for_each(|n| {
        let irq_bit = IRQ * u16::from(irq[n as usize]);
        let count_up = COUNT_UP * u16::from(n > 0);
        start(
            &mut timers,
            n,
            reloads[n as usize],
            START | irq_bit | count_up,
            0,
        );
    });
    timers
}

/// IF bits 3 to 6 are the timers' overflows (`timers.md`, "IF bits").
const IF0: u32 = 1 << 3;
const IF1: u32 = 1 << 4;
const IF2: u32 = 1 << 5;
const IF3: u32 = 1 << 6;

// ------------------------------------------------------------------ overflow time per prescaler

/// For each prescaler, the overflow time of a timer started at `at` with `reload`, against
/// `at + ticks * cycles_per_tick`. Returns `(prescaler, got, want)` for every mismatch.
fn overflow_time_failures(at: u64, reload: u16, ticks: u64) -> Vec<(u16, Option<u64>, u64)> {
    (0..4u16)
        .filter_map(|prescaler| {
            let timers = timer0(reload, START | prescaler, at);
            let want = at + ticks * CYCLES_PER_TICK[usize::from(prescaler)];
            let got = timers.overflow_time(0);
            (got != Some(want)).then_some((prescaler, got, want))
        })
        .collect()
}

#[test]
fn given_reload_fff0_when_started_at_time_0_with_each_prescaler_then_overflow_is_16_ticks_later() {
    assert_eq!(overflow_time_failures(0, 0xFFF0, 16), vec![]);
}

#[test]
fn given_reload_fff0_when_started_at_time_1000_with_each_prescaler_then_overflow_is_16_ticks_after_the_start()
 {
    assert_eq!(overflow_time_failures(1000, 0xFFF0, 16), vec![]);
}

#[test]
fn given_reload_ffff_when_started_with_each_prescaler_then_overflow_is_one_tick_later() {
    assert_eq!(overflow_time_failures(500, 0xFFFF, 1), vec![]);
}

#[test]
fn given_reload_0_when_started_with_each_prescaler_then_overflow_is_65536_ticks_later() {
    assert_eq!(overflow_time_failures(0, 0x0000, 65_536), vec![]);
}

#[test]
fn given_a_started_timer_when_asked_for_an_event_then_it_has_one_and_it_is_an_overflow_of_that_timer()
 {
    let timers = timer0(0xFFF0, START, 0);
    assert_eq!(
        (timers.has_event(0), Arm9Timers::event_kind(0)),
        (
            true,
            EventKind::TimerOverflow {
                arm9: true,
                index: 0
            }
        )
    );
}

#[test]
fn given_the_arm7_timers_when_asked_for_the_event_kind_then_it_belongs_to_the_arm7() {
    assert_eq!(
        Timers::<false>::event_kind(3),
        EventKind::TimerOverflow {
            arm9: false,
            index: 3
        }
    );
}

// ------------------------------------------------------------------------- reload and start

#[test]
fn given_a_reload_when_the_timer_is_started_then_the_counter_holds_the_reload() {
    let timers = timer0(0x1234, START, 700);
    assert_eq!(timers.counter(0, 700), 0x1234);
}

#[test]
fn given_a_stopped_timer_when_the_reload_is_written_then_the_counter_does_not_change() {
    let mut timers = Arm9Timers::new();
    write16(&mut timers, reload_addr(0), 0x1234, 0);
    assert_eq!((timers.reload(0), timers.counter(0, 50)), (0x1234, 0));
}

#[test]
fn given_a_running_timer_when_the_reload_is_written_then_the_counter_and_the_overflow_time_do_not_change()
 {
    let mut timers = timer0(0xFFF0, START, 0);
    write16(&mut timers, reload_addr(0), 0xFF00, 10);
    assert_eq!(
        (
            timers.counter(0, 10),
            timers.overflow_time(0),
            timers.reload(0)
        ),
        (0xFFF5, Some(32), 0xFF00)
    );
}

#[test]
fn given_a_reload_written_while_running_when_the_timer_overflows_then_the_counter_takes_the_new_reload_and_the_next_period_uses_it()
 {
    let mut timers = timer0(0xFFF0, START, 0);
    write16(&mut timers, reload_addr(0), 0xFF00, 10);
    timers.overflow(0, 32);
    // 0x10000 - 0xFF00 = 256 ticks = 512 master cycles after the overflow at 32.
    assert_eq!(
        (timers.counter(0, 32), timers.overflow_time(0)),
        (0xFF00, Some(32 + 512))
    );
}

#[test]
fn given_a_reload_write_when_checked_then_the_overflow_event_is_not_marked_for_replacement() {
    let mut timers = timer0(0xFFF0, START, 0);
    let mask = write16(&mut timers, reload_addr(0), 0xFF00, 10);
    assert_eq!(mask, 0);
}

// -------------------------------------------------------------------- counter reads by time

/// Reads the counter of a timer 0 started at `at` with `reload` and `control` at each time of
/// `cases`; returns `(now, got, want)` for every mismatch.
fn counter_failures(
    reload: u16,
    control: u16,
    at: u64,
    cases: &[(u64, u16)],
) -> Vec<(u64, u16, u16)> {
    let timers = timer0(reload, control, at);
    cases
        .iter()
        .filter_map(|&(now, want)| {
            let got = timers.counter(0, now);
            (got != want).then_some((now, got, want))
        })
        .collect()
}

#[test]
fn given_f1_and_reload_fff0_started_at_100_when_read_at_several_times_then_it_counts_every_2_cycles_and_wraps_to_the_reload()
 {
    let cases = [
        (100, 0xFFF0),
        (101, 0xFFF0),
        (102, 0xFFF1),
        (110, 0xFFF5),
        (111, 0xFFF5),
        (130, 0xFFFF),
        (131, 0xFFFF),
        // 16 ticks after the start the counter would pass FFFFh: it takes the reload.
        (132, 0xFFF0),
        (137, 0xFFF2),
        (138, 0xFFF3),
    ];
    assert_eq!(counter_failures(0xFFF0, START, 100, &cases), vec![]);
}

#[test]
fn given_f64_and_reload_fffe_when_read_over_three_periods_then_it_alternates_fffe_ffff() {
    let cases = [
        (0, 0xFFFE),
        (127, 0xFFFE),
        (128, 0xFFFF),
        (255, 0xFFFF),
        (256, 0xFFFE),
        (384, 0xFFFF),
        (512, 0xFFFE),
    ];
    assert_eq!(counter_failures(0xFFFE, START | 1, 0, &cases), vec![]);
}

#[test]
fn given_f256_and_reload_0_when_read_around_the_full_period_then_it_reaches_ffff_and_wraps_to_0() {
    let cases = [
        (511, 0),
        (512, 1),
        (512 * 65_535, 0xFFFF),
        (512 * 65_536, 0),
        (512 * 65_537, 1),
    ];
    assert_eq!(counter_failures(0, START | 2, 0, &cases), vec![]);
}

#[test]
fn given_f1024_and_reload_ffff_when_read_at_any_time_then_it_stays_ffff_because_every_tick_overflows()
 {
    let cases = [
        (50, 0xFFFF),
        (2097, 0xFFFF),
        (2098, 0xFFFF),
        (100_000, 0xFFFF),
    ];
    assert_eq!(counter_failures(0xFFFF, START | 3, 50, &cases), vec![]);
}

#[test]
fn given_f1_and_reload_ffff_when_overflowed_then_the_next_overflow_is_one_tick_later() {
    let mut timers = timer0(0xFFFF, START, 0);
    timers.overflow(0, 2);
    assert_eq!(timers.overflow_time(0), Some(4));
}

#[test]
fn given_f1_and_reload_0_when_read_around_the_full_period_then_it_reaches_ffff_and_wraps_to_0() {
    let cases = [(2, 1), (131_070, 0xFFFF), (131_072, 0), (131_074, 1)];
    assert_eq!(counter_failures(0, START, 0, &cases), vec![]);
}

#[test]
fn given_a_running_timer_when_read_before_its_origin_then_no_tick_has_passed_model() {
    // An ARM9 read just after a lagging ARM7 write sees a clock behind the origin (`timers.md`,
    // "Model", "Event time is not before `now`").
    let timers = timer0(0xFFF0, START, 100);
    assert_eq!(timers.counter(0, 50), 0xFFF0);
}

// -------------------------------------------------------------------------------- overflow

#[test]
fn given_an_overflow_when_it_is_processed_then_the_counter_is_the_reload_and_the_next_period_starts_at_the_overflow_time()
 {
    let mut timers = timer0(0xFFF0, START, 0);
    timers.overflow(0, 32);
    assert_eq!(
        (timers.counter(0, 32), timers.overflow_time(0)),
        (0xFFF0, Some(64))
    );
}

#[test]
fn given_irq_enable_set_when_the_timer_overflows_then_its_if_bit_is_requested() {
    let mut timers = timer0(0xFFF0, START | IRQ, 0);
    assert_eq!(timers.overflow(0, 32), IF0);
}

#[test]
fn given_irq_enable_clear_when_the_timer_overflows_then_nothing_is_requested_but_it_still_reloads()
{
    let mut timers = timer0(0xFFF0, START, 0);
    let requests = timers.overflow(0, 32);
    assert_eq!(
        (requests, timers.counter(0, 40), timers.overflow_time(0)),
        (0, 0xFFF4, Some(64))
    );
}

#[test]
fn given_each_timer_with_irq_enabled_when_it_overflows_then_it_requests_if_bit_3_plus_its_index() {
    let requests: Vec<u32> = (0..4u32)
        .map(|n| {
            let mut timers = Arm9Timers::new();
            start(&mut timers, n, 0xFFF0, START | IRQ, 0);
            timers.overflow(n as usize, 32)
        })
        .collect();
    assert_eq!(requests, vec![IF0, IF1, IF2, IF3]);
}

// ---------------------------------------------------------------------------------- cascade

#[test]
fn given_four_chained_timers_at_ffff_with_irqs_when_timer_0_overflows_then_all_four_overflow_in_one_call()
 {
    let mut timers = chain([true; 4], [0xFFFF; 4]);
    assert_eq!(timers.overflow(0, 2), IF0 | IF1 | IF2 | IF3);
}

#[test]
fn given_four_chained_timers_at_ffff_when_timer_0_overflows_then_each_reloads() {
    let mut timers = chain([true; 4], [0xFFFF; 4]);
    timers.overflow(0, 2);
    let counters: Vec<u16> = (0..4).map(|n| timers.counter(n, 2)).collect();
    assert_eq!(counters, vec![0xFFFF; 4]);
}

#[test]
fn given_a_chained_timer_with_irq_clear_when_timer_0_overflows_then_it_cascades_without_requesting()
{
    let mut timers = chain([true, false, true, true], [0xFFFF; 4]);
    assert_eq!(timers.overflow(0, 2), IF0 | IF2 | IF3);
}

#[test]
fn given_timer_0_with_irq_clear_when_it_overflows_then_the_chain_still_runs_and_requests_its_own_irqs()
 {
    let mut timers = chain([false, true, true, true], [0xFFFF; 4]);
    assert_eq!(timers.overflow(0, 2), IF1 | IF2 | IF3);
}

#[test]
fn given_a_count_up_timer_below_ffff_when_timer_0_overflows_then_it_adds_one_and_the_chain_stops() {
    let mut timers = chain([true; 4], [0xFFFF, 0x0000, 0xFFFF, 0xFFFF]);
    let requests = timers.overflow(0, 2);
    let counters: Vec<u16> = (0..4).map(|n| timers.counter(n, 2)).collect();
    // Timer 1 started at its reload 0 and now holds 1; timers 2 and 3 saw nothing.
    assert_eq!((requests, counters), (IF0, vec![0xFFFF, 1, 0xFFFF, 0xFFFF]));
}

/// Timer 1 counts up from `0xFFFD`; timer 0 overflows `times` times. Returns each call's IF mask.
fn masks_over_successive_overflows(times: u64) -> Vec<u32> {
    let mut timers = chain([true; 4], [0xFFF0, 0xFFFD, 0x0000, 0x0000]);
    (1..=times).map(|k| timers.overflow(0, 32 * k)).collect()
}

#[test]
fn given_a_count_up_timer_at_fffd_when_timer_0_overflows_three_times_then_only_the_third_overflows_it()
 {
    assert_eq!(
        masks_over_successive_overflows(3),
        vec![IF0, IF0, IF0 | IF1]
    );
}

#[test]
fn given_a_count_up_timer_when_the_clock_advances_then_its_counter_does_not_move_by_itself() {
    let timers = chain([true; 4], [0xFFF0, 0x1234, 0x0000, 0x0000]);
    assert_eq!(timers.counter(1, 10_000_000), 0x1234);
}

#[test]
fn given_a_count_up_timer_when_started_then_the_counter_loads_the_reload_and_no_event_exists() {
    let timers = chain([true; 4], [0xFFF0, 0x1234, 0x0000, 0x0000]);
    assert_eq!(
        (
            timers.counter(1, 0),
            timers.has_event(1),
            timers.overflow_time(1)
        ),
        (0x1234, false, None)
    );
}

#[test]
fn given_a_count_up_timer_that_is_stopped_when_the_previous_timer_overflows_then_it_does_not_tick()
{
    let mut timers = Arm9Timers::new();
    start(&mut timers, 0, 0xFFF0, START | IRQ, 0);
    // Timer 1: count-up and IRQ enable but never started.
    start(&mut timers, 1, 0xFFFF, COUNT_UP | IRQ, 0);
    let requests = timers.overflow(0, 32);
    assert_eq!((requests, timers.counter(1, 32)), (IF0, 0));
}

#[test]
fn given_a_stopped_middle_timer_when_timer_0_overflows_then_the_chain_does_not_pass_through_it() {
    let mut timers = Arm9Timers::new();
    start(&mut timers, 0, 0xFFF0, START | IRQ, 0);
    start(&mut timers, 1, 0xFFFF, COUNT_UP | IRQ, 0);
    start(&mut timers, 2, 0xFFFF, START | COUNT_UP | IRQ, 0);
    let requests = timers.overflow(0, 32);
    // Timer 2 would overflow on its first tick; it never gets one.
    assert_eq!((requests, timers.counter(2, 32)), (IF0, 0xFFFF));
}

#[test]
fn given_a_clock_driven_next_timer_when_timer_0_overflows_then_it_is_not_ticked() {
    let mut timers = Arm9Timers::new();
    start(&mut timers, 0, 0xFFF0, START | IRQ, 0);
    start(&mut timers, 1, 0xFFF0, START | IRQ, 0);
    let requests = timers.overflow(0, 32);
    assert_eq!(
        (requests, timers.counter(1, 0), timers.overflow_time(1)),
        (IF0, 0xFFF0, Some(32))
    );
}

#[test]
fn given_timer_3_when_it_overflows_then_only_its_own_irq_is_requested() {
    let mut timers = chain([true; 4], [0xFFF0, 0xFFF0, 0xFFF0, 0xFFF0]);
    assert_eq!(timers.overflow(3, 100), IF3);
}

#[test]
fn given_a_timer_index_past_the_last_when_overflow_is_processed_then_nothing_is_requested() {
    let mut timers = chain([true; 4], [0xFFFF; 4]);
    assert_eq!(timers.overflow(4, 100), 0);
}

#[test]
fn given_count_up_on_timer_0_when_started_then_it_is_ignored_and_it_still_has_a_clock_event_unverified()
 {
    // TODO(hw-unverified): `timers.md`, "Count-up" and "Unverified": timer 0's bit 2 has no
    // effect and reads back as written.
    let timers = timer0(0xFFF0, START | COUNT_UP, 0);
    assert_eq!(
        (
            timers.has_event(0),
            timers.overflow_time(0),
            timers.control(0)
        ),
        (true, Some(32), START | COUNT_UP)
    );
}

#[test]
fn given_count_up_on_timer_0_when_read_later_then_the_counter_still_ticks_from_the_clock_unverified()
 {
    let timers = timer0(0xFFF0, START | COUNT_UP, 0);
    assert_eq!(timers.counter(0, 10), 0xFFF5);
}

// ---------------------------------------------------------------------------- start and stop

#[test]
fn given_a_running_timer_when_stopped_then_the_counter_freezes_and_has_no_event() {
    let mut timers = timer0(0xFFF0, START, 0);
    write16(&mut timers, control_addr(0), 0, 10);
    assert_eq!(
        (
            timers.counter(0, 10),
            timers.counter(0, 100_000),
            timers.overflow_time(0)
        ),
        (0xFFF5, 0xFFF5, None)
    );
}

#[test]
fn given_a_stopped_timer_when_started_again_then_the_reload_is_loaded_and_the_period_restarts() {
    let mut timers = timer0(0xFFF0, START, 0);
    write16(&mut timers, control_addr(0), 0, 10);
    write16(&mut timers, control_addr(0), START, 2000);
    assert_eq!(
        (timers.counter(0, 2000), timers.overflow_time(0)),
        (0xFFF0, Some(2032))
    );
}

#[test]
fn given_a_stopped_timer_when_the_reload_changes_and_it_restarts_then_the_new_reload_is_loaded() {
    let mut timers = timer0(0xFFF0, START, 0);
    write16(&mut timers, control_addr(0), 0, 10);
    write16(&mut timers, reload_addr(0), 0x8000, 20);
    write16(&mut timers, control_addr(0), START, 30);
    assert_eq!(timers.counter(0, 30), 0x8000);
}

#[test]
fn given_a_running_timer_when_start_is_written_again_then_the_counter_is_not_reloaded() {
    let mut timers = timer0(0xFFF0, START, 0);
    let mask = write16(&mut timers, control_addr(0), START, 10);
    assert_eq!(
        (timers.counter(0, 10), timers.overflow_time(0), mask),
        (0xFFF5, Some(32), 0)
    );
}

#[test]
fn given_a_timer_started_when_the_write_returns_then_the_mask_names_that_timer() {
    let mut timers = Arm9Timers::new();
    let mask = start(&mut timers, 2, 0xFFF0, START, 0);
    assert_eq!(mask, 0b100);
}

#[test]
fn given_a_running_timer_when_stopped_then_it_is_marked_for_replacement() {
    let mut timers = timer0(0xFFF0, START, 0);
    let mask = write16(&mut timers, control_addr(0), 0, 10);
    assert_eq!(mask, 0b1);
}

#[test]
fn given_a_running_timer_when_the_prescaler_changes_then_it_is_marked_for_replacement() {
    let mut timers = timer0(0xFFF0, START, 0);
    let mask = write16(&mut timers, control_addr(0), START | 1, 10);
    assert_eq!(mask, 0b1);
}

#[test]
fn given_a_running_clock_timer_when_count_up_is_set_then_its_event_goes_away_and_it_is_marked() {
    let mut timers = Arm9Timers::new();
    start(&mut timers, 1, 0xFFF0, START, 0);
    let mask = write16(&mut timers, control_addr(1), START | COUNT_UP, 10);
    assert_eq!(
        (timers.has_event(1), timers.overflow_time(1), mask),
        (false, None, 0b10)
    );
}

#[test]
fn given_a_running_timer_when_the_prescaler_changes_then_the_counter_is_kept_and_the_new_rate_applies_from_the_write_unverified()
 {
    // TODO(hw-unverified): `timers.md`, "Unverified": the new prescaler applies from the write,
    // the counter is kept, and the first tick comes one full period after it.
    let mut timers = timer0(0xFFF0, START, 0);
    write16(&mut timers, control_addr(0), START | 1, 10);
    // Counter 0xFFF5 at time 10; 11 ticks to go at 128 cycles each.
    assert_eq!(
        (timers.counter(0, 10), timers.overflow_time(0)),
        (0xFFF5, Some(10 + 11 * 128))
    );
}

#[test]
fn given_irq_enable_changed_on_a_running_timer_when_written_then_the_counter_origin_and_reschedule_mask_do_not_move()
 {
    let mut timers = timer0(0xFFF0, START, 0);
    let mask = write16(&mut timers, control_addr(0), START | IRQ, 10);
    assert_eq!(
        (
            timers.counter(0, 10),
            timers.overflow_time(0),
            timers.control(0),
            mask
        ),
        (0xFFF5, Some(32), START | IRQ, 0)
    );
}

#[test]
fn given_irq_enable_set_after_start_when_the_timer_overflows_then_the_irq_is_requested() {
    let mut timers = timer0(0xFFF0, START, 0);
    write16(&mut timers, control_addr(0), START | IRQ, 10);
    assert_eq!(timers.overflow(0, 32), IF0);
}

// ------------------------------------------------------------------ 32-bit write ordering

#[test]
fn given_a_word_with_reload_and_start_when_written_at_tmncnt_l_then_the_timer_starts_with_that_reload()
 {
    let mut timers = Arm9Timers::new();
    write32(
        &mut timers,
        reload_addr(1),
        (u32::from(START) << 16) | 0xFFF0,
        40,
    );
    assert_eq!(
        (
            timers.reload(1),
            timers.counter(1, 40),
            timers.overflow_time(1)
        ),
        (0xFFF0, 0xFFF0, Some(72))
    );
}

#[test]
fn given_the_control_byte_written_before_the_reload_when_the_timer_starts_then_the_old_reload_is_loaded()
 {
    // The reverse order is not what a 32-bit write does; it shows the order is what matters.
    let mut timers = Arm9Timers::new();
    timers.write_byte(control_addr(0), START as u8, 0);
    write16(&mut timers, reload_addr(0), 0xFFF0, 0);
    assert_eq!((timers.reload(0), timers.counter(0, 0)), (0xFFF0, 0));
}

#[test]
fn given_a_word_written_at_tmncnt_l_when_read_back_then_the_two_halves_are_reload_side_counter_and_control()
 {
    let mut timers = Arm9Timers::new();
    write32(
        &mut timers,
        reload_addr(0),
        (u32::from(START | IRQ | 2) << 16) | 0xFFF0,
        0,
    );
    // A stopped-then-read timer shows the counter, not the reload, in TMnCNT_L. At time 0 the
    // counter has just loaded the reload, so they agree here; the control reads back.
    let bytes: Vec<Option<u8>> = (0..4)
        .map(|i| timers.read_byte(reload_addr(0) + i, 0))
        .collect();
    assert_eq!(bytes, vec![Some(0xF0), Some(0xFF), Some(0xC2), Some(0x00)]);
}

// ------------------------------------------------------------- byte and halfword access

#[test]
fn given_two_byte_writes_to_tmncnt_l_when_the_reload_is_read_then_each_byte_landed_in_its_half_hw_unverified()
 {
    // TODO(hw-unverified): `timers.md`, "Unverified": a byte of TMnCNT_L writes that byte of the
    // reload.
    let mut timers = Arm9Timers::new();
    timers.write_byte(reload_addr(0), 0x34, 0);
    timers.write_byte(reload_addr(0) + 1, 0x12, 0);
    assert_eq!(timers.reload(0), 0x1234);
}

#[test]
fn given_a_reload_when_only_the_low_byte_is_written_then_the_high_byte_is_kept_hw_unverified() {
    let mut timers = Arm9Timers::new();
    write16(&mut timers, reload_addr(0), 0xABCD, 0);
    timers.write_byte(reload_addr(0), 0x11, 0);
    assert_eq!(timers.reload(0), 0xAB11);
}

#[test]
fn given_a_reload_when_only_the_high_byte_is_written_then_the_low_byte_is_kept_hw_unverified() {
    let mut timers = Arm9Timers::new();
    write16(&mut timers, reload_addr(0), 0xABCD, 0);
    timers.write_byte(reload_addr(0) + 1, 0x11, 0);
    assert_eq!(timers.reload(0), 0x11CD);
}

#[test]
fn given_a_frozen_counter_when_each_byte_is_read_then_it_is_that_byte_of_the_counter_hw_unverified()
{
    // TODO(hw-unverified): `timers.md`, "Unverified": a byte of TMnCNT_L reads that byte of the
    // counter.
    let mut timers = timer0(0x1234, START, 0);
    write16(&mut timers, control_addr(0), 0, 0);
    assert_eq!(
        (
            timers.read_byte(reload_addr(0), 99),
            timers.read_byte(reload_addr(0) + 1, 99)
        ),
        (Some(0x34), Some(0x12))
    );
}

#[test]
fn given_a_byte_write_of_start_to_the_low_control_byte_when_checked_then_the_timer_runs_hw_unverified()
 {
    // TODO(hw-unverified): `timers.md`, "Unverified": a byte write to TMnCNT_H starts the timer
    // as its value says.
    let mut timers = Arm9Timers::new();
    write16(&mut timers, reload_addr(0), 0xFFF0, 0);
    timers.write_byte(control_addr(0), START as u8, 5);
    assert_eq!(
        (timers.has_event(0), timers.overflow_time(0)),
        (true, Some(37))
    );
}

#[test]
fn given_a_byte_write_to_the_high_control_byte_when_checked_then_nothing_changes_hw_unverified() {
    // TODO(hw-unverified): `timers.md`, "Unverified": the high byte of TMnCNT_H holds no bits.
    let mut timers = timer0(0xFFF0, START, 0);
    let outcome = timers.write_byte(control_addr(0) + 1, 0xFF, 10);
    assert_eq!(
        (
            timers.control(0),
            timers.overflow_time(0),
            outcome.reschedule
        ),
        (START, Some(32), 0)
    );
}

#[test]
fn given_a_halfword_write_of_ffff_to_the_control_when_read_back_then_only_the_defined_bits_remain()
{
    let mut timers = Arm9Timers::new();
    write16(&mut timers, control_addr(0), 0xFFFF, 0);
    // Bits 0-2, 6 and 7 exist; bits 3-5 and 8-15 read 0 (`timers.md`, "Unverified").
    let bytes = [
        timers.read_byte(control_addr(0), 0),
        timers.read_byte(control_addr(0) + 1, 0),
    ];
    assert_eq!(bytes, [Some(0xC7), Some(0x00)]);
}

#[test]
fn given_unused_control_bits_written_when_a_timer_is_started_then_it_still_runs_on_the_defined_bits()
 {
    let mut timers = Arm9Timers::new();
    write16(&mut timers, reload_addr(0), 0xFFF0, 0);
    write16(&mut timers, control_addr(0), 0xFF38 | START, 0);
    assert_eq!(
        (timers.control(0), timers.overflow_time(0)),
        (START, Some(32))
    );
}

// ----------------------------------------------------------------------- the address block

/// The addresses in the block whose power-on byte is not 0.
fn nonzero_power_on_bytes() -> Vec<u32> {
    let timers = Arm9Timers::new();
    (TM0CNT_L_ADDR..TM0CNT_L_ADDR + 16)
        .filter(|&addr| timers.read_byte(addr, 0) != Some(0))
        .collect()
}

#[test]
fn given_a_power_on_machine_when_each_timer_byte_is_read_then_it_is_0_hw_unverified() {
    // TODO(hw-unverified): `timers.md`, "Unverified": reset values are 0.
    assert_eq!(nonzero_power_on_bytes(), Vec::<u32>::new());
}

#[test]
fn given_addresses_around_the_block_when_read_then_only_the_16_bytes_are_timer_registers() {
    let timers = Arm9Timers::new();
    let around = [
        timers.read_byte(TM0CNT_L_ADDR - 1, 0),
        timers.read_byte(TM0CNT_L_ADDR, 0),
        timers.read_byte(TM0CNT_L_ADDR + 15, 0),
        timers.read_byte(TM0CNT_L_ADDR + 16, 0),
    ];
    assert_eq!(around, [None, Some(0), Some(0), None]);
}

#[test]
fn given_writes_outside_the_block_when_applied_then_no_timer_changes() {
    let mut timers = Arm9Timers::new();
    timers.write_byte(TM0CNT_L_ADDR - 1, 0xFF, 0);
    timers.write_byte(TM0CNT_L_ADDR + 16, 0xFF, 0);
    assert_eq!((timers,), (Arm9Timers::new(),));
}

/// Writes 0x1111, 0x2222, ... to timer 0, 1, ... and reads the four reloads back.
fn reloads_after_distinct_writes() -> Vec<u16> {
    let mut timers = Arm9Timers::new();
    (0..4u32).for_each(|n| {
        write16(&mut timers, reload_addr(n), 0x1111 * (n as u16 + 1), 0);
    });
    (0..4).map(|n| timers.reload(n)).collect()
}

#[test]
fn given_each_timer_when_written_then_it_sits_4_bytes_above_the_previous_one() {
    assert_eq!(
        reloads_after_distinct_writes(),
        vec![0x1111, 0x2222, 0x3333, 0x4444]
    );
}

#[test]
fn given_an_index_past_the_last_timer_when_asked_then_everything_reads_zero_or_none() {
    let timers = chain([true; 4], [0xFFF0; 4]);
    assert_eq!(
        (
            timers.counter(4, 10),
            timers.reload(4),
            timers.control(4),
            timers.overflow_time(4),
            timers.has_event(4)
        ),
        (0, 0, 0, None, false)
    );
}

#[test]
fn given_the_arm7_timers_when_driven_like_the_arm9_then_they_overflow_at_the_same_master_time() {
    let mut arm7 = Timers::<false>::new();
    arm7.write_byte(reload_addr(0), 0xF0, 0);
    arm7.write_byte(reload_addr(0) + 1, 0xFF, 0);
    arm7.write_byte(control_addr(0), START as u8, 0);
    assert_eq!(arm7.overflow_time(0), Some(32));
}

// -------------------------------------------------------------------------- well-formedness

#[test]
fn given_new_timers_when_checked_then_they_are_well_formed() {
    assert!(Arm9Timers::new().is_well_formed());
}

#[test]
fn given_timers_written_when_checked_then_they_are_well_formed() {
    assert!(chain([true; 4], [0xFFF0; 4]).is_well_formed());
}

#[test]
fn given_a_control_with_an_unused_bit_when_checked_then_they_are_not_well_formed() {
    let mut timers = Arm9Timers::new();
    timers.timers[2].control = 0x0008;
    assert!(!timers.is_well_formed());
}

#[test]
fn given_a_control_with_a_high_bit_when_checked_then_they_are_not_well_formed() {
    let mut timers = Arm9Timers::new();
    timers.timers[0].control = 0x8000;
    assert!(!timers.is_well_formed());
}

// ------------------------------------------------------------------------------- properties

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn given_any_reload_and_prescaler_when_started_then_overflow_is_period_ticks_later(
            reload in any::<u16>(),
            prescaler in 0u16..4,
            at in 0u64..1_000_000,
        ) {
            let timers = timer0(reload, START | prescaler, at);
            let period = 0x1_0000 - u64::from(reload);
            prop_assert_eq!(
                timers.overflow_time(0),
                Some(at + period * CYCLES_PER_TICK[usize::from(prescaler)])
            );
        }

        #[test]
        fn given_any_running_timer_when_read_at_a_whole_period_later_then_it_is_back_at_the_reload(
            reload in any::<u16>(),
            periods in 1u64..4,
        ) {
            let timers = timer0(reload, START, 0);
            let period = 0x1_0000 - u64::from(reload);
            prop_assert_eq!(timers.counter(0, periods * period * 2), reload);
        }

        #[test]
        fn given_any_running_timer_when_read_at_any_time_then_the_counter_is_at_least_the_reload_after_the_first_overflow(
            reload in any::<u16>(),
            now in 0u64..10_000_000,
        ) {
            let timers = timer0(reload, START, 0);
            prop_assert!(timers.counter(0, now) >= reload);
        }

        #[test]
        fn given_any_control_halfword_when_written_then_it_reads_back_masked_to_the_defined_bits(
            value in any::<u16>(),
        ) {
            let mut timers = Arm9Timers::new();
            write16(&mut timers, control_addr(0), value, 0);
            prop_assert_eq!(timers.control(0), value & 0xC7);
        }
    }
}
