# Timers

Sources: GBATEK "DS Timers" (the timer registers, prescaler and count-up bits, the overflow
description); GBATEK "DS Interrupts" for the IF bits (see `interrupts.md`).
Used by: `crates/core/src/io/timers/mod.rs`, `crates/core/src/io/mod.rs`, `crates/core/src/bus/view.rs`, `crates/core/src/machine/mod.rs`,
`crates/core/src/scheduler/mod.rs`   Phase: 3 (#25)

Each CPU has four 16-bit timers. They sit at the same addresses on the ARM9 and the ARM7 but are
separate hardware: a write by one CPU never shows on the other, and each CPU's overflows raise
that CPU's own interrupt controller. The sound unit's use of the ARM7 timers is Phase 6 and is
not part of this note.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0100h` | TM0CNT_L | 16 | R: counter, W: reload | not stated, 0 here |
| `0400_0102h` | TM0CNT_H | 16 | R/W control | not stated, 0 here |
| `0400_0104h` / `0106h` | TM1CNT_L / TM1CNT_H | 16 | as above | as above |
| `0400_0108h` / `010Ah` | TM2CNT_L / TM2CNT_H | 16 | as above | as above |
| `0400_010Ch` / `010Eh` | TM3CNT_L / TM3CNT_H | 16 | as above | as above |

Timer *n* sits at `0400_0100h + 4n`; its `_H` half is two bytes above. A 32-bit access at
`TMnCNT_L` therefore covers the reload (low half) and the control (high half) of one timer.

### TMnCNT_H bits

| Bits | Meaning |
|---|---|
| 0-1 | Prescaler: 0 = F/1, 1 = F/64, 2 = F/256, 3 = F/1024 |
| 2 | Count-up: tick when the previous timer overflows instead of from the clock (not on timer 0) |
| 3-5 | Not used |
| 6 | IRQ enable: raise the timer's IF bit on overflow |
| 7 | Start: 0 = stopped, 1 = running |
| 8-15 | Not used |

### IF bits

An overflow of timer *n* requests IF bit 3 + *n* on the CPU that owns the timer (`interrupts.md`,
"IE and IF bits"): bits 3, 4, 5 and 6.

## Behavior

### Clock

The base clock F is the bus clock, 33.513982 MHz, for both CPUs (`scheduler.md`, "Clocks"). A
timer in prescaler mode F/1 ticks once per bus cycle, so in the master clock of `scheduler.md`
(ARM9 cycles) one tick is 2 master cycles; F/64, F/256 and F/1024 tick every 128, 512 and 2048
master cycles.

### Counting

A running timer that is not in count-up mode counts up by one each tick. When the 16-bit
counter would pass `FFFFh` it overflows: the counter takes the reload value, the next count
continues from there, and the IRQ is requested if bit 6 is set. A timer therefore overflows
every `10000h - reload` ticks.

- **Start.** When bit 7 goes from 0 to 1 the reload value is loaded into the counter. The timer
  counts from there.
- **Reload.** A write to `TMnCNT_L` sets the reload value only. It does not change the counter,
  running or not. A running timer picks it up at its next overflow.
- **Reading `TMnCNT_L`** returns the current counter, also while the timer runs.
- **Stop.** When bit 7 goes from 1 to 0 the counter stops and keeps its value; a read returns it.
  A start after that loads the reload value again.

### Count-up

A timer with bit 2 set ignores the clock and its prescaler. It adds one to its counter each time
the previous timer (n-1) overflows, and overflows itself on passing `FFFFh`, as above. Chaining
timers 0 to 3 gives up to 64 bits of range. A count-up timer is modeled as counting only while it is started (see "Unverified");
the previous timer's overflow counts whether or not that timer's own IRQ is enabled. Timer 0 has
no previous timer: its bit 2 has no effect.

### Writing a 32-bit word

The bus splits a wider write into bytes in ascending address order (`arm9-memory-map.md`), so
the reload reaches `TMnCNT_L` before the control byte reaches `TMnCNT_H`. See "Unverified" for
what this model assumes about it.

## Edge cases

- **Reload `FFFFh`** gives one tick per period: an overflow every tick.
- **Reload 0** gives a full 65,536-tick period.
- **Count-up with the previous timer stopped** never ticks.
- **Cascade:** one overflow of timer *n* can overflow *n+1*, which can overflow *n+2*; each
  requests its own IRQ. The model settles the whole chain in the one event of timer *n*.
- **IRQ disabled** (bit 6 clear): the timer still overflows, reloads and cascades; only the IF
  request is not raised.

## Model (not hardware)

These are choices so timers fit the event scheduler (`scheduler.md`); none is claimed to be
hardware behavior.

- **The counter is computed from time.** A running, non-count-up timer stores its counter at an
  origin time; a read at time *t* gives `counter` plus the ticks since the origin, wrapped
  through the overflow into `reload + ticks mod (10000h - reload)`. Nothing steps the counter
  per instruction.
- **Time comes from the reading CPU.** A read or write at an ARM9 instruction is at the master
  clock; at an ARM7 instruction it is at the ARM7 clock. The two differ by less than one ARM7
  instruction outside an ARM7 slice (`scheduler.md`, "Interleaving").
- **Overflows are events.** One `TimerOverflow` event per running, non-count-up timer, at the
  master time of its next overflow. Count-up timers have no event: the overflow that feeds them
  adds to them. A write to `TMnCNT_H` that starts, stops, or changes the prescaler or count-up
  mode replaces that timer's event, at the write itself: the bus view that carries the write holds
  the scheduler, so the event is queued before the writing instruction retires and nothing looks
  for changed timers afterwards. The write also replaces the event of any timer whose overdue
  overflows it settled (see "A write in that gap settles first").
- **Event time is not before the end of the writing instruction.** A timer written by the ARM7
  while it lags the master clock can compute an overflow that is already past; its event is
  queued for the master clock, which the ARM7's step leaves where it is. For the ARM9 the floor
  is one instruction cost past the clock (`scheduler.md`, "Cost per instruction"), the master time
  at which the writing instruction retires. The floor is an input to the bus view, which the
  caller computes before the step, because the step's cost is known only when it ends. An F/1 timer's shortest
  period is 2 master cycles, so an ARM9 write that starts a timer is never raised to the floor; only
  a long-overdue settle (see below) can be. The next overflow stays on the timer's own tick grid,
  so the count does not drift. Only the delivery of the IRQ is late, by less than one slice.
- **Events queued by one instruction keep the order they were written.** Two timers that overflow
  at the same master time fire in the order they were scheduled (`scheduler.md`, "Events"). A
  store-multiple writes ascending addresses, so timer 0 is queued before timer 1. A timer rewritten within the
  instruction keeps one event, queued by its last write. A store-multiple that settles a lagging
  ARM7 timer in an earlier write and retimes a lower-numbered one in a later write queues them in
  that write order; both fire together at the same dispatch, so only the sequence numbers differ.
- **An event settles all of its timer's due overflows.** A late event (see above) processes every
  overflow of its timer already due, at most `SLICE`, so the queue always moves forward. A saved
  state is refused unless each timer's event is at the later of its overflow time and `now`, and
  that overflow time is at most one slice behind `now`.
- **A write in that gap settles first.** The ARM9 stops at a queued event, so for its own timers
  an event fires at the first instruction boundary at or after its time (`scheduler.md`,
  "Interleaving"). The gap arises only for an ARM7 timer whose computed overflow lies before the
  master clock: its event is raised to the master clock and fires at the end of a round, so a
  register write can land after an overflow's time but before its event. Any write to a timer
  register first processes the overflows already due at the write's time, with the settings they
  happened under: their IF bits are raised and a count-up timer behind them ticks. A reload or
  control change then applies to what follows. The gap is under one slice, so at most `SLICE / 2`
  overflows are settled this way. A read in the gap computes a clock-driven counter from time and is
  right; a count-up timer read there is behind by the overflows of the timer feeding it that
  are due but not yet fired, at most one slice's worth (`SLICE` divided by that timer's period,
  so for a fast feeder several ticks) until the event fires. A feeder with a period longer
  than a slice is never late this way.

## Unverified

- Start delay: whether the first tick comes at once or after a delay (GBATEK is silent) —
  `crates/core/src/io/timers/mod.rs` (`TODO(hw-unverified)`) — verify by a test ROM on hardware.
- Changing the prescaler of a running timer: the new rate applies from the write, with the
  counter kept — `io/timers/mod.rs` — verify on hardware.
- Whether starting a timer resets the prescaler's phase: modeled as yes (the first tick is one
  full period after the start) — `io/timers/mod.rs`.
- Reset values: 0 for reload, counter and control — `io/timers/mod.rs`.
- Count-up on timer 0: bit 2 reads back as written and has no effect — `io/timers/mod.rs`.
- Unused bits (3-5, 8-15) of `TMnCNT_H`: dropped on write and read as 0 — `io/timers/mod.rs`.
- Byte and halfword access: a byte of `TMnCNT_L` writes that byte of the reload and reads that
  byte of the counter; a byte write to `TMnCNT_H` starts, stops or changes the timer as its
  value says — `io/timers/mod.rs`.
- A start or stop write by a byte access to the high byte of `TMnCNT_H` does nothing (no
  control bit lives there) — `io/timers/mod.rs`.
- A 32-bit write is split into bytes and the reload lands before the start, so a word that sets
  both starts the timer with that reload — `bus/view.rs` (`Access::write_registers`), `io/timers/mod.rs` — verify
  on hardware.
- A count-up timer that is stopped ignores the previous timer's overflow — `io/timers/mod.rs` — verify
  on hardware.
