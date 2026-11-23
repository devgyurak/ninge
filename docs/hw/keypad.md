# Keypad

Sources: GBATEK "GBA Keypad Input" (KEYINPUT, KEYCNT) and "DS Keypad" (the DS additions,
EXTKEYIN).
Used by: `crates/core/src/io/keypad/`, `crates/core/src/input/`   Phase: 3 (the "nothing
pressed" stub); host buttons and the hinge are wired in Phase 4 (#177); the pen bit and the touch
controller are Phase 5 (#41, `touch-power.md`); the keypad and hinge IRQs are Phase 5 (#231,
"The keypad IRQ")

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
that ran. Both CPUs' keypads are latched together: `Io` keeps the one latched copy, and each
keypad read gets it with the access, so no keypad stores an input of its own. The latched input
is machine state (it is saved); no input waits for the next frame, as `run_frame` takes it.

A held button reads as 0 in its bit and a released one as 1. KEYINPUT bits 0-9 follow the ten
keys; bits 10-15 read 0 as before. EXTKEYIN bit 0 follows X and bit 1 follows Y the same way, and
bit 7 follows the hinge (1 = closed, 0 = open). With the default `Input` (nothing held, hinge
open) both registers read what the "nothing pressed" stub read.

### The pen bit

The pen bit (EXTKEYIN bit 6) reads 0 while the latched `Input` has the panel touched **and** the
touch controller's pen output is enabled by the power-down mode of its last control byte, and 1
otherwise (`touch-power.md`, "The pen bit"). The keypad does not look at the controller: `Io` reads
the pen output from the SPI bus and hands it over with the access. The bit follows the latched
touch state, not the analog line (`touch-power.md`, "Unverified").

### The keypad IRQ

Each CPU's KEYCNT chooses the condition for its own copy of IF bit 12. Bits 0-9 select keys in
KEYINPUT order. With bit 15 clear (OR) the condition holds while **any** selected key is held; with
bit 15 set (AND) it holds while **every** selected key is held. X and Y are not in KEYCNT, so
they never take part, whichever other keys are selected. Bit 14 enables the IRQ: with it clear
nothing is requested. IE and IME decide, as for any source, whether the CPU takes it
(`interrupts.md`).

The model checks the condition at two points and at no other:

- **every frame-boundary latch**, when `run_frame` latches the frame's `Input`, and
- **every write to KEYCNT**, with the buttons of the latched input.

At either point, if the IRQ enable is set and the condition holds, the CPU's IF bit 12 is
requested. A key held across several frames therefore requests once per frame, and an
acknowledge of IF bit 12 does not request again until the next check: the request is not level
driven (contrast the geometry FIFO's bit, `gx-stub.md`). When the condition is checked, and the
re-request behavior, are the model's choice ("Unverified").

With **no key selected** (bits 0-9 all clear) the condition is never met, in OR or in AND mode
(the model's choice, "Unverified").

### The hinge IRQ

IF bit 22 exists on the ARM7 only. GBATEK says the hinge generates it when the screens unfold and
that there seems to be no way to disable the request;
IE (with IME and the CPU's I flag, as for any source) decides whether the ARM7 takes it.
The model compares the previously latched `Input` with the new one at each frame-boundary latch:
a change from closed to open requests ARM7 IF bit 22, whatever KEYCNT, its enable and IE say.
Closing the lid requests nothing, and a frame whose hinge state equals the last one requests
nothing. The first frame compares with the reset input, whose hinge is open, so a machine that
starts with the lid open raises nothing. That closing raises nothing is the model's choice
("Unverified").

### What is still a stub

The debug button reads released, and the unknown bits read as before.

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

- **When the keypad condition is sampled, and that it is an edge or a level.** GBATEK says neither.
  The model samples at each frame-boundary latch and each KEYCNT write, requests every time the
  condition holds, and never re-raises after an acknowledge — `crates/core/src/io/keypad/mod.rs`
  (`Keypad::latch_requests`, `Keypad::write`) — verify by a test ROM that holds a key across an
  acknowledge on hardware.
- **The empty selection.** Neither OR nor AND is ever met with no key selected; GBATEK does not
  say what an AND over an empty set does — `crates/core/src/io/keypad/mod.rs`
  (`Keypad::condition_met`) — verify likewise.
- **Whether closing the lid raises IF bit 22.** The register's name says "unfolding" only; the
  model raises it on opening and not on closing — `crates/core/src/io/keypad/mod.rs`
  (`Keypad::latch_requests`) — verify by a test ROM that closes the lid on hardware.

- That a button change reaches the registers at a frame boundary only is a choice of the model:
  on hardware the key matrix is read when the CPU reads the register — `crates/core/src/input/`
  (`Machine::run_frame`) — verify by a test ROM that polls KEYINPUT during a frame.
