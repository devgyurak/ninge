# HLE BIOS: the BIOS area, the vectors, SWI entry and the IRQ dispatcher

Sources: GBATEK "DS Memory Maps" (the NDS9 and NDS7 BIOS rows and the BIOS ROM sizes) and GBATEK "DS
Interrupts" (the handler pointer and IRQ check flag addresses), both read; GBATEK "BIOS Interrupt
Handling" for the GBA (the register set the BIOS saves around the user handler and the
`SUBS PC, LR, #4` return), read for the GBA only; GBATEK "BIOS Misc Functions" (WaitByLoop: the
loop it runs and the counts that make one millisecond; GetSineTable, GetPitchTable and
GetVolumeTable: the index and entry ranges, what the sine table holds, and that an index outside
the range returns garbage), read; GBATEK "BIOS Function Summary" (the rows of the three table
numbers `1Ah`-`1Ch` and the CPUs that have them), read. Two more are recalled from memory of
GBATEK, not read, so each is under "Unverified": the SWI comment field ("ARM Opcodes ... SWI",
"THUMB Opcodes ... SWI") and the other DS function numbers ("BIOS Function Summary"). Also
`arm-exceptions-modes.md` for the exception entry.
Used by: `crates/core/src/hle/`, `crates/core/src/bus/memory/mod.rs`,
`crates/core/src/machine/mod.rs`   Phase: 3 (#27, parts 1 to 3 of 3); SoundBias: 5 (#42); the
sound tables: 6

ninge boots without a BIOS dump (AGENTS.md §3.2). The BIOS is high-level emulated: the CPU
enters its exception vectors exactly as on hardware, and the machine does in Rust what the BIOS
code at those addresses would do. No BIOS image exists in ninge, and no instruction of it is
written down anywhere. This note covers the BIOS area, the vectors, how an SWI number is found,
the IRQ dispatcher, the wait routines (Halt, WaitByLoop, IntrWait, VBlankIntrWait), the copy,
math, checksum and decompression routines, SoundBias, the three sound tables, and the SWI numbers
nothing implements.

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
table"). Of them the machine looks at two, plus six addresses of its own:

| Offset | Name | Taken by |
|---|---|---|
| `+08h` | SWI vector | the SWI instruction |
| `+18h` | IRQ vector | an interrupt (`interrupts.md`) |
| `+20h` | IRQ return | ninge's own address (below); the user handler returns to it |
| `+24h` | IntrWait loop | ninge's own address (below); IntrWait waits here |
| `+28h`-`+34h` | WaitByLoop loop | ninge's own four addresses (below); the delay is spent here |

`+20h`, `+24h` and `+28h`-`+34h` are not hardware vectors. On hardware the BIOS IRQ code gives the
user handler a return address inside the BIOS, and GBATEK does not state the DS address; ninge
picks `+20h` (an address past the FIQ vector that nothing uses) and says so in "Unverified". `+24h`
is the place where the IntrWait code of this model loops, and `+28h`, `+2Ch`, `+30h`, `+34h` are
the four steps of one pass of the WaitByLoop loop ("The wait SWIs"). The other vectors (reset,
undefined, prefetch and data abort, FIQ) are not handled: the CPU enters them and executes the zero
words of the synthetic area.

## How the machine takes the BIOS's place

The CPU enters the exception normally: the mode and the banked registers change, `R14_<mode>`
and `SPSR_<mode>` are written, I is set, T is cleared, and R15 becomes the vector
(`arm-exceptions-modes.md`). The pipeline is refilled from the vector with two fetches of the
synthetic area.

Before it executes an instruction, the machine compares R15 with the trap addresses. R15 reads
the address of the next instruction + 8 in ARM state (`arm-branch-pipeline.md`); a refill from
the vector `V` leaves R15 = `V + 8`, so the trap addresses in R15 terms are base + `10h` (SWI),
base + `20h` (IRQ), base + `28h` (IRQ return), base + `2Ch` (IntrWait loop) and base + `30h`,
`34h`, `38h`, `3Ch` (the WaitByLoop loop). When R15 is one of them, T is clear and the CPU
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
round exactly at their own time, `scheduler.md`). A halted ARM9 skips to the earlier of the round
target and the next queued event, so an event its last instruction queued (a timer started just
before the wait) still fires on time.

A DMA unit can write HALTCNT as well (it is an ordinary ARM7-side write). The ARM7 halts when the
transfer ends, not after its next instruction, because a DMA unit runs in the gap between two
instructions; this is the model's order, as GBATEK does not describe it (see "Unverified").

## The wait SWIs

| Number | Name | Arguments | Effect |
|---|---|---|---|
| `03h` | WaitByLoop | `R0` = loop count | a delay of four of the CPU's cycles per count (below) |
| `04h` | IntrWait | `R0` = discard old flags, `R1` = flags to wait for | waits until one of the flags is set (below) |
| `05h` | VBlankIntrWait | none | IntrWait with `R0 = 1`, `R1 = 1` (the V-blank bit) |
| `06h` | Halt | none | halts the CPU (above) and returns at once; execution resumes after the wake |

**WaitByLoop.** GBATEK gives the function as a two-instruction loop inside the BIOS (a subtraction
of 1 from `R0`, then a branch back on "greater than"), run from BIOS memory so that its timing does
not depend on where the caller's code is. The branch tests the flags the subtraction left
(`arm-data-processing.md`), so it is taken while the count before the subtraction was greater than
1 as a signed number: a count of 1 makes one pass, and so does a count of zero or less
(`8000_0000h` included, whose subtraction overflows). GBATEK asks for a count of 1 to `7FFF_FFFFh`,
so the counts outside that are under "Unverified". GBATEK lists the counts that make one
millisecond: `20BAh` (8,378) on the ARM7 and twice that (`4174h`, 16,756) on the ARM9 with the cache
on. A millisecond is 33,514 ARM7 cycles and 67,028 ARM9 cycles, so one pass is **four cycles of the
calling CPU** in both cases. Without the cache GBATEK's ARM9 count is `105Dh`, sixteen cycles a
pass; ninge models no cache and runs the ARM9 at its full clock everywhere (`scheduler.md`, "Cost
per instruction"), so it uses the cached figure (see "Unverified"). GBATEK names no return value.

A program can depend on the delay: one CPU writes a value the other is to answer, waits by loop,
and then reads the answer once. With no delay the answer cannot arrive in time, whatever the other
CPU does.

In ninge WaitByLoop is a small state machine whose state is the CPU's own, like IntrWait: `R0` is
the count and R15 is the position inside a pass, so no extra state is saved.

1. The SWI trap (`03h`) jumps to the first loop address (`+28h`) in SVC mode, leaving `LR_svc`,
   `SPSR_svc` and the CPSR as the SWI entry made them (I set).
2. The traps at `+28h`, `+2Ch` and `+30h` each only move on to the next address.
3. The trap at `+34h` subtracts 1 from `R0`. If `R0` was greater than 1 as a signed number before
   that, it goes back to `+28h`; otherwise it returns to the caller as the SWI return does.

Each trap costs one instruction (see "Cost"), which is one cycle of the CPU in this model, so a
call with count `n` takes the entry plus `4n` instructions (four for `n` of zero or less) and
leaves `R0` at zero (at `n - 1`, wrapped, for `n` of zero or less). An interrupt that becomes
pending during the wait is taken after the return, because I stays set (see "Unverified").

**IntrWait.** This description is recalled from GBATEK's text (and from how the GBA text
describes the same function), not read from the page; "Unverified" lists what is least certain.
GBATEK describes the function: it waits in the halt state until one of the interrupts
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

## The copy, math and decompression SWIs

Chosen from what #27 lists (Div, CpuSet, CpuFastSet, Sqrt, LZ77 in its two forms) and from a
survey made for this note: a libnds console `printf` program built with the BlocksDS v1.24.0 image
that `scripts/build_testroms.sh` pins, its ARM9 ELF and the image's stock ARM7 ELF disassembled
for `swi`/`svc`. The ARM9 calls `0Fh` and `03h`; the ARM7 calls `03h`, `06h`, `07h`, `08h` and
`0Eh`, and no number above `1Fh`. So GetCRC16 and IsDebugger are added. Every one runs
through the calling CPU's own bus view, so a write reaches I/O registers and VRAM as an instruction's
would, and costs one instruction (see "Cost").

| Number | Name | Arguments | Result |
|---|---|---|---|
| `08h` | SoundBias | `R0` level (0 or not), `R1` delay count | SOUNDBIAS |
| `09h` | Div | `R0` numerator, `R1` denominator, both signed | `R0` quotient, `R1` remainder, `R3` absolute quotient |
| `0Bh` | CpuSet | `R0` source, `R1` destination, `R2` control | memory |
| `0Ch` | CpuFastSet | same | memory |
| `0Dh` | Sqrt | `R0` unsigned | `R0` integer square root |
| `0Eh` | GetCRC16 | `R0` initial value, `R1` address, `R2` length in bytes | `R0` the CRC |
| `0Fh` | IsDebugger | none | `R0 = 0` |
| `11h` | LZ77 to 8-bit writes | `R0` source, `R1` destination | memory |
| `12h` | LZ77 to 16-bit writes | same | memory |

**Div.** The quotient is the numerator divided by the denominator, truncated toward zero; the
remainder has the sign of the numerator; `R3` is the absolute value of the quotient as an
unsigned number. A zero denominator and `-80000000h / -1` are under "Unverified".

**Sqrt.** The largest `r` with `r * r <= R0`, at most `FFFFh`.

**CpuSet.** `R2` bits 0-20 hold the unit count, bit 24 selects fill (1) or copy (0), bit 26 selects
32-bit (1) or 16-bit (0) units; the other bits are ignored. Copy moves `count` units from the source
to the destination, in increasing order. Fill reads one unit at the source and writes it `count`
times. The addresses are used with their low bits dropped to the unit size (the bus ignores them,
`arm9-memory-map.md`, "Alignment"). A count of 0 does nothing. The registers are left as they were.

**CpuFastSet.** The same, but always 32-bit units, and the count is rounded up to a multiple of 8
words: a count of 9 moves 16 words, a count of 1 fills 8.

**GetCRC16.** CRC-16/MODBUS (`cart-header.md`, "The header CRC16") continued from `R0`, over `R2`
bytes read as halfwords from `R1`: for each halfword the low byte, then the high byte. `R2` is
halved. Only `R0` changes. An odd length and the registers other than `R0` are under
"Unverified".

**SoundBias.** ARM7 only (GBATEK lists `08h` as SoundBias for the ARM7 and DSi7). `R0 = 0`
sets the bias in SOUNDBIAS bits 0-9 to `000h`, any other `R0` to `200h`; the bits above bit 9 are
kept (`sound-registers.md`). `R1` is the BIOS's delay between the steps it takes, and is ignored.
The write goes through the ARM7's bus view like an instruction's. The delay, the single steps and
the register's value in between are under "Unverified".

**IsDebugger.** Retail hardware has no debugger: `R0 = 0`.

**LZ77.** The source starts with a word: bits 4-7 are the type (1 for LZ77) and bits 8-31 the
decompressed size in bytes; the data follows. Data is read in groups: a flag byte whose bits are
used from bit 7 down to bit 0, each followed by one item. A 0 bit is a literal byte. A 1 bit is a
block of two bytes `b0`, `b1`: the length is `(b0 >> 4) + 3` and the distance is
`((b0 & 0Fh) << 8 | b1) + 1`; the length bytes are copied from `distance` bytes behind the output
position, one byte at a time, so a block may overlap the bytes it is writing (distance 1 repeats
the last byte). Decoding stops when the output reaches the size; a block that would pass the
size is cut at it. A size of 0 does nothing. A distance reaching before the start of the output
reads the memory there, as the BIOS would.

- `11h` writes each output byte with a byte write (the "WRAM" form): to VRAM or palette memory
  the byte is lost (`vram-banks.md`, "Behavior").
- `12h` collects output bytes in pairs and writes each pair with one halfword write (the "VRAM"
  form), low byte first. A block copy reads bytes already collected in the pair or already
  written. A size that is odd leaves the last byte unwritten.

## The sound table SWIs

Three numbers of the ARM7 return one entry of a table that sits in the BIOS ROM (GBATEK, "BIOS
Misc Functions"). A sound driver uses them to turn a note and a loudness into register values:
the pitch table for a channel's timer, the volume table for its volume, the sine table for a
vibrato.

| Number | Name | `R0` in | `R0` out |
|---|---|---|---|
| `1Ah` | GetSineTable | index `00h`-`3Fh` | an entry, `0000h`-`7FF5h` |
| `1Bh` | GetPitchTable | index `000h`-`2FFh` | an entry, `0000h`-`FF8Ah` |
| `1Ch` | GetVolumeTable | index `000h`-`2D3h` | an entry, `00h`-`7Fh` |

Only `R0` changes, and the cost is the one instruction of any SWI here (see "Cost"). The ARM9 has
none of the three: GBATEK's function summary lists them for the ARM7 alone, so on the ARM9 they
take the "Unimplemented number" path.

GBATEK gives each table's index range and the range of its entries, and says what the sine table
holds. It does not list the entries. ninge has no BIOS to read them from (AGENTS.md §3.2), so the
entries are **computed**: each table from one formula, each entry rounded to the nearest whole
number. The formulas are below; how far they can be trusted is under "Unverified".

**The sine table** (64 entries) is a quarter of a sine wave: entry `i` is `sin(i * 90° / 64) *
7FFFh`. GBATEK calls it the sine of 0 to 88.6 degrees times `8000h`, with `7FF5h` the largest
entry. The last entry is the sine of 88.59°, 0.999699: times `7FFFh` that is 32757.1, which is
`7FF5h`; times `8000h` it would be `7FF6h`. So the scale here is `7FFFh`.

**The pitch table** (768 entries) splits an octave in 768 equal steps, 64 to a semitone: entry `i`
is `(2^(i/768) - 1) * 10000h`, the fraction by which a frequency `i` steps up exceeds the one it
started from. Entry 0 is 0. The last is 0.998196 times `10000h`, 65417.8, which rounds to `FF8Ah`,
GBATEK's largest entry (rounded down it would be `FF89h`): this is why entries are rounded to the
nearest and not down.

**The volume table** (724 entries) is taken to be in tenths of a decibel: the top entry, 723, is
the full level and every entry below it is a tenth of a decibel quieter, so the level of entry `i`
is `10^((i - 723) / 200)`. An entry is a value for the 7-bit volume of SOUNDxCNT, and at full level
it is `7Fh`. Seven bits alone cannot carry 72 dB: near the bottom the whole range would be a
handful of values. SOUNDxCNT has a volume divider beside the volume (/2, /4, /16), and the table is
taken to be made for use with it: an entry holds the level times the divider that goes with it,
and the divider changes where the level has fallen to a half, a quarter and a sixteenth, which is
60, 120 and 240 tenths of a decibel down (6.02 dB is a factor of two), at entries 663, 603 and 483:

```
k = 16 below 483, 4 for 483-602, 2 for 603-662, 1 from 663 up
entry[i] = min(7Fh, 127 * 10^((i - 723) / 200) * k)
```

So the entries climb four times: from 0 to 127 over entries 0 to 482, from 32 to 126 over 483 to
602, from 64 to 126 over 603 to 662, and from 64 to 127 over 663 to 723. The level a program gets,
the entry over the divider, never falls as the index rises.

**An index outside a table's range.** GBATEK says the result is garbage. Here it is 0.

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

The machine tests for an interrupt before it tests for a BIOS trap, in every step: at a vector I
is set, so the trap runs; at the IntrWait loop address I is clear, so a pending interrupt is
taken first and its return lands on the loop address again (`The wait SWIs`, step 3). Reversing
the order would halt again with the IRQ line still asserted.

Acknowledging IF is the handler's job (`interrupts.md`); the dispatcher never touches IF, IE or
IME, and does not set the IRQ check flags: GBATEK says the user handler should do it so that
IntrWait works.

## Cost

The HLE routines take no real cycles, except WaitByLoop, whose delay is what it is for ("The wait
SWIs"). Each BIOS entry (the SWI vector, the IRQ vector, the IRQ return, each address of the
IntrWait and WaitByLoop loops) costs one instruction of the CPU that took it (ARM9 1 master cycle,
ARM7 2; `scheduler.md`), the same as the exception entry itself costs (`interrupts.md`, "Behavior").
This is a model, not a measurement (see "Unverified"); it also keeps a step from costing more than
one instruction, which the savestate's check of the ARM7 clock relies on (`scheduler.md`, "Savestate
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

- **The entries of the three sound tables.** They are computed, not read, and no entry has been
  compared with a BIOS. What is known is what GBATEK gives: the sizes and the smallest and largest
  entry of each table, which the formulas meet (`0000h` and `7FF5h`, `0000h` and `FF8Ah`, `00h` and
  `7Fh`). An entry in between can differ from the real one by the rounding, or by more where a
  formula is wrong. — `hle::swi::sound` — verify by a test ROM on hardware that holds the
  formulas itself, asks for every index, and reports only whether each answer agrees (or the
  first index that does not). What the hardware answers is BIOS data: it is never written down,
  in the test ROM's source, a fixture, an issue or a PR (AGENTS.md §3.1, §3.2).
- **The formulas of the pitch and volume tables.** GBATEK describes the sine table; for the other
  two it gives only the ranges. That the pitch table is `2^(i/768) - 1` is inferred from its size
  (64 steps to each of 12 semitones) and its largest entry. That the volume table is in tenths of a
  decibel, and holds the level times a divider that changes at a half, a quarter and a sixteenth,
  is inferred from its size, its range and the divider SOUNDxCNT has. One observation is
  consistent with it and is not its source: in a local run a program wrote the divider /2 beside
  the entries 603 to 662, /4 beside 483 to 602 and /16 below 483. That is an observation of a
  program, not of the BIOS. — `hle::swi::sound`.
- **The scale of the sine table**, `7FFFh` where GBATEK writes `8000h`, chosen to meet its largest
  entry. — `hle::swi::sound`.
- **An index outside a table's range** returns 0 here. GBATEK calls what the BIOS returns
  garbage. A program that relies on it is not expected. — `hle::swi::sound`.
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
- **A DMA write to HALTCNT.** That the ARM7 halts once the transfer ends is the model's order, not
  from GBATEK. — `Machine::run_dma` in `crates/core/src/machine/mod.rs` (`take_halt`) — verify
  by a test ROM on hardware that DMAs a halt byte.
- **Interrupt before BIOS trap.** A pending, unmasked IRQ is taken before the BIOS trap at a
  vector address is looked at; hardware has no such ordering to copy (the BIOS is code), so this
  is the model's choice. — `Machine::step_arm9_instruction` — invisible unless an IRQ arrives
  while a CPU stands on a vector.
- **IME and the CPSR I flag in the wake.** The wake depends only on `IE & IF` (GBATEK's Halt
  wording); for the ARM9's CP15 wait-for-interrupt GBATEK does not say whether the core's own IRQ
  input (which includes IME) is what wakes it. — `InterruptController::wakes` in
  `crates/core/src/io/irq/mod.rs` — verify by a test ROM on hardware that halts with IME = 0.
- **IntrWait details.** That a hit clears the bits of `R1` from the flag word, that the flag word
  is rewritten even without a hit, that IME is left at 1, that the CPSR I flag is cleared during
  the wait and the caller's CPSR comes back through `SPSR_svc` are taken from how the BIOS and
  the GBA text describe the function; whether a wait that is already satisfied halts at all is
  not stated. — `hle::bios::intr_wait_loop` — verify likewise.
- **WaitByLoop.** Four cycles a pass is GBATEK's figure for the ARM7 and for the ARM9 with the cache
  on; the ARM9 without the cache (sixteen cycles a pass by GBATEK's table) is not modeled, as the
  model has no cache. That `R0` is left at zero, that a count of zero or less (outside the range
  GBATEK asks for) makes one pass, and that the other registers are unchanged follow from the loop
  as GBATEK writes it, not from a measurement. Whether the BIOS lets interrupts in during the delay
  is not stated (GBATEK, "How BIOS Processes SWIs", says only that it may in some cases); the model
  keeps I set, so an interrupt waits for the return. — `hle::bios::wait_loop` —
  `TODO(hw-unverified)`; verify by a test ROM on hardware timing the call against a timer, and one
  taking a timer interrupt inside it.
- **The IntrWait loop address `base + 24h` and the WaitByLoop loop addresses `base + 28h`-`34h`.**
  ninge's own. — `hle::bios::INTR_WAIT_VECTOR`, `hle::bios::WAIT_LOOP_VECTOR` —
  invisible to a game unless it reads `LR` inside IntrWait, or takes an interrupt inside the
  model's WaitByLoop (it cannot: I is set).
- **Div by zero and `-80000000h / -1`.** The model
  returns, for a zero denominator, `R0 = 1` (`-1` for a negative numerator), `R1` = the numerator,
  `R3 = 1`, and for the overflow case the wrapped result (`R0 = 80000000h`, `R1 = 0`,
  `R3 = 80000000h`); both are choices, not findings. — `hle::swi::math::divide` — verify by a
  test ROM on hardware.
- **CpuSet and CpuFastSet details.** That a count of 0 does nothing, that `R2` bits other than 0-20,
  24 and 26 are ignored, and that CpuFastSet rounds its count up to a multiple of 8 words are the
  model's (GBATEK may state the multiple of 8 as a requirement on the caller instead of a
  round-up). Misaligned addresses are aligned down by the bus; the registers after the call are
  left unchanged although the BIOS leaves them undefined; the source of a fill at an unaligned
  address is aligned down. — `hle::swi::copy` — verify likewise.
- **GetCRC16.** `R2` bytes are taken as `R2 / 2` halfwords, so an odd length leaves its last byte
  out; only `R0` is set and `R1`-`R3` are left as they were. — `hle::swi::misc` — verify
  likewise.
- **LZ77.** A type other than 1 is decoded anyway; the 16-bit form's distance-1 block and an odd
  size follow the model above; on the DS GBATEK describes `12h` as reading through callbacks given
  in `R3` (`LZ77UnCompReadByCallbackWrite16bit`) and the model reads memory directly and ignores
  `R3`, as it also treats `11h`. — `hle::swi::lz77` — verify likewise.
- **SoundBias.** The BIOS steps SOUNDBIAS toward the target with a delay between steps (`R1`
  counts it); the model writes the target at once and has no delay. Which `R1`
  values are valid, and what the BIOS does on a target equal to the current level, are not
  described by GBATEK. — `hle::swi` — `TODO(hw-unverified)`; verify by a test ROM reading
  SOUNDBIAS from a timer interrupt during the call.
- **Not implemented.** Sleep (`07h`, `TODO(phase5)`), BitUnPack,
  Huffman and run-length decompression (`10h`, `13h`-`15h`), the DSi-only numbers, and the other
  GBA-compatible numbers take the "Unimplemented number" path.
- **The hardware divider and square root** (`DIVCNT`, `SQRTCNT` at `04000280h`/`040002B0h`, ARM9)
  are not modeled; nothing in the console survey reads them.
- **Cost.** One instruction per BIOS entry is a model; the real BIOS runs tens of instructions per
  call. — `hle::ENTRY_INSTRUCTIONS` in `crates/core/src/hle/bios.rs` — revisit with the Phase 8
  timing work.
- **The reset, undefined, abort and FIQ vectors** execute zero words. Their BIOS behavior is not
  modeled; no phase needs it. — `crates/core/src/hle/` — `TODO(phase8)`.
