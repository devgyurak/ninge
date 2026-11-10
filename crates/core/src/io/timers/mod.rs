// SPDX-License-Identifier: Apache-2.0
//! One CPU's four timers (`docs/hw/timers.md`).
//!
//! Each CPU owns one set at the same addresses; they never see each other: the "an independent
//! copy for each CPU" class of `AGENTS.md` §4 ("Per-CPU devices"), two instances of
//! `Timers<const ARM9: bool>`. The counter is not stepped: a running timer stores the counter it
//! had at an origin time and every read works the value out from the clock it is given
//! (`timers.md`, "Model"). An overflow is a scheduler event; this module has no way to reach the
//! scheduler or an interrupt controller, so a write or an overflow hands back an [`Effect`]: the
//! requests it would raise and the mask of timers whose overflow event must be replaced. `Io`
//! schedules them, asking each timer for its next overflow ([`Timers::overflow_time`]); that
//! and the overflow itself are the timers' side of the event-slot interface (`Device::slot_kind`,
//! `slot_due` and `slot_fire`).

use crate::io::device::{Device, Effect, Fired, Width, read_bytes, write_bytes};
use crate::io::irq::{Requests, TIMER0_IRQ, TIMER1_IRQ, TIMER2_IRQ, TIMER3_IRQ};
use crate::scheduler::{EventKind, SLICE};
use serde::{Deserialize, Serialize};

/// TM0CNT_L, at `0400_0100h` (`timers.md`, "Registers"). Timer *n* is `4n` bytes above it.
pub const TM0CNT_L_ADDR: u32 = 0x0400_0100;
/// Timers per CPU (`timers.md`).
pub const TIMER_COUNT: usize = 4;
/// Bytes of the register block: four timers, each a reload/counter half and a control half.
const BLOCK_BYTES: u32 = 4 * TIMER_COUNT as u32;

/// IF bit of each timer's overflow (`timers.md`, "IF bits").
const OVERFLOW_IRQ: [u32; TIMER_COUNT] = [TIMER0_IRQ, TIMER1_IRQ, TIMER2_IRQ, TIMER3_IRQ];

/// TMnCNT_H bits (`timers.md`, "TMnCNT_H bits").
const CONTROL_PRESCALER_MASK: u16 = 0b11; // bits 0-1
const CONTROL_COUNT_UP: u16 = 1 << 2;
const CONTROL_IRQ_ENABLE: u16 = 1 << 6;
const CONTROL_START: u16 = 1 << 7;
/// The control bits that exist (`timers.md`, "TMnCNT_H bits"); the rest read 0.
// TODO(hw-unverified): the unused bits are dropped on write and read 0 (`timers.md`, "Unverified").
const CONTROL_MASK: u16 =
    CONTROL_PRESCALER_MASK | CONTROL_COUNT_UP | CONTROL_IRQ_ENABLE | CONTROL_START;

/// Counter values in one full period: it overflows when it would pass `FFFFh`.
const COUNTER_RANGE: u64 = 0x1_0000;
/// Master cycles per tick at F/1: one bus cycle is two ARM9 cycles (`timers.md`, "Clock").
const MASTER_CYCLES_PER_TICK: u64 = 2;
/// log2 of the clock divider each prescaler setting picks: F/1, F/64, F/256, F/1024.
const PRESCALER_SHIFT: [u32; 4] = [0, 6, 8, 10];

/// One timer's registers and the counter's value at a moment. Plain data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Timer {
    reload: u16,
    control: u16,
    // The counter as it was at `origin`. A stopped or count-up timer's counter is exactly this;
    // for a clock-driven one it is the base of the ticks counted since `origin`.
    counter: u16,
    // Master time of `counter`; used only while the timer is clock-driven and running.
    origin: u64,
}

impl Timer {
    fn running(&self) -> bool {
        self.control & CONTROL_START != 0
    }

    fn irq_enabled(&self) -> bool {
        self.control & CONTROL_IRQ_ENABLE != 0
    }

    /// Master cycles one tick takes at the prescaler this timer is set to.
    fn tick_cycles(&self) -> u64 {
        MASTER_CYCLES_PER_TICK
            << PRESCALER_SHIFT[usize::from(self.control & CONTROL_PRESCALER_MASK)]
    }

    /// Ticks from a counter value to its overflow: `10000h - counter`, 1 to 65,536.
    fn ticks_to_overflow(counter: u16) -> u64 {
        COUNTER_RANGE - u64::from(counter)
    }

    /// Ticks in one period from a reload: `10000h - reload`.
    fn period_ticks(&self) -> u64 {
        Self::ticks_to_overflow(self.reload)
    }

    /// The counter at master time `now`, for a clock-driven running timer.
    fn counter_at(&self, now: u64) -> u16 {
        // A clock behind the origin (the ARM9 reading just after an ARM7 write) sees no ticks.
        let ticks = now.saturating_sub(self.origin) / self.tick_cycles();
        let first = Self::ticks_to_overflow(self.counter);
        match ticks.checked_sub(first) {
            // Still in the first period: no overflow yet. `ticks < first <= 65,536`.
            None => self.counter.wrapping_add(ticks as u16),
            // Past it: the reload took over, and the remaining ticks wrap through whole periods.
            Some(past) => self
                .reload
                .wrapping_add((past % self.period_ticks()) as u16),
        }
    }
}

/// The four timers of one CPU: the ARM9's when `ARM9` is true, the ARM7's otherwise. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timers<const ARM9: bool> {
    timers: [Timer; TIMER_COUNT],
}

impl<const ARM9: bool> Timers<ARM9> {
    /// Timers in their power-on state: stopped, everything 0.
    // TODO(hw-unverified): the reset values are not stated (`timers.md`, "Unverified").
    pub fn new() -> Self {
        Self {
            timers: [Timer::default(); TIMER_COUNT],
        }
    }

    /// Timer `index`'s counter at master time `now`. An `index` past the last timer reads 0.
    pub fn counter(&self, index: usize, now: u64) -> u16 {
        self.timers.get(index).map_or(0, |timer| {
            if timer.running() && !self.counts_up(index) {
                timer.counter_at(now)
            } else {
                timer.counter
            }
        })
    }

    /// Timer `index`'s reload value. An `index` past the last timer reads 0.
    pub fn reload(&self, index: usize) -> u16 {
        self.timers.get(index).map_or(0, |timer| timer.reload)
    }

    /// Timer `index`'s control bits (TMnCNT_H). An `index` past the last timer reads 0.
    pub fn control(&self, index: usize) -> u16 {
        self.timers.get(index).map_or(0, |timer| timer.control)
    }

    /// Whether timer `index` is driven by the previous timer's overflow. Timer 0 has none, so
    /// its count-up bit has no effect (`timers.md`, "Count-up").
    // TODO(hw-unverified): timer 0's bit 2 is ignored (`timers.md`, "Unverified").
    fn counts_up(&self, index: usize) -> bool {
        index > 0
            && self
                .timers
                .get(index)
                .is_some_and(|timer| timer.control & CONTROL_COUNT_UP != 0)
    }

    /// Whether timer `index` has an overflow event: started and driven by the clock.
    pub(crate) fn has_event(&self, index: usize) -> bool {
        self.timers
            .get(index)
            .is_some_and(|timer| timer.running() && !self.counts_up(index))
    }

    /// The master time of timer `index`'s next overflow, or `None` when it has no event: it is
    /// stopped, count-up, or does not exist.
    pub fn overflow_time(&self, index: usize) -> Option<u64> {
        let timer = self.timers.get(index).filter(|_| self.has_event(index))?;
        let ticks = Timer::ticks_to_overflow(timer.counter);
        Some(
            timer
                .origin
                .saturating_add(ticks.saturating_mul(timer.tick_cycles())),
        )
    }

    /// The event that carries timer `index`'s overflow.
    pub(crate) fn event_kind(index: usize) -> EventKind {
        EventKind::TimerOverflow {
            arm9: ARM9,
            // The index is below 4 wherever it comes from a timer.
            index: index as u8,
        }
    }

    /// Timer `index` overflows at master time `time`: it reloads, the timers it feeds tick, and
    /// the requests to raise come back, filled for this set's CPU (`timers.md`, "Counting" and
    /// "Count-up").
    ///
    /// A clock-driven timer's next period starts at `time`, so the grid never drifts.
    pub(crate) fn overflow(&mut self, index: usize, time: u64) -> Requests {
        let mut mask = 0;
        let mut index = index;
        while let Some(timer) = self.timers.get_mut(index) {
            timer.counter = timer.reload;
            timer.origin = time;
            if timer.irq_enabled() {
                mask |= OVERFLOW_IRQ.get(index).copied().unwrap_or(0);
            }
            // The next timer ticks once; it overflows in turn when that passes `FFFFh`.
            index += 1;
            let Some(next) = self.timers.get_mut(index) else {
                break;
            };
            if !(next.running() && next.control & CONTROL_COUNT_UP != 0) {
                break;
            }
            match next.counter.checked_add(1) {
                Some(counter) => {
                    next.counter = counter;
                    break;
                }
                None => continue,
            }
        }
        Requests::for_cpu::<ARM9>(mask)
    }

    /// The byte at `offset` of the register block, read at master time `now`; 0 where no timer
    /// holds it.
    // TODO(hw-unverified): a byte read returns that byte of the counter or the control
    // (`timers.md`, "Unverified").
    fn read_byte(&self, offset: u32, now: u64) -> u8 {
        let Some((index, high_half, shift)) = Self::register_at(offset) else {
            return 0;
        };
        let value = if high_half {
            self.control(index)
        } else {
            self.counter(index, now)
        };
        (value >> shift) as u8
    }

    /// Writes a byte at `offset` at master time `now`; a byte no timer holds is ignored. A write
    /// to the low half sets the reload; one to the control half may start, stop or retime the
    /// timer.
    // TODO(hw-unverified): a byte write acts on its own byte of the reload, and a write to the
    // low byte of the control acts at once (`timers.md`, "Unverified").
    ///
    /// Overflows already due at `now` are processed first, with the settings they happened
    /// under (`timers.md`, "Model"); the IF bits they raise come back, and so does the set of
    /// timers whose overflow event is now stale: those that overflowed, and the one the write
    /// retimed.
    fn write_byte(&mut self, offset: u32, value: u8, now: u64) -> Effect {
        let Some((index, high_half, shift)) = Self::register_at(offset) else {
            return Effect::default();
        };
        let settled = self.settle(now);
        if high_half {
            // TODO(hw-unverified): the high byte of TMnCNT_H holds no bits (`timers.md`,
            // "Unverified").
            let retimed = if shift == 0 {
                self.write_control(index, value, now)
            } else {
                0
            };
            return settled.merge(Effect {
                requests: Requests::default(),
                reschedule: retimed,
            });
        }
        if let Some(timer) = self.timers.get_mut(index) {
            let byte_mask = 0xFF_u16 << shift;
            timer.reload = (timer.reload & !byte_mask) | (u16::from(value) << shift);
        }
        settled
    }

    /// Processes every overflow due at or before `now` that its event has not fired yet: a
    /// register write can land in the gap between an overflow's time and its event (an event
    /// fires at the end of the round, `scheduler.md`, "Interleaving"). Each timer that overflowed
    /// is marked so its event is replaced.
    ///
    /// The gap is under one slice, so a timer overflows at most `SLICE / 2` times in it; the bound
    /// also keeps a hand-made state with a far-off origin from looping for ages.
    fn settle(&mut self, now: u64) -> Effect {
        (0..TIMER_COUNT).fold(Effect::default(), |outcome, index| {
            outcome.merge(self.settle_timer(index, now))
        })
    }

    /// [`Timers::settle`] for timer `index` alone: what its event does when it fires.
    pub(crate) fn settle_timer(&mut self, index: usize, now: u64) -> Effect {
        let mut outcome = Effect::default();
        for _ in 0..SLICE {
            let Some(due) = self.overflow_time(index).filter(|&due| due <= now) else {
                break;
            };
            outcome.requests = outcome.requests.merge(self.overflow(index, due));
            outcome.reschedule |= 1 << index;
        }
        outcome
    }

    /// Applies the low byte of TMnCNT_H at `now`: the counter is settled under the old settings
    /// first, then the new ones take over from `now`.
    // TODO(hw-unverified): the new prescaler applies from the write and the phase restarts
    // there; the first tick comes one full period after a start, with no start delay
    // (`timers.md`, "Unverified").
    ///
    /// Returns the mask of timers whose overflow event no longer matches: the timer itself when
    /// the write started, stopped or retimed it, nothing otherwise.
    fn write_control(&mut self, index: usize, value: u8, now: u64) -> u8 {
        let was_running = self.timers.get(index).is_some_and(Timer::running);
        let settled = self.counter(index, now);
        let Some(timer) = self.timers.get_mut(index) else {
            return 0;
        };
        let old = timer.control;
        let new = u16::from(value) & CONTROL_MASK;
        timer.control = new;
        // An IRQ-enable change leaves the counter and the overflow time alone, and so does a
        // change of timer 0's count-up bit, which has no effect (`counts_up`).
        let inert = CONTROL_IRQ_ENABLE | if index == 0 { CONTROL_COUNT_UP } else { 0 };
        if old & !inert == new & !inert {
            return 0;
        }
        timer.counter = match (was_running, new & CONTROL_START != 0) {
            // A start loads the reload value.
            (false, true) => timer.reload,
            // Running, or stopping: the counter is where it stood.
            _ => settled,
        };
        timer.origin = now;
        // The overflow event no longer matches: start, stop, the clock or the mode changed.
        // The index is below 4 here, so the shift cannot overflow.
        1 << index
    }

    /// Which timer holds `offset`, whether it is the control half, and the bit position of the
    /// byte within its 16-bit register.
    fn register_at(offset: u32) -> Option<(usize, bool, u32)> {
        (offset < BLOCK_BYTES).then_some(((offset / 4) as usize, offset % 4 >= 2, (offset % 2) * 8))
    }

    /// Whether the decoded timers hold only bits that exist, and none overflowed more than one
    /// slice before `now`: that cannot arise, and the run loop would take forever to catch up
    /// (`scheduler.md`, "Savestate checks"). A savestate that decodes to anything else is corrupt
    /// (`savestate`).
    pub(crate) fn is_well_formed(&self, now: u64) -> bool {
        self.timers
            .iter()
            .all(|timer| timer.control & !CONTROL_MASK == 0)
            && (0..TIMER_COUNT).all(|index| {
                self.overflow_time(index)
                    .is_none_or(|due| now.saturating_sub(due) <= SLICE)
            })
    }
}

/// An independent copy for each CPU (`AGENTS.md` §4). Its registers are bytes; a wider access is
/// split into them, and a read has no side effect.
impl<const ARM9: bool> Device<ARM9> for Timers<ARM9> {
    fn peek(&self, offset: u32, width: Width, now: u64) -> u32 {
        read_bytes(offset, width, |at| self.read_byte(at, now))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, now: u64) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte(at, byte, now)
        })
    }

    fn slot_kind(&self, slot: u8) -> Option<EventKind> {
        (usize::from(slot) < TIMER_COUNT).then(|| Self::event_kind(usize::from(slot)))
    }

    fn slot_due(&self, slot: u8) -> Option<u64> {
        self.overflow_time(usize::from(slot))
    }

    /// A timer's overflow event fired: it reloads, its IRQ is requested, the timer it feeds ticks
    /// and its next overflow is scheduled (`timers.md`, "Counting").
    ///
    /// Every overflow of this timer due by `now` is processed, not just one: an event queued late
    /// (an ARM7 write while it lagged) would otherwise catch up one overflow a round with the
    /// CPUs standing still. The timer is rescheduled whether or not one was due.
    fn slot_fire(&mut self, slot: u8, now: u64) -> Fired {
        let index = usize::from(slot);
        let mut effect = self.settle_timer(index, now);
        // A slot past the last timer (a hand-made event) has no bit to set.
        effect.reschedule |= 1_u8.checked_shl(u32::from(slot)).unwrap_or(0);
        Fired {
            effect,
            transfer: None,
        }
    }
}

impl<const ARM9: bool> Default for Timers<ARM9> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
