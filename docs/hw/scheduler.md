# Scheduler: clocks, frame timing, and CPU interleaving

Sources: GBATEK "DS Memory Timings" / "DS Technical Data" (clock rates); GBATEK "DS Display
Dimensions / Timings" (lines, dots, frame length); GBATEK "DS Memory Maps" (ARM7 WRAM).
Used by: `crates/core/src/scheduler/`, `crates/core/src/lib.rs` (`run_frame`, `step_arm9`),
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

The visible part is 192 lines of 256 dots; the rest is blanking. Which of the 263 lines is
which, and the DISPSTAT / VCOUNT registers that report it, are not part of this note (no issue
covers them yet).

### ARM7 memory seen by #21

The ARM7 sees main RAM (shared, same 4 MiB as the ARM9) and its own 64 KiB of WRAM at
`0380_0000h` (`memory-map.md`). The rest of its map (shared WRAM, WRAMCNT, the `037F_8000h`
mirror, I/O) belongs to #22.

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

With these costs a frame gives the ARM9 1,120,380 instructions and the ARM7 560,190. This is
the best-case "one cycle per instruction" reading of the clock ratio: it ignores the waitstates
GBATEK describes, multi-cycle instructions, and cache misses.

### Events

A timed effect is an event: a kind and a timestamp in master cycles. Events live in a binary
heap. Order is **(timestamp, sequence number)**, where the sequence number is assigned from a
counter that only goes up, so two events with the same timestamp fire in the order they were
scheduled. The heap's internal order is never relied on.

Only one kind exists so far, `FrameEnd`, scheduled 1,120,380 master cycles apart. It ends
`run_frame`.

### Interleaving

One round works like this:

1. `target` = the smaller of the next event's time and `now + SLICE`, where SLICE is 64 master
   cycles. If an event is already due (its time is not after `now`), `target` is `now`.
2. The ARM9 runs until its clock reaches `target`.
3. The ARM7 runs until its clock reaches `target`. An ARM7 instruction that starts before
   `target` finishes even if it ends one master cycle past it; the ARM7 simply starts the next
   round that much ahead.
4. Every event due at `target` is dispatched, in (timestamp, sequence) order.

**Tie order is explicit: ARM9, then ARM7, then events.** An event at time T therefore sees
the effects of every instruction either CPU ran before T, and no instruction that starts at or
after T.

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

A saved machine is refused on load when:

- a queued event's time is before `now`;
- the sequence counter is not above every queued sequence number (the next event would
  tie-break before an existing one);
- the queue does not hold exactly one frame end, or that frame end is not due within one frame
  (`FRAME_CYCLES`) of `now`: two at the same time would fire in one round and `run_frame` would
  step over its frame, and one far ahead would take effectively forever;
- `now` is within two frames of `u64::MAX`, where the next frame end would saturate and stay
  due forever;
- a program is booted and the ARM7 clock is behind `now` or two or more master cycles ahead of
  it. A booted machine is always settled, so the ARM7 is at `now` or one master cycle past it.
  (A machine with no program steps only the ARM9, so its ARM7 clock may lag.)

## Edge cases

- Two events at the same time fire in scheduling order.
- Events scheduled out of timestamp order fire in timestamp order.
- An event in the future is not returned when popping at an earlier time.
- A sequence counter is a `u64` and cannot realistically wrap; it uses wrapping arithmetic so
  that a hand-made state cannot panic.

## Unverified

- **Instruction costs** (ARM9 1, ARM7 2 master cycles) ignore waitstates, multi-cycle
  instructions and cache effects. — `crates/core/src/scheduler/mod.rs`
  (`ARM9_INSTRUCTION_COST`, `ARM7_INSTRUCTION_COST`) — verify against a test ROM timing a
  fixed loop against a hardware timer (Phase 8), or by trace comparison with the oracle.
- **SLICE = 64 master cycles** is a model choice. — `crates/core/src/scheduler/mod.rs`
  (`SLICE`) — not a hardware quantity; revisit when a test ROM depends on how closely the two
  CPUs interleave (shared-memory races, IPC).
- **ARM7 WRAM mirroring**: the 64 KiB block is modeled as repeating through
  `0380_0000h`-`03FF_FFFFh`, GBATEK's general mirroring rule. Anything else on the ARM7 bus is
  open bus and reads 0. — `crates/core/src/bus/memory.rs` — revisit with the full ARM7 map (#22).
- **ARM7 entry state**: R13 values come from `direct-boot.md`; the CPSR is chosen as for the
  ARM9 (System mode, ARM state, I and F set). — `crates/core/src/hle/direct_boot.rs` — verify
  as for the ARM9.
