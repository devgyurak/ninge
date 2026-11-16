# Keypad

Sources: GBATEK "GBA Keypad Input" (KEYINPUT, KEYCNT) and "DS Keypad" (the DS additions,
EXTKEYIN).
Used by: `crates/core/src/io/keypad/`, `crates/core/src/input/`   Phase: 3 (the "nothing
pressed" stub); host buttons and the hinge are wired in Phase 4 (#177); the keypad IRQ, the pen
and the touch controller are Phase 5 (#44)

## Registers

Both CPUs have the two GBA-style registers, each with its own KEYCNT. EXTKEYIN exists on the
ARM7 only.

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_0130 | KEYINPUT | 16 | R | 0x03FF (nothing pressed) |
| 0x0400_0132 | KEYCNT | 16 | R/W | 0 |
| 0x0400_0136 | EXTKEYIN (ARM7 only) | 16 | R | 0x007F (nothing pressed, hinge open) |

### KEYINPUT bits

A bit is **0 while the key is pressed** and 1 while it is released.

| Bits | Meaning |
|---|---|
| 0 | A |
| 1 | B |
| 2 | Select |
| 3 | Start |
| 4 | Right |
| 5 | Left |
| 6 | Up |
| 7 | Down |
| 8 | R |
| 9 | L |
| 10-15 | Not used |

### KEYCNT bits

| Bits | Meaning |
|---|---|
| 0-9 | Which keys take part in the interrupt condition (same order as KEYINPUT; 1 = selected) |
| 10-13 | Not used |
| 14 | Keypad IRQ enable |
| 15 | Condition: 0 = any selected key pressed (OR), 1 = all selected keys pressed (AND) |

### EXTKEYIN bits

| Bits | Meaning |
|---|---|
| 0 | X (0 = pressed) |
| 1 | Y (0 = pressed) |
| 2 | Unknown, reads set |
| 3 | Debug button (0 = pressed, 1 = released or not fitted) |
| 4-5 | Unknown, read set |
| 6 | Pen down (0 = pressed, 1 = released or disabled) |
| 7 | Hinge (0 = open, 1 = closed) |
| 8-15 | Unknown, read zero |

## Behavior

### Where the bits come from

The host gives the machine one `Input` for a frame: which of the twelve buttons are held (the ten
KEYINPUT keys plus X and Y), where the touch panel is pressed (if it is), and whether the hinge
is closed. It arrives with `Machine::run_frame(input)`, so every frame's input is explicit, and
**it is latched as `run_frame` begins**: every read of KEYINPUT and EXTKEYIN during that frame
sees the same buttons whichever CPU reads them, and an input log of one `Input` per frame replays
the same machine. A debugger single-step does not latch: it keeps the buttons of the last frame
that ran. Both CPUs' keypads are latched together. The latched input is machine state (it is saved); no input waits for the next frame, as
`run_frame` takes it.

A held button reads as 0 in its bit and a released one as 1. KEYINPUT bits 0-9 follow the ten
keys; bits 10-15 read 0 as before. EXTKEYIN bit 0 follows X and bit 1 follows Y the same way, and
bit 7 follows the hinge (1 = closed, 0 = open). With the default `Input` (nothing held, hinge
open) both registers read what the "nothing pressed" stub read.

### What is still a stub

The pen bit (EXTKEYIN bit 6) reads released whatever the touch position is: the touch point is
carried in the `Input` and latched, but nothing reads it until the touch controller exists
(Phase 5). The debug button reads released, and the unknown bits read as before. The keypad
interrupt condition is never met, so no IRQ is raised, and the hinge IRQ is not raised either.

KEYCNT stores the bits it defines (0-9, 14, 15) and reads them back; the other bits read 0. Each
CPU has its own copy.

The block is read at any width; a wider read spans the registers it covers, as with the other
byte-wise register blocks. The address between KEYCNT and EXTKEYIN (0x0400_0134, RCNT) is not
modeled and reads 0.

## Edge cases

- A write to KEYINPUT or EXTKEYIN is ignored: they are read-only.
- On the ARM9 the address of EXTKEYIN holds nothing: it reads 0 and drops writes.
- A released key reads 1 and a pressed key reads 0. A block that read 0 (an unmapped port) would
  therefore look like every key held; that is why the default `Input` reads all released.
- Opposite directions held together (Left and Right) both read 0: the machine does not filter
  them; a frontend that wants to is free to.
- A bit of `Buttons` above X and Y is dropped when the input is built, so a stray bit cannot
  reach a register.

## Unverified

- KEYCNT's reset value — GBATEK states none; the stub resets it to 0 —
  `crates/core/src/io/keypad/mod.rs` (`Keypad::new`) — verify by a test ROM on real hardware.
- KEYINPUT bits 10-15 read 1 or 0 on hardware — GBATEK says "not used" and nothing more. The
  stub reads 0 — `crates/core/src/io/keypad/mod.rs` — verify by a test ROM on real hardware.
- The value of EXTKEYIN bits 2, 4 and 5 (GBATEK: "unknown / set") and bits 8-15 ("unknown /
  zero"), and that the debug button reads released — same site.
- Whether KEYCNT bits 10-13 are writable — same site.

Not modeled, so no code site: the keypad IRQ condition with an empty selection mask in AND mode,
the hinge IRQ (GBATEK: it cannot be disabled), and whether EXTKEYIN's pen bit follows the touch
controller's pen-IRQ enable. All three arrive with the keypad IRQ and the touch SPI (Phase 5,
#44).

- That a button change reaches the registers at a frame boundary only is a choice of the model:
  on hardware the key matrix is read when the CPU reads the register — `crates/core/src/input/`
  (`Machine::run_frame`) — verify by a test ROM that polls KEYINPUT during a frame.
