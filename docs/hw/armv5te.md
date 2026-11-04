# ARMv5TE extensions (ARM9)

Sources: GBATEK "ARM Opcodes: Branch with Link and Exchange (BLX)", "Count Leading Zeros (CLZ)",
"Saturated Arithmetic (QADD, QSUB, QDADD, QDSUB)", "Signed Multiplies (SMLAxy, SMLAWy, SMULWy,
SMLALxy, SMULxy)", "Memory: Halfword, Doubleword, Signed Transfer (LDRD, STRD)", "PLD",
"Software Breakpoint (BKPT)", "THUMB Opcodes: Register Operations (BLX)" and "Long Branch with
Link (BLX)", and the CPSR bit table; ARM ARM (ARMv5TE) for the instruction descriptions;
ARM946E-S TRM for the core-specific notes (PLD, the Q flag). Written from memory of those
sources, not re-read line by line while writing, so a human should check the claims outside
"Unverified" (Q-setting rules, the W multiplies, the PLD encoding, the BLX link values) against
GBATEK; every claim that rests on inference is under "Unverified".
Used by: `crates/core/src/cpu/decode.rs`, `thumb_decode.rs`, `armv5.rs`, `arm.rs`,
`arm_transfer.rs`, `thumb.rs`, `exception.rs`   Phase: 1

These instructions exist on the ARM9 (ARMv5TE) only. The ARM7 (ARMv4T) keeps the behavior of
the earlier notes for every encoding below. Registers and modes are in `cpu-registers.md`;
the neighbouring notes are `arm-branch-pipeline.md`, `arm-load-store.md`, `arm-multiply.md`,
`thumb.md` and `arm-exceptions-modes.md`.

## Differences at a glance

| Item | ARM7 (ARMv4T) | ARM9 (ARMv5TE) |
|---|---|---|
| LDR / LDM into R15 | bit 0 ignored, state unchanged | bit 0 of the loaded value sets T |
| Thumb POP {PC} | bit 0 ignored, stays Thumb | bit 0 sets T |
| Condition field `1111` | never executes | unconditional space: BLX immediate, PLD |
| CLZ, QADD, QSUB, QDADD, QDSUB | not executed | executed |
| SMLAxy, SMLAWy, SMULWy, SMLALxy, SMULxy | not executed | executed |
| BLX (ARM and Thumb, register and immediate) | not executed; Thumb H1 = 1 runs as BX | executed |
| LDRD, STRD | not executed | executed |
| PLD | not executed | executes as a no-op |
| BKPT (ARM and Thumb) | not executed | prefetch abort |
| CPSR bit 27 (Q) | stored as written by MSR, nothing sets it | sticky saturation flag |

"Not executed" is what the earlier notes call unhandled: the step only advances R15.

## The Q flag

CPSR bit 27 is Q, "sticky overflow". The saturating adds and the accumulating halfword
multiplies set it; nothing clears it except an MSR that writes the flags byte. Data processing
with S, the multiplies and the other flag writers leave it alone. An MSR with the `f` field
writes it like any other flag bit; on the ARM7 the same write is stored too, with no meaning.
The SPSR/CPSR copies on exception entry and return carry it along with the rest of the word.

## Switching to Thumb on a load into R15

On the ARM9 an ARM word load into R15 is an interworking branch: LDR (word, not LDRB) and LDM
with R15 in the list. The loaded word's bit 0 becomes T and the pipeline refills in that
state; the target is the loaded word with bit 0 cleared for Thumb. Thumb `POP {PC}` is an LDM
and does the same, so it can switch Thumb to ARM when the popped bit 0 is 0.

- LDM with the S bit and R15 in the list restores CPSR from SPSR (see `arm-load-store.md`);
  T then comes from the SPSR and the loaded bit 0 is not used.
- The other loads that can name R15 as the destination (LDRB, LDRH, LDRSB, LDRSH, SWP, data
  processing, MOV) do not switch state, on either core.
- A loaded ARM target with bit 1 set and bit 0 clear (low bits `10`) is Unverified.

## BLX

| Form | Encoding | Effect |
|---|---|---|
| ARM, immediate | `1111 101H offset24` | LR = address of the next instruction; T = 1; PC = R15 + signExtend(offset24) × 4 + H × 2 |
| ARM, register | `cond 0001 0010 1111 1111 1111 0011 Rm` | target = Rm read first; LR = address of the next instruction; T = Rm bit 0; PC = Rm |
| Thumb, register | `0100 0111 1 Rm(4) 000` (BX with H1 = 1) | target = Rm read first; LR = address of the next instruction + 1; T = Rm bit 0; PC = Rm |
| Thumb, immediate | prefix `11110 offset11`, suffix `11101 offset11` | prefix as for BL (LR = R15 + offset × 4096); suffix: target = (LR + offset × 2) with bits 1–0 cleared; LR = address of the next instruction + 1; T = 0 |

- R15 is the executing address + 8 in ARM state and + 4 in Thumb state, as for every
  instruction. The immediate forms use that view.
- The ARM immediate form ignores the condition field value: `1111` is its encoding. H is
  bit 24 and adds a halfword to the target, so the target is halfword aligned and bit 0 is 0.
- The ARM register form is conditional like any other. The link is written after the target is
  read, so `BLX LR` jumps to the old LR.
- The link value written from Thumb state has bit 0 set so that a later `BX LR` returns to
  Thumb.
- The pipeline refills in the new state from the target (`arm-branch-pipeline.md`).
- The Thumb immediate suffix on the ARM7 is not executed (`thumb.md` covers BL).

## CLZ

`cond 0001 0110 1111 Rd 1111 0001 Rm`: Rd = the number of leading zero bits of Rm; an input of
0 gives 32. No flags change. R15 as Rd or Rm is not allowed.

## Saturating arithmetic

`cond 0001 0 op 0 Rn Rd 0000 0101 Rm`, op in bits 22–21:

| op | Mnemonic | Result |
|---|---|---|
| 00 | QADD | Rd = sat(Rm + Rn) |
| 01 | QSUB | Rd = sat(Rm − Rn) |
| 10 | QDADD | Rd = sat(Rm + sat(2 × Rn)) |
| 11 | QDSUB | Rd = sat(Rm − sat(2 × Rn)) |

`sat` clamps the signed 32-bit sum to −2^31 … 2^31 − 1. If any `sat` clamps (the doubling
included), Q is set; otherwise Q is left as it was. N, Z, C and V never change.

## Signed halfword multiplies

The forms are `cond 0001 0 op 0 Rd Rn Rs 1 y x 0 Rm` with op in bits 22–21; in the Rd field
position (19–16) the multiplies name their destination, Rn (15–12) is the accumulator or, for
SMLALxy, RdLo. `x` is bit 5 and selects the halfword of Rm (0 = bottom, 1 = top), `y` is bit 6
and selects the halfword of Rs. Halfwords are signed.

| op | Bit 5 | Mnemonic | Result | Q |
|---|---|---|---|---|
| 00 | x | SMLAxy | Rd = Rm.x × Rs.y + Rn | set if the 32-bit add overflows |
| 01 | 0 | SMLAWy | Rd = ((Rm × Rs.y) >> 16) + Rn | set if the 32-bit add overflows |
| 01 | 1 | SMULWy | Rd = (Rm × Rs.y) >> 16 | never |
| 10 | x | SMLALxy | RdHi:RdLo = RdHi:RdLo + Rm.x × Rs.y (64-bit) | never |
| 11 | x | SMULxy | Rd = Rm.x × Rs.y | never |

- For the W forms, Rm is the full signed 32-bit register and the 48-bit product is shifted
  right arithmetically by 16 before the accumulate; bit 5 of those forms selects SMULW or
  SMLAW, not a halfword.
- The accumulate is a plain wrapping add: on overflow Rd holds the wrapped sum, and Q records
  that it happened. It is not a saturating add.
- SMLALxy: RdHi is bits 19–16, RdLo is bits 15–12.
- N, Z, C and V never change.

## LDRD and STRD

The halfword-transfer encoding with L = 0 and SH = 10 (LDRD) or SH = 11 (STRD) — on the ARMv4T
those SH values with L = 0 do not exist. The addressing is the halfword form's: P, U, W, the
8-bit split immediate (bit 22 = 1) or Rm (bit 22 = 0), pre- or post-indexed, with writeback
under the same rules as `arm-load-store.md`.

- Rd must be even and not R14; the pair is Rd and Rd + 1. LDRD loads the word at the address
  into Rd and the word at the address + 4 into Rd + 1; STRD stores them the same way.
- The accesses are two word accesses at the raw address and address + 4.
- A pre-indexed address is Rn ± offset; a post-indexed one is Rn, with Rn ± offset written back.
- Rn and Rm must not be R15, and with writeback Rn must not be Rd or Rd + 1.

## PLD

`1111 01 I1 U101 Rn 1111 offset` (bit 25 = I: 0 immediate offset, 1 register offset with
bit 4 = 0). It is a cache hint. The ARM946E-S executes it as a no-op: no register, flag or
memory access changes, and no abort is taken. PLD is in the unconditional space, so on the ARM9
it runs whatever the other bits of the condition position say.

## Unconditional space (condition field 1111)

On the ARM9 the encodings with condition `1111` are not "never": BLX immediate (bits 27–25
= 101) and PLD (above) are defined; every other word in that space is Unverified. On the ARM7
the condition `1111` still never executes.

## BKPT

ARM: `1110 0001 0010 imm12 0111 imm4`; Thumb: `1011 1110 imm8`. Executing it takes a **prefetch
abort**, exactly as the abort entry in `arm-exceptions-modes.md`: the mode becomes Abort, the
old CPSR is saved to SPSR_abt, I is set and T cleared, R14_abt is the address of the BKPT + 4 in
both states, and the PC becomes the vector base + 0x0C. The
immediate is ignored by the core. On the ARM7 neither encoding is executed.

## Unverified

- LDRD and STRD with an odd Rd or with Rd = R14, and doubleword accesses at an address that
  is not 8-byte aligned; we treat an odd Rd or R14 as not executed and pass each word's raw
  address to the bus — `crates/core/src/cpu/arm_transfer.rs`, `double_transfer` — verify by a
  test ROM on hardware.
- LDRD with writeback and Rn equal to Rd or Rd + 1: the loaded value is taken to win over the
  written-back base, as for a single load — `arm_transfer.rs`, `double_transfer` — verify by
  a test ROM on hardware.
- BLX register with Rm = R15 (ARM: not allowed; Thumb: the H2 form reads R15 as usual) —
  `crates/core/src/cpu/armv5.rs`, `branch_link_exchange` — verify by a test ROM.
- A loaded ARM target whose low two bits are `10` for LDR/LDM into R15 — `arm_transfer.rs`,
  `single_transfer`, `block_transfer` — verify by a test ROM on hardware.
- LDM with R15 and S in User/System mode (no SPSR): the loaded bit 0 switches state —
  `arm_transfer.rs`, `block_transfer` — verify by a test ROM.
- The words of the `1111` condition space other than BLX and PLD are not executed (stepped
  past); the real core may take Undefined — `crates/core/src/cpu/decode.rs`,
  `unconditional_kind` — verify by a test ROM.
- PLD's Rd field (bits 15–12 should be 1111) is not checked — `decode.rs`,
  `unconditional_kind`.
- MSR of bit 27 on the ARM7 is stored; whether the ARM7TDMI keeps the bit is not stated —
  `crates/core/src/cpu/psr.rs`, `FLAG_Q`.
- LDRD and STRD with R15 as Rn or Rm, and the should-be-one fields (bits 19–8) of BLX register,
  CLZ and BKPT's non-immediate bits, are not checked; R15 reads as for any instruction —
  `arm_transfer.rs`, `double_transfer`; `decode.rs` — verify by a test ROM.
- BKPT with a condition field other than AL runs conditionally here; the ARM ARM states it
  is unconditional — `decode.rs`, `Kind::Breakpoint`.
- Destination R15 for CLZ, the saturating adds and the halfword multiplies (not allowed): the
  result is written to R15 and the pipeline refills — `armv5.rs`.
- Thumb BLX suffix with offset bit 0 set (ARM ARM: should be zero): the target is word
  aligned, so the bit is dropped — `crates/core/src/cpu/thumb.rs`, `long_branch_exchange_suffix`.
