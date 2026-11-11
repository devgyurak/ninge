# Hardware divider and square root

Sources: GBATEK "DS Maths" (DIVCNT, DIV_NUMER, DIV_DENOM, DIV_RESULT, DIVREM_RESULT, SQRTCNT,
SQRT_RESULT, SQRT_PARAM).
Used by: `crates/core/src/io/divsqrt/`   Phase: 3 (#135)

Both units belong to the ARM9 only. The ARM7 has no such registers.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_0280 | DIVCNT | 16 | R/W | 0 |
| 0x0400_0290 | DIV_NUMER | 64 | R/W | 0 |
| 0x0400_0298 | DIV_DENOM | 64 | R/W | 0 |
| 0x0400_02A0 | DIV_RESULT | 64 | R | 0 |
| 0x0400_02A8 | DIVREM_RESULT | 64 | R | 0 |
| 0x0400_02B0 | SQRTCNT | 16 | R/W | 0 |
| 0x0400_02B4 | SQRT_RESULT | 32 | R | 0 |
| 0x0400_02B8 | SQRT_PARAM | 64 | R/W | 0 |

The 64-bit registers are read and written as two words, low word first in memory. DIVCNT and
SQRTCNT are 16 bits wide: bits 16-31 of their words are not used (the other halves of the words
read 0 and drop writes). Access in smaller pieces than the register is not described by GBATEK;
see "Unverified".

### DIVCNT bits

| Bits | Meaning |
|---|---|
| 0-1 | Mode: 0 = 32-bit / 32-bit, 1 = 64-bit / 32-bit, 2 = 64-bit / 64-bit, 3 = reserved, acts as mode 1 |
| 2-13 | Not used |
| 14 | Division by zero (read only): set when the full 64-bit DIV_DENOM is zero |
| 15 | Busy (read only) |

### SQRTCNT bits

| Bits | Meaning |
|---|---|
| 0 | Mode: 0 = 32-bit input, 1 = 64-bit input |
| 1-14 | Not used |
| 15 | Busy (read only) |

## Behavior

**Starting.** A write to DIVCNT, DIV_NUMER or DIV_DENOM starts a division. A write to
SQRTCNT or SQRT_PARAM starts a square root. A write to a result register changes nothing and
starts nothing, and the busy and flag bits of the control registers cannot be written.

**Duration.** The units count in 33.51 MHz units, which is one bus cycle, two ARM9 cycles
(`scheduler.md`, "One master clock"). Division takes 18 bus cycles in mode 0 and 34 in modes 1
and 2; the square root takes 13 in both modes. The busy bit is set from the start until that
time has passed.

**Division.** Operands are signed. Mode 0 uses the low 32 bits of numerator and denominator as
signed 32-bit values and returns a quotient and a remainder that are 32-bit values sign-extended
to 64 bits. Mode 1 divides the signed 64-bit numerator by the low 32 bits of the denominator,
taken as a signed 32-bit value; the quotient is 64-bit and the remainder is a 32-bit value
sign-extended to 64 bits. Mode 2 divides 64 bits by 64 bits. The quotient is truncated toward
zero and the remainder takes the numerator's sign (`numer = quotient * denom + remainder`).

**Overflows** happen on a zero denominator and on the most negative number divided by -1.

- Zero denominator: the remainder is the numerator, and the quotient is +1 when the numerator
  is negative and -1 otherwise ("opposite sign"). The DIVCNT bit 14 flag is set only when the
  full 64-bit denominator is zero, even in mode 0.
- Most negative / -1: the quotient is the most negative number (not the positive one) and the
  remainder is 0.
- In mode 0, after either overflow the upper 32 bits of the sign-extended quotient are
  inverted. For the most negative / -1 this turns the wrong 32-bit result into the correct
  positive 64-bit one (`80000000h / FFFFFFFFh` gives `0000_0000_8000_0000h`). For a zero
  denominator with a non-negative numerator it gives `0000_0000_FFFF_FFFFh`, and with a
  negative numerator `FFFF_FFFF_0000_0001h`.

**Square root.** SQRT_PARAM is unsigned. Mode 0 uses its low 32 bits, mode 1 all 64. The result
is the integer square root as a 32-bit value (the rounding, down, is inferred: see "Unverified").

**Reading.** DIVCNT and SQRTCNT read the stored mode bits, the flag, and the busy bit as of the
time of the read.

## Edge cases

- Software that writes the numerator as two words starts a division after each write; the
  result belongs to the last one.
- Mode 3 is not an error; it divides as mode 1.
- In mode 0 the upper 32 bits of both operands are ignored, except that the zero-denominator
  flag looks at all 64 denominator bits: a denominator whose low half is zero and whose upper
  half is not leaves the flag clear.
- The ARM7 reads and writes `0400_0280h`-`0400_02BFh` as unmapped I/O (0, writes dropped).

## Modeling choice

The result is computed when the operation starts and is visible at once; only the busy bit
follows the duration. The busy bit is a function of the time of the read, so no scheduler event
is needed and nothing is saved beyond the registers and the end time. Software that waits for
the busy bit to clear before reading a result sees the same values as on hardware. This is a
choice, not a hardware fact: see "Unverified".

## Unverified

Code sites are in `crates/core/src/io/divsqrt/mod.rs`; each carries `TODO(hw-unverified)`.

- What a result register returns when read while the unit is busy — GBATEK does not say; the
  finished value here — `DivSqrt::byte` — verify by a test ROM that reads the result before the
  busy bit clears.
- Reset values of all registers (all zero here) — `DivSqrt::new`.
- Whether DIVCNT bits 0-1 read back as written when the mode is 3 — `DivSqrt::divcnt`.
- A denominator whose low half is zero and whose upper half is not, in modes 0 and 1: the
  division is a division by zero without the flag here. Mode 0's flag is sourced (GBATEK: the
  flag looks at the full 64-bit denominator); the quotient and remainder are not, and in mode 1
  neither the flag nor the results are — `divide`, `DivSqrt::start_division`.
- The remainder of a zero-denominator division in mode 1 (the numerator is 64 bits and the
  remainder 32): the numerator's low 32 bits sign-extended here — `divide`.
- The quotient of 0 / 0 (taken as a non-negative numerator, so -1) — `signed_divide`.
- Whether a new start while the unit is busy restarts the count, and whether a write that does
  not change the operands (the same value again) does — `DivSqrt::start_division`,
  `DivSqrt::start_sqrt` — verify by a test ROM that starts a second operation mid-way and times
  the busy bit.
- That the square root is rounded down (GBATEK gives only the unsigned 64-bit parameter and the
  unsigned 32-bit result) — `square_root` — verify by a test ROM on a non-square input.
- Byte and halfword accesses to the registers, and which bytes of a register count as a write
  to it (a write to the unused upper half of a DIVCNT or SQRTCNT word starts nothing here) —
  `Device::write` — verify by a test ROM with narrow writes to each register.
