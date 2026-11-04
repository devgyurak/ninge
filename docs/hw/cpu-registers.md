# CPU registers and modes

Sources: GBATEK "ARM CPU Register Set"; GBATEK "ARM CPU Flags & Condition Field (cond)";
GBATEK "ARM CPU Exceptions".
Used by: `crates/core/src/cpu/` (#7, #76)   Phase: 1

The ARM7TDMI (ARM7) and the ARM946E-S (ARM9) share one register model. This note covers the
register file, the operating modes, and the status registers. Instruction semantics belong to
their own notes (#8–#13).

## Registers

There are 31 general registers and 6 status registers. Which of them a program sees depends on
the current mode.

| Visible name | Shared by | Own copy in |
|---|---|---|
| R0–R7 | every mode | — |
| R8–R12 | every mode except FIQ | FIQ (R8_fiq–R12_fiq) |
| R13 (SP), R14 (LR) | User and System share one pair | FIQ, Supervisor, Abort, IRQ, Undefined each have their own pair |
| R15 (PC) | every mode | — |
| CPSR | every mode | — |
| SPSR | — | FIQ, Supervisor, Abort, IRQ, Undefined (User and System have none) |

So FIQ banks R8–R14 (seven registers), and each of the other four exception modes banks
R13–R14 (two registers). Writing R0 in one mode changes R0 in every mode; writing R14 changes
only the current mode's R14.

### Roles

- R13 is the stack pointer by convention (in Thumb state it is the stack pointer for the
  instructions that use one). Each exception handler needs its own stack, which the banking
  provides.
- R14 receives the return address on a branch with link and when an exception is taken.
- R15 is the program counter. Reading it usually gives the instruction address plus an offset
  because of the pipeline; the offset depends on the instruction and the state (ARM or Thumb); `arm-branch-pipeline.md`
  models it.
  In Thumb state only R0–R7 are freely accessible.

### CPSR bits

| Bits | Name | Meaning |
|---|---|---|
| 31 | N | negative (sign) flag |
| 30 | Z | zero flag |
| 29 | C | carry flag (not-borrow for subtraction) |
| 28 | V | overflow flag |
| 27 | Q | sticky overflow, ARMv5TE and up only; set by QADD, QSUB, QDADD, QDSUB, SMLAxy and SMLAWy when they overflow, left unchanged otherwise, and cleared only by an MSR |
| 7 | I | IRQ disable (1 = disabled) |
| 6 | F | FIQ disable (1 = disabled) |
| 5 | T | state: 0 = ARM, 1 = Thumb; changed by the BX family, not by writing the register |
| 4–0 | M | mode |

Bits 26–8 are reserved. GBATEK says not to rely on their value.

### Mode bits

| M4–M0 | Mode | Banked SPSR |
|---|---|---|
| 10000 (0x10) | User | none |
| 10001 (0x11) | FIQ | SPSR_fiq |
| 10010 (0x12) | IRQ | SPSR_irq |
| 10011 (0x13) | Supervisor | SPSR_svc |
| 10111 (0x17) | Abort | SPSR_abt |
| 11011 (0x1B) | Undefined | SPSR_und |
| 11111 (0x1F) | System (privileged User; ARMv4 and up) | none |

The old 26-bit modes (M4 = 0) exist only on ARMv3 and some non-Thumb ARMv4 parts and are not
modeled. GBATEK says any other value must not be written to the mode bits.

### Condition codes

An ARM instruction runs only if its 4-bit condition holds against N, Z, C, V: 0 EQ (Z=1),
1 NE (Z=0), 2 CS (C=1), 3 CC (C=0), 4 MI (N=1), 5 PL (N=0), 6 VS (V=1), 7 VC (V=0),
8 HI (C=1 and Z=0), 9 LS (C=0 or Z=1), A GE (N=V), B LT (N≠V), C GT (Z=0 and N=V),
D LE (Z=1 or N≠V), E AL (always), F reserved (never on ARMv1/v2). In Thumb state only branch
instructions are conditional.

## Behavior

- On exception entry the CPU copies CPSR into the SPSR of the mode it enters, then switches
  mode. There is one SPSR per mode, so a nested exception in the same mode must save the SPSR
  first.
- A reset enters Supervisor mode with I = 1 and F = 1 and clears T (ARM state). The registers
  R0–R14 and the flags N, Z, C, V are not stated for a reset.
- Changing the mode swaps which physical registers R8–R14 (FIQ) or R13–R14 (the others) the
  program sees; the values of the mode being left are kept and are visible again on return.

## Edge cases

- User and System mode share the same registers and neither has an SPSR; reading an SPSR in
  those modes is not meaningful.

## Unverified

- Mode bits that match none of the rows above (including the 26-bit values): GBATEK only says
  not to write them. The emulator keeps the written bits in CPSR and leaves the register bank
  as it was. What the instructions do *from* such a mode (SPSR choice, User-mode check) is
  inference with no suite evidence; see `arm-data-processing.md`, "Unverified". —
  `crates/core/src/cpu/` — verify by a test ROM on real hardware or a trace
  against the reference emulator; carries `TODO(hw-unverified)`.
- What an SPSR read gives in User/System mode — `Registers` exposes no SPSR there. The MRS
  handler reads the CPSR instead, which is what the JSON suite shows (see
  `arm-data-processing.md`, "Unverified"). — `Registers::spsr` and `Registers::set_spsr` in `crates/core/src/cpu/regs.rs`, which
  carry `TODO(hw-unverified)` — verify by a test ROM on real hardware.
- R0–R14 and N, Z, C, V after a reset: GBATEK "ARM CPU Exceptions" fixes the mode, I, F and
  T but not these. The emulator starts them at zero. — `crates/core/src/cpu/regs.rs`
  (`POWER_ON_CPSR`) — verify by a test ROM on real hardware or a trace against the reference
  emulator; carries `TODO(hw-unverified)`.
