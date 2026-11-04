# ARM loads and stores

Sources: GBATEK, the ARM-opcode sections "Memory: Single Data Transfer (LDR, STR, PLD)",
"Memory: Halfword, Doubleword, and Signed Data Transfer", "Memory: Block Data Transfer (LDM,
STM)", "Memory: Single Data Swap (SWP)", and "ARM CPU Memory Alignments". Measured facts come
from the pinned SingleStepTests/ARM7TDMI suite (MIT); each is pinned by a test in
`crates/runner/tests/cpu_suite.rs` and is marked **(suite)** below. The ARM ARM and the two TRMs
are not re-read for this note.
Used by: `crates/core/src/cpu/decode.rs`, `crates/core/src/cpu/arm_transfer.rs`,
`crates/core/src/cpu/bus.rs`   Phase: 1 (#10)

Out of scope here, deferred with a marker at the code site: LDRD, STRD and PLD, and the ARMv5
switch to Thumb state on a load into R15 (#14); the real pipeline (#11); data aborts and cycle
timing. Registers and modes are in `cpu-registers.md`; the PC convention is the interim one of
`arm-data-processing.md`.

## Encodings

### Single data transfer (LDR, STR, LDRB, STRB, and the T forms)

`cond 01 I P U B W L Rn Rd offset`

| Bits | Meaning |
|---|---|
| 27-26 | 01 |
| 25 I | 0 = 12-bit immediate offset, 1 = register offset shifted by an immediate |
| 24 P | 1 = pre-indexed (offset applied before the access), 0 = post-indexed |
| 23 U | 1 = add the offset, 0 = subtract it |
| 22 B | 1 = byte, 0 = word |
| 21 | P = 1: W, write the address back to Rn. P = 0: T, a non-privileged access (LDRT, STRT) |
| 20 L | 1 = load, 0 = store |
| 19-16 Rn | base register |
| 15-12 Rd | source or destination |
| 11-0 | I = 0: unsigned immediate 0-4095. I = 1: bits 11-7 shift amount, 6-5 shift type, bit 4 = 0, 3-0 Rm |

With I = 1 and bit 4 = 1 the word is not a load/store; it is in the undefined space and stays
unhandled. The register offset is shifted as a data-processing operand is (`arm-barrel-shifter.md`),
using the immediate-shift forms only; its carry-out is not used and no flag changes. Post-indexed
forms always write the new address back; the T form applies only to them.

### Halfword and signed transfer (LDRH, STRH, LDRSB, LDRSH)

`cond 000 P U I W L Rn Rd hi 1 S H 1 lo`

| Bits | Meaning |
|---|---|
| 27-25 | 000 |
| 24 P, 23 U | as above |
| 22 I | 1 = immediate offset (the byte is split: high nibble in 11-8, low nibble in 3-0), 0 = register offset Rm in 3-0 (bits 11-8 are 0) |
| 21 W | P = 1: write back. P = 0: must be 0 (post-indexing always writes back) |
| 20 L | 1 = load, 0 = store |
| 7, 4 | both 1 |
| 6-5 SH | 01 = halfword, 10 = signed byte, 11 = signed halfword; 00 is SWP |

With L = 0, SH = 01 is STRH; SH = 10 and 11 are LDRD and STRD (ARMv5TE, #14), which stay
unhandled. L = 1 with SH = 00 is the multiply or SWP space (`arm-multiply.md`). The signed forms
exist for loads only.

### Block data transfer (LDM, STM)

`cond 100 P U S W L Rn rlist`

| Bits | Meaning |
|---|---|
| 24 P, 23 U | IB = 11, IA = 01, DB = 10, DA = 00 |
| 22 S | see "S bit" |
| 21 W | write the final address back to Rn |
| 20 L | 1 = LDM, 0 = STM |
| 19-16 Rn | base (R15 not allowed) |
| 15-0 | register list, bit i = Ri |

### Single data swap (SWP, SWPB)

`cond 00010 B 00 Rn Rd 0000 1001 Rm`: Rd = [Rn] and [Rn] = Rm. B = 1 swaps a byte. It works with
Rm = Rd, Rm = Rn and so on, because Rm is read before the store and the read before the write.
R15 is not allowed in any field.

## Behavior

Flags are never changed by these instructions. A failed condition makes the instruction a no-op
that still advances the PC.

### Addressing

The offset is added or subtracted (U). Pre-indexed forms access the moved address; post-indexed
forms access the unmodified base and then move it. The moved address is written to Rn when W = 1
(P = 1) or always (P = 0). A load whose Rd is the written-back Rn keeps the **loaded** value
**(suite)**.

A block transfer touches `n` registers, `n` = the number of list bits. Registers are processed
lowest register first at ascending addresses, whatever the direction: the lowest address is
computed first. IA starts at Rn, IB at Rn + 4, DA at Rn - 4n + 4 and DB at Rn - 4n. Writeback
sets Rn to Rn + 4n (U = 1) or Rn - 4n (U = 0).

### Alignment (GBATEK, "ARM CPU Memory Alignments")

The CPU has no misaligned access. **The memory system drops the low address bits**, so STR, STRH,
STM, LDM go to the next lower aligned address with the data unrotated. LDR and SWP read the
aligned word and rotate it right by 8 × (address & 3); SWP writes unrotated. LDRB and the byte
forms read one byte. Halfword loads differ by CPU:

| Access at an odd address | ARM7 (ARMv4T) | ARM9 (ARMv5TE) |
|---|---|---|
| LDRH | the aligned halfword, rotated right by 8 (bits 7-0 and 31-24 hold the data) | the aligned halfword, no rotation |
| LDRSH | behaves as LDRSB at that address: the byte, sign-extended | the aligned halfword, sign-extended |

What the CPU hands the bus, **measured (suite)**: every transaction carries the **raw effective
address**, low bits included, never the aligned one. This holds for single transfers (LDR, STR,
LDRH, STRH, LDRSH), for every register of an LDM or STM (register i is at the raw start + 4i; the
low bits of the base are kept), for the written-back base (the raw base ± 4n), and for SWP (the
read and the write both at the raw address). The recorded read data is the value the bus
returns at that address; the CPU rotates or sign-extends it using the low bits of the same
address. Rd after an LDR is the read data rotated right by 8 × (address & 3); after an odd
LDRH the halfword rotated right by 8; after an odd LDRSH the sign-extended high byte of the
halfword read; a store writes Rd unrotated (the word, the low halfword, or the low byte); SWP
writes Rm unrotated (the low byte for SWPB). Ignoring the bits below the access width is
therefore the bus implementation's job (the memory map, Phase 2).

### R15

| Use | Value / result | Source |
|---|---|---|
| R15 as Rn | address + 8 | GBATEK; (suite) |
| R15 as the stored Rd of STR/STRB/STM | address + 12 | GBATEK; (suite) for STR, STRB, STM |
| LDR or LDM loads R15 | R15 ends as the loaded value + 8, low bits kept, T unchanged | (suite), interim convention (#11) |

Also measured on the suite, though GBATEK does not allow R15 in these fields:

- A single or halfword transfer whose written-back Rn is R15 ends with R15 = the moved address + 8,
  and the step still advances the PC by 4 (so + 12 in all). A block transfer in the same situation
  ends with R15 = the moved address + 8 and no further advance.
- SWP reads an R15 Rn or Rm as address + 12.

The ARMv5TE switch to Thumb on a loaded bit 0 is #14; until then ARM9 behaves as ARMv4T here.

### Block transfers: S bit, base in the list, empty list

- **S bit.** LDM with R15 in the list and S = 1 copies the SPSR to the CPSR as R15 loads.
  Otherwise S = 1 makes the list refer to the **User** bank registers, whatever the mode; writeback
  must not be used with it. The suite does exercise S with writeback: the base is read from the
  current mode's register, and the moved base is written to the **User-bank** Rn **(suite)**. (After an LDM into the user bank an immediately following read of a
  banked register may still see the old value.)
- **Base in the list**, with writeback:

  | | ARM7 | ARM9 |
  |---|---|---|
  | STM | the old base if the base is the first listed register, otherwise the new (written-back) base **(suite)** | always the old base |
  | LDM | no writeback; the loaded value stays **(suite)** | writeback happens unless the base is the last register of a list longer than one, so the loaded value stays only in that case |

- **Empty list.** ARM7: R15 is stored (STM) or loaded (LDM), and the base moves by 0x40 (down for
  the decrementing modes, up for the incrementing ones). ARM9: the base moves by 0x40 as well;
  GBATEK implies no transfer of R15. STM with R15 stores address + 12 **(suite)**. The
  one empty-list case the suite holds (STMDB with writeback) stores R15 at the lowest address,
  Rn - 0x40, and writes Rn - 0x40 back **(suite)**.

### ARMv4T vs ARMv5TE

| Behavior | ARMv4T (ARM7) | ARMv5TE (ARM9) |
|---|---|---|
| LDR/LDM into R15 | T unchanged | bit 0 sets T (#14, not yet) |
| Odd-address LDRH | rotated right 8 | aligned, not rotated |
| Odd-address LDRSH | sign-extended byte | sign-extended aligned halfword |
| STM, base in list, writeback | old base only when first | always old |
| LDM, base in list, writeback | none | unless the base is last (and not alone) |
| Empty list | R15 transfer and base ± 0x40 | base ± 0x40 |
| LDRD, STRD, PLD | absent | present (#14, not yet) |

## Edge cases

- A post-indexed or writeback form with Rn = R15, and a halfword word with W = 1 and P = 0, are
  unpredictable in the sources; see Unverified.
- Rd = R15 for a plain load ends with the PC written, so the step does not also advance it.
- The T forms (LDRT, STRT) differ only in the privilege of the access; the bus has no privilege
  yet, so they behave like the plain forms until CP15 protection exists.

## Unverified

Each item has `TODO(hw-unverified)` at the named site in `arm_transfer.rs`.

- ARM9 odd-address LDRH/LDRSH, STM/LDM base-in-list and empty-list rules come from GBATEK's
  prose; the suite is ARMv4T only, so each is checked by a self-written unit test taken from this
  note, not by a trace — `halfword` and `block` handlers — verify with a test ROM or a trace
  against the reference emulator.
- ARM9 LDM with the base last in a list longer than one: GBATEK gives when writeback happens but
  not which value ends in Rn; the loaded value is assumed — `block` handler.
- ARM9 empty list: no R15 transfer is inferred from "ARMv4: R15 is loaded or stored" — `block`.
- Whether an empty list adjusts the base when W = 0 (taken as: only with W = 1) — `block`.
- S = 1 together with writeback, in a block transfer — `write_back`; GBATEK says not to use it.
  The User-bank writeback is the suite's behavior; for an LDM that also loads R15 (the CPSR
  restore form) the suite agrees with it too, and no ARM9 evidence exists.
- Writeback to R15 and SWP with R15 operands — `write_back`, `block_transfer`, `swap`; GBATEK
  allows neither, the suite shows both (see "R15").
- LDM with R15 in the list and S = 1 in User/System, which have no SPSR: the CPSR stays as it is
  and R15 is loaded — `block_transfer`; the suite shows it, GBATEK does not say.
- R15 as the stored Rd of STRH — `halfword`: GBATEK says address + 12 for Rd; the suite checks
  it only for STR, STRB and STM here.
- Rn = R15 with writeback, and a halfword word with P = 0 and W = 1 — `single`, `halfword`.
- STM with S = 1 and R15 in the list stores the user-bank PC value as for plain STM —
  `block_store_value`.
- T-form privilege (LDRT, STRT) — `single`; revisit when CP15 protection lands.
