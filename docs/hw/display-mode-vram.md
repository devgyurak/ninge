# Engine A display mode 2 (VRAM display) and the screen swap

Sources: GBATEK "DS Video BG Modes / Control" (`4000000h` DISPCNT for engine A, display mode and
VRAM block fields); GBATEK "DS Power Control" (`4000304h` POWCNT1); GBATEK "DS Video - Color
Format" (15-bit colour); GBATEK "DS Memory Control - VRAM" (LCDC mapping of bank A).
Used by: `crates/core/src/gpu2d/`, `crates/core/src/io.rs` (the I/O slow path)   Phase: 2 (#20)

Phase 2 draws one thing: a 256x192 bitmap that the CPU wrote into VRAM bank A, shown by
graphics engine A. Everything else DISPCNT and POWCNT1 control is a later phase.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0000h` | DISPCNT (engine A) | 32 | R/W | 0 (see "Unverified") |
| `0400_0304h` | POWCNT1 | 16 (GBATEK's table is 32 bits wide with bits 16-31 unused; a word read gives 0 there) | R/W | `820Fh` (a choice, see "Unverified") |

Both registers accept byte, halfword and word accesses; a wider access is the same as the
byte accesses to consecutive addresses.

### DISPCNT bits (engine A)

| Bits | Meaning | Phase |
|---|---|---|
| 0-2 | BG mode | `TODO(phase3)` |
| 3 | BG0 is 2D (0) or 3D (1) | `TODO(phase7)` |
| 4-6 | OBJ options: tile OBJ mapping, bitmap OBJ 2D dimension, bitmap OBJ mapping | `TODO(phase4)` |
| 7-15 | same as the GBA register: forced blank (7), BG0-BG3 enables (8-11), OBJ enable (12), windows (13-15) | `TODO(phase3)` / `TODO(phase4)`; bit 7 see "Unverified" |
| 16-17 | display mode: 0 off, 1 graphics, 2 VRAM display, 3 main-memory display | 0 and 2 now |
| 18-19 | VRAM block for mode 2: 0 = bank A, 1 = B, 2 = C, 3 = D | block 0 now |
| 20-31 | tile OBJ boundary, bitmap OBJ options, H-blank OBJ processing, character/screen base, extended palettes | `TODO(phase4)` |

All 32 bits are stored and read back. Only bits 16-19 change what is drawn in Phase 2.

### POWCNT1 bits

| Bits | Meaning | Phase |
|---|---|---|
| 0 | LCD enable | see "Unverified" |
| 1 | engine A (2D) power | see "Unverified" |
| 2 | 3D rendering engine power | `TODO(phase7)` |
| 3 | 3D geometry engine power | `TODO(phase7)` |
| 9 | engine B (2D) power | `TODO(phase4)` |
| 15 | display swap: 1 = engine A to the upper screen, engine B to the lower; 0 = engine A to the lower, B to the upper | now |

All 16 bits are stored and read back. Bits 4-8 and 10-14 are not defined by GBATEK; they are
kept anyway so a read returns what was written (see "Unverified").

## Behavior

### What engine A shows, by display mode (DISPCNT bits 16-17)

- **Mode 0, display off:** the screen is white.
- **Mode 1, graphics:** backgrounds and objects. `TODO(phase3)` (text BG) and
  `TODO(phase4)` (the rest). Until then the screen is a black placeholder, which is not a
  hardware claim.
- **Mode 2, VRAM display:** the bitmap described below.
- **Mode 3, main-memory display:** a bitmap streamed from main RAM through the display FIFO.
  `TODO(phase4)`. Until then the screen is a black placeholder.

### Mode 2: reading the bitmap

The selected VRAM block (DISPCNT bits 18-19) holds 256 x 192 pixels of 16 bits each, from the
start of the block, row by row, left to right, top row first. Pixel (x, y) is the halfword at
byte offset `(y * 256 + x) * 2` of the block, little-endian. That is 98,304 bytes, so the
bitmap fits in a 128 KiB bank with the rest of the bank unused.

Each halfword is 15-bit colour: bits 0-4 red, bits 5-9 green, bits 10-14 blue, five bits per
channel. Bit 15 is not used by the colour; it has no effect on the displayed pixel.

Only block 0 (bank A) exists in Phase 2. Blocks 1-3 (banks B, C, D) are `TODO(phase4)` (#35).

GBATEK also says modes 2 and 3 bypass the 2D and 3D pipelines but the master brightness unit
still applies to their output. Master brightness is not modeled: `TODO(phase4)` (#34).

### The screen swap (POWCNT1 bit 15)

The words upper and lower name the physical screens. POWCNT1 bit 15 decides which screen shows
engine A and which shows engine B: with the bit set, engine A is on the upper screen; with it
clear, engine A is on the lower screen. Engine B is not drawn in Phase 2 (`TODO(phase4)`): the
screen it is routed to stays a black placeholder.

### When the picture is made

The hardware draws the screen line by line while the CPU runs. Phase 2 renders the whole frame
at once at the end of each frame, from the registers and VRAM as they are then.
`TODO(phase3)`: per-scanline drawing needs the line timing of `display-timing.md` (#104); until then the frame is drawn when VCOUNT wraps to line 0.

## Colour expansion (host choice, not hardware)

The 15-bit colour has to become the emulator's host pixel (xRGB8888, `0x00RRGGBB`, `display.md`,
decided in #68). That is a choice on the host side, not a hardware fact: the LCD's own colour
depth is not what this expansion models. Each 5-bit channel `c` becomes the 8-bit channel
`(c << 3) | (c >> 2)`, so the extremes map to the extremes (0 to 0, 31 to 255) and white
stays white. Changing this formula changes every golden frame that contains colour (#68).

## Edge cases

- A DISPCNT write changes the display mode or block from that point on; nothing is cached.
- Mode 2 with a block that holds nothing yet (all zero) shows black: colour 0 expands to black.
- Bit 15 of a pixel halfword set or clear gives the same colour.
- Byte, halfword and word writes to DISPCNT and POWCNT1 change only the bytes they cover.

## Unverified

- **DISPCNT reset value** is 0 (display off, so the screen is white until software sets a
  mode). GBATEK gives no reset value. — `Gpu2d::new` in `crates/core/src/gpu2d/mod.rs` — verify
  by a test ROM on real hardware that reads DISPCNT before writing it.
- **POWCNT1 reset value after direct boot** is `820Fh` (LCD, both 2D engines, both 3D units on,
  engine A on the upper screen). This is a choice made for lack of evidence, not a claim about
  hardware or a loader. The `smoke` golden frame depends on it: smoke never writes POWCNT1, so
  its white engine-A screen lands on the upper screen only because bit 15 starts set. The
  `vramdisp` test ROM (the follow-up PR for #20) writes POWCNT1 explicitly, so its golden does
  not depend on it. —
  `Gpu2d::new` — verify on hardware, or from what a loader leaves in the register.
- **Effects of POWCNT1 bit 0 (LCD enable) and bit 1 (engine A enable)** on what a screen
  shows are not modeled: the bits are stored and the picture ignores them. —
  `Gpu2d::engine_a_on_top` and the render entry point in `crates/core/src/gpu2d/mod.rs` — verify
  by a test ROM that clears each bit and observes the screens on hardware.
- **DISPCNT bit 7 (forced blank)** is stored but ignored. It draws a white line on the GBA;
  GBATEK has no DS description of its effect in display modes 0 and 2. —
  `Gpu2d::render_engine_a` — verify by a test ROM on real hardware that sets it in mode 2.
- **POWCNT1 undefined bits** (4-8, 10-14) read back as written. — `Gpu2d::write_byte`
  — verify likewise.
- **A selected block that is not readable as LCDC VRAM** — bank A disabled, or its MST not
  0 — is drawn as if the block read zero, which is black. GBATEK describes mode 2 only for a
  mapped block. — `Gpu2d::render_engine_a` and `Memory::vram_a_lcdc` — verify likewise.
- **Mode 2 with block 1-3** draws black in Phase 2, because banks B-D are not modeled
  (`TODO(phase4)`, #35); on hardware it would show those banks. — `Gpu2d::render_engine_a`.
- **Whether mode 2 shows a bank that is enabled with a non-LCDC MST** (for example MST 1,
  the BG mapping) is not settled; Phase 2 treats it as unreadable (previous item).
- **Mirrors** of the I/O block are not modeled (`arm9-memory-map.md`, "Unverified").
