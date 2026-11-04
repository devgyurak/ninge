# Thumb instruction set (ARMv4T)

Sources: GBATEK "THUMB Instruction Set" and its format pages (THUMB.1 to THUMB.19: "Move
Shifted Register", "Add/Subtract", "Move/Compare/Add/Subtract Immediate", "ALU Operations",
"Hi Register Operations/Branch Exchange", "PC-Relative Load", "Load/Store with Register
Offset", "Load/Store Sign-Extended Byte/Halfword", "Load/Store with Immediate Offset",
"Load/Store Halfword", "SP-Relative Load/Store", "Load Address", "Add Offset to Stack
Pointer", "Push/Pop Registers", "Multiple Load/Store", "Conditional Branch", "Software
Interrupt", "Unconditional Branch", "Long Branch with Link"); GBATEK "ARM CPU Register Set"
(R15 in Thumb state). Measured facts come from the pinned SingleStepTests/ARM7TDMI suite (MIT)
as oracle evidence, not hardware evidence.
Used by: `crates/core/src/cpu/thumb_decode.rs`, `crates/core/src/cpu/thumb.rs`
(`step_thumb`), `crates/core/src/cpu/mod.rs` (`Cpu::step`)   Phase: 1 (#13)

Out of scope here: the ARMv5 Thumb forms (the two BLX encodings, `POP {PC}` switching state on
the ARM9, #14); cycle timing; the disassembler (#16). Registers and modes are in
`cpu-registers.md`; the pipeline, B/BX and the refill are in `arm-branch-pipeline.md`;
exception entry is in `arm-exceptions-modes.md`.

## State and registers

A Thumb instruction is 16 bits. The CPU is in Thumb state while CPSR bit T is 1. The
pipeline, the step and the refill work as in `arm-branch-pipeline.md` with halfwords: while
an instruction executes, R15 is its address + 4; a step that does not write R15 advances it
by 2; a write to R15 refills from the written value with its bit 0 dropped.

Most instructions reach only R0-R7 (the "low" registers). The exceptions are the hi-register
instructions (format 5, R8-R15), PUSH (adds LR), POP (adds PC), the SP-relative forms (R13),
the PC-relative forms (R15) and BL (LR). Most Thumb data-processing instructions set the
flags (the exceptions are listed under "Flags"); none of them has a condition field, only the
conditional branch does.

## Formats

The format is read from the top bits of the halfword. `Rd`, `Rs`, `Rn`, `Rb`, `Ro` are 3-bit
register fields unless stated.

| # | Bits 15-8 (pattern) | Fields | Instruction |
|---|---|---|---|
| 1 | `000 op(2) off5` | off5 bits 10-6, Rs 5-3, Rd 2-0 | LSL, LSR, ASR by an immediate (op 0-2) |
| 2 | `00011 I op` | bits 8-6 Rn or imm3, Rs 5-3, Rd 2-0 | ADD (op 0) or SUB (op 1) of a register (I = 0) or a 3-bit immediate (I = 1) |
| 3 | `001 op(2) Rd` | imm8 bits 7-0 | MOV, CMP, ADD, SUB with an 8-bit immediate (op 0-3) |
| 4 | `010000 op(4)` | op bits 9-6, Rs 5-3, Rd 2-0 | AND, EOR, LSL, LSR, ASR, ADC, SBC, ROR, TST, NEG, CMP, CMN, ORR, MUL, BIC, MVN (op 0-15) |
| 5 | `010001 op(2) H1 H2` | op bits 9-8, H1 bit 7, H2 bit 6, Rs 5-3, Rd 2-0 | ADD, CMP, MOV on R0-R15 (op 0-2); BX (op 3) |
| 6 | `01001 Rd` | imm8 | LDR Rd, [PC, #imm8 * 4] |
| 7 | `0101 L B 0` | Ro 8-6, Rb 5-3, Rd 2-0 | STR, STRB, LDR, LDRB with a register offset |
| 8 | `0101 H S 1` | Ro, Rb, Rd as above | STRH (H S = 00), LDSB (01), LDRH (10), LDSH (11) with a register offset |
| 9 | `011 B L off5` | off5 bits 10-6, Rb 5-3, Rd 2-0 | STR, LDR (offset * 4), STRB, LDRB (offset * 1) with an immediate offset |
| 10 | `1000 L off5` | off5 bits 10-6, Rb, Rd | STRH, LDRH, offset * 2 |
| 11 | `1001 L Rd` | imm8 | STR, LDR Rd, [SP, #imm8 * 4] |
| 12 | `1010 S Rd` | imm8 | ADD Rd, PC or SP, #imm8 * 4 (S = 1: SP) |
| 13 | `10110000 S` | imm7 bits 6-0 | ADD SP, #imm7 * 4 (S = 0) or SUB SP, #imm7 * 4 (S = 1) |
| 14 | `1011 L 10 R` | rlist bits 7-0 | PUSH (L = 0, R adds LR), POP (L = 1, R adds PC) |
| 15 | `1100 L Rb` | rlist bits 7-0 | STMIA, LDMIA Rb!, {rlist} |
| 16 | `1101 cond` | soff8 | B cond, target = R15 + soff8 * 2 (cond 0000-1110) |
| 17 | `11011111` | value8 | SWI |
| 18 | `11100` | off11 | B, target = R15 + off11 * 2 |
| 19 | `1111 H` | off11 | BL as two halfwords (below) |

Condition 1111 of format 16 is SWI (format 17). Format 16 uses the same conditions as ARM
(`cpu-registers.md`, "Condition codes"), including 1110, "always"; the instruction-set
documentation sets that case aside as undefined, but the suite shows the ARM7TDMI taking it as an unconditional
branch (see "Unverified"). `off8` and `off11` are signed, sign-extended, then doubled.

## Behavior

### Flags

- **Formats 1 and 4 shifts.** N and Z come from the result; C is the shifter's carry-out;
  V is unchanged. The immediate form follows the ARM rules for the amount (`arm-barrel-shifter.md`):
  LSL #0 keeps C and the value; LSR #0 and ASR #0 mean a shift by 32. A register-specified shift
  uses the low byte of Rs, and an amount of 0 leaves the value and C alone.
- **Formats 2 and 3, ADD, SUB, CMP, CMN, NEG, ADC, SBC.** N, Z, C, V all update. C is "not
  borrow" for subtraction. ADC and SBC take C as the carry in.
- **Format 3 MOV, and the logical operations (AND, EOR, ORR, BIC, MVN, TST).** N and Z update;
  C and V are unchanged (no shift, so no carry-out).
- **ROR by register.** The amount is the low byte of Rs. Rotating by a multiple of 32 other than 0
  leaves the value and sets C to its bit 31; an amount of 0 changes nothing.
- **NEG.** Rd = 0 - Rs, with the flags of that subtraction.
- **MUL.** Rd = Rd * Rs (low 32 bits); N and Z update. C and V are left unchanged here (see
  "Unverified").
- **Format 5.** ADD and MOV set no flags; CMP sets N, Z, C, V like a normal CMP.
- **Formats 6 to 15 and 18 to 19** set no flags.

### Format 5 and BX

- Rd is `H1 : Rd` and Rs is `H2 : Rs`, so either side can name R8-R15. Reading R15 gives the
  instruction's address + 4.
- ADD Rd, Rs: Rd = Rd + Rs. MOV Rd, Rs: Rd = Rs. CMP Rd, Rs: flags of Rd - Rs, no write.
- A write to R15 by ADD or MOV flushes the pipeline. The target's bit 0 is dropped for the
  fetches; the state stays Thumb.
- BX Rs (op 3): the target is Rs (R15 reads the instruction's address + 4). Bit 0 of the
  target is copied to T: 1 stays in or enters Thumb state, 0 returns to ARM state. The
  refill is in the state BX selected, as in `arm-branch-pipeline.md`.
- GBATEK: BX R15 auto-aligns the PC as `(address + 4) AND NOT 2` and should be issued from a
  word-aligned address only; bits 2-0 of the instruction (Rd) must be zero. We read R15 as
  address + 4 without aligning it and ignore the Rd bits (see "Unverified").
- With H1 = 1 and op 3 the encoding is BLX (register) on ARMv5, which is #14's. On ARMv4T the
  suite shows it running as a plain BX (see "Unverified").

### PC-relative forms

- **Format 6 (LDR Rd, [PC, #imm])** and **format 12 (ADD Rd, PC, #imm)** use the word-aligned
  PC: `(address + 4) & !2`, so bit 1 of the PC is ignored. The 8-bit immediate is a word offset
  (x 4).
- The loaded word (format 6) is at an aligned address, so it needs no rotation.
- SP-relative forms (formats 11 and 12 with S = 1) use SP as it is.

### Loads and stores (formats 7-11)

Alignment follows the ARM rules (`arm-load-store.md`, "Alignment"): the bus gets the raw
address; LDR rotates the word right by 8 x (address & 3); LDRH at an odd address on ARM7
rotates the halfword by 8; LDSH at an odd address on ARM7 sign-extends the high byte; a store
writes its register unrotated. LDSB and LDSH sign-extend the byte or halfword; LDRB zero-extends.
The immediate offsets are scaled by the access size (4 for words, 2 for halfwords, 1 for
bytes). No Thumb transfer writes the base back.

### Format 13

SP = SP + imm7 x 4 (S = 0) or SP - imm7 x 4 (S = 1). No flags.

### PUSH and POP (format 14)

- PUSH stores the listed registers (and LR if R is set) to the stack, full-descending: SP
  decreases by 4 x the number of registers first, then the registers are stored in ascending
  register order at ascending addresses from the new SP. LR, if listed, is the highest.
- POP loads the listed registers (and PC if R is set) from ascending addresses starting at SP,
  then SP increases by 4 x the number of registers. PC, if listed, is the highest.
- A POP that loads PC flushes the pipeline. On ARMv4T the loaded value's bit 0 is ignored and
  the state stays Thumb; the ARM9's state switch is #14's.
- An empty list with R = 0 follows the multiple load/store rule below.

### LDMIA and STMIA (format 15)

- The base is always written back after the transfer: `Rb = Rb + 4 x count`.
- Registers transfer in ascending order at ascending addresses starting at the old Rb.
- **Rb in the list, STMIA.** On the ARM7, if Rb is the first register of the list the old base
  is stored; otherwise the new (written-back) base is stored. On the ARM9 the old base is
  always stored.
- **Rb in the list, LDMIA.** There is no write-back and the loaded value wins, on the ARM7 and
  on the ARM9 alike: GBATEK notes that Thumb differs from the ARM form here (where the ARM9
  writes back unless Rb is the last register of a list longer than one).
- **Empty list.** The hardware transfers R15 (ARM7; nothing on the ARM9) and moves the base
  by 0x40. A store writes the instruction's address + 6 for R15 (a Thumb instruction's late
  R15 read is one halfword past the usual +4). A load puts the word in R15 and the pipeline
  refills from it with bit 0 dropped for the fetches; R15 itself ends as the loaded value + 4
  with bit 0 kept (see "Unverified").

### Conditional branch (format 16)

If the condition holds, R15 = R15 + sign-extended(off8) x 2 and the pipeline refills. If not,
nothing happens but the normal advance by 2. The range is -256 to +254 bytes.

### Unconditional branch (format 18)

R15 = R15 + sign-extended(off11) x 2; the range is -2048 to +2046 bytes.

### Long branch with link (format 19)

BL is two halfwords that must run in order, each executed as its own instruction:

1. **Prefix** (`11110 off11`): LR = R15 + (sign-extended(off11) << 12). R15 here is the
   prefix's address + 4. No branch happens.
2. **Suffix** (`11111 off11`): the target is LR + (off11 << 1) (the offset unsigned, bit 0
   zero); R15 = target, and LR = (the suffix's address + 2) | 1, i.e. the address of the next
   instruction with bit 0 set, so a later `BX LR` returns in Thumb state.

The pair reaches +-4 MiB. A suffix on its own uses whatever LR holds. `11101 off11` is the
ARMv5 BLX suffix and is #14's.

### SWI and the undefined encoding

- **SWI (format 17).** Enters Supervisor mode through the vector at 0x08; the CPSR is saved in
  SPSR_svc, T is cleared, I is set; LR_svc = the SWI's address + 2 (the next Thumb instruction,
  so one return sequence serves both states: `arm-exceptions-modes.md`, "Return address").
- **Condition 1110 of format 16** is documented as undefined, but the suite shows the ARM7TDMI taking it as a
  branch whose condition always holds, so it is format 16 with a true condition (see
  "Unverified"). No Thumb encoding enters the Undefined exception on ARMv4T here.

## Edge cases

- `MOV PC, Rx` and `ADD PC, Rx` flush; `MOV R8, R9` does not.
- A POP or hi-register write that loads an odd value into PC does not change state on ARMv4T:
  only BX changes T.
- `LDR Rd, [PC, #imm]` at an address with bit 1 set (a halfword-aligned instruction) still
  uses the word-aligned PC, so it reads the same word as the instruction before it would.
- A conditional branch whose condition fails leaves everything but R15 (+2) alone.
- BL's prefix alone changes only LR; a suffix without a prefix jumps to LR + offset.

## Unverified

Each item is backed by the JSON suite as oracle evidence only; where GBATEK is silent the code
site carries `TODO(hw-unverified)`. Verify by a test ROM on real hardware or a trace against the
reference emulator.

- **Format 5 with H1 = H2 = 0 for ADD, CMP and MOV.** GBATEK marks the low-low combination as
  undefined on ARMv4T; what hardware does is taken from the suite. — `hi_register` in
  `thumb.rs`.
- **Format 5 BX with H1 = 1.** The suite shows BX with H1 = 1 and op 3 running as BX on the
  register `H2 : Rs`, H1 ignored (128 halfwords, 25,102 cases). That encoding is BLX (register)
  on ARMv5; the ARM9 differs (#14). — `thumb_decode.rs`, `branch_exchange_thumb`.
- **BX R15 and BX with non-zero Rd bits.** The alignment of R15 and the should-be-zero bits are
  not modeled (GBATEK states the alignment and says to issue it from a word-aligned address).
  — `branch_exchange_thumb` in `thumb.rs`.
- **Format 16 with condition 1110.** The suite shows a branch that is always taken, with the
  usual target and no exception (50,000 cases); the documentation calls the encoding
  undefined. The suite is ARMv4T only, so the ARM9 may differ (it may take the Undefined
  exception). — `thumb_decode.rs`.
- **MUL's C and V.** C after MUL is "meaningless" per GBATEK on ARMv4T and the suite shows it
  changing (the same gap as ARM multiply, #81); V is unchanged. We leave C unchanged. The one
  suite file this affects, `thumb_data_proc`, is pinned by a test that expects only MUL's C bit
  to differ. — `alu_register` in `thumb.rs`.
- **Empty register list in PUSH, POP, STMIA and LDMIA.** GBATEK states the R15 transfer and the
  0x40 base move for LDMIA and STMIA; the suite shows PUSH and POP doing the same. The
  stored R15 value is the instruction's address + 6 (suite; ARM's late read is +12, Thumb's is
  +6), and a loaded R15 leaves R15 = the value + 4 with the value's bit 0 kept, whereas a
  POP {PC} with a non-empty list clears bit 0 (suite). — `store_operand` in `arm.rs`,
  `step_thumb`.
- **Encodings with no Thumb meaning on ARMv4T** besides the 1110 conditional branch: the
  `1011` space outside formats 13 and 14 (later architectures give it extend, hint and BKPT
  meanings) and the BLX suffix `11101`. They run as a plain step here, which is a guess about
  hardware. — `thumb_decode.rs`.
- **Fetch transactions.** The step fetches one halfword at R15 before executing and a refill
  fetches two, in 16-bit width; this matches the suite and is oracle evidence only, as in
  `arm-branch-pipeline.md`. — `step_thumb`, `refill`.
