# ARM branches, BX, and the prefetch pipeline

Sources: GBATEK "ARM Opcodes: Branch and Branch with Link (B, BL, BLX_imm)"; GBATEK "ARM
Opcodes: Branch and Exchange (BX, BLX_reg)"; GBATEK "ARM CPU Register Set" (R15 and the
pipeline); GBATEK "ARM CPU Instruction Set Overview". Measured facts come from the pinned
SingleStepTests/ARM7TDMI suite (MIT), as oracle evidence, not hardware evidence.
Used by: `crates/core/src/cpu/decode.rs`, `crates/core/src/cpu/arm.rs` (`step_arm`, `branch`,
`branch_exchange`, `refill`), `crates/core/src/cpu/mod.rs` (`pipeline`)   Phase: 1 (#11)

Out of scope here: Thumb execution (#13); BLX and the ARMv5 interworking loads (`LDR PC`,
`POP {PC}`, #14); exceptions (#12); cycle timing; filling the pipeline at direct boot (#17,
Phase 2). Registers and modes are in `cpu-registers.md`.

## Encodings

`cond 101 L offset24` (B, BL) and `cond 0001 0010 1111 1111 1111 0001 Rn` (BX).

| Bits | Meaning |
|---|---|
| 31-28 | condition |
| 27-25 | 101 (B, BL) |
| 24 | L: 1 = link (BL) |
| 23-0 | signed word offset |

BX is bits 27-20 = 0x12 with bits 19-8 all ones (should-be-one fields) and bits 7-4 = 0001;
bits 3-0 name Rn. It lies inside the PSR-transfer space (TEQ without S), so a decoder must
test for it before MSR.

## The prefetch pipeline

The core fetches ahead of execution: while one instruction executes, the next one is already
fetched and the one after that is being fetched. We model this as a two-entry buffer of
fetched opcodes plus R15:

- `pipeline[0]` is the opcode that executes next; `pipeline[1]` is the one after it.
- While an instruction executes, R15 is the instruction's address + 8 in ARM state (+ 4 in
  Thumb state). That is the distance GBATEK gives for an R15 read.

One step, in order:

1. Take `pipeline[0]` as the instruction.
2. Fetch one opcode at R15 (address + 8) and shift it in: `pipeline = [pipeline[1], fetched]`.
   This happens before the instruction does anything, so a data access comes after the fetch.
3. Execute. R15 still reads as address + 8.
4. If the instruction did not write R15, R15 advances by the instruction width (4 in ARM
   state). A failed condition is the same: the instruction does nothing, the pipeline still
   shifts and R15 still advances.
5. If the instruction wrote R15, the pipeline is flushed and refilled from the new R15
   (below).

### Refill after an R15 write

With target `t` taken from the written value:

- ARM state: fetch a word at `t & !3`, then a word at `(t & !3) + 4`; the pipeline becomes
  those two opcodes and R15 = `t + 8`.
- Thumb state (T set): fetch a halfword at `t & !1`, then at `(t & !1) + 2`; R15 =
  `(t & !1) + 4`.

Which state refills is decided by T at the moment of the refill, so BX changes T first.

### Late R15 reads

Some instructions read R15 after the prefetch has advanced it by one more word, so they see
address + 12 instead of address + 8: a register-specified shift (Rn and Rm; the shift amount
register Rs still reads address + 8), the stored register of STR, STRB, STRH and STM, the
operands of a multiply, and the registers of SWP. (GBATEK gives the register-shift and store
cases; the rest are measured.)

## B and BL

- The offset is the 24-bit field, sign-extended, shifted left by 2 (a word offset).
- The target is R15 (the instruction's address + 8) + that offset, so `B .` (branch to
  itself) encodes -2 words.
- BL also writes the address of the next instruction (the BL's address + 4) to R14 (LR),
  before the branch. With the condition false, nothing is written, including LR.
- The offset covers ±32 MiB around the instruction.

## BX

- The target is Rn, read as a normal register (R15 reads address + 8).
- Bit 0 of Rn becomes the T bit of the CPSR: 1 enters Thumb state, 0 stays in or returns to
  ARM state. The CPSR flags and mode are untouched.
- The pipeline refills in the state BX selected (see "Refill after an R15 write"), with
  `t = Rn`.

## Edge cases

- A branch target needs no alignment from the branch: BX to Thumb drops bit 0 for the
  fetch address; an ARM refill drops the low two bits for the fetch address.
- A branch whose condition fails does not touch LR, T or the pipeline contents except for
  the normal shift.
- MRS with Rd = R15 is not a flush; see "Unverified".

## Unverified

Each item is backed by the JSON suite as oracle evidence only; where GBATEK is silent the
code site carries `TODO(hw-unverified)`. Verify by a test ROM on real hardware or a trace
against the reference emulator.

- **Low bits kept in R15 after an ARM refill.** Fetches use the aligned address, but R15 =
  target + 8 keeps the low two bits as written (R15 − first fetch is 8, 9, 10 or 11, suite).
  GBATEK says nothing about an unaligned ARM target. — `refill` in `arm.rs`.
- **ARM BX with bit 1 set.** The fetches are word-aligned and R15 = target + 8 stays
  unaligned (R15 − first fetch = 10; 5,936 suite cases). The ARM ARM calls this
  unpredictable on ARMv4T. — `branch_exchange` / `refill`.
- **Writeback to R15 by a single or halfword transfer (no PC load).** The suite appears to
  flush with target = the moved address + 4, so R15 = moved + 12 in all; GBATEK does not allow
  R15 as a base with writeback. A block transfer with writeback to R15 flushes to the moved
  address (R15 = moved + 8). — `write_back` in `arm_transfer.rs`.
- **MRS with Rd = R15.** The suite shows no flush: R15 ends as the value + 4 (1,580 cases),
  i.e. a plain register write followed by the normal advance. The ARM ARM leaves it
  unpredictable. — `mrs` in `arm.rs`.
- **BX with should-be-one fields that are not all ones.** The decoder keys on bits 27-20 and
  7-4 only, so any such word runs as BX; what hardware does with other values in bits 19-8 is
  unknown. — `decode.rs` (and `branch_exchange`).
- **Late R15 reads of multiply operands and SWP registers.** GBATEK gives the register-shift
  and store cases only; the +12 reads of multiply and SWP operands are the suite's. —
  `LATE_PC_EXTRA` in `arm.rs` — verify by a test ROM on real hardware.
- **Fetch transactions.** The suite lists a fetch per refill and per step; the model matches
  its sequence and sizes (step fetch, then two refill fetches; 32/16/16 bits for BX to
  Thumb). Whether real hardware fetches the same way (no extra speculative fetch) is oracle
  evidence only. The data a suite fetch returns is generated by the suite (the address itself,
  or its low halfword for a 16-bit fetch), not a hardware fact. — `step_arm`, `refill`.
