# ARM data processing and PSR transfer

Sources: GBATEK "ARM Opcodes: Data Processing (ALU)"; GBATEK "ARM Opcodes: PSR Transfer
(MRS, MSR)"; GBATEK "ARM CPU Flags & Condition Field (cond)"; ARM ARM "Data-processing
instructions", "MRS", "MSR" (listed as background; not re-read for this note).
Used by: `crates/core/src/cpu/` (`alu.rs`, `psr.rs`, `arm.rs`)   Phase: 1 (#8)

Shifter details are in `arm-barrel-shifter.md`; registers and modes in `cpu-registers.md`.

## Encoding

Data processing: `cond 00 I opcode S Rn Rd operand2`. Bit 25 (I) selects the rotated
immediate; otherwise operand 2 is a shifted register (bit 4 picks immediate or register
shift). When bit 25 = 0, bit 4 = 1 and bit 7 = 1 the word is not data processing (multiplies,
halfword transfers).

| Opcode | Mnemonic | Result | Writes Rd |
|---|---|---|---|
| 0 | AND | Rn & Op2 | yes |
| 1 | EOR | Rn ^ Op2 | yes |
| 2 | SUB | Rn − Op2 | yes |
| 3 | RSB | Op2 − Rn | yes |
| 4 | ADD | Rn + Op2 | yes |
| 5 | ADC | Rn + Op2 + C | yes |
| 6 | SBC | Rn − Op2 − 1 + C | yes |
| 7 | RSC | Op2 − Rn − 1 + C | yes |
| 8 | TST | Rn & Op2 | no |
| 9 | TEQ | Rn ^ Op2 | no |
| A | CMP | Rn − Op2 | no |
| B | CMN | Rn + Op2 | no |
| C | ORR | Rn \| Op2 | yes |
| D | MOV | Op2 | yes |
| E | BIC | Rn & !Op2 | yes |
| F | MVN | !Op2 | yes |

MOV and MVN ignore Rn. TST, TEQ, CMP and CMN always set flags; their S bit is 1 in every
valid encoding (S = 0 there is the PSR-transfer space, below).

## Flags (S = 1, Rd ≠ R15)

- **N** = bit 31 of the result. **Z** = result is zero.
- **Logical operations** (AND, EOR, TST, TEQ, ORR, MOV, BIC, MVN): C = shifter carry-out;
  V unchanged.
- **Arithmetic operations** (SUB, RSB, ADD, ADC, SBC, RSC, CMP, CMN): C and V come from the
  addition or subtraction. For addition, C = unsigned carry out of bit 31; V = signed overflow.
  For subtraction, C = **not borrow** (1 when the minuend ≥ the subtrahend, unsigned),
  computed with the carry-in where the opcode has one (SBC, RSC: `a − b − 1 + C`). V = signed
  overflow of that subtraction. The shifter carry-out is ignored.
- Without S, flags are untouched.

## Reading R15

An instruction reads R15 as its own address + 8 in ARM state. If the second operand is a
register shifted **by a register**, the extra internal cycle makes every R15 read (Rn or Rm)
give address + 12.

## Writing R15 with S

- `S = 1`, Rd = R15, opcode not TST/TEQ/CMP/CMN, in a mode with an SPSR: the result is written
  to R15 and **CPSR ← SPSR of the current mode** (the exception-return form). The mode, and
  with it the register bank, switches as the SPSR dictates.
- `S = 1`, Rd = R15 in User/System (modes without an SPSR): GBATEK says it should not be
  used in User mode; see Unverified.
- TST/TEQ/CMP/CMN write no register, so with Rd = R15 R15 itself is not written. See
  Unverified for what the S bit does there.

## PSR transfer

`MRS Rd, CPSR|SPSR` — `cond 00010 R 001111 Rd 000000000000`; R = 1 selects SPSR. Rd receives
the whole register.

`MSR CPSR|SPSR_fields, Rm` — `cond 00010 R 10 mask 1111 00000000 Rm`;
`MSR … , #imm` — `cond 00110 R 10 mask 1111 imm12` (rotated immediate, as in data
processing). The 4-bit `mask` in bits 19–16 selects which bytes of the PSR are written:
bit 19 = `f` (bits 31–24), bit 18 = `s` (23–16), bit 17 = `x` (15–8), bit 16 = `c` (7–0).
Unselected bytes keep their value.

- In **User mode** only the `f` byte of the CPSR can change; the other mask bits are ignored.
  The control byte (mode, T, I, F) is writable only in privileged modes.
- SPSR writes: allowed in modes that have an SPSR.
- Bit 5 (T) is part of the control byte. GBATEK says software should not change it with MSR.
- Writing mode bits switches the register bank (`cpu-registers.md`).

## Edge cases

- A failed condition makes the instruction a no-op that still advances the PC.
- ADC/SBC/RSC use the C flag as it is before the instruction.
- `MOV R15, …` without S is a plain branch; the low bits of the value are not aligned by the
  data-processing instruction itself (the state change belongs to BX, #11).

## Unverified

Each item is backed by the JSON suite as *oracle evidence* (a program that models the same
chip), not by hardware.

- Low two bits of a value written to R15 by an ALU result, and how R15 reads after that —
  the suite keeps them as written. The pipeline is #11. — `write_pc` in
  `crates/core/src/cpu/arm.rs` — carries `TODO(hw-unverified)`; verify by a test ROM on
  real hardware.
- TST/TEQ/CMP/CMN with S = 1 and Rd = R15 in a mode with an SPSR — GBATEK limits the
  `{P}` forms (TSTP, TEQP, CMPP, CMNP) to ARMv2 and below, yet the suite restores
  CPSR ← SPSR (the old `TEQP` form) and leaves R15 alone. — `data_processing` in `arm.rs` —
  carries `TODO(hw-unverified)`.
- MSR writing the CPSR control byte — the suite forces bit 4 (M4) to 1 in the result for
  every written value; this is also why a written value like `0x1A` reads back `0x1A | 0x10`.
  — `msr` in `arm.rs` — carries `TODO(hw-unverified)`.
- MSR changing the T bit from a privileged mode — the suite shows cases where it changes;
  the code lets the control byte through unfiltered. — `msr` in `crates/core/src/cpu/arm.rs` —
  carries `TODO(hw-unverified)`.
- MSR writing mode bits that match no mode — the suite leaves every register unchanged and
  keeps the written bits (matches `cpu-registers.md`). — `Registers::set_cpsr` — carries
  `TODO(hw-unverified)`.
- Rs = R15 in a register-shift operand — see `arm-barrel-shifter.md`.
- Rm = R15 in MSR (register form) — GBATEK does not define it (the ARM ARM was not re-read for this note; the claim comes from the PR #79 review). The
  suite reads the +8 view: all 784 passing-condition CPSR cases match it, and 286 of them (the
  ones writing the control byte) would differ with +12. — `msr` in `arm.rs` — carries
  `TODO(hw-unverified)`.
- Behavior once the mode bits name no mode (after the case above): the SPSR selected by MRS,
  MSR and the S + R15 restore follows the bank that was kept, and `is_user` treats such a
  mode as privileged. **No suite evidence**: none of the 300,000 cases in the six files starts
  in such a mode, so this is inference. — `data_processing`, `mrs`, `msr` in `arm.rs` and
  `is_user` in `psr.rs` — carry `TODO(hw-unverified)`; see also `cpu-registers.md`.
- `S = 1`, Rd = R15 in User/System (no SPSR) — the suite behaves as an ordinary flag-setting
  op: flags from the result, control bits untouched, no SPSR restore. — `arm.rs` — carries
  `TODO(hw-unverified)`.
- MRS with R = 1 (SPSR) in User/System — the suite returns the CPSR (4,404 cases). —
  `mrs` in `arm.rs` — carries `TODO(hw-unverified)`. SPSR reads/writes in `Registers` stay as #76
  defined them.
- MRS with Rd = R15 (unpredictable per the PR #79 review of the ARM ARM; the ARM ARM was not re-read for this note) — the suite ends with R15 = value + 4:
  an ordinary register write followed by the normal advance, without the +8 of an ALU PC
  write (1,580 cases). — `arm.rs` — carries `TODO(hw-unverified)`.
- MSR to SPSR in User/System — ignored. — `msr` in `arm.rs` — carries `TODO(hw-unverified)`.
