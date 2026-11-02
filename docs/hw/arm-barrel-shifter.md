# ARM barrel shifter

Sources: GBATEK "ARM Opcodes: Data Processing (ALU)" (operand 2 and the shift-type
paragraphs); ARM ARM "Addressing Mode 1 — Data-processing operands" (background; not re-read for this note).
Used by: `crates/core/src/cpu/shifter.rs`   Phase: 1 (#8; reused by #10 and #13)

Every ARM data-processing instruction and the single-data-transfer register offset pass one
operand through a shifter before the ALU. The shifter yields a 32-bit value and a *carry-out*.
For logical operations with the S bit the carry-out becomes the C flag.

## Operand forms

| Form | Encoding | Shift amount |
|---|---|---|
| Rotated immediate | bit 25 = 1: 8-bit value, 4-bit rotate field | rotate field × 2, rotating right |
| Register, immediate shift | bit 25 = 0, bit 4 = 0 | 5-bit field in bits 11–7 |
| Register, register shift | bit 25 = 0, bit 4 = 1, bit 7 = 0 | low byte of the register in bits 11–8 |

The shift type is bits 6–5: 0 LSL, 1 LSR, 2 ASR, 3 ROR.

## Behavior

Let `v` be the value, `n` the amount, `C` the current carry flag.

- **Rotated immediate.** Result is the 8-bit value rotated right by `2 × field`. If the rotation
  is zero the carry-out is `C`; otherwise it is bit 31 of the result.
- **LSL.** `n = 0`: result `v`, carry-out `C`. `1 ≤ n ≤ 31`: result `v << n`, carry-out is
  the last bit shifted out (bit `32 − n` of `v`). `n = 32`: result 0, carry-out bit 0 of `v`.
  `n > 32`: result 0, carry-out 0.
- **LSR.** Immediate field 0 means **LSR #32**. `1 ≤ n ≤ 31`: result `v >> n`, carry-out bit
  `n − 1` of `v`. `n = 32`: result 0, carry-out bit 31 of `v`. `n > 32`: result 0, carry-out 0.
- **ASR.** Immediate field 0 means **ASR #32**. `1 ≤ n ≤ 31`: arithmetic shift right,
  carry-out bit `n − 1` of `v`. `n ≥ 32`: every bit becomes the sign bit; carry-out is the sign
  bit.
- **ROR.** Immediate field 0 means **RRX**: result `(C << 31) | (v >> 1)`, carry-out bit 0 of
  `v`. For a nonzero immediate or a register amount, rotate right by `n mod 32`; carry-out is
  bit 31 of the result. A register amount that is a nonzero multiple of 32 leaves the value
  unchanged with carry-out bit 31 of `v`.
- **Register-specified amount of zero** (any type): result `v`, carry-out `C`.
- Only the **low byte** of the amount register counts, so 256 behaves as 0.

## Edge cases

- The special meanings of an immediate field of 0 (LSR #32, ASR #32, RRX) apply only to the
  immediate-shift form. A register amount of 0 is "no shift" for every type.
- With a register-specified shift, reading R15 as the shifted register or as Rn gives the
  instruction address + 12 (see `arm-data-processing.md`). This is GBATEK's statement ("PC+12
  if I=0,R=1 (shift by register), otherwise PC+8"); GBATEK also lists Rs as R0–R14 only. The
  ARM ARM (not re-read for this note; the claim comes from the PR #79 review) treats R15 as Rd, Rn,
  Rm or Rs in a register-shifted operand as UNPREDICTABLE, so the +12 rule is GBATEK's reading
  of a case the ARM ARM leaves undefined.
- Arithmetic operations ignore the carry-out of the shifter; they derive C from the ALU.

## Unverified

- Rs = R15 (the amount register is the PC) — GBATEK allows Rs only as R0–R14, and the ARM ARM
  is reported (PR #79 review; not re-read) to leave R15 unpredictable. The suite reads the amount from the **+8** view (the instruction
  address + 8), while Rn and Rm as R15 read +12; 483 cases of `arm_data_proc_register_shift`
  fail if Rs uses +12 too (1,591 passing-condition cases have Rs = R15). — `data_processing`
  in `crates/core/src/cpu/arm.rs` — carries `TODO(hw-unverified)`; verify by a test ROM on real
  hardware.
