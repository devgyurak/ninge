# Background modes, affine layers and mosaic (both engines)

Sources: GBATEK "DS Video BG Modes / Control" (DISPCNT, the BG mode table, the extended-mode
selection, BGxCNT bases and sizes, the direct-colour alpha bit, engine B differences); GBATEK "LCD
I/O BG Control" (BGxCNT, screen sizes, area overflow); GBATEK "LCD I/O BG Rotation/Scaling"
(reference point, PA-PD, the internal registers); GBATEK "LCD I/O Mosaic Function" (MOSAIC);
GBATEK "LCD VRAM BG Screen Data Format (BG Map)" (the one-byte affine entry); GBATEK "DS Power
Control" (POWCNT1); GBATEK "DS Video Extended Palettes" (what the palettes do).
Used by: `crates/core/src/gpu2d/`   Phase: 4 (#30); sprites (#31), extended palettes (#32),
windows (#33, `windows.md`), blending and master brightness (#34) and capture (#36) are other issues

`bg-text.md` describes the text layer and the picture of one line. This note adds the rest of the
background layers (affine, extended, large bitmap), mosaic, and the second graphics engine. Both
engines are the same circuit with a few parts missing from engine B; everything below holds for
both unless it says "engine A only".

## Registers

Engine A's block starts at `0400_0000h`, engine B's at `0400_1000h`; the offsets below are the
same in both. All are halfword or word aligned; a byte write changes that byte alone.

| Offset | Name | Width | Access | Reset |
|---|---|---|---|---|
| `+0000h` | DISPCNT | 32 | R/W | 0 (`display-mode-vram.md`) |
| `+0008h`-`+000Eh` | BG0CNT-BG3CNT | 16 | R/W | 0 (`bg-text.md`) |
| `+0010h`-`+001Eh` | BGxHOFS / BGxVOFS | 16 | W | 0 (`bg-text.md`) |
| `+0020h` / `22h` / `24h` / `26h` | BG2PA / PB / PC / PD | 16 | W | 0 (see "Unverified") |
| `+0028h` | BG2X | 32 | W | 0 |
| `+002Ch` | BG2Y | 32 | W | 0 |
| `+0030h` / `32h` / `34h` / `36h` | BG3PA / PB / PC / PD | 16 | W | 0 |
| `+0038h` | BG3X | 32 | W | 0 |
| `+003Ch` | BG3Y | 32 | W | 0 |
| `+004Ch` | MOSAIC | 16 | W | 0 |

Engine B's POWCNT1 enable is bit 9 of `0400_0304h` (`display-mode-vram.md`).

### DISPCNT bits used here

| Bits | Meaning | Engines |
|---|---|---|
| 0-2 | BG mode, 0-6 (7 is not defined) | both; mode 6 is engine A only |
| 3 | BG0 is 2D (0) or 3D (1) | engine A only |
| 8-11 | BG0-BG3 on | both |
| 16-17 | display mode | engine A 0-3, engine B 0-1 |
| 24-26 | character base, `N * 64 KiB` | engine A only; engine B's is 0 |
| 27-29 | screen base, `N * 64 KiB` | engine A only; engine B's is 0 |
| 30 | BG extended palettes (`extended-palettes.md`) | both |

### BGxCNT bits that this note adds to `bg-text.md`

| Bits | Meaning |
|---|---|
| 2 | for an extended layer with bit 7 set: 0 = 256-colour bitmap, 1 = direct-colour bitmap |
| 6 | mosaic on for this layer |
| 7 | for an extended layer: 0 = tile layer with 16-bit entries, 1 = bitmap |
| 13 | BG2, BG3: area overflow, 0 = transparent outside the layer, 1 = wrap around |
| 14-15 | size, by layer kind (the table below) |

### Reference point and parameters

- **PA, PB, PC, PD** are signed 16-bit numbers with 8 fraction bits (8.8).
- **BGxX, BGxY** are signed numbers of 28 bits: 8 fraction bits, 19 integer bits, bit 27 the sign
  (20.8). A write keeps bits 0-27 of the value and bit 27 is extended into the upper four bits, so
  an ordinary signed 32-bit write gives the number it says. The registers are write-only.

## The mode table

The BG mode says what kind each layer is. A kind is how the layer gets its pixels; the table is
`bg-text.md`'s extended with the new kinds:

| Mode | BG0 | BG1 | BG2 | BG3 |
|---|---|---|---|---|
| 0 | text (or 3D) | text | text | text |
| 1 | text (or 3D) | text | text | affine |
| 2 | text (or 3D) | text | affine | affine |
| 3 | text (or 3D) | text | text | extended |
| 4 | text (or 3D) | text | affine | extended |
| 5 | text (or 3D) | text | extended | extended |
| 6 | 3D only | none | large bitmap | none |

- **Text** is `bg-text.md`.
- **Affine** is a rotated and scaled tile layer with one-byte map entries (below).
- **Extended** is one of three kinds, picked per layer by BGxCNT bits 7 and 2:
  - bit 7 = 0: tile layer with 16-bit map entries ("extended map");
  - bit 7 = 1, bit 2 = 0: 256-colour bitmap;
  - bit 7 = 1, bit 2 = 1: direct-colour bitmap.
- **Large bitmap** is a 256-colour bitmap that spans the whole 512 KiB of engine A's BG VRAM.
- **3D** is the 3D engine's picture and is not drawn yet (`TODO(phase7)`).
- **None** draws nothing: the layer is absent even if its DISPCNT enable bit is set.

Engine B is the same table except that BG0 is always text (DISPCNT bit 3 does not exist there)
and mode 6 does not exist: an engine B layer in mode 6 is none. A layer is drawn only when its
DISPCNT bit 8 + n is set and the display mode shows layers (display mode 1).

## Sizes and where the data is

Sizes are in pixels; BGxCNT bits 14-15 pick the column. All sizes wrap or clip at their own edge.

| Bits 14-15 | Text | Affine and extended map | Bitmap (256-colour, direct) | Large bitmap |
|---|---|---|---|---|
| 0 | 256x256 | 128x128 | 128x128 | 512x1024 |
| 1 | 512x256 | 256x256 | 256x256 | 1024x512 |
| 2 | 256x512 | 512x512 | 512x256 | not defined |
| 3 | 512x512 | 1024x1024 | 512x512 | not defined |

Bases. "BG VRAM" is the engine's window (`vram-banks.md`), all offsets from its start. Engine B's
DISPCNT bases are 0.

- **Tile data** (text, affine, extended map): `DISPCNT.char_base * 64 KiB + BGxCNT.char_block *
  16 KiB`. Bitmaps do not use a character base.
- **Map** (text, affine, extended map): `DISPCNT.screen_base * 64 KiB + BGxCNT.screen_block *
  2 KiB`.
- **Bitmap** (256-colour, direct): `BGxCNT.screen_block * 16 KiB`. The DISPCNT screen base is not
  added. The block number has all five bits (0-31).
- **Large bitmap**: from the start of BG VRAM; the screen block is not used.

## Behavior

### Affine layers

An affine layer is drawn by walking a line of the screen through the layer along a straight line:

1. The layer keeps an **internal reference point** `(rx, ry)`, a pair of 20.8 numbers per layer.
   It is the layer point that lands on the left-most pixel of the line being drawn.
2. Pixel `x` of the line shows the layer point `(rx + PA * x, ry + PC * x)`. The result of each
   product is 20.8 too (PA is 8.8, `x` an integer). The pixel's integer coordinates `(tx, ty)` are
   that point with the 8 fraction bits dropped, rounding toward minus infinity (an arithmetic
   shift right by 8).
3. When the line is done, the internal reference point moves down one line:
   `rx += PB`, `ry += PD`.
4. PA and PC are the step along a screen line; PB and PD are the step from one line to the next.
   The identity transform is PA = PD = 100h and PB = PC = 0.

The **internal reference point is loaded from BGxX/BGxY** at two moments:

- at the start of VBlank, so line 0 of the next frame starts from the registers;
- when software writes BGxX or BGxY: the new value is copied to the internal point at once. A
  write while the picture is being drawn therefore sets the origin of the line drawn next, not of
  the top line. (A byte, halfword or word write is the same as the bytes in turn; the internal
  point has the whole register after the last byte.)

PA-PD are read whenever a pixel is made, so a change shows from the next line.

The point `(tx, ty)` is **inside** the layer when `0 <= tx < W` and `0 <= ty < H`. Outside it, the
BGxCNT bit 13 decides: 0 gives a transparent pixel, 1 wraps (`tx mod W`, `ty mod H`, the size a
power of two). The bit counts in the affine, extended-map, bitmap and large-bitmap kinds alike;
The DS BG Modes page says explicitly that the DS bitmaps support it, unlike the GBA's (see
"Unverified": the Rotation/Scaling page says the opposite).

**Affine map entries** are one byte, the tile number (0-255): no flips, no palette number, always
256 colours in one palette. The map is `W/8 x H/8` entries, row by row. The tile is 64 bytes, a
byte per pixel, rows of 8; the pixel inside it is `(tx mod 8, ty mod 8)`. Value 0 is transparent;
otherwise the colour is the BG palette entry `value`.

### Extended layers

- **Extended map** (bit 7 = 0): the map is the text layer's 16-bit entry (tile 0-1023, bit 10
  horizontal flip, bit 11 vertical flip, bits 12-15 palette number) laid out like an affine map:
  `W/8 x H/8` entries, row by row, one block (not the text layer's 32x32 blocks). The tile is
  always 256 colours (64 bytes). Flips mirror the pixel inside the tile. Value 0 is transparent.
  The palette number is the extended palette's (`extended-palettes.md`): with DISPCNT bit 30 set the
  colour is entry `value` of palette `number` of the layer's slot; with it clear the colour is the BG
  palette entry `value` whatever the number.
- **256-colour bitmap** (bit 7 = 1, bit 2 = 0): the pixel is the byte at `base + ty * W + tx`;
  value 0 is transparent, otherwise the BG palette entry `value`.
- **Direct-colour bitmap** (bit 7 = 1, bit 2 = 1): the pixel is the halfword at
  `base + (ty * W + tx) * 2`: a 15-bit colour (as in `display-mode-vram.md`) with bit 15 as an alpha
  flag. Bit 15 clear is transparent and bit 15 set is the colour, so values 0000h-7FFFh are never
  shown.

### Large bitmap (mode 6, engine A only)

BG2 is a 256-colour bitmap of 512x1024 (size 0) or 1024x512 (size 1) pixels, a byte a pixel, rows
in order, from the start of engine A's BG VRAM; it needs banks A-D (512 KiB) all mapped there.
Position, wrap, transparent 0 and the palette are the 256-colour bitmap's. The reference point and
PA-PD move it like any other affine layer. BG0, BG1 and BG3 do not exist in this mode.

### Mosaic

MOSAIC has four 4-bit fields: BG horizontal size minus 1 (bits 0-3), BG vertical size minus 1
(4-7), and OBJ's (8-11 and 12-15; they size the sprite mosaic, `obj.md`, "Mosaic", and
the BG layers ignore them). A layer is mosaic when BGxCNT bit 6 is set; with both BG fields 0 the effect does nothing.

With size `h = field + 1` a mosaic pixel is a block of `h` screen pixels, and each takes the colour
of the first pixel of its block:

- **Horizontal**: screen pixel `x` shows what pixel `x - (x mod h)` would show.
- **Vertical**: line `y` shows the line `y - (y mod v)`. The blocks are counted from the top of
  the screen, whatever the layer's scroll.

The first pixel is taken from the layer's own picture (after scroll, flip and the affine
transform), so mosaic works on every kind: text, affine, extended and large bitmap. The
transparency of the source pixel carries over: a block whose first pixel is transparent is
transparent. For an affine layer the vertical mosaic must not move the layer's point down while a
block lasts: line `y` of a block uses the reference point of the block's first line.

### Engine B

Engine B is drawn by the same rules, with these parts missing:

| Part | Engine A | Engine B |
|---|---|---|
| display modes | 0-3 | 0-1 |
| VRAM block (DISPCNT 18-19), the capture unit | yes | no |
| character and screen base (DISPCNT 24-29) | yes | no: both 0 |
| BG0 as the 3D layer (DISPCNT bit 3) | yes | no: BG0 is text |
| BG mode 6, the large bitmap | yes | no |

Where engine B reads:

- BG VRAM: its own window, up to 128 KiB at `0620_0000h` (`vram-banks.md`).
- BG palette: the first 512 bytes of its half of the palette RAM, `0500_0400h` (`palette-ram.md`).
- Backdrop: BG palette entry 0 of that palette.

POWCNT1 bit 9 powers engine B. GBATEK lists the ports it gates as `1008h-105Fh` (engine A's:
`008h-05Fh`), so DISPCNT at `1000h` is not among them. With the bit clear those registers are
read-only; DISPCNT of engine B stays writable; with the bit set all work as engine A's do. A
powered-off engine draws nothing, so the screen it feeds is black.

POWCNT1 bit 15 chooses which screen each engine feeds (`display-mode-vram.md`); engine B takes
the other one.

### Putting the layers together

Unchanged from `bg-text.md`: at each pixel the front-most layer that is not transparent gives the
colour, ordered by BGxCNT priority and then layer number, else the backdrop. Sprites, windows (`windows.md`) and
blending are not part of this note.

### When the picture is made

Lines are made at the start of each visible line, as in `bg-text.md`. After a visible line is
made, the internal reference points of both affine layers of both engines step by PB and PD. At
the start of VBlank (line 192) they are loaded from BGxX and BGxY. A frame draws the lines top to
bottom, so the first line of a frame uses the registers as they were at VBlank.

## Edge cases

- A negative PA with a wrap draws the layer mirrored; a PA of 0 repeats one column of the layer
  across the line.
- A reference point that puts every pixel outside the layer, with wrap off, gives an all-transparent
  layer; the layers behind it or the backdrop show.
- An affine or extended layer in a mode where the layer is another kind uses the kind the mode
  gives. A layer that is none draws nothing, even with its enable bit set.
- Mode 6 with BG0's enable bit: BG0 is the 3D layer, so it draws nothing yet.
- A direct-colour pixel of 0000h is transparent, so is 7FFFh; 8000h is opaque black.
- The same tile data can serve several layers, and an affine map may use tile 0.
- BGxX or BGxY written twice before a line leaves the second value as the line's origin.
- A mosaic size field of 0 for both BG fields draws the picture unchanged.

## Unverified

- **Reset values** of PA-PD, BGxX, BGxY and MOSAIC are not stated; they start at 0, so an affine
  layer that software forgot to set up is a single point. — `Engine::new` in
  `crates/core/src/gpu2d/engine.rs` — verify by a test ROM on real hardware.
- **Reads** of PA-PD, BGxX, BGxY and MOSAIC return 0: they are write-only and GBATEK does not say
  what a read gives. — `Engine::read_byte` — verify likewise.
- **The 28-bit width of the internal reference point.** GBATEK says the registers hold 28 bits
  and does not say whether the internal point keeps adding at 28 bits or at 32; this model adds
  at 32 bits (wrapping). — `Affine::end_line` — verify likewise.
- **The sampling instant** of PA-PD, BGxX and BGxY inside a line is the line's start, as for every
  register (`bg-text.md`, "Unverified"); the hardware reads them while it draws. — `Machine::render_line`
  — verify against a trace.
- **A write to BGxX/BGxY in the same line as the reload** at VBlank: the register wins, as a
  write always loads the internal point. — `Engine::write_byte` — verify likewise.
- **Mosaic and affine layers**: that line `y` of a block uses the reference point of the block's
  first line (so PB and PD are not added while the block lasts) is inferred from GBATEK's "colorized
  by the upper-left covered pixel"; GBATEK does not describe the internal point under mosaic. —
  `Engine::render_line` in `crates/core/src/gpu2d/engine.rs` — verify likewise.
- **A BGxX or BGxY write inside a mosaic block**: the point loaded by the write is the origin of the
  next line, but a mosaic layer steps the point back by `y mod v` lines to reach the block's first
  line, so the rewind crosses the write and the lines after it start from a point that is not the one
  written. The hardware's behavior is not described. — `Engine::render_line` — verify likewise.
- **Mosaic and BG scroll or the transform**: the first pixel of a block is the layer's own pixel
  at the block's first screen position; GBATEK does not say whether the scroll is applied before
  or after the mosaic. — `Engine::render_line` — verify likewise.
- **Mosaic on transparent pixels**: a block whose first pixel is transparent is transparent. —
  `Engine::render_line` — verify likewise.
- **Large bitmap sizes 2 and 3** are not defined; a layer with them draws nothing. —
  `AffineBg::decode` in `crates/core/src/gpu2d/affine.rs` — verify likewise.
- **Large bitmap and the area overflow bit** are not described together; the bit works as in the
  other bitmaps. — `affine::pixel` — verify likewise.
- **BGxCNT bits 7 and 2 in mode 6** do nothing: BG2 is the large bitmap whatever they say (they
  pick the kind only in the extended modes). — `AffineBg::decode` — verify likewise.
- **Mode 6 on engine B** (reserved) draws no layer. — `kind::kind_of` — verify likewise.
- **Extended map with the standard palette**: the palette number of a 16-bit entry has no
  effect and the colour is the standard BG palette entry `value`. GBATEK lists the extended
  palette as the one these tiles use and does not say what the standard palette does for them.
  — `affine::pixel` — verify likewise.
- **Engine B with bits it does not have**: DISPCNT bits 3, 18-19, 24-29 on engine B and the
  display modes 2 and 3 are stored as written and ignored; display mode bit 17 does not exist
  there, so engine B's mode is bit 16 alone. — `EngineSpec` in `crates/core/src/gpu2d/spec.rs` —
  verify likewise.
- **Engine B powered off**: the registers `1008h-105Fh` are read-only and the screen is black.
  GBATEK says the ports become read-only and the palette "becomes read-only-zero-filled"; what the
  LCD shows is not said, and the palette RAM here is not zero-filled (a powered-off engine draws
  nothing, so it does not matter to the picture, but a read of `0500_0400h` still returns the
  stored bytes). — `Gpu2d::write_byte`, `Gpu2d::render_line_b` — verify likewise.
- **The area overflow bit in bitmap modes**: the "LCD I/O BG Rotation/Scaling" page says it is
  ignored in bitmap modes and the bitmap's outside is always transparent; the "DS Video BG Modes /
  Control" page says DS bitmaps support it. This note follows the DS page. — `affine::pixel` —
  verify likewise.
- **Engine A's POWCNT1 bit 1** still does nothing (`display-mode-vram.md`, "Unverified").
- **Mode 7** (not defined) draws no layer, on both engines (`bg-text.md`, "Unverified").
- **Extended palettes** (DISPCNT bit 30) are `extended-palettes.md`, with its own "Unverified".

## Evidence

Each test ROM builds its tiles, maps, bitmaps and palette from arithmetic, and the expected
pixels are worked out from this note in `crates/runner/tests/`, never read from the emulator.
`bgengb` shows engine B's text layers, `bgaffine` affine layers with rotation, scale, wrap on and
off and a reference-point write in mid-frame, `bgext` the three extended kinds, `bglarge` the large
bitmap and `bgmosaic` mosaic on text, affine and bitmap layers. Between them both engines
and BG modes 0-6 are covered: mode 0 (`bgtext`, `bgengb`), modes 2 and 1 (`bgaffine`, engine A and
B), modes 5 and 3 (`bgext`, engine A and B), mode 4 (`bgmosaic`), mode 6 (`bglarge`).
