# HLE BIOS: the BIOS area, the vectors, SWI entry and the IRQ dispatcher

Sources: GBATEK "DS Memory Maps" (the NDS9 and NDS7 BIOS rows and the BIOS ROM sizes) and GBATEK
"DS Interrupts" (the handler pointer and IRQ check flag addresses), both read; GBATEK "BIOS
Interrupt Handling" for the GBA (the register set the BIOS saves around the user handler and the
`SUBS PC, LR, #4` return), read for the GBA only. Two more are recalled from memory of GBATEK,
not read, so each is under "Unverified": the SWI comment field ("ARM Opcodes ... SWI", "THUMB
Opcodes ... SWI") and the DS function numbers ("BIOS Function Summary"). Also
`arm-exceptions-modes.md` for the exception entry.
Used by: `crates/core/src/hle/`, `crates/core/src/bus/memory/mod.rs`,
`crates/core/src/machine/mod.rs`   Phase: 3 (#27, parts 1 and 2 of 3)

ninge boots without a BIOS dump (AGENTS.md §3.2). The BIOS is high-level emulated: the CPU
enters its exception vectors exactly as on hardware, and the machine does in Rust what the BIOS
code at those addresses would do. No BIOS image exists in ninge, and no instruction of it is
written down anywhere. This note covers the BIOS area, the vectors, how an SWI number is found,
the IRQ dispatcher, the wait routines (Halt, WaitByLoop, IntrWait, VBlankIntrWait) and the SWI
numbers nothing implements. The copy, math and decompression routines are the next part (#27,
PR 3); until then they take the "Unimplemented number" path below.

## The BIOS area

| CPU | Base | Size | Notes |
|---|---|---|---|
| ARM9 | `FFFF_0000h` | 32 KiB | GBATEK: "only 3K used"; the area ends at `FFFF_7FFFh` |
| ARM7 | `0000_0000h` | 16 KiB | the vectors are always here; the ARM7 has no vector base register |

Each CPU sees only its own BIOS. The ARM9 BIOS is at `FFFF0000h` whether or not the CPU uses
the high vectors, but a read there only matters when CP15 bit 13 (`cp15.md`) puts the vectors
there. Without the bit the vectors are at `0000_0000h` (`arm-exceptions-modes.md`, "Vector
base"), which is the instruction TCM if the game enabled it; the game then owns the handlers and
the BIOS is not involved (below).

The area is **synthetic**: it reads as zero, and a write to it is ignored (it is ROM). The page
table maps it as its own region so that a fetch from the vectors takes the fast path and the
debugger can tell it from unmapped space; its bytes carry no BIOS data.

## The vectors

The exception vectors are at the base, 4 bytes apart (`arm-exceptions-modes.md`, "Vector
table"). Of them the machine looks at two, plus two addresses of its own:

| Offset | Name | Taken by |
|---|---|---|
| `+08h` | SWI vector | the SWI instruction |
| `+18h` | IRQ vector | an interrupt (`interrupts.md`) |
| `+20h` | IRQ return | ninge's own address (below); the user handler returns to it |
| `+24h` | IntrWait loop | ninge's own address (below); IntrWait waits here |

`+20h` and `+24h` are not hardware vectors. On hardware the BIOS IRQ code gives the user handler a
return address inside the BIOS, and GBATEK does not state the DS address; ninge picks `+20h` (an
address past the FIQ vector that nothing uses) and says so in "Unverified". `+24h` is the place
where the IntrWait code of this model loops. The other vectors (reset, undefined, prefetch and data abort, FIQ) are not handled:
the CPU enters them and executes the zero words of the synthetic area.

## How the machine takes the BIOS's place

The CPU enters the exception normally: the mode and the banked registers change, `R14_<mode>`
and `SPSR_<mode>` are written, I is set, T is cleared, and R15 becomes the vector
(`arm-exceptions-modes.md`). The pipeline is refilled from the vector with two fetches of the
synthetic area.

Before it executes an instruction, the machine compares R15 with the trap addresses. R15 reads
the address of the next instruction + 8 in ARM state (`arm-branch-pipeline.md`); a refill from
the vector `V` leaves R15 = `V + 8`, so the trap addresses in R15 terms are base + `10h` (SWI),
base + `20h` (IRQ), base + `28h` (IRQ return) and base + `2Ch` (IntrWait loop). When R15 is one of them, T is clear and the CPU
is the one that owns the BIOS area, the machine does the BIOS's work and then leaves exactly as
the BIOS's return instruction would. The check is one compare on the common path (R15 outside
the span of the BIOS vectors).

**The prefetch pipeline.** The two opcodes in the pipeline when the trap fires are zero words from
the synthetic area. They are discarded: the return refills the pipeline from the return address
with the state the restored CPSR names, exactly as a branch would (`arm-branch-pipeline.md`,
"Refill after an R15 write"). Because the trap is decided from R15 before the instruction at the
vector runs, the pipeline contents never matter to it, and a handler that was not entered by
the vector but by a jump to `base + 08h` is trapped the same way: the address decides.

**The game's own vectors.** The ARM9 takes the trap only inside `FFFF_0000h`-`FFFF_7FFFh`. A game
that leaves CP15 bit 13 clear and puts its handlers in the ITCM at `0000_0000h` never reaches
the BIOS vectors, and the BIOS is not involved: that is what the hardware does. The ARM7's
vectors are always in its BIOS, so its SWI and IRQ always go through the traps.

## SWI

**Finding the number.** A SWI instruction carries a comment field; the BIOS uses only its low
byte as the function number. In ARM state the instruction is at `LR - 4` and the number is bits
23-16 of the opcode, which is the byte at `LR - 2`; in Thumb state the instruction is at `LR - 2`
and the number is bits 7-0 of the halfword, again the byte at `LR - 2`. The BIOS reads it from
memory, not from a register, so the machine reads one byte at `LR - 2` through the CPU's own bus
view, as the BIOS's load would. `LR` is `R14_svc` (`arm-exceptions-modes.md`, "Return address"):
the address of the instruction after the SWI in either state.

**Returning.** The BIOS's SWI code saves a few registers, calls the routine, restores them and
returns with `MOVS PC, LR`: R15 becomes `LR`, and the CPSR is loaded from the SPSR, which
restores the mode, the flags, the interrupt masks and the T bit, so the caller continues in the
state it called from (ARM or Thumb). The machine does the same: the CPSR is set to `SPSR_svc`,
R15 to `R14_svc`, and the pipeline is refilled in the restored state. The supervisor stack is
not touched by the HLE.

**Unimplemented number.** A number no routine implements has no effect at all: nothing but
R15 and the CPSR changes, and the return is the one above. The machine keeps a record a debugger
can read: the last number seen and the last number that was not implemented, for each CPU. A
guest SWI never panics the core.

On hardware, GBATEK says the debug handler pointer (`027FFD9Ch` ARM9, `0380FFDCh` ARM7) is called
for the unused numbers `00h`-`1Fh`; no debug handler exists here (see "Unverified").

## Halt, and what wakes a CPU

A CPU can be halted: it executes nothing until an interrupt is pending. Three things halt it:

- the ARM9's CP15 wait-for-interrupt (`cp15.md`, "C7 — cache commands and halt");
- the ARM7's write to HALTCNT (`interrupts.md`, "HALTCNT"), mode 2;
- the BIOS function Halt (SWI `06h`), which on hardware does one of the two.

A halted CPU wakes when `IE & IF` is not zero: IME and the CPSR I flag do not matter for the wake
itself (GBATEK, "DS Interrupts": the halt lasts while `IE AND IF` is zero). After the wake the
CPU continues with the next instruction; if IME is 1 and I is clear the IRQ is then taken at once,
as for any pending request (`interrupts.md`, "The IRQ line"). In ninge a halted CPU's clock does
not run instructions: the machine moves it to the end of the time it is asked to run, in one
step, and the wake is tested at the start of the next round (the events that raise IF end a
round exactly at their own time, `scheduler.md`).

## The wait SWIs

| Number | Name | Arguments | Effect |
|---|---|---|---|
| `03h` | WaitByLoop | `R0` = loop count | a delay; no effect here beyond the entry cost (see "Unverified") |
| `04h` | IntrWait | `R0` = discard old flags, `R1` = flags to wait for | waits until one of the flags is set (below) |
| `05h` | VBlankIntrWait | none | IntrWait with `R0 = 1`, `R1 = 1` (the V-blank bit) |
| `06h` | Halt | none | halts the CPU (above) and returns at once; execution resumes after the wake |

**IntrWait.** GBATEK describes the function: it waits in the halt state until one of the interrupts
named in `R1` has happened, and it forces IME to 1. How it knows that an interrupt has happened is
the IRQ check flags (the word at DTCM base + `3FF8h` on the ARM9, `0380FFF8h` on the ARM7, in the
IE/IF format; `interrupts.md`): the user's IRQ handler ORs the bit of each interrupt it serves
into that word, and IntrWait consumes it. IntrWait does not set IE, does not acknowledge IF and
does not set the flags itself.

In ninge IntrWait is a small state machine whose state is the CPU's own: its position is the
IntrWait loop address (`+24h`, trap `+2Ch`), so no extra state is saved.

1. The SWI trap (`04h` or `05h`; `05h` first sets `R0 = 1`, `R1 = 1`) writes IME = 1, and if
   `R0` is not zero clears the bits of `R1` in the flag word (the old flags are discarded). It
   clears the CPSR I flag (the BIOS lets interrupts in while it waits) and jumps to the IntrWait
   loop address in SVC mode, leaving `LR_svc` and `SPSR_svc` as the SWI entry made them.
2. At the loop trap: read the flag word `w`. Write back `w` with the bits of `R1` cleared. If
   any bit of `R1` was set in `w`, return to the caller as the SWI return does
   (`CPSR = SPSR_svc`, `PC = LR_svc`). Otherwise halt the CPU and stay at the loop address.
3. When the CPU wakes, an interrupt is pending and enabled in IE; with I clear and IME = 1 it is
   taken first: the IRQ dispatcher and the user handler run, and the dispatcher's return goes to
   the loop address (`LR_irq - 4`, the address the interrupt came from). The loop trap then runs
   again from step 2, which now sees the bit the handler set.

`R1` survives the IRQ because the dispatcher saves and restores `R0-R3`.

## The IRQ dispatcher

Hardware: the BIOS IRQ code saves `R0-R3`, `R12` and `LR` on the IRQ-mode stack, loads the
address of the user handler from a fixed place, calls it with the return address inside the BIOS,
and when the handler returns restores the saved registers and returns from the interrupt with
`SUBS PC, LR, #4`.

| CPU | Handler pointer | IRQ check flags (IntrWait) |
|---|---|---|
| ARM9 | DTCM base + `3FFCh` (the DTCM base is CP15 C9,C1,0, `cp15.md`) | DTCM base + `3FF8h` |
| ARM7 | `0380_FFFCh` | `0380_FFF8h` |

The ARM7 handler must be ARM code. The ARM9 handler may be ARM or Thumb, by bit 0 of the pointer.

The machine does at the IRQ trap, in order:

1. Pushes `R0`, `R1`, `R2`, `R3`, `R12`, `LR` (`R14_irq`) on the IRQ stack, full descending:
   `SP_irq` is decremented by 24 and the registers are stored at increasing addresses from
   there, lowest register first.
2. Reads the handler pointer (a 32-bit read through the CPU's bus view, so the ARM9 reaches its
   DTCM).
3. Sets `R0 = 0400_0000h` (GBATEK: the BIOS builds the address of the I/O area in `R0` to reach
   the pointer; the user handler may use it) and `LR` to the IRQ return address `base + 20h`.
4. Jumps to the pointer: on the ARM9 bit 0 selects Thumb and is dropped from the address; on the
   ARM7 the CPU stays in ARM state. The CPSR is the IRQ-mode entry value, so the handler runs
   in IRQ mode with I set. The pipeline is refilled from the pointer.

The handler returns with `BX LR` (or any jump to its `LR`). At the IRQ-return trap the machine:

1. Pops the six registers from `SP_irq` (the reverse of the push) and adds 24 to it.
2. Returns as `SUBS PC, LR, #4` does: R15 = restored `LR - 4`, CPSR = `SPSR_irq`, refilled in
   the restored state.

`LR` as the CPU wrote it on entry is the next instruction's address + 4 in ARM state and the next
instruction's address + 4 in Thumb state as well (`arm-exceptions-modes.md`, "Return address"),
so the same `-4` returns to the interrupted instruction in both states.

Acknowledging IF is the handler's job (`interrupts.md`); the dispatcher never touches IF, IE or
IME, and does not set the IRQ check flags: GBATEK says the user handler should do it so that
IntrWait works.

## Cost

The HLE routines take no real cycles. Each BIOS entry (the SWI vector, the IRQ vector, the IRQ
return) costs one instruction of the CPU that took it (ARM9 1 master cycle, ARM7 2;
`scheduler.md`), the same as the exception entry itself costs (`interrupts.md`, "Behavior"). This
is a model, not a measurement (see "Unverified"); it also keeps a step from costing more than one
instruction, which the savestate's check of the ARM7 clock relies on (`scheduler.md`, "Savestate
checks").

## Edge cases

- A SWI number that no routine implements: no effect, recorded, never a panic.
- A SWI in Thumb state and in ARM state read the same byte; the CPU state the SPSR names decides
  how the caller is resumed, not how the number is found.
- An IRQ whose handler pointer is zero or points at unmapped memory: the machine jumps there. The
  CPU then executes what is there (zero words), as the hardware's `LDR PC` would; the core never
  faults.
- A handler that returns with a different `SP_irq` than it was given leaves the registers popped
  from the wrong place, as on hardware.
- ARM9 with the vectors at `0000_0000h`: the BIOS is not involved (above).
- SWI from IRQ or SVC mode: the CPU enters SVC mode with `LR_svc` and `SPSR_svc` set from that
  mode; the return restores it. No special case.

## Unverified

- **The IRQ return address `base + 20h`.** GBATEK does not give the DS BIOS's own address for the
  user handler's return. — `hle::IRQ_RETURN_VECTOR` in `crates/core/src/hle/` — it matters only to
  a handler that reads `LR` for itself; verify by a test ROM on hardware that prints `LR` in
  the handler.
- **`R0 = 0400_0000h` at the user handler.** GBATEK's GBA text builds it in `R0` and
  jumps through `[R0 - 4]`; that the DS BIOS does the same, in both CPUs, is carried over. —
  `hle::irq_enter` — verify likewise (print `R0` at entry).
- **ARM7 `LDR PC` with bit 0 set.** The ARM7TDMI ignores the interworking bit of a loaded PC;
  the pointer is documented as ARM code only, so the pointer's bit 0 is dropped on the ARM7. —
  `hle::irq_enter` — verify likewise.
- **The registers the BIOS saves.** `R0-R3`, `R12`, `LR` are GBATEK's GBA text; the DS BIOS
  is assumed to do the same. — `hle::irq_enter`, `hle::irq_return` — verify by a handler that
  inspects the stack.
- **ARM9 handler pointer when the DTCM is disabled.** The address is the DTCM base register + `3FFCh`
  whether or not the DTCM is enabled; the read then goes to whatever memory is at that address. —
  `hle::irq_enter` — verify by a test ROM on hardware.
- **Reading the BIOS.** On the ARM7 the BIOS is protected from reads outside it (GBATEK's
  BIOSPROT) and a read returns something else; here it reads zero for both CPUs. — `Region::Bios`
  in `crates/core/src/bus/memory/mod.rs` — verify likewise.
- **Debug handler.** GBATEK says the unused numbers `00h`-`1Fh` call the debug vector; none is
  called here. — `hle::swi` — verify likewise.
- **SWI numbers above `1Fh`.** What the BIOS does for them is not stated; they have no effect. —
  `hle::swi::run` (`TODO(hw-unverified)` on its catch-all) — verify likewise.
- **SWI comment field and DS function numbers.** The comment-field position (ARM bits 23-16,
  Thumb bits 7-0) and the numbering of the DS BIOS functions are recalled, not read. —
  `hle::bios::software_interrupt` and `hle::swi` constants — verify against GBATEK's pages.
- **The SWI number is read with a normal load.** If `LR - 2` points at an I/O register the read has
  its side effect, as the BIOS's own load would; the model does not use a side-effect-free peek.
  — `hle::bios::software_interrupt` — verify likewise.
- **IME and the CPSR I flag in the wake.** The wake depends only on `IE & IF` (GBATEK's Halt
  wording); for the ARM9's CP15 wait-for-interrupt GBATEK does not say whether the core's own IRQ
  input (which includes IME) is what wakes it. — `InterruptController::wakes` in
  `crates/core/src/io/irq/mod.rs` — verify by a test ROM on hardware that halts with IME = 0.
- **IntrWait details.** That a hit clears the bits of `R1` from the flag word, that the flag word
  is rewritten even without a hit, that IME is left at 1, that the CPSR I flag is cleared during
  the wait and the caller's CPSR comes back through `SPSR_svc` are taken from how the BIOS and
  the GBA text describe the function; whether a wait that is already satisfied halts at all is
  not stated. — `hle::bios::intr_wait_loop` — verify likewise.
- **WaitByLoop.** The delay is not modeled: the SWI costs one instruction. A delay would stall the
  CPU's clock past events, which the scheduler does not support (`scheduler.md`). —
  `hle::swi::run` — `TODO(hw-unverified)`; revisit with the Phase 8 timing work.
- **The IntrWait loop address `base + 24h`.** ninge's own. — `hle::bios::INTR_WAIT_VECTOR` —
  invisible to a game unless it reads `LR` inside IntrWait.
- **Cost.** One instruction per BIOS entry is a model; the real BIOS runs tens of instructions per
  call. — `hle::ENTRY_INSTRUCTIONS` in `crates/core/src/hle/bios.rs` — revisit with the Phase 8
  timing work.
- **The reset, undefined, abort and FIQ vectors** execute zero words. Their BIOS behavior is not
  modeled; no phase needs it. — `crates/core/src/hle/` — `TODO(phase8)`.
