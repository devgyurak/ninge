# Windows: WIN0, WIN1, the OBJ window and the outside region (both engines)

Sources: GBATEK "LCD I/O Window Feature" (the registers, the rectangle rules, the OBJ window,
the priority of the regions); GBATEK "DS Video BG Modes / Control" (DISPCNT bits 8-15); GBATEK "DS
Video 3D Display Control" (the one line that says the window feature works on the DS as on the
GBA).
Used by: `crates/core/src/gpu2d/`   Phase: 4 (#33); the colour special effects the window
enables are #34, the debugger panel that shows the rectangles is #58

The window feature splits the screen into up to four regions and lets each region decide which
layers are shown. `bg-text.md` and `bg-modes.md` say how the background layers make a pixel, and
`obj.md` how the sprites do. This note says which of those layers a pixel may use. Both engines
have the same windows at the same offsets; everything below holds for both.

## Registers

Engine A's block starts at `0400_0000h`, engine B's at `0400_1000h`; the offsets are the same.

| Offset | Name | Width | Access | Reset |
|---|---|---|---|---|
| `+0040h` | WIN0H | 16 | W | 0 (see "Unverified") |
| `+0042h` | WIN1H | 16 | W | 0 |
| `+0044h` | WIN0V | 16 | W | 0 |
| `+0046h` | WIN1V | 16 | W | 0 |
| `+0048h` | WININ | 16 | R/W | 0 |
| `+004Ah` | WINOUT | 16 | R/W | 0 |

### DISPCNT bits used here

| Bits | Meaning |
|---|---|
| 8-11 | BG0-BG3 on (the master enables) |
| 12 | OBJ on |
| 13 | window 0 on |
| 14 | window 1 on |
| 15 | OBJ window on |

Bits 8-12 already decide whether a layer exists (`bg-text.md`, `obj.md`). A layer is shown at a
pixel only when its DISPCNT bit **and** the enable bit of the region the pixel is in are both set.

### WIN0H, WIN1H, WIN0V, WIN1V

| Register | Bits 0-7 | Bits 8-15 |
|---|---|---|
| WINxH | X2: the rightmost column of the window, plus 1 | X1: the leftmost column |
| WINxV | Y2: the bottom row of the window, plus 1 | Y1: the top row |

So a window covers the columns `X1 <= x < X2` and the rows `Y1 <= y < Y2`: the end value is one
past the last pixel, the start value is the first one.

### WININ and WINOUT

Each holds two groups of six bits; a group is `BG0-BG3` in its bits 0-3, `OBJ` in bit 4 and the
colour-special-effect enable in bit 5 (bits 6-7 of each byte are not used).

| Register | Bits 0-5 (low byte) | Bits 8-13 (high byte) |
|---|---|---|
| WININ | inside window 0 | inside window 1 |
| WINOUT | outside every window | inside the OBJ window |

Group bits: 0-3 the layers BG0-BG3, 4 the OBJ layer, 5 the colour special effect. A set bit means
"shown" (or "effect allowed"); a clear bit hides the layer there.

## Behavior

### The four regions

A pixel is in exactly one region, picked by this priority:

1. window 0, when DISPCNT bit 13 is set and the pixel is inside window 0's rectangle;
2. else window 1, when bit 14 is set and the pixel is inside window 1's rectangle;
3. else the OBJ window, when bit 15 is set and the pixel is an OBJ window pixel (below);
4. else the outside region.

Window 0 therefore wins over window 1 where they overlap, and both win over the OBJ window.

The region's six-bit group says what is shown at the pixel: the BG layers whose bit is set, the
OBJ layer when its bit is set. A layer whose bit is clear acts as if it were not there: it does not
cover the layers behind it, and a hidden sprite does not hide a background pixel it would have
beaten. The backdrop has no enable bit and is always the last layer.

### No window on

When DISPCNT bits 13, 14 and 15 are all clear there is no windowing at all: every layer that
DISPCNT turns on is shown at every pixel, whatever WININ and WINOUT hold. When any of the three is
set, the outside region applies to every pixel not covered by an enabled window, so with a single
window on, WINOUT decides everything outside it.

### The rectangles

A window's rectangle is read from its two registers as they stand when the line is made
(`bg-text.md`, "When the picture is made"). The columns are those of the screen, `0` to `255`, the
rows `0` to `191`.

Values outside the screen are not an error: X2 greater than the screen width, or X1 greater than
X2, count as X2 = the width; Y2 greater than the screen height, or Y1 greater than Y2, count as Y2 =
the height. The start value is used as written. A window with `X1 = X2` or `Y1 = Y2` (after that
rule) covers nothing.

### The OBJ window

An object whose attr0 mode (bits 10-11) is 2 draws no colour. Every one of its pixels that is not
transparent (the transparency of its own kind of object, `obj.md`) marks its screen pixel as an
OBJ window pixel. Its palette, its colour and its priority do not matter. Both DISPCNT bit 12
(the OBJ layer) and bit 15 must be set for the OBJ window to exist: with bit 12 clear no object is
processed (`obj.md`, "The layer is on when"), so no pixel is marked.

An OBJ window object goes through the same placement, affine transform, mosaic and per-line cycle
budget as any other object, so a mask can be cut off by the line limit like a picture (`obj.md`,
"The line limit"), and mosaic blocks the mask the same way it blocks colour.

### The colour special effect bit

Bit 5 of each group says whether the colour special effect (blending, brightening, darkening)
applies to the pixels of that region. The effect is #34; this note only records the bit. Until
then the bit is stored and read back and changes no pixel.

### Both engines

Engine B's windows are the same circuit at `0400_1000h + offset`; there is no difference. GBATEK
notes that the 3D layer (BG0 in the 3D mode) follows the same window rules; the 3D engine is a
later phase (`TODO(phase7)`).

## Edge cases

- DISPCNT 13-15 all clear: the registers are inert, the picture is the one without windows.
- A window with its end before its start: it does not vanish and does not wrap; its end becomes the
  screen edge, so it covers from the start to the edge.
- Two windows covering a pixel: window 0's group applies; window 1's does not combine with it.
- An OBJ window pixel inside window 0 or window 1 belongs to that window.
- A region whose OBJ bit is clear: sprites do not show there, and neither do they decide
  priority; an OBJ window object is unaffected, since it draws nothing.

## Unverified

Each item carries `TODO(hw-unverified)` at its code site.

- **The screen limit of the clamp.** GBATEK writes the rule for the GBA screen: "X2 > 240 or X1 >
  X2 counts as X2 = 240", "Y2 > 160 or Y1 > Y2 counts as Y2 = 160". The DS screen is 256 by 192.
  We use the DS size. With X2 an 8-bit field the X2 > width part can never apply, so only X1 > X2
  matters horizontally. — `gpu2d::window::WindowRect::decode` — verify with a test ROM on hardware.
- **X2 or Y2 equal to 0, X1 or Y1 at 255.** Not stated; they follow from the rule (an empty window,
  or X1 > X2 and so the full width). — `gpu2d::window::WindowRect::decode` — verify likewise.
- **Vertical test.** Whether the row test is a comparison of the current line against Y1 and Y2, or
  a latch that switches on at Y1 and off at Y2 (so that changing the registers mid-frame behaves
  differently). We compare the line number against the values as they stand when the line is made.
  — `gpu2d::window::LineWindows::new` — verify with a mid-frame write.
- **Reads.** WIN0H-WIN1V are marked write-only; we return 0 for them, as for the other
  write-only registers (`bg-modes.md`). — `Engine::read_byte` — verify likewise.
- **WININ and WINOUT bits 6-7 and 14-15** ("not used"): we clear them on write, so they read back
  0. — `gpu2d::window::GROUP_MASK` — verify likewise.
- **Power-on values** of all six registers: not stated; they start at 0. — `Engine::new` —
  verify likewise.
- **The backdrop with every layer hidden:** it has no enable bit and is shown. This is what the
  absence of a bit implies; GBATEK does not spell it out. — `Engine::render_line` — verify
  likewise.
- **What an OBJ window object goes through.** GBATEK does not say that a mode 2 object has the
  same cost, affine transform, mosaic and line limit as a drawn one. We give it all of them, so the
  mask can be cut off by the limit and follows a transform. — `obj::draw_line` and
  `obj::draw_object` — verify likewise.
- **The window on the 3D layer**, and GBATEK's own doubt there that the effect bit may be
  overridden when the 3D layer is in front. — none until Phase 7.
- **Windows outside the normal picture mode.** We apply them only in display mode 1, where the
  layers are drawn; modes 0, 2 and 3 do not use layers (`display-mode-vram.md`). — `Gpu2d::render_line_a` and `render_line_b` — verify likewise.

## Evidence

Test ROMs pin this note (#33): `win` (WIN0, WIN1 and the outside region on both engines, one with
the rectangle rules for a start past an end) and `objwin` (mode 2 objects: plain, affine,
half-transparent and mosaic, with ordinary sprites and window 1 over them). Their expected pixels
are worked out from this note, never read from the emulator. The unit tests in `gpu2d/tests/window.rs`
add byte and halfword writes, the register masks, the line budget and the savestate.
