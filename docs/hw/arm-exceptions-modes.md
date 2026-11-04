# ARM exceptions, mode entry, and the vector base

Sources: GBATEK "ARM CPU Exceptions"; GBATEK "ARM Opcodes: Branch and Branch with Link (B, BL,
BX, BLX, SWI, BKPT)" (the SWI and undefined-instruction parts); GBATEK "ARM Opcodes: Coprocessor
(MRC/MCR, LDC/STC, CDP, MCRR/MRRC)"; GBATEK "ARM CP15 Control Register" (bit 13).
Used by: `crates/core/src/cpu/exception.rs`, `crates/core/src/cpu/decode.rs`,
`crates/core/src/cpu/arm.rs`   Phase: 1 (#12)

The banked registers, the SPSRs, and the CPSR mode bits are in `cpu-registers.md`; the
instructions that return from an exception (`MOVS PC,LR`, `SUBS PC,LR,#4`, `LDM {..,PC}^`) are
in `arm-data-processing.md` and `arm-load-store.md`. This note covers how an exception is
entered. Items marked "(suite)" are measured on the pinned ARM7TDMI JSON suite (MIT license),
which is oracle evidence for the ARM7 only, not a hardware statement; they are listed again
under "Unverified" where GBATEK is silent.

## Registers

The exception machinery adds no register of its own. It reads and writes the CPSR bits below
and the banked R14 and SPSR of the mode it enters.

| CPSR bit | Name | Role in an exception |
|---|---|---|
| 7 | I | set on every exception entry |
| 6 | F | set on Reset and FIQ entry only; otherwise left as it was |
| 5 | T | cleared: every exception is handled in ARM state |
| 4–0 | M | set to the mode the exception enters |

### Vector table

The vector base is `00000000h` unless the high vectors are selected (see "Vector base").

| Offset | Exception | Mode entered | Priority (1 = highest) |
|---|---|---|---|
| +00h | Reset | Supervisor | 1 |
| +04h | Undefined instruction | Undefined | 7 |
| +08h | Software interrupt (SWI) | Supervisor | 6 |
| +0Ch | Prefetch abort | Abort | 5 |
| +10h | Data abort | Abort | 2 |
| +14h | "Address exceeds 26 bits" | Supervisor | not given |
| +18h | IRQ | IRQ | 4 |
| +1Ch | FIQ | FIQ | 3 |

The +14h vector belongs to the old 26-bit address space, which is not modeled
(`cpu-registers.md`); nothing raises it.

## Behavior

### Entry

Taking any exception, the CPU does these things:

1. The new mode's R14 receives the return address (see "Return address").
2. The new mode's SPSR receives the CPSR as it was before the exception.
3. The CPSR changes: the mode bits become the new mode, T becomes 0 (ARM state), I becomes 1,
   and F becomes 1 for Reset and FIQ only. N, Z, C, V and the reserved bits keep their values.
4. R15 becomes the vector address, and the pipeline refills from it
   (`arm-branch-pipeline.md`): two word fetches at the vector and the vector + 4, R15 =
   vector + 8.

The registers of the mode that was left are not touched; they come back when a return
instruction restores the CPSR. A second exception that enters the same mode overwrites that
mode's R14 and SPSR, so a handler that can be interrupted by one saves them first.

### Return address

GBATEK gives the return address through the instruction each handler returns with. R14 is
therefore defined relative to the interrupted instruction:

| Exception | Handler returns with | R14 holds |
|---|---|---|
| SWI | `MOVS PC,R14` | the address of the instruction after the SWI |
| Undefined | `MOVS PC,R14` | the address of the instruction after the undefined one |
| Prefetch abort | `SUBS PC,R14,#4` | the aborted instruction's address + 4 |
| Data abort | `SUBS PC,R14,#8` | the aborted instruction's address + 8 |
| IRQ | `SUBS PC,R14,#4` | the address of the instruction that would have run next, + 4 |

For an instruction that is executing, R15 reads its address + 8 in ARM state
(`arm-branch-pipeline.md`), and between two instructions R15 reads the next instruction's
address + 8. In terms of that R15 value, R14 is:

| Exception | ARM state | Thumb state (R15 = address + 4) |
|---|---|---|
| SWI, Undefined | R15 − 4 | R15 − 2 |
| Prefetch abort | R15 − 4 | R15 |
| Data abort | R15 | R15 + 4 |
| IRQ, FIQ | R15 − 4 | R15 |

GBATEK says the same return instructions serve both states. The Thumb column is that
statement worked out for each row; it is not stated per row (see "Unverified"). FIQ has no
return instruction in GBATEK; it is given the IRQ value.

### SWI

Bits 27–24 are `1111`; bits 23–0 are a comment the processor ignores (a handler can read it
from the opcode at R14 − 4). With the condition true, the CPU enters Supervisor mode through
vector +08h. With the condition false it is an ordinary skipped instruction.

### Undefined instruction

Bits 27–25 = `011` with bit 4 = 1 is the architecturally undefined space: the other bits are
reserved. GBATEK says the encoding `cond 0111 1111 .... .... .... 1111 ....` is set aside for
user use, which means it is guaranteed to be undefined. A word in this space, with its
condition true, enters Undefined mode through vector +04h.

GBATEK adds that not every unused-looking opcode traps (its example is a multiply with bit 6
set, which is accepted as a legal multiply). Only the space above is classified as undefined
here; the other unallocated encodings are not (see "Not modeled").

### Coprocessor instructions

A coprocessor instruction is CDP, MCR or MRC (bits 27–24 = `1110`, bit 4 selects MCR/MRC),
LDC or STC (bits 27–25 = `110`), or MCRR/MRRC (the `1100010x` corner of the LDC/STC space,
ARMv5TE). The coprocessor number is bits 11–8.

- **ARM7:** GBATEK says the coprocessor opcodes are irrelevant because no coprocessor exists
  (apart from a dummy CP14). It does not say what an absent coprocessor does. The suite shows
  that every CDP, MCR/MRC and LDC/STC whose condition is true, whatever the coprocessor number
  (0 to 15, CP14 included), enters Undefined mode through vector +04h, with the same R14,
  SPSR and flag effects as any undefined instruction. (suite)
- **ARM9:** there is a working CP15 (`ARM CP15 System Control Coprocessor`), implemented in
  #15. Until then an MCR or MRC to coprocessor 15 does nothing and the CPU steps to the next
  instruction. Every other coprocessor instruction (CDP, LDC and STC to any coprocessor,
  MCR/MRC to any other coprocessor) enters Undefined mode, as on ARM7. MCRR/MRRC are ARMv5TE
  instructions and stay unhandled until #14.

### Prefetch and data aborts

The entry is as described above (vector +0Ch and +10h, Abort mode). The DS has no MMU, the
protection unit is not modeled yet (#15), and BKPT, which raises a prefetch abort on ARMv5, is
#14, so nothing in the CPU raises an abort today. The entry exists and is tested; a caller
raises it while the aborting instruction is executing.

### IRQ and FIQ

The CPU takes an IRQ only when CPSR bit I is 0 and a FIQ only when bit F is 0. If both lines
are pending and both are unmasked, FIQ wins (priority 3 against 4). The entry happens between
two instructions. The interrupt controller that raises the lines (IE, IF, IME) is Phase 3.

### Reset

Reset enters Supervisor mode with I = 1, F = 1 and T = 0, and jumps to vector +00h. GBATEK does
not say what R14 and the SPSR hold afterwards, nor the other registers; they are left as they
were here.

### Vector base

- **ARM7:** the vectors are always at `00000000h`.
- **ARM9:** bit 13 of the CP15 control register selects the base: 0 = `00000000h`, 1 =
  `FFFF0000h`; the bit is read/write on the DS ARM9. Until #15 provides the control register,
  the CPU holds the bit as a plain flag that the host can set.

## Edge cases

- Entering an exception from User or System mode works the same way; User/System have no SPSR
  of their own to protect, the entered mode's SPSR receives the CPSR.
- An exception that enters the mode the CPU is already in (an IRQ inside an IRQ handler, once
  I is cleared) overwrites that mode's R14 and SPSR.
- An exception taken in Thumb state switches to ARM state; the return restores T from the SPSR.
- The registers R0–R7 are untouched by entry. R8–R12 change only through the bank switch.

## Unverified

- **Coprocessor instructions on ARM7 trap as undefined, for every coprocessor number.** GBATEK
  does not say what an absent coprocessor does; the suite shows it for all 16 numbers. —
  `Cpu::coprocessor` in `crates/core/src/cpu/exception.rs`, which carries
  `TODO(hw-unverified)` — verify by a test ROM on real hardware.
- **ARM9 coprocessor behavior other than MCR/MRC to CP15:** that CDP, LDC and STC to CP15, and
  every instruction to another coprocessor (CP14 included), enter Undefined is inferred from
  the ARM7 evidence. — `Cpu::coprocessor` — verify by a test ROM on real hardware.
- **Reset value of the ARM9 vector-base bit.** GBATEK does not give it. The flag starts at 0
  (low vectors). — `Cpu::new` in `crates/core/src/cpu/mod.rs` — verify by a test ROM on real
  hardware.
- **Thumb-state return values** in the table above are worked out from GBATEK's statement that
  one return sequence serves both states, not read from it. Thumb execution is #13. —
  `Exception::link_offset` in `crates/core/src/cpu/exception.rs` — verify by a test ROM on real
  hardware or a trace against the reference emulator once Thumb exists.
- **FIQ return value** equal to the IRQ one: GBATEK gives no return instruction for FIQ. —
  `Exception::link_offset` — verify as above.
- **Reset leaves R14 and the SPSR alone**, because GBATEK is silent. —
  `Exception::link_offset` and `Cpu::reset` in `crates/core/src/cpu/exception.rs`, which carry
  `TODO(hw-unverified)` — verify by a test ROM on real hardware.
- **Only the `011`/bit 4 space is undefined.** The other encodings that ARMv4T leaves
  unallocated, and the ARMv5TE encodings (CLZ, BLX, saturating arithmetic, LDRD and STRD, PLD,
  BKPT, MCRR/MRRC, and the `1111` condition) are not trapped; they stay unhandled until #14. —
  `Kind::Unhandled` in `crates/core/src/cpu/decode.rs` carries `TODO(phase1)`.
- **Suite-measured ARM7 facts.** For SWI and the undefined entry that the coprocessor files trigger, every case with a true
  condition shows: R14 of the entered mode = instruction address + 4, SPSR = the starting
  CPSR, T = 0, I = 1, F unchanged, vector fetches at the vector and the vector + 4 after the
  fetch at address + 8, R15 = vector + 8. They match GBATEK where it speaks (SWI: R14 =
  PC + 4); for the undefined entry, GBATEK's `MOVS PC,R14` return implies the same R14. The suite has no case
  for the bits 27–25 = `011` undefined space itself; that space rests on GBATEK and the decoder
  tests.
  (suite) — pinned in `crates/runner/tests/cpu_suite.rs`.
- **Abort and IRQ/FIQ entry has no suite evidence** (the suite has no cases for them); the
  tests derive the values from the table above.

## Not modeled

- The 26-bit address exception (vector +14h).
- The ARM946E-S's own behavior for an instruction fetched from a protected region (#15).
