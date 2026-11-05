# Direct boot (ARM9)

Sources: GBATEK "DS Cartridge Header" (header copy); GBATEK "NDS BIOS RAM Usage" (memory used at
cartridge boot time, register settings at cartridge boot time); GBATEK "DS Memory Control -
Cache and TCM"; GBATEK "DS Display Dimensions / Timings"; GBATEK "DS Memory Timings".
Used by: `crates/core/src/hle/direct_boot.rs`, `crates/core/src/lib.rs` (`load_rom`,
`run_frame`)   Phase: 2 (#17), ARM9 only

ninge boots without BIOS or firmware: it puts the machine in the state the BIOS and firmware
would leave at the start of the cartridge's code and starts the ARM9 there. Nothing here comes
from a BIOS or firmware dump; the values are GBATEK's description of the state.

## Placement

1. Reset the machine to power-on (a host contract, not hardware).
2. Parse the header (`cart-header.md`); a refused header stops here and leaves the power-on
   state with no program.
3. Copy the first `170h` bytes of the image to `027F_FE00h` (the header copy). GBATEK says the
   header is loaded to `027FFE00h` at power-up and the `170h` bytes after offset `0` are the part
   that is stored. Main RAM mirrors, so this is the top of the 4 MiB (`arm9-memory-map.md`). No
   binary can reach it: a binary ends at `023B_FE00h` at the latest.
4. Copy the ARM9 binary: `size` bytes from its ROM offset to its RAM address, in main RAM.
5. Copy the ARM7 binary the same way, only when its RAM address is in main RAM. An ARM7
   destination in the `037F_8000h` range is accepted but not placed (`TODO(phase3)`, #22).
   Nothing executes the ARM7 in Phase 2.

All other memory is zero: GBATEK's summary of memory at cartridge boot says everything not
holding cartridge data is zero-filled. The values the BIOS and firmware write into `027FF800h`-
`027FFC41h`, the user settings and the firmware boot-code fragments are not modeled
(`TODO(phase3)`: shared state and IPC need them). Binaries go through `Memory::write8`, so
they land only in main RAM.

## ARM9 state at entry

GBATEK, "register settings at cartridge boot time":

| Register | Value |
|---|---|
| R0-R11 | 0 |
| R12, R14, R15 | the entry address |
| R13 (the running mode) | `0300_2F7Ch` |
| R13_irq | `0300_3F80h` |
| R13_svc | `0300_3FC0h` |
| R14_irq, SPSR_irq, R14_svc, SPSR_svc | 0 |

The ARM7's values are `0380_FD80h`, `0380_FF80h`, `0380_FFC0h` for the running mode, IRQ and
Supervisor stacks, with the same zeros (Phase 3).

The stack pointers lie in shared WRAM, which Phase 2 does not map (`arm9-memory-map.md`); they
are only register values here.

GBATEK does not give the processor mode, the I and F bits or the T bit at entry. The running
mode's R13 is listed apart from the IRQ and Supervisor ones, so the mode is neither of them.
**Chosen so the machine can run:** System mode (`1Fh`), ARM state, I and F set (interrupts
disabled), flags clear; CPSR = `0000_00DFh`. See "Unverified".

Registers of the other banks (FIQ, Abort, Undefined) are not stated and stay at power-on zero.

### The pipeline

The CPU starts like a branch to the entry address: it fetches the opcode at the entry address and
the one after it, so `pipeline[0]` is the opcode at the entry address and R15 reads entry + 8
(`arm-branch-pipeline.md`, "Refill after an R15 write"). The fetch goes through the same bus
view as every instruction fetch.

## CP15 state at entry

GBATEK says the firmware configures the ITCM size to 32 MiB (mirrors through `0000_0000h`-
`01FF_FFFFh`) and gives `027C_0000h` as the default DTCM base (16 KiB). It does not say that
either TCM is enabled when the cartridge code starts, nor what the control register holds.
Phase 2 programs none of it: CP15 stays at its power-on state (`cp15.md`), so both TCMs are
disabled and the exception vectors are low. This is the answer to the default-DTCM-base question
in `arm9-memory-map.md`. See "Unverified".

## The per-frame ARM9 budget

`run_frame` steps the ARM9 a fixed number of instructions. There is no scheduler until #21
(Phase 3), so the number is an approximation:

- One frame is 263 lines of 355 dots (GBATEK, display timings); a dot is 6 cycles of the 33.513982
  MHz bus clock: 263 x 355 x 6 = 560,190 bus cycles.
- The ARM9 runs at twice the bus clock: 1,120,380 ARM9 cycles per frame.
- One instruction is stepped per ARM9 cycle, so the budget is **1,120,380 steps per frame**.

This overstates the hardware. GBATEK says the 66 MHz clock only holds inside the cache and TCM,
and other accesses wait for the bus (several wait cycles per nonsequential access), so a real
instruction takes at least one cycle and often many. It is a placeholder for the scheduler
(`TODO(phase3)`, #21), not a timing claim.

A machine with no booted program (no ROM, or a refused one) does not step the ARM9: `run_frame`
only advances the frame counter.

## Edge cases

- An entry address that is not word aligned is not constrained by GBATEK; the pipeline refill
  fetches from the aligned address and keeps the low bits in R15 (`arm-branch-pipeline.md`).
- A second `load_rom` resets everything first; nothing of the earlier program or its RAM stays.

## Unverified

- **CPSR at entry** (System mode, ARM state, I and F set). Chosen so the machine can run;
  GBATEK gives only the stack registers. — `crates/core/src/hle/direct_boot.rs` (`boot`) —
  verify by a test ROM that reads CPSR at entry on hardware.
- **CP15 at entry** is left at power-on (TCMs disabled), though the firmware configures an
  ITCM of 32 MiB and a DTCM at `027C_0000h` by GBATEK's own description; whether they are on
  at entry is not stated. — `boot` in `hle/direct_boot.rs` — verify likewise. The code that
  runs before the cartridge's (and a game that expects the TCM) decides this in a later phase.
- **Other register banks** (FIQ, Abort, Undefined) start at zero. — `boot` — verify likewise.
- **The ARM9 and ARM7 placement order** when the two binaries overlap: the ARM9 binary is copied
  first, then the ARM7's. GBATEK says nothing. — `boot` — verify likewise.
- **The values the BIOS and firmware leave outside the header copy** (chip IDs, user settings,
  boot flags at `027FF800h` and up) are zero here. — `boot` — verify likewise; `TODO(phase3)`.
- **The step budget** is an estimate (see above). — `Machine::run_frame` in
  `crates/core/src/lib.rs` — replaced by the scheduler (#21).
