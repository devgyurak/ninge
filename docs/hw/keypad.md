# Keypad

Sources: GBATEK "GBA Keypad Input" (KEYINPUT, KEYCNT) and "DS Keypad" (the DS additions,
EXTKEYIN).
Used by: `crates/core/src/io/keypad/`   Phase: 3 (the "nothing pressed" stub); real input is
Phase 5 (#44)

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

The stub models a console with nothing touched and the lid open. KEYINPUT reads with all ten key
bits set. EXTKEYIN reads with X, Y, the debug button and the pen released, the unknown bits 2,
4 and 5 set, and the hinge bit clear (open). Nothing is ever pressed, so a keypad interrupt
condition is never met and the stub raises no IRQ.

KEYCNT stores the bits it defines (0-9, 14, 15) and reads them back; the other bits read 0. Each
CPU has its own copy.

The block is read at any width; a wider read spans the registers it covers, as with the other
byte-wise register blocks. The address between KEYCNT and EXTKEYIN (0x0400_0134, RCNT) is not
modeled and reads 0.

## Edge cases

- A write to KEYINPUT or EXTKEYIN is ignored: they are read-only.
- On the ARM9 the address of EXTKEYIN holds nothing: it reads 0 and drops writes.
- A released key reads 1 and a pressed key reads 0. A block that read 0 (an unmapped port) would
  therefore look like every key held; that is why the stub exists.

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
controller's pen-IRQ enable. All three arrive with real input (Phase 5, #44) and the touch SPI.
