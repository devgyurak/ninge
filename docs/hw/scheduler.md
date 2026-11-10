# Scheduler: clocks, frame timing, and CPU interleaving

Sources: GBATEK "DS Memory Timings" / "DS Technical Data" (clock rates); GBATEK "DS Display
Dimensions / Timings" (lines, dots, frame length); GBATEK "DS Memory Maps" (ARM7 WRAM).
Used by: `crates/core/src/scheduler/`, `crates/core/src/machine/mod.rs` (`run_frame`, `step_arm9`),
`crates/core/src/bus/` (the ARM7 view)   Phase: 3 (#21)

The machine has two processors on one bus clock. The emulator advances them in turns and
fires timed effects (a frame ending, later a timer overflow or a DMA start) between turns.
The first half of this note is hardware; the second half is the model ninge chooses and is
**not** hardware.

## Hardware

### Clocks

| Clock | Rate | Relation |
|---|---|---|
| Bus | 33.513982 MHz | the reference clock |
| ARM7 | 33.513982 MHz | 1 x bus |
| ARM9 | 67.027964 MHz | 2 x bus |

The ARM9 clock is therefore exactly twice the ARM7 clock. GBATEK adds that the full ARM9 rate
only holds for accesses that stay inside the cache and the TCM; any other access waits for the
bus and takes several cycles. How many is a separate topic (waitstates, Phase 8).

### Frame timing

A frame is 263 scanlines of 355 dots, and a dot lasts 6 bus cycles:

- 263 x 355 x 6 = 560,190 bus (ARM7) cycles per frame;
- x 2 = 1,120,380 ARM9 cycles per frame.

The visible part is 192 lines of 256 dots; the rest is blanking. The line layout, when VBlank
and HBlank start, and the DISPSTAT / VCOUNT registers that report them are in
`display-timing.md` (#104).

### ARM7 memory

The ARM7 sees main RAM (shared, same 4 MiB as the ARM9) and its own 64 KiB of WRAM at
`0380_0000h` (`memory-map.md`). Shared WRAM, WRAMCNT and the `037F_8000h`
mirror are in `wramcnt.md` (#22); the rest of its I/O is still unmodeled.

## Model (not hardware)

Everything in this section is a choice made so the two CPUs can run in turns with a
deterministic result. None of it is claimed to match hardware timing.

### One master clock

Time is a `u64` counted in **ARM9 cycles**. The ARM9 is the reference CPU: `now` is how far the
ARM9 has run. One bus cycle is two master cycles, so the ARM7 clock, counted in the same unit,
advances in steps of 2.

### Cost per instruction

- One ARM9 instruction costs 1 master cycle.
- One ARM7 instruction costs 2 master cycles (1 ARM7 cycle).
- The cost is the return value of `Cpu::step`, and of `Cpu::take_interrupt` when it takes an
  interrupt: an interrupt entry costs the same as an instruction. The machine advances the CPU's
  clock by what the call returns. The constants live with the CPU (`cpu/mod.rs`), which does not
  depend on the scheduler.
- A step cannot tell the bus view its own end time, so the caller hands the view a **floor**: the
  earliest time an event the step schedules may fire (`timers.md`, "Model"). It is the clock plus
  the instruction cost for the ARM9 and the master clock for the ARM7. A savestate's bound on the
  ARM7's lead over the master clock uses the same cost; once costs vary it becomes the largest
  cost.

With these costs a frame gives the ARM9 1,120,380 instructions and the ARM7 560,190. This is
the best-case "one cycle per instruction" reading of the clock ratio: it ignores the waitstates
GBATEK describes, multi-cycle instructions, and cache misses.

### Events

A timed effect is an event: a kind and a timestamp in master cycles. Events live in a binary
heap. Order is **(timestamp, priority of the kind)**: two events with the same timestamp fire in
a fixed order that depends only on what they are, never on when they were scheduled. The order
is:

1. `LineStart`
2. `HBlankStart`
3. `TimerOverflow`, ARM9 timers 0-3, then ARM7 timers 0-3
4. `DmaStart`, ARM9 channels 0-3, then ARM7 channels 0-3

Why this order: the order in which the events raise their IF bits changes nothing, because IF
is only read between rounds. A tie matters only where one event's effect is seen by another,
and that is the DMA transfer, which moves data that a line, HBlank or timer handler may have
just changed. So transfers go last, within a CPU in ascending channel order (a lower channel has
the higher priority on hardware, `dma.md`). The ARM9 goes before the ARM7 in each group to match
the round's own tie order (ARM9, ARM7, events). The order is a model choice, not a hardware
fact (see "Unverified"). It is a function of the kind, so it is the same for a machine that
was saved and loaded, whose queue was rebuilt (see "Savestate checks"). The queue holds at most
one event of each kind (a reschedule cancels the pending one first), so `(timestamp, priority)`
orders every event completely. A sequence number, assigned from a counter that only goes up,
is kept only for the guard below; it is not part of the order and is not saved. The heap's internal
order is never relied on.

Events due at the target fire in priority order. An event a handler schedules during the
dispatch waits for the next round (the sequence number tells it from the ones queued before):
the dispatch only looks at the top of the heap, so if such an event sorts to the top, the
other events due at the same time wait for the next round too. That round is zero cycles long
and fires them in the same order, so nothing observable changes. This keeps a handler that reschedules itself
from being popped forever if the clock ever saturated; the machine would stall, not the host.

Four kinds exist so far, `LineStart` (every 4,260 master cycles), `HBlankStart` (once per
line, 3,212 master cycles after it; see `display-timing.md`), `TimerOverflow` (one per
started, clock-driven timer of either CPU, so at most eight; see `timers.md`) and `DmaStart` (one
per channel whose transfer is waiting to run, at most eight; see `dma.md`). A pending event of
a kind can be cancelled, which a timer write uses to replace its overflow: the bus view holds the
scheduler, so the write schedules at once, never before the end of the writing instruction
(`timers.md`, "Model"). The `LineStart` that begins line 0
draws the screens and counts the frame, which ends `run_frame`: frames stay 1,120,380 master
cycles apart. Each reschedules from its own time, so nothing drifts.

### Interleaving

One round works like this:

1. `target` = the smaller of the next event's time and `now + SLICE`, where SLICE is 64 master
   cycles. If an event is already due (its time is not after `now`), `target` is `now`.
2. The ARM9 runs until its clock reaches `target` or the earliest queued event, whichever is
   earlier, checked before every instruction. That includes an event the ARM9 itself queues
   during the round: it stops at the first instruction boundary at or after that event's time.
   Call the clock where it stopped `stop`; it is the ARM9's actual clock, so it stays right when
   an instruction costs more than one master cycle.
3. The ARM7 runs until its clock reaches `stop`. An ARM7 instruction that starts before
   `stop` finishes even if it ends one master cycle past it; the ARM7 simply starts the next
   round that much ahead.
4. Every event due at `stop` is dispatched, in (timestamp, priority) order. An event a handler
   queues waits for the next round.

**Tie order is explicit: ARM9, then ARM7, then events.** An event at time T therefore sees
the effects of every instruction either CPU ran before T, and no instruction that starts at or
after T.

**The exception is the ARM7.** The ARM9 runs ahead to `stop` before the ARM7 starts, so an event
the ARM7 queues is floored at `now`, which is already `stop` (`timers.md`, "Model"). A timer
whose computed overflow lies before `now` therefore fires at the end of the round instead of
at its time. That lag is a known, bounded approximation (under one slice) and is not changed by
the early ARM9 stop; an overflow that lies after `now` is queued at its time and fires on time.

SLICE bounds how far one CPU runs ahead of the other. It is a trade-off between interleaving
fidelity and per-round overhead, not a hardware fact.

### Debugger single-step

`Machine::step_arm9` advances the ARM9 by one instruction and `now` by its cost (1). With a
program booted it then settles: the ARM7 runs up to `now` and every event that became due
fires, so the queue never holds an event before `now` and a step across a frame end draws and
counts the frame. A machine with no program steps only the ARM9.

Stepping therefore catches the ARM7 up after every ARM9 instruction, where `run_frame` does it
once per slice. A program whose two CPUs race over the same memory can behave differently when
stepped than when run. Replay determinism is not affected: a given sequence of calls always
gives the same result.

### No program loaded

A machine with no booted program neither runs a CPU nor advances `now`: `run_frame` only counts
the frame.

### Savestate checks

The queue is not saved. It is derived from the device state and `now`, like the page tables,
and a load rebuilds it, so no check has to prove that the queue and the devices agree:

- `LineStart` and `HBlankStart` sit on a fixed grid, at `k * LINE_CYCLES` and
  `k * LINE_CYCLES + HBLANK_START_CYCLES`, because each reschedules from its own time starting
  at time 0. Between rounds no line event is due at `now`, so the next of each is the first
  grid point strictly after `now`.
- Every started, clock-driven timer gets a `TimerOverflow` at the later of the overflow time
  its own counter gives and `now` (`timers.md`, "Model").
- Every channel with a start waiting gets a `DmaStart` at `now`: a waiting start is due at once
  and the queue never holds an event before the master clock (`dma.md`, "Model").

Every event the queue can hold between rounds is covered, except the known gap under "Edge cases": the event a handler scheduled during
the last dispatch and that waits for the next round is one of these (a DMA start the VBlank
raised, due at `now`).

A saved machine is refused on load when:

- a started, clock-driven timer's overflow time is more than one slice behind `now`: that
  cannot arise, and the run loop would take forever to catch up (checked by the timers);
- `now` is above `MAX_TIME` (2^62 master cycles, about 2,180 years of emulated time, which no
  real run reaches). A smaller margin below `u64::MAX` cannot work: a frame end would
  eventually saturate at the maximum and stay due;
- a program is booted and the ARM7 clock is behind `now` or two or more master cycles ahead of
  it. A booted machine is always settled, so the ARM7 is at `now` or one master cycle past it.
  (A machine with no program steps only the ARM9, so its ARM7 clock may lag.)

## Edge cases

- Two events at the same time fire in priority order, whatever order they were scheduled in.
- Events scheduled out of timestamp order fire in timestamp order.
- An event in the future is not returned when popping at an earlier time.
- A machine with no program that is stepped past its frame end with `step_arm9` only advances
  `now`; no event fires, so its queue holds events before `now`. A savestate does not hold
  them, and a load rebuilds the queue from `now`, so those past events are gone. Such a machine
  never fires its events anyway (there is no program to run), so the only difference a caller
  can see is `pending()`.
- A sequence counter is a `u64` and cannot realistically wrap; it uses wrapping arithmetic so
  that a hand-made state cannot panic.

## Unverified

- **Instruction costs** (ARM9 1, ARM7 2 master cycles) ignore waitstates, multi-cycle
  instructions and cache effects. — `crates/core/src/cpu/mod.rs`
  (`ARM9_INSTRUCTION_COST`, `ARM7_INSTRUCTION_COST`) — verify against a test ROM timing a
  fixed loop against a hardware timer (Phase 8), or by trace comparison with the oracle.
- **The tie order of same-time events** (line, HBlank, timers, DMA; ARM9 before ARM7) is a model
  choice. IF is read only between rounds, so only a DMA transfer can see another event's effect,
  and the real order on hardware is not known. — `crates/core/src/scheduler/mod.rs`
  (`EventKind::priority`) — revisit if a test ROM or the oracle shows two same-time events
  ordered differently.
- **SLICE = 64 master cycles** is a model choice. — `crates/core/src/scheduler/mod.rs`
  (`SLICE`) — not a hardware quantity; revisit when a test ROM depends on how closely the two
  CPUs interleave (shared-memory races, IPC).
- **An ARM7 event is floored at the ARM9's clock.** The ARM9 reaches `stop` before the ARM7
  runs, so an ARM7 event computed behind it is raised to `now` and fires up to one slice late
  (`timers.md`, "Model"). A model choice, not a hardware quantity. — `crates/core/src/bus/view.rs`
  (`at`) — revisit with the ARM7 own-timer lag when a test ROM depends on it.
- **ARM7 WRAM mirroring**: the 64 KiB block is modeled as repeating through
  `0380_0000h`-`03FF_FFFFh`, GBATEK's general mirroring rule. Anything else on the ARM7 bus is
  open bus and reads 0. The shared WRAM window now sits in front of it (`wramcnt.md`). — `crates/core/src/bus/memory/mod.rs`
- **ARM7 entry state**: R13 values come from `direct-boot.md`; the CPSR is chosen as for the
  ARM9 (System mode, ARM state, I and F set). — `crates/core/src/hle/direct_boot.rs` — verify
  as for the ARM9.
