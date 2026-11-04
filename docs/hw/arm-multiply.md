# ARM multiply and long multiply

Sources: GBATEK "ARM Opcodes: Multiply and Multiply-Accumulate (MUL, MLA)" (the same
section covers the long forms) is the primary source for the behavior; ARM ARM (ARMv5TE) "MUL",
"MLA", "UMULL", "UMLAL", "SMULL", "SMLAL" and ARM7TDMI TRM "Multiply instructions" are listed as
background; not re-read for this note. For the ARM7TDMI carry only: US 5,583,804 and
US 5,528,529 (see "ARM7TDMI multiply carry"). Measured facts come from the pinned SingleStepTests/ARM7TDMI suite (MIT).
Used by: `crates/core/src/cpu/decode.rs`, `crates/core/src/cpu/arm.rs` (handlers `multiply`,
`multiply_long`)   Phase: 1 (#9)

The v5TE DSP multiplies (SMULxy and friends, `armv5te.md`) are a different encoding space and are not
covered here. Registers and modes are in `cpu-registers.md`; the data-processing sibling is
`arm-data-processing.md`.

## Encoding

`cond 0000 L U A S Rd/RdHi Rn/RdLo Rs 1001 Rm`

| Bits | Meaning |
|---|---|
| 31-28 | condition |
| 27-24 | 0000 |
| 23 | L: 0 = 32-bit result (MUL, MLA), 1 = 64-bit result (long forms) |
| 22 | long forms: 0 = unsigned (UMULL, UMLAL), 1 = signed (SMULL, SMLAL) |
| 21 | A: 1 = accumulate (MLA, UMLAL, SMLAL) |
| 20 | S: set flags |
| 19-16 | Rd (MUL, MLA) or RdHi (long) |
| 15-12 | Rn, the MLA accumulator (MUL: should be 0000), or RdLo (long) |
| 11-8 | Rs, the multiplier operand |
| 7-4 | 1001 |
| 3-0 | Rm, the multiplicand operand |

Bits 23-21 give: 000 MUL, 001 MLA, 100 UMULL, 101 UMLAL, 110 SMULL, 111 SMLAL.
Bits 23-22 = 01 (patterns 010 and 011) is **not** a multiply on ARMv4T/ARMv5TE: GBATEK lists
010 as UMAAL, an ARMv6 instruction, and 011 as nothing. This feature defines no behavior for them; the decoder keeps them in the
unhandled class until a later issue covers them.

Other words with bits 7-4 = 1001 are not multiplies: SWP (bits 27-23 = 00010) and the
halfword transfers (bit 7 = 1, bit 4 = 1, bits 6-5 not 00) share the space. A multiply is
exactly bits 27-24 = 0000 together with bits 7-4 = 1001.

## Behavior

All forms read their operands first and write the destination(s) afterwards.

| Instruction | Result |
|---|---|
| MUL | Rd = low 32 bits of Rm × Rs |
| MLA | Rd = low 32 bits of Rm × Rs + Rn |
| UMULL | RdHi:RdLo = Rm × Rs, both operands unsigned |
| UMLAL | RdHi:RdLo = Rm × Rs + RdHi:RdLo (the old 64-bit pair), unsigned |
| SMULL | RdHi:RdLo = Rm × Rs, both operands signed |
| SMLAL | RdHi:RdLo = Rm × Rs + RdHi:RdLo, signed |

MUL and MLA keep only the low 32 bits, which are identical for signed and unsigned operands,
so one instruction serves both. The 64-bit sum of the accumulating long forms wraps modulo 2^64.
RdLo holds the low 32 bits of the 64-bit result, RdHi the high 32 bits.

### Flags (S = 1)

| Flag | MUL, MLA, and long forms |
|---|---|
| N | bit 31 of the 32-bit result, or bit 63 of the 64-bit result |
| Z | the whole result (32 or 64 bits) is zero |
| V | not affected (see Unverified for the ARMv4 long forms: GBATEK marks it "destroyed???") |
| C | **ARM9 (ARMv5TE): not affected.** **ARM7 (ARMv4T): destroyed**: a fixed function of the operands that this feature does not yet compute (it leaves C unchanged; see "ARM7TDMI multiply carry") |

Without S the CPSR is not changed. A failed condition makes the instruction a no-op that still
advances the PC.

### Cycle count (recorded, not implemented)

`m` is set by the multiplier operand Rs: 1 if bits 31-8 are all equal to the sign of the
class (all zero for UMULL and UMLAL; all zero or all one for MUL, MLA, SMULL, SMLAL), 2 if
bits 31-16 are, 3 if bits 31-24 are, otherwise 4. On the ARM7: MUL 1S + m I; MLA 1S + (m+1) I;
UMULL and SMULL 1S + (m+1) I; UMLAL and SMLAL 1S + (m+2) I. GBATEK gives no ARM9 timing for
these; the project's timing is approximate anyway. `m` is also the number of multiplier
cycles under early termination (1 to 4); MUL and MLA treat all-ones as terminal
like the signed long forms.

### Measured on the pinned suite

These were measured by the maintainer on `arm_mul_mla.json.bin` and `arm_mull_mlal.json.bin`
(50,000 cases each, about 25,000 pass their condition, no case makes a data access), not
taken from GBATEK. They are oracle evidence, not hardware evidence.

- Results match plain multiply and multiply-accumulate arithmetic, including Rd = Rm and
  RdHi or RdLo = Rm (the plain arithmetic result, computed from the operands as read before
  any write).
- An R15 operand (Rm, Rs, Rn, or RdHi/RdLo read as the accumulator) reads as the instruction
  address + 12, not + 8: a late R15 read (`arm-branch-pipeline.md`, "Late R15 reads").
- Rd = R15 (MUL, MLA), or RdHi or RdLo = R15 (long), flushes the pipeline: R15 ends as the
  value written + 8, the same as an ALU result written to R15. In a long multiply the other half is written
  normally.
- RdHi = RdLo: the Hi half is what remains in the register.
- N and Z come from the full result; V never changes; without S the CPSR never changes.
- With S, C changes in about half the cases (MUL, MLA: 6,443 of 12,715; long: 6,257 of
  12,637). The suite therefore models the ARM7TDMI multiplier carry, not "unchanged"; this feature
  does not reproduce it yet.

## ARM7TDMI multiply carry

ARMv4 documents C after a flag-setting multiply as meaningless, but the suite shows it is a
fixed function of the operands (see "Measured on the pinned suite"). An earlier revision of
this note pointed at one public article for the cause (recorded in #80). #81 forbids reusing it:
nothing below comes from it, and the session that wrote this section did not open it.

### Sources read for #81

Both are ARM patents with largely the same specification text, filed in the mid-1990s; read in full
on patents.google.com (specification and example tables, not the abstract):

- **US 5,583,804**, "Data processing using multiply-accumulate instructions" (Advanced RISC
  Machines Ltd). Called **P1** below.
- **US 5,528,529**, "Electronic multiplying and adding apparatus and method" (Advanced RISC
  Machines Ltd). Called **P2**. It repeats P1's example multiplier (its own five-cycle table
  and early-termination remark), and its claims cover the carry-save accumulate-bit technique.

Searches for a patent that states how a multiply sets a CPSR flag, or that describes the
early-terminated form, found none (only other assignees' patents, which are not used).

### What the sources settle

The example multiplier in P1 and P2 (P1 "An example multiplier will now be described" and the
per-cycle table that follows it; P2 the same table) computes multiplier x multiplicand
+ accumulate as follows. It is a description of one embodiment, not of a named ARM core.

- **Recoding.** Radix-4 modified Booth. The multiplier is extended to 33 bits, with a copy of
  its sign bit when signed and 0 when unsigned (P1: `R[32] = SIGNED AND MPLIER[31]`). Each
  digit reads three overlapping multiplier bits, `BoothEnc(R[2:0])`, `BoothEnc(R[4:2])`, and so on.
- **Rate.** Four multiplier rows work per cycle and each retires two result bits, so eight
  multiplier bits are consumed per cycle: Booth digits for `R[8:0]` in cycle 1 up to
  `R[32:24]` in cycle 4 (P1 table, "Booth encoders"). A 32 x 32 multiply takes five cycles; cycles
  1 to 4 consume the multiplier and cycle 5 finishes the sum.
- **Start.** The save vector starts as the accumulate value (zero without accumulate) and the
  carry vector as the bitwise inverse of the multiplicand when `R[0]` is 1, else zero (P1,
  "Carry-save initialiser"). That inversion is a negation whose +1 is deferred, which is why
  the first final-adder pass takes carry-in `R[0]` (P1, "Multiplexer C"). A negative multiple
  inside a row carries its own +1 as the row's `XC` bit into the carry vector (P1, "Multiplier
  rows" table); that one is not deferred.
- **Each row.** Adds one multiple of the multiplicand into the carry-save pair without a
  carry chain, shifts the active region two bits up, and drops the two now-final low bits of
  save and carry into the low latches (P1, "Multiplier rows 1, 2, 4 and 5" equations).
- **Final addition.** One 32-bit adder produces `(COUT, SUM[31:0])` from the save and carry
  inputs and a carry-in, used twice. Cycle 4 adds the low latches with carry-in `R[0]`,
  keeps `COUT` in a carry latch, and writes `SUM` to `RESULT[31:0]`. Cycle 5 adds the last
  row's save and carry with the latched carry-out as carry-in and writes `SUM` to
  `RESULT[63:32]` (P1, "Multiplexer C", "Carry Latch", "Final adder", "Result latches"; the text
  says the double-width sum is "two uses of a single width adder" with the first carry-out
  feeding the second).
- **32-bit results.** P1 says 32 x 32 -> 32 forms come from stopping once the lowest 32 bits
  are known, with the accumulate value initialised to its low 32 bits.

### What the sources do not settle

The step this issue needs, which adder carry-out becomes CPSR.C, is in no passage read.
P1 says only that the set-condition-code flag (40) decides whether the CPSR flags are
updated and that the CPSR holds "carries and the like"; it never says which signal feeds C.
`COUT` above is an internal adder signal. Also not settled:

- That this embodiment is the multiplier of the ARM7TDMI. The patents name no core; they
  name the ARM6 only as prior art with one addition per clock.
- Early termination. P1 and P2 list it as a refinement "not illustrated", needing "some
  rather messy rearrangement of the bits in the 'low', 'active' and 'high' regions", and give
  no rule. The MUL, MLA and signed forms of this note's `m` (1 to 4 cycles) live exactly there,
  so no cycle-by-cycle carry model for them can be derived from these sources.
- Which cycle's adder (and so which `COUT`) is last when the multiply stops early.

No model is implemented. Writing one now would mean choosing the missing link by trial against
the suite, which #80 and #81 forbid.

**What this feature implements.** On the ARM7 an S multiply updates N and Z only and leaves
C as it was. That is a documented gap against the suite, not hardware behavior. On the ARM9
this is the documented behavior (C not affected).

## Edge cases

- Rd = Rm (MUL, MLA): GBATEK says the destination may not equal Rm on ARMv4; the suite
  (measured) gives the plain arithmetic result, so the handler reads all operands before it
  writes. Same for RdHi/RdLo = Rm.
- RdHi = RdLo: both halves write the same register; the Hi half is written last and stays.
- An R15 operand or destination is "may not be used" per GBATEK; the suite behavior is
  recorded above.
- MUL with a nonzero Rn field: GBATEK says the field should be 0000 and states nothing else;
  MUL has no accumulator, so the field is not read. Not measured; see Unverified.
- A result that needs more than 32 (MUL, MLA) or 64 (long) bits wraps; nothing saturates, and
  there is no overflow flag.

## Unverified

Each item is backed (if at all) by the JSON suite as oracle evidence, not by hardware.
Code sites are the handlers the implementation adds to `crates/core/src/cpu/arm.rs`
(`multiply`, `multiply_long`); each carries `TODO(hw-unverified)`.

- **ARM7 C after an S multiply** is not implemented: the handlers leave C unchanged. The suite
  differs from that in about half of the S cases. Site: both handlers. #81 read two ARM
  patents (see "Sources read for #81"); they give the carry-save datapath but not the link from
  an adder carry-out to CPSR.C, nor the early-terminated form (including which cycle's adder
  is last when a multiply stops early). Remaining routes: further
  clean-room sources a maintainer can supply (a textbook summary of the ARM7TDMI multiplier),
  and/or measuring C on real ARM7TDMI hardware with a test ROM of our own. If those fail,
  whether the single bit becomes a documented Phase 1 DoD exception is decided in #80.
- **The origin of the ARM7 C** (which stage's carry-out reaches CPSR.C, and whether the
  patents' embodiment is the ARM7TDMI's multiplier) is not in GBATEK, the ARM ARM, the patents
  read, or the suite. Site: both handlers (a comment only).
- **C on ARM9 is unchanged.** GBATEK says so for ARMv5 and up; there is no suite for ARM9.
  Site: both handlers (the `ARM9` const generic). Verify by a test ROM on real ARM9
  hardware; the self-written unit test pins the documented behavior.
- **V on ARMv4 long multiplies** is "destroyed???" in GBATEK; the suite shows it unchanged.
  Site: `multiply_long`. Verify by suite and hardware.
- **R15 operands** (Rm, Rs, Rn, RdHi, RdLo = R15): GBATEK and the ARM ARM say unpredictable.
  The suite reads the instruction address + 12 and writes the value + 8. Site: both
  handlers. Verify by hardware (the pipeline model is `arm-branch-pipeline.md`).
- **RdHi = RdLo**: the Hi half remains (suite). Site: `multiply_long`. Verify by hardware.
- **Rd = Rm on ARMv4** (and RdHi/RdLo = Rm): GBATEK says not allowed; the suite gives the plain
  result. Site: both handlers. Verify by hardware.
- **MUL with a nonzero Rn field** is not read (inferred; GBATEK only says "should be 0000").
  Site: `multiply`. Verify by suite (if a case sets the field) or hardware.
