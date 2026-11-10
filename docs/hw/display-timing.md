# Display timing: lines, VBlank, HBlank, DISPSTAT and VCOUNT

Sources: GBATEK "DS Video" display dimensions and timings (dot clock, dots and lines per
frame); GBATEK "LCD I/O Display Status and Interrupt Control" (DISPSTAT, VCOUNT) with its DS
notes; GBATEK "DS Interrupts" for the three IE/IF bits (see `interrupts.md`).
Used by: `crates/core/src/gpu2d/timing.rs`, `crates/core/src/scheduler/`,
`crates/core/src/io.rs`, `crates/core/src/lib.rs`   Phase: 3 (#104)

The screens are drawn line by line. Each line has a visible part and a blanking part, and the
frame ends with a block of lines that are not shown. Two registers report where the display is
(VCOUNT, DISPSTAT) and three interrupt sources hang off them. Frame length and clocks are in
`scheduler.md`; this note owns the line layout and the registers.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0004h` | DISPSTAT | 16 | flags R, enables and setting R/W | not stated, 0 here |
| `0400_0006h` | VCOUNT | 16 | R (GBATEK: W exists, see "Edge cases") | line 0 at power-on |

Both exist on the ARM9 and on the ARM7. On the ARM9 they belong to engine A's port block
(GBATEK lists the engine A+B pair; only one copy is modeled).

### DISPSTAT bits

| Bits | Meaning | Access |
|---|---|---|
| 0 | VBlank flag: 1 while in the vertical blanking lines | R |
| 1 | HBlank flag: 1 while in the horizontal blanking part of a line | R |
| 2 | VCount-match flag: 1 while VCOUNT equals the VCount setting | R |
| 3 | VBlank IRQ enable | R/W |
| 4 | HBlank IRQ enable | R/W |
| 5 | VCount IRQ enable | R/W |
| 6 | not used on the DS (reads 0; GBATEK says a later model uses it) | R |
| 7 | bit 8 (MSB) of the VCount setting | R/W |
| 8-15 | bits 0-7 of the VCount setting | R/W |

The setting is a 9-bit line number (0-262) split across bit 7 (the top bit) and bits 8-15 (the
low byte). Writable bits are 3, 4, 5, 7 and 8-15 (mask `FFB8h`); a write to a flag or to bit 6
changes nothing.

### VCOUNT bits

| Bits | Meaning |
|---|---|
| 0-8 | the current line, 0-262 |
| 9-15 | not used, read 0 |

## Behavior

### Frame layout

- A line is 355 dots; a dot is 6 bus cycles (the bus clock is the ARM7 clock). A line is
  therefore 2,130 bus cycles, which is 4,260 master cycles (`scheduler.md`, "One master clock").
- A frame is 263 lines: lines 0-191 are visible (256 dots of 355 each), lines 192-262 are the
  71 blanking lines. 263 x 4,260 = 1,120,380 master cycles, the frame length in `scheduler.md`.
- The master clock is 0 at the start of line 0. Line *n* starts at `n x 4,260` into the frame.

### VCOUNT

VCOUNT advances at the start of each line, 0 up to 262, and wraps from 262 to 0 at the start of
the next frame. It reads the same value on both CPUs.

### Flags

- **VBlank flag** sets at the start of line 192 and clears at the start of line 262: GBATEK
  gives it as set in lines 192-261 and not in line 262. It reads the same on both CPUs.
- **HBlank flag.** GBATEK says the visible part of a line takes 1,536 bus cycles (256 dots) but
  that the ARM9's HBlank flag reads 0 for the first 1,606 bus cycles of the line (about 1,613
  for the ARM7). The flag sets that far into the line and clears at the start of the next line.
  It toggles in every line, including the VBlank lines. One point, 1,606 bus cycles = 3,212
  master cycles after the line start, is used for both CPUs.
- **VCount-match flag.** At the start of each line, each CPU compares VCOUNT with its own VCount
  setting; the flag is 1 when they are equal and 0 otherwise. So it is per CPU, and it is
  re-evaluated only at the start of a line (see "Unverified").

### Interrupt requests

A flag rising requests the matching bit in that CPU's IF (`interrupts.md`) when that CPU's
enable bit in its own DISPSTAT is set:

| Event | IF bit | Enable bit |
|---|---|---|
| VBlank flag rises (start of line 192) | 0 | DISPSTAT bit 3 |
| HBlank flag rises (every line) | 1 | DISPSTAT bit 4 |
| VCount-match flag rises (start of a line) | 2 | DISPSTAT bit 5 |

DISPSTAT's enables and setting are separate per CPU: a write by one CPU does not show on the
other. The VBlank and HBlank flags and VCOUNT are shared, because there is one display.

### Where the frame is drawn

Per-scanline drawing is not modeled. The frame is drawn once, from the registers and VRAM as
they are at that moment, when VCOUNT wraps from 262 to 0 (the start of line 0), the same point
as before this note; `display-mode-vram.md` has the rule. This is a model choice, not
hardware. The VBlank start (line 192) is a separate event that raises the flag and the IRQ.

### Hooks for later features

The start of VBlank and the start of HBlank are where VBlank-start and HBlank-start DMA
(`dma.md`, "Starting") and the IntrWait SWIs (#27) attach. Each is handled in one place in the
machine.

## Edge cases

- Software that writes 1 to a flag bit of DISPSTAT sees no change.
- A VCount setting above 262 can never match: VCOUNT never reaches it.
- GBATEK says VCOUNT is writable, to synchronize linked consoles. Its guidance is to write a
  new line value only in the range 202-212, and only while the old value is also in 202-212.
  It does not say what the register does with a write outside that guidance, nor whether the
  guidance is enforced. Writes are recorded here as ignored and stubbed with
  `TODO(hw-unverified)`.
- Reading DISPSTAT or VCOUNT is a plain read; it has no side effects.
- The requests are level-free: a flag that stays 1 requests once, when it rises.

## Unverified

- **HBlank point.** One point (1,606 bus cycles into the line, 3,212 master cycles) for both
  CPUs, although GBATEK gives about 1,613 for the ARM7. — `scheduler/mod.rs` (`HBLANK_START_CYCLES`) —
  verify with a test ROM on hardware that polls the ARM7's flag against a timer.
- **HBlank flag and IRQ in the VBlank lines.** The flag is modeled as toggling in every line
  and the IRQ as raised in every line. GBATEK's DISPSTAT text for the GBA says HBlank
  conditions occur once per scanline "including the hidden ones" while its timing page says no
  HBlank interrupts occur in VBlank; the DS pages do not settle it. — `gpu2d/timing.rs`
  (`hblank_started`) — verify by a hardware test ROM counting HBlank IRQs per frame.
- **VBlank clears at line 262.** From GBATEK's "not in line 262"; the exact cycle within the
  line is not stated, taken as the line start. — `gpu2d/timing.rs` (`line_started`).
- **VCount-match is evaluated at the start of a line only.** A write to the setting while a
  line runs does not change the flag or request an IRQ until the next line start. The flag
  starts 0 at power-on and is not evaluated for line 0 before the first line change. —
  `gpu2d/timing.rs` (`line_started`) — verify by a hardware test ROM writing the setting
  mid-line.
- **The VBlank IRQ is requested at the start of line 192**, not at some cycle into it. —
  `gpu2d/timing.rs` — verify as above.
- **Reset values.** DISPSTAT is not stated at reset; 0. — `gpu2d/timing.rs` (`Timing::new`).
- **VCOUNT writes.** Ignored, including the in-range (202-212) writes GBATEK describes for
  console synchronization; what a write does is not stated. — `gpu2d/timing.rs` (`Timing::write_byte`) — verify by a hardware test
  ROM writing VCOUNT.
- **DISPSTAT bit 6 and a write to the flag bits** read 0 and change nothing, from the
  register's description. — `gpu2d/timing.rs` (`Timing::write_byte`).
- **Whether the ARM7's register is the same copy of the hardware.** GBATEK lists DISPSTAT and
  VCOUNT for both CPUs; separate enables per CPU follows from the IRQ being per CPU. —
  `gpu2d/timing.rs` (`Timing`, fields `arm9` and `arm7`) — verify by writing the ARM9's and reading the ARM7's.
