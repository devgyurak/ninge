# ARM multiply and long multiply

Sources: GBATEK "ARM Opcodes: Multiply and Multiply-Accumulate (MUL, MLA)" (the same
section covers the long forms) is the primary source for the behavior; ARM ARM (ARMv5TE) "MUL",
"MLA", "UMULL", "UMLAL", "SMULL", "SMLAL" and ARM7TDMI TRM "Multiply instructions" are listed as
background; not re-read for this note (the carry section uses the TRM's `m` rule and block
diagram, as quoted in "Cycle count" above). For the ARM7TDMI carry only: US 5,583,804, US 5,528,529 and
US 5,557,563 (see "ARM7TDMI multiply carry"). Measured facts come from the pinned SingleStepTests/ARM7TDMI suite (MIT).
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
| C | **ARM9 (ARMv5TE): not affected.** **ARM7 (ARMv4T): destroyed**: a fixed function of the operands, computed by the carry-save model in "ARM7TDMI multiply carry" |

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
  12,637). The suite therefore models the ARM7TDMI multiplier carry, not "unchanged"; the
  model in "ARM7TDMI multiply carry" reproduces it.

## ARM7TDMI multiply carry

ARMv4 documents C after a flag-setting multiply as meaningless, but the suite shows it is a
fixed function of the operands (see "Measured on the pinned suite"). Nothing below comes from
the public article an earlier revision of this note pointed at (recorded in #80; #81 forbids
reusing it), and the sessions that wrote this section did not open it.

### Sources read for #81

P1 and P2 are ARM patents with largely the same specification text, filed in the mid-1990s; read in full
on patents.google.com (specification and example tables, not the abstract):

- **US 5,583,804**, "Data processing using multiply-accumulate instructions" (Advanced RISC
  Machines Ltd). Called **P1** below.
- **US 5,528,529**, "Electronic multiplying and adding apparatus and method" (Advanced RISC
  Machines Ltd). Called **P2**. It repeats P1's example multiplier (its own five-cycle table
  and early-termination remark), and its claims cover the carry-save accumulate-bit technique.
- **US 5,557,563**, called **P3**, is described below ("The early-terminated core and the
  final cycles").

Searches for a patent that states how a multiply sets a CPSR flag, or that describes the
early-terminated form, found none (only other assignees' patents, which are not used).

### What the sources settle

The example multiplier in P1 and P2 (P1 "An example multiplier will now be described" and the
per-cycle table that follows it; P2 the same table) computes multiplier x multiplicand
+ accumulate as follows. It is a description of one embodiment, not of a named ARM core.

- **Recoding.** Radix-4 modified Booth. The multiplier is extended to 33 bits, with a copy of
  its sign bit when signed and 0 when unsigned (P1: `R[32] = SIGNED AND MPLIER[31]`); the
  multiplicand gets the same 33rd bit (P1: `D[32] = SIGNED AND MCAND[31]`). Each
  digit reads three overlapping multiplier bits, `BoothEnc(R[2:0])`, `BoothEnc(R[4:2])`, and so on.
- **Rate.** Four multiplier rows work per cycle and each retires two result bits, so eight
  multiplier bits are consumed per cycle: Booth digits for `R[8:0]` in cycle 1 up to
  `R[32:24]` in cycle 4 (P1 table, "Booth encoders"). A 32 x 32 multiply takes five cycles; cycles
  1 to 4 consume the multiplier and cycle 5 finishes the sum.
- **Start.** The save vector starts as the low 33 bits of the accumulate value (zero without
  accumulate) and the carry vector as the bitwise inverse of the multiplicand when `R[0]` is 1,
  else zero (P1, "Carry-save initialiser", `SI[i] = ACCUM AND ACCVAL[i]` for i = 0 to 32).
  That inversion is a negation whose +1 is deferred, which is why the first final-adder pass
  takes carry-in `R[0]` (P1, "Multiplexer C"; P2's body text). P1's paragraph before its second
  initialisation method says `R[1]` where its next sentence and the Multiplexer C table say
  `R[0]`; that is a slip in P1, and `R[0]` is used here. A negative multiple
  inside a row carries its own +1 as the row's `XC` bit into the carry vector (P1, "Multiplier
  rows" table); that one is not deferred.
- **Upper accumulate bits.** Accumulate bits 63 down to 33 do not enter at the start. They
  wait in the "accumulate value high latches", which hold one bit more than that range: a
  copy of bit 63 when signed, 0 when unsigned (P1: `ACCHI[31] = SIGNED AND ACCUM AND
  ACCVAL[63]`). Each row takes two of them per cycle (P1 table, "Accumulate value high
  latches", `AH1`, `AH2`, `AH4`, `AH5`; the technique is claimed in P2 claims 1 to 3).
- **Each row.** Adds one multiple of the multiplicand into the carry-save pair without a
  carry chain, shifts the active region two bits up, and drops the two now-final low bits of
  save and carry into the low latches (P1, "Multiplier rows 1, 2, 4 and 5" equations). The top
  two bits of a row are not plain carry-save: they sum the row's first incoming accumulate bit
  with the complemented top carry and the complemented top multiple bit, and the top carry bit
  becomes the complement of the second accumulate bit (which does not enter that sum).
  P1 gives these as `(S(k+1)[32], S(k+1)[31])` and `C(k+1)[32]` in the same equations.
- **Final addition.** One 32-bit adder produces `(COUT, SUM[31:0])` from the save and carry
  inputs and a carry-in. It evaluates every cycle, but only two outputs are used: cycle 4 and
  cycle 5 (P1, "Final adder": "On all cycles"). Cycle 4 adds the low latches with carry-in `R[0]`,
  keeps `COUT` in a carry latch, and writes `SUM` to `RESULT[31:0]`. Cycle 5 adds the last
  row's save and carry with the latched carry-out as carry-in and writes `SUM` to
  `RESULT[63:32]` (P1, "Multiplexer C", "Carry Latch", "Final adder", "Result latches"; the text
  says the double-width sum is "two uses of a single width adder" with the first carry-out
  feeding the second).
- **32-bit results.** P1 says 32 x 32 -> 32 forms come from stopping once the lowest 32 bits
  are known, with the accumulate value initialised to its low 32 bits. That is a stop by
  result width, a different thing from the data-dependent early termination below.

### How the datapath is read (P1/P2 table, P3 examples)

The five-cycle table in P1 and P2 is off by one bit as printed: its row equations add the
first Booth digit at the same weight as the initialiser vectors. P1's text says the
initialisation takes a low bit before the first multiple is added ("Li = 2i - 1" before the
i-th multiple), and P3 Table 2 selects 8m + 1 low save bits after m iterations. Both mean the
initialiser's bit 0 sits in the low region before row 1. With that reading the datapath
reproduces the four worked examples in P3 (below) exactly, and it keeps the identity
`save + carry + R[0] = plain result`. The model therefore does this:

- Low region (bit 0 only at the start): `SL[0] = SI[0]`, `CL[0] = CI[0]`. The active region
  is absolute bits 33 down to 1 of the initial vectors (33 bits). The initial carry is the
  inverse of the multiplicand taken over 34 bits after sign extension (`D[33] = D[32]`), or 0
  when `R[0]` is 0. The accumulate value is sign-extended when SIGNED, so
  `ACCHI = acc_ext >> 34`.
- Row k (0-based, k = 0 to 4m - 1) uses the digit from `R[2k+2:2k]`, takes `AH = ACCHI[2k+1:2k]`,
  applies P1's row equations, and places the two dropped bits at absolute bits `1+2k` and
  `2+2k` of the low latches.
- After m iterations the low latches hold `lb = 8m + 1` bits. Together: save = low save +
  (active save << lb) + the accumulate bits from `lb + 33` up; carry = low carry + (the active
  carry, sign-extended from its bit 32, << lb); both modulo 2^64.

P1 and P2 both print `SL2` where `SL1` is meant in the cycle-2 low-latch row. P3's example
titles ("100x100" and so on) are a typo: the printed bus values only work with the multiplier
as the variable operand and the multiplicand as 1 (P3 examples 1 to 4: multiplier 0x1, 0x100,
0x10000, 0x1000000, multiplicand 1; one, two, three and four iterations).

### The early-terminated core and the final cycles (P3, TRM)

**US 5,557,563** ("Data processing method and apparatus including iterative multiplier", ARM,
**P3**) describes the same kind of core inside a processor with a general-purpose barrel
shifter and ALU. Read in full (abstract, tables, examples):

- The core can stop after one, two, three or four iterations. After it stops, the result passes
  through the barrel shifter, an "early terminate shift mechanism", and the ALU into the
  register bank in a WL cycle (always, low word) and a WH cycle (only when the instruction
  writes the high word).
- Table 1 gives the barrel shifter's operation on the core's carry value in each of those
  eight cases: WL rotates right by 22, 14, 6, 30 bits for 1 to 4 iterations; WH shifts
  arithmetically right by 22, 14, 6 for 1 to 3 iterations and rotates right by 30 for 4.
- Table 2 gives which save-vector bits feed the ALU's other input: low word `suma[8:0]` with
  the top of `sumb` (1), `suma[16:0]` (2), `suma[24:0]` (3), `suma[31:0]` (4); the high word
  from the upper save bits. This is consistent with `8m + 1` low save bits.
- The four worked examples print, for WL, the shifter output `ShiftOut` = 0x4, 0x480, 0x48000,
  0x4800000, the save low bits `suma` = 0x1FC, 0x1FC80, 0x1FC8000, 0x1FC800000 and the active
  save `sumb` = 0xFFFFFFFF. The model reproduces all of them (a unit test pins them).

The ARM7TDMI TRM (DDI 0029G) block diagram shows a 32 x 8 multiplier next to the barrel
shifter and ALU, and its `m` rule (the "Cycle count" section above) counts all-ones as terminal
for MUL and MLA. That rule is only valid for a signed multiplier, so MUL, MLA and Thumb MUL are
modelled with SIGNED = 1 (an inference, see Unverified).

### The C rule

None of P1, P2, P3 or the TRM says which signal feeds CPSR.C. Two candidates defined by the
sources were fixed in writing before any comparison with the suite:

- the ALU carry-out of the last WL or WH pass, and
- the barrel shifter's carry-out in that cycle (for a rotate right by n, bit 31 of the shifter
  output; for an arithmetic shift right, the last bit shifted out).

Compared once with the pinned suite, the ALU candidate matched about 63% of the cases and was
rejected. "Evaluable" there means S cases whose condition passes, with no R15 operand and a
defined mode, and m = 3 or 4: 9,623 + 70 + 10,129 + 71 + 29 + 3,127 = 23,049 cases (long m = 4
and 3, MUL/MLA m = 4 and 3, Thumb MUL m = 4 and 3). The shifter candidate matched all of them.
The suite has no case with m = 1 or 2, because its operands are random. The implemented model
then also matches every case of the three suite files, R15 operands included. So the rule is:

- **MUL, MLA, Thumb MUL, and long forms with m < 4: C = bit 31 of the multiplier's carry-save
  carry vector** (the vector after the m iterations, defined above). For a rotate right of the
  WL word that is bit 31 of the rotated value, and for the m < 4 WH arithmetic shift it is the
  same bit.
- **Long forms with m = 4: C = bit 63 of the carry vector** (WH rotates right by 30, so the
  last bit of the high word is bit 63).
- N and Z come from the result as before; V is unchanged; ARM9 leaves C alone.

For a long form with m < 4 the WH bus slice that makes the shift amounts of Table 1 work is
`carry_active[32:1]` (contiguous); the model assumes that slice because no source states it. Its
shifter carry-out is then carry bit 31, the same as WL. This is inferred from Table 1's shift
amounts, not read.

**Evidence level.** The match is oracle evidence only: the pinned suite, not hardware, and the
link from shifter to flag is not in any source. The decision to adopt this model on that
evidence is recorded in #80. A research session ran the comparison before this feature. It knew
from #80's text that the article attributes C to a shifter stage, so the choice of candidates
was informed by that: only the datapath (from the patents) is independent of it. Both
candidates were fixed in writing before any count was seen, which keeps the counts themselves
honest, but the shifter idea cannot be claimed to come from the permitted sources. The first
attempt stopped at a datapath self-check (the off-by-one above) and was rerun with the datapath
as read here; the only amendment before comparing was the sign extension of the multiplicand's
inverse.

**What this feature implements.** On the ARM7 an S multiply sets N and Z from the result and C by
the rule above; V stays. This covers MUL, MLA, UMULL, UMLAL, SMULL, SMLAL and the Thumb MUL
(`thumb.md`; its multiplier operand is Rd, the incoming value, per GBATEK THUMB.4 "m ...
depending on MSBs of incoming Rd", and its multiplicand is Rs). On the ARM9 C is unchanged
(documented, no suite). The model is `crates/core/src/cpu/multiply_carry.rs`.

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

- **ARM7 C after an S multiply** follows the shifter-carry rule above; the rule is not in any
  source and rests on the pinned suite (oracle evidence). Site: both handlers, Thumb MUL and
  `multiply_carry.rs`. Verify by a test ROM measuring C on real ARM7TDMI hardware, or a
  clean-room summary of the multiplier from a maintainer.
- **m = 1 and m = 2** are untested: the suite has no such case. For long forms with m < 4 the
  WH bus slice is inferred from P3 Table 1's shift amounts. Site: `multiply_carry.rs`.
- **SIGNED = 1 for MUL, MLA and Thumb MUL** is inferred from the TRM `m` rule, which counts
  all-ones as terminal. Site: `multiply_carry.rs` callers.
- **That P1/P2's embodiment is the ARM7TDMI's multiplier** is an inference: the patents name no
  core, and P3 describes a core of the same kind with early termination. Site:
  `multiply_carry.rs` (module comment).
- **The off-by-one reading** of P1/P2's table is fixed by P3's examples and P1's text, not
  stated outright. Site: `multiply_carry.rs` (module comment).
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
