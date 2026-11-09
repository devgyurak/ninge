// SPDX-License-Identifier: Apache-2.0
//! The timers driving both CPUs through `Machine`. Expected values come from
//! `docs/hw/timers.md` ("Clock": one F/1 tick is 2 master cycles; "Counting": a timer overflows
//! every `10000h - reload` ticks and raises IF bit 3 + n on its own CPU only when bit 6 is set;
//! "Model": one event per running clock-driven timer, a write to TMnCNT_H replaces it, an event is
//! never before `now`, and the next overflow stays on the timer's own tick grid) and
//! `docs/hw/scheduler.md` ("Cost per instruction": ARM9 1 master cycle, ARM7 2; "Interleaving";
//! "Savestate checks"), never from running the code.
//!
//! A timer started at master time `T` with reload `0xFFF0` at F/1 overflows at `T + 16 * 2`. The
//! programs are `b .`-style spins, so nothing else touches the timers.

use super::*;
use crate::cart::test_image::{Image, words};
use crate::savestate::SavestateError;
use crate::scheduler::{EventKind, LINE_CYCLES};
use crate::timers::TM0CNT_L_ADDR;

const IME: u32 = 0x0400_0208;
const IE: u32 = 0x0400_0210;
const START: u16 = 0x80;
const IRQ: u16 = 0x40;
const COUNT_UP: u16 = 0x04;
/// IF bit 3, timer 0's overflow (`interrupts.md`, "IE and IF bits").
const IF_TIMER0: u32 = 1 << 3;
const IF_TIMER1: u32 = 1 << 4;
/// 16 ticks at F/1: reload `0xFFF0`.
const RELOAD_16_TICKS: u16 = 0xFFF0;
const SYSTEM_I_CLEAR: u32 = 0x1F;
/// IRQ mode, I set, ARM state, flags clear (`arm-exceptions-modes.md`).
const IRQ_ENTRY_CPSR: u32 = 0x92;

/// `mov r0, #0x55`, `mov r1, #0x66`, `b .`.
const SPIN: [u32; 3] = [0xE3A0_0055, 0xE3A0_1066, 0xEAFF_FFFE];
const B_SELF: u32 = 0xEAFF_FFFE;

/// A program that stores the word `0x00C0_FFF0` at `0x0400_0100`: TM0CNT_L = reload `0xFFF0`,
/// TM0CNT_H = start + IRQ enable at F/1. The store is the sixth instruction (index 5).
const TIMER_WRITER: [u32; 7] = [
    0xE3A0_0404, // mov r0, #0x04 ror 8          r0 = 0x0400_0000
    0xE280_0C01, // add r0, r0, #0x01 ror 24     r0 = 0x0400_0100
    0xE3A0_18C0, // mov r1, #0xC0 ror 16         r1 = 0x00C0_0000
    0xE381_1CFF, // orr r1, r1, #0xFF ror 24     r1 |= 0xFF00
    0xE381_10F0, // orr r1, r1, #0xF0
    0xE580_1000, // str r1, [r0]
    B_SELF,
];

const fn reload_addr(index: u32) -> u32 {
    TM0CNT_L_ADDR + 4 * index
}
const fn control_addr(index: u32) -> u32 {
    TM0CNT_L_ADDR + 4 * index + 2
}

fn kind9(index: u8) -> EventKind {
    EventKind::TimerOverflow { arm9: true, index }
}
fn kind7(index: u8) -> EventKind {
    EventKind::TimerOverflow { arm9: false, index }
}

fn rom_with(arm9: &[u32], arm7: &[u32]) -> Vec<u8> {
    let mut image = Image::valid();
    image.arm9_code = words(arm9);
    image.arm7_code = words(arm7);
    image.build()
}

fn booted(rom: &[u8]) -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(rom.to_vec()).expect("synthetic image");
    machine
}

fn spin_rom() -> Vec<u8> {
    rom_with(&SPIN, &SPIN)
}

fn machine() -> Machine {
    booted(&spin_rom())
}

fn step(m: &mut Machine, steps: u64) {
    (0..steps).for_each(|_| m.step_arm9());
}

fn run_frames(m: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| m.run_frame());
}

/// The ARM9's data view at the machine's current master time.
fn tcm_now(m: &Machine) -> TcmConfig {
    TcmConfig::from_cp15(m.arm9.cp15()).at(m.scheduler.now())
}

/// Writes timer `index` of the ARM9 (reload, then control) at the current time and lets the
/// machine look at what changed, as it does after every instruction.
fn set9(m: &mut Machine, index: u32, reload: u16, control: u16) {
    let tcm = tcm_now(m);
    m.memory.write16(&tcm, reload_addr(index), reload);
    m.memory.write16(&tcm, control_addr(index), control);
    m.sync_timer_events();
}

/// The same for the ARM7, at the ARM7 clock `at`.
fn set7_at(m: &mut Machine, at: u64, index: u32, reload: u16, control: u16) {
    m.memory.write16_arm7(at, reload_addr(index), reload);
    m.memory.write16_arm7(at, control_addr(index), control);
    m.sync_timer_events();
}

fn set7(m: &mut Machine, index: u32, reload: u16, control: u16) {
    let at = m.arm7_clock;
    set7_at(m, at, index, reload, control);
}

/// The pending timer-overflow events as `(time, kind)`, in firing order.
fn timer_events(m: &Machine) -> Vec<(u64, EventKind)> {
    m.scheduler
        .pending()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::TimerOverflow { .. }))
        .map(|event| (event.time, event.kind))
        .collect()
}

// ------------------------------------------------------------------------- IF on overflow

/// IF of the ARM9 after `steps` ARM9 steps with timer 0 set to overflow after 16 ticks.
fn if9_after(steps: u64, control: u16) -> u32 {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, control);
    step(&mut m, steps);
    m.irq9().if_()
}

fn if7_after(steps: u64, control: u16) -> u32 {
    let mut m = machine();
    set7(&mut m, 0, RELOAD_16_TICKS, control);
    step(&mut m, steps);
    m.irq7().if_()
}

#[test]
fn given_an_arm9_timer_with_irq_enable_when_one_cycle_before_its_overflow_then_if_is_clear() {
    assert_eq!(if9_after(31, START | IRQ), 0);
}

#[test]
fn given_an_arm9_timer_with_irq_enable_when_the_clock_reaches_its_overflow_then_if_bit_3_is_set() {
    assert_eq!(if9_after(32, START | IRQ), IF_TIMER0);
}

#[test]
fn given_an_arm9_timer_without_irq_enable_when_it_overflows_then_if_stays_clear() {
    assert_eq!(if9_after(40, START), 0);
}

#[test]
fn given_an_arm7_timer_with_irq_enable_when_one_cycle_before_its_overflow_then_if_is_clear() {
    assert_eq!(if7_after(31, START | IRQ), 0);
}

#[test]
fn given_an_arm7_timer_with_irq_enable_when_the_clock_reaches_its_overflow_then_if_bit_3_is_set() {
    assert_eq!(if7_after(32, START | IRQ), IF_TIMER0);
}

#[test]
fn given_an_arm7_timer_without_irq_enable_when_it_overflows_then_if_stays_clear() {
    assert_eq!(if7_after(40, START), 0);
}

#[test]
fn given_timer_1_with_irq_enable_when_it_overflows_then_if_bit_4_is_set_and_bit_3_is_not() {
    let mut m = machine();
    set9(&mut m, 1, RELOAD_16_TICKS, START | IRQ);
    step(&mut m, 32);
    assert_eq!(m.irq9().if_(), IF_TIMER1);
}

#[test]
fn given_a_timer_that_overflowed_when_the_event_is_handled_then_the_next_overflow_is_one_period_later()
 {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    step(&mut m, 33);
    assert_eq!(timer_events(&m), vec![(64, kind9(0))]);
}

#[test]
fn given_a_timer_without_irq_enable_when_it_overflows_then_it_keeps_overflowing_every_period() {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    step(&mut m, 100);
    // Overflows at 32, 64 and 96: the counter is 4 cycles = 2 ticks into the fourth period.
    assert_eq!(
        (timer_events(&m), m.timers9().counter(0, 100)),
        (vec![(128, kind9(0))], 0xFFF2)
    );
}

#[test]
fn given_a_chained_pair_when_timer_0_overflows_through_the_machine_then_timer_1_ticks_and_has_no_event()
 {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    set9(&mut m, 1, 0x0005, START | COUNT_UP | IRQ);
    step(&mut m, 32);
    assert_eq!(
        (m.timers9().counter(1, 32), m.irq9().if_(), timer_events(&m)),
        (6, IF_TIMER0, vec![(64, kind9(0))])
    );
}

// ----------------------------------------------------------------------------- independence

#[test]
fn given_an_arm9_timer_with_irq_when_it_overflows_then_the_arm7_if_and_timers_are_untouched() {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    step(&mut m, 40);
    assert_eq!(
        (
            m.irq7().if_(),
            m.timers7().control(0),
            m.timers7().reload(0)
        ),
        (0, 0, 0)
    );
}

#[test]
fn given_an_arm7_timer_with_irq_when_it_overflows_then_the_arm9_if_and_timers_are_untouched() {
    let mut m = machine();
    set7(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    step(&mut m, 40);
    assert_eq!(
        (
            m.irq9().if_(),
            m.timers9().control(0),
            m.timers9().reload(0)
        ),
        (0, 0, 0)
    );
}

#[test]
fn given_the_arm9_writes_a_reload_when_the_arm7_reads_the_same_address_then_it_reads_its_own_timer()
{
    let mut m = machine();
    set9(&mut m, 0, 0x1234, 0);
    assert_eq!(
        (
            m.arm9_peek16(reload_addr(0)),
            m.memory.read16_arm7(0, reload_addr(0))
        ),
        (0, 0)
    );
}

#[test]
fn given_the_arm9_writes_a_reload_when_it_is_read_back_without_start_then_the_counter_not_the_reload_is_read()
 {
    // TMnCNT_L reads the counter (`timers.md`, "Reading TMnCNT_L"); a reload write does not move
    // it (`timers.md`, "Reload").
    let mut m = machine();
    set9(&mut m, 0, 0x1234, 0);
    assert_eq!(
        (m.arm9_peek16(reload_addr(0)), m.timers9().reload(0)),
        (0, 0x1234)
    );
}

#[test]
fn given_both_cpus_start_the_same_timer_when_run_then_each_overflows_on_its_own_and_both_if_bits_set()
 {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    set7(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    step(&mut m, 32);
    assert_eq!(
        (m.irq9().if_(), m.irq7().if_(), timer_events(&m)),
        (IF_TIMER0, IF_TIMER0, vec![(64, kind9(0)), (64, kind7(0))])
    );
}

#[test]
fn given_the_arm7_timer_stopped_when_the_arm9_timer_runs_then_only_the_arm9_event_exists() {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    assert_eq!(timer_events(&m), vec![(32, kind9(0))]);
}

// -------------------------------------------------------------------- ARM7 clock for reads

#[test]
fn given_equal_timers_on_both_cpus_when_read_after_21_arm9_steps_then_the_arm7_reads_at_its_own_clock_22()
 {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    set7(&mut m, 0, RELOAD_16_TICKS, START);
    step(&mut m, 21);
    // The ARM9 is at master time 21 (10 whole ticks); the ARM7 ran 11 two-cycle instructions
    // and is at 22 (11 ticks). Each CPU reads against its own clock (`timers.md`, "Model").
    let arm7_now = m.arm7_clock;
    assert_eq!(
        (
            m.scheduler().now(),
            arm7_now,
            m.arm9_peek16(reload_addr(0)),
            m.memory.read16_arm7(arm7_now, reload_addr(0))
        ),
        (21, 22, 0xFFFA, 0xFFFB)
    );
}

#[test]
fn given_an_arm7_timer_when_read_at_two_arm7_clocks_then_the_counter_follows_the_clock_given() {
    let mut m = machine();
    set7_at(&mut m, 0, 0, RELOAD_16_TICKS, START);
    assert_eq!(
        (
            m.memory.read16_arm7(0, reload_addr(0)),
            m.memory.read16_arm7(10, reload_addr(0)),
            m.memory.read16_arm7(11, reload_addr(0))
        ),
        (0xFFF0, 0xFFF5, 0xFFF5)
    );
}

// ------------------------------------------------------------- events: cancel and replace

#[test]
fn given_a_timer_rewritten_three_times_between_looks_when_the_machine_looks_then_exactly_one_event_remains()
 {
    let mut m = machine();
    let tcm = tcm_now(&m);
    m.memory.write16(&tcm, reload_addr(0), RELOAD_16_TICKS);
    m.memory.write16(&tcm, control_addr(0), START);
    m.memory.write16(&tcm, control_addr(0), START | 1);
    m.memory.write16(&tcm, control_addr(0), START | 2);
    m.sync_timer_events();
    // F/256 is the last setting: 16 ticks at 512 cycles.
    assert_eq!(timer_events(&m), vec![(16 * 512, kind9(0))]);
}

#[test]
fn given_a_timer_rewritten_after_each_look_when_the_prescaler_changes_then_the_old_event_is_replaced()
 {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    set9(&mut m, 0, RELOAD_16_TICKS, START | 1);
    assert_eq!(timer_events(&m), vec![(16 * 128, kind9(0))]);
}

#[test]
fn given_a_running_timer_when_stopped_then_its_event_is_removed() {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    set9(&mut m, 0, RELOAD_16_TICKS, 0);
    assert_eq!(timer_events(&m), vec![]);
}

#[test]
fn given_a_stopped_timer_when_it_is_started_then_one_event_appears_at_its_overflow() {
    let mut m = machine();
    set9(&mut m, 2, RELOAD_16_TICKS, START);
    assert_eq!(timer_events(&m), vec![(32, kind9(2))]);
}

#[test]
fn given_a_count_up_timer_when_started_then_it_has_no_event_while_the_clock_timer_has_one() {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    set9(&mut m, 1, 0x0000, START | COUNT_UP);
    assert_eq!(timer_events(&m), vec![(32, kind9(0))]);
}

#[test]
fn given_a_running_timer_switched_to_count_up_when_the_machine_looks_then_its_event_is_removed() {
    let mut m = machine();
    set9(&mut m, 1, RELOAD_16_TICKS, START);
    set9(&mut m, 1, RELOAD_16_TICKS, START | COUNT_UP);
    assert_eq!(timer_events(&m), vec![]);
}

#[test]
fn given_an_arm7_timer_when_started_then_its_event_belongs_to_the_arm7_and_the_arm9_has_none() {
    let mut m = machine();
    set7(&mut m, 3, RELOAD_16_TICKS, START);
    assert_eq!(timer_events(&m), vec![(32, kind7(3))]);
}

#[test]
fn given_an_irq_enable_only_change_when_the_machine_looks_then_the_event_is_the_same_one() {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    let before = m.scheduler.pending();
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    assert_eq!(m.scheduler.pending(), before);
}

#[test]
fn given_a_reload_write_only_when_the_machine_looks_then_the_event_is_the_same_one() {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    let before = m.scheduler.pending();
    let tcm = tcm_now(&m);
    m.memory.write16(&tcm, reload_addr(0), 0x8000);
    m.sync_timer_events();
    assert_eq!(m.scheduler.pending(), before);
}

#[test]
fn given_a_timer_started_by_a_rom_store_when_the_store_has_run_then_the_event_is_32_cycles_after_it()
 {
    // The store is the sixth instruction, so it runs at master time 5 (1 cycle each).
    let mut m = booted(&rom_with(&TIMER_WRITER, &SPIN));
    step(&mut m, 6);
    assert_eq!(timer_events(&m), vec![(5 + 32, kind9(0))]);
}

#[test]
fn given_a_timer_started_by_an_arm7_rom_store_when_the_store_has_run_then_the_event_is_32_cycles_after_it()
 {
    // The ARM7 runs one instruction per 2 cycles: the sixth is at its clock 10, and it is
    // executed once the ARM9 has reached 11.
    let mut m = booted(&rom_with(&SPIN, &TIMER_WRITER));
    step(&mut m, 11);
    assert_eq!(timer_events(&m), vec![(10 + 32, kind7(0))]);
}

// ---------------------------------------------------------------------------- scheduler ties

#[test]
fn given_a_timer_overflowing_at_the_same_time_as_a_line_start_when_both_fire_then_the_line_start_fires_first()
 {
    // Reload 0xF7AE is 2130 ticks to overflow: 2130 * 2 = 4260 = one line, the first LineStart.
    let mut m = machine();
    set9(&mut m, 0, 0xF7AE, START);
    step(&mut m, LINE_CYCLES);
    // Both rescheduled for 8520: the one that fired first was queued first.
    let at_8520: Vec<EventKind> = m
        .scheduler
        .pending()
        .iter()
        .filter(|event| event.time == 2 * LINE_CYCLES)
        .map(|event| event.kind)
        .collect();
    assert_eq!(at_8520, vec![EventKind::LineStart, kind9(0)]);
}

#[test]
fn given_two_timers_with_events_at_the_same_time_when_they_fire_then_they_fire_in_schedule_order_not_index_order()
 {
    let mut m = machine();
    set9(&mut m, 1, RELOAD_16_TICKS, START);
    set9(&mut m, 0, RELOAD_16_TICKS, START);
    step(&mut m, 32);
    // Timer 1 was scheduled first, so it fired first and is queued first again.
    assert_eq!(timer_events(&m), vec![(64, kind9(1)), (64, kind9(0))]);
}

#[test]
fn given_two_timers_started_between_the_same_two_looks_when_the_machine_looks_then_the_lower_index_is_scheduled_first()
 {
    let mut m = machine();
    let tcm = tcm_now(&m);
    m.memory.write16(&tcm, reload_addr(1), RELOAD_16_TICKS);
    m.memory.write16(&tcm, control_addr(1), START);
    m.memory.write16(&tcm, reload_addr(0), RELOAD_16_TICKS);
    m.memory.write16(&tcm, control_addr(0), START);
    m.sync_timer_events();
    assert_eq!(timer_events(&m), vec![(32, kind9(0)), (32, kind9(1))]);
}

// ---------------------------------------------------------- a lagging ARM7 and the queue

/// A machine at master time 1000 whose ARM7 started a timer at its own clock 0, as a lagging
/// ARM7 would: reload `0xFFFF` at F/1 overflows at time 2, already past.
fn lagging_arm7_timer(control: u16) -> Machine {
    let mut m = machine();
    step(&mut m, 1000);
    set7_at(&mut m, 0, 0, 0xFFFF, control);
    m
}

#[test]
fn given_an_arm7_write_that_computes_an_overflow_already_past_when_the_machine_looks_then_the_event_is_queued_at_now()
 {
    let m = lagging_arm7_timer(START | IRQ);
    assert_eq!(timer_events(&m), vec![(1000, kind7(0))]);
}

#[test]
fn given_an_event_queued_at_now_for_a_lagging_arm7_when_the_state_is_saved_and_loaded_then_it_loads()
 {
    let m = lagging_arm7_timer(START | IRQ);
    let mut target = machine();
    assert_eq!(target.load_state(&m.save_state()), Ok(()));
}

#[test]
fn given_a_late_event_when_it_fires_then_the_next_overflow_is_on_the_timers_own_tick_grid() {
    let mut m = lagging_arm7_timer(START | IRQ);
    step(&mut m, 1);
    // The overflow was at 2, so the next is 2 + 1 tick = 4, not late-based.
    assert_eq!(
        (m.irq7().if_(), m.timers7().overflow_time(0)),
        (IF_TIMER0, Some(4))
    );
}

#[test]
fn given_a_reload_written_in_the_gap_before_the_late_event_fires_when_it_is_written_then_the_overflow_already_due_is_processed_first()
 {
    // The overflow was due at 2; the write at 3 lands before the event (queued for 1000) fires.
    let mut m = lagging_arm7_timer(START | IRQ);
    m.memory.write16_arm7(3, reload_addr(0), 0xFF00);
    m.sync_timer_events();
    // It latched IF with the reload it happened under, and the next one is one tick on (4).
    assert_eq!(
        (
            m.irq7().if_(),
            m.timers7().overflow_time(0),
            timer_events(&m)
        ),
        (IF_TIMER0, Some(4), vec![(1000, kind7(0))])
    );
}

#[test]
fn given_a_stop_written_in_the_gap_before_the_late_event_fires_when_it_is_written_then_the_overflow_already_due_still_raises_its_irq()
 {
    let mut m = lagging_arm7_timer(START | IRQ);
    m.memory.write16_arm7(3, control_addr(0), 0);
    m.sync_timer_events();
    assert_eq!((m.irq7().if_(), timer_events(&m)), (IF_TIMER0, vec![]));
}

// ---------------------------------------------------------- ARM9 IRQ reaches the CPU

#[test]
fn given_a_timer_irq_enabled_in_ie_and_ime_when_the_timer_overflows_then_the_arm9_enters_irq_mode_on_the_next_step()
 {
    let mut m = machine();
    let tcm = tcm_now(&m);
    m.memory.write32(&tcm, IME, 1);
    m.memory.write32(&tcm, IE, IF_TIMER0);
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    step(&mut m, 33);
    assert_eq!(
        (m.arm9().regs().cpsr(), m.irq9().if_()),
        (IRQ_ENTRY_CPSR, IF_TIMER0)
    );
}

#[test]
fn given_a_timer_irq_enabled_in_ie_when_one_cycle_before_the_overflow_then_the_arm9_has_not_entered_irq_mode()
 {
    let mut m = machine();
    let tcm = tcm_now(&m);
    m.memory.write32(&tcm, IME, 1);
    m.memory.write32(&tcm, IE, IF_TIMER0);
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    step(&mut m, 31);
    assert_eq!(m.arm9().regs().cpsr() & 0x1F, SYSTEM_I_CLEAR);
}

#[test]
fn given_a_timer_irq_with_ime_off_when_the_timer_overflows_then_if_is_set_but_the_arm9_keeps_running()
 {
    let mut m = machine();
    let tcm = tcm_now(&m);
    m.memory.write32(&tcm, IE, IF_TIMER0);
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    step(&mut m, 40);
    assert_eq!(
        (m.arm9().regs().cpsr() & 0x1F, m.irq9().if_()),
        (SYSTEM_I_CLEAR, IF_TIMER0)
    );
}

// ------------------------------------------------------------------------------ savestates

/// Both CPUs with a mix of timers: F/1 with IRQ, a count-up behind it, an ARM7 F/256 and an
/// ARM7 F/64 timer.
fn machine_with_timers() -> Machine {
    let mut m = machine();
    set9(&mut m, 0, RELOAD_16_TICKS, START | IRQ);
    set9(&mut m, 1, 0xFF00, START | COUNT_UP | IRQ);
    set7(&mut m, 0, 0x8000, START | 2 | IRQ);
    set7(&mut m, 2, 0xFFFE, START | 1);
    m
}

#[test]
fn given_a_machine_with_running_timers_when_saved_after_a_frame_and_loaded_then_it_equals_an_uninterrupted_run()
 {
    let mut uninterrupted = machine_with_timers();
    run_frames(&mut uninterrupted, 1);
    let saved = uninterrupted.save_state();
    run_frames(&mut uninterrupted, 1);

    let mut resumed = machine();
    resumed
        .load_state(&saved)
        .expect("a state saved by the same build loads");
    run_frames(&mut resumed, 1);
    assert!(resumed.save_state() == uninterrupted.save_state());
}

#[test]
fn given_a_saved_machine_with_timers_when_loaded_then_the_timers_and_events_match() {
    let mut original = machine_with_timers();
    step(&mut original, 1000);
    let mut loaded = machine();
    loaded.load_state(&original.save_state()).expect("loads");
    assert_eq!(
        (
            loaded.timers9().clone(),
            loaded.timers7().clone(),
            loaded.scheduler.pending()
        ),
        (
            original.timers9().clone(),
            original.timers7().clone(),
            original.scheduler.pending()
        )
    );
}

#[test]
fn given_a_saved_machine_with_timers_when_loaded_and_saved_again_then_the_bytes_are_identical() {
    let mut original = machine_with_timers();
    step(&mut original, 777);
    let saved = original.save_state();
    let mut loaded = machine();
    loaded.load_state(&saved).expect("loads");
    assert!(loaded.save_state() == saved);
}

/// The save-state bytes after running `frames` frames of the timer machine.
fn timer_run_bytes(frames: u32) -> Vec<u8> {
    let mut m = machine_with_timers();
    run_frames(&mut m, frames);
    m.save_state()
}

#[test]
fn given_the_same_rom_and_timer_setup_when_run_twice_then_the_savestate_bytes_are_identical() {
    assert!(timer_run_bytes(2) == timer_run_bytes(2));
}

#[test]
fn given_timers_running_when_the_arm9_irq_is_taken_in_two_runs_then_the_states_are_identical() {
    let run = || {
        let mut m = machine_with_timers();
        let tcm = tcm_now(&m);
        m.memory.write32(&tcm, IME, 1);
        m.memory.write32(&tcm, IE, IF_TIMER0);
        m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
        step(&mut m, 5000);
        m.save_state()
    };
    assert!(run() == run());
}

/// What loading the state of `mutated` into a fresh machine gives, where `mutated` is the timer
/// machine after `mutate` ran. A refused state returns its error.
fn load_after(mutate: impl FnOnce(&mut Machine)) -> Result<(), SavestateError> {
    let mut m = machine_with_timers();
    mutate(&mut m);
    machine().load_state(&m.save_state())
}

#[test]
fn given_an_unmodified_timer_state_when_loaded_then_it_is_accepted() {
    assert_eq!(load_after(|_| {}), Ok(()));
}

#[test]
fn given_a_running_timer_whose_event_is_missing_when_loaded_then_it_is_refused_as_corrupt() {
    assert_eq!(
        load_after(|m| m.scheduler.cancel(kind9(0))),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_a_running_arm7_timer_whose_event_is_missing_when_loaded_then_it_is_refused_as_corrupt() {
    assert_eq!(
        load_after(|m| m.scheduler.cancel(kind7(2))),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_a_running_timer_with_two_events_when_loaded_then_it_is_refused_as_corrupt() {
    assert_eq!(
        load_after(|m| m.scheduler.schedule(100, kind9(0))),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_an_event_for_a_stopped_timer_when_loaded_then_it_is_refused_as_corrupt() {
    assert_eq!(
        load_after(|m| m.scheduler.schedule(100, kind9(3))),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_an_event_for_a_count_up_timer_when_loaded_then_it_is_refused_as_corrupt() {
    assert_eq!(
        load_after(|m| m.scheduler.schedule(100, kind9(1))),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_a_timer_event_earlier_than_the_overflow_when_loaded_then_it_is_refused_as_corrupt() {
    // Timer 0's overflow is at 32; an event at 5 would fire it 27 cycles early.
    assert_eq!(
        load_after(|m| {
            m.scheduler.cancel(kind9(0));
            m.scheduler.schedule(5, kind9(0));
        }),
        Err(SavestateError::Corrupt)
    );
}

/// The saved bytes of the timer machine with timer 0 of the ARM9 stopped and holding `control`
/// instead of what it had, found by its own encoding, or `None` when that is not unique.
fn bytes_with_arm9_control(control: u8) -> Option<Vec<u8>> {
    let mut m = machine();
    // Stopped, reload 0x0012 (one varint byte), control 0x04: a distinctive encoding.
    set9(&mut m, 0, 0x0012, COUNT_UP);
    let needle = postcard::to_allocvec(m.timers9()).expect("plain data encodes");
    let bytes = m.save_state();
    let hits: Vec<usize> = bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle.as_slice())
        .map(|(at, _)| at)
        .collect();
    let at = *hits.first().filter(|_| hits.len() == 1)?;
    let mut patched = bytes;
    // The encoding is reload (1 byte), control (1 byte), ...: the control is the second byte.
    patched[at + 1] = control;
    Some(patched)
}

#[test]
fn given_a_state_whose_timer_control_has_an_unused_bit_when_loaded_then_it_is_refused_as_corrupt() {
    let bytes = bytes_with_arm9_control(0x08).expect("the timer block is found once");
    assert_eq!(machine().load_state(&bytes), Err(SavestateError::Corrupt));
}

#[test]
fn given_a_state_whose_timer_control_is_valid_when_patched_to_the_same_value_then_it_loads() {
    // The control of this state is 0x04 (count-up on timer 0, stopped): patching it to itself
    // proves the harness finds the right byte, so the refusal above is about the unused bit.
    let bytes = bytes_with_arm9_control(0x04).expect("the timer block is found once");
    assert_eq!(machine().load_state(&bytes), Ok(()));
}
