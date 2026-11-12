# Sprites (OBJ): OAM, tile, affine and bitmap objects, priority, mosaic and the line limit

Sources: GBATEK "LCD OBJ - Overview" (the per-line cycle budget), "LCD OBJ - OAM Attributes",
"LCD OBJ - OAM Rotation/Scaling Parameters" and "LCD OBJ - VRAM Character (Tile) Mapping" (the
object format, inherited from the GBA); GBATEK "DS Video OBJs" (the DS priority rule, 1D
boundaries, bitmap objects, mode 3, vertical wrap); GBATEK "DS Video BG Modes / Control" (DISPCNT);
GBATEK "LCD I/O Mosaic Function" (MOSAIC); GBATEK "LCD I/O BG Rotation/Scaling" (what PA-PD mean);
GBATEK "LCD Color Palettes" (the OBJ palette, transparent colour 0); GBATEK "DS Memory Control -
Memory Access" and "DS Memory Maps" (OAM location and the write widths).
Used by: `crates/core/src/gpu2d/`, `crates/core/src/bus/memory/`   Phase: 4 (#31); the
semi-transparent mode (#34), the OBJ window (#33), OBJ extended palettes (#32), capture (#36) and
the OAM panel in the debugger (#58) are other issues

`bg-text.md` describes the picture of one line and `bg-modes.md` the layers behind the sprites.
This note adds the sprites: where their attributes live, how one object is turned into pixels, and
how those pixels meet the background layers. Both engines are the same circuit and everything below
holds for both unless it says "engine A only".

## Registers and memory

OAM is not a register block. It is 2 KiB of memory the ARM9 reads and writes.

| Address | What |
|---|---|
| `0700_0000h`-`0700_03FFh` | engine A's OAM, 1 KiB |
| `0700_0400h`-`0700_07FFh` | engine B's OAM, 1 KiB |

The 2 KiB repeat through the whole 16 MiB block `0700_0000h`-`07FF_FFFFh`, like palette RAM
(`palette-ram.md`). Only the ARM9 sees OAM. A store is a halfword or a word; a byte store is
ignored and changes nothing (`vram-banks.md`, "Behavior"; GBATEK "Writing 8bit Data to Video
Memory" states that 8-bit writes to OAM are ignored). A word or halfword store goes to the
aligned address (`arm9-memory-map.md`, "Alignment").

### DISPCNT bits used here

| Bits | Meaning | Engines |
|---|---|---|
| 4 | tile OBJ mapping: 0 = 2D, 1 = 1D | both |
| 5 | bitmap OBJ 2D width: 0 = 128 pixels, 1 = 256 pixels | both |
| 6 | bitmap OBJ mapping: 0 = 2D, 1 = 1D | both |
| 12 | sprites on | both |
| 20-21 | tile OBJ 1D boundary, `32 << n` bytes | both |
| 22 | bitmap OBJ 1D boundary: 0 = 128 bytes, 1 = 256 bytes | engine A only |
| 23 | OBJ processing during H-Blank (the line budget, below) | both |
| 31 | OBJ extended palettes | both (`TODO(phase4)`, #32) |

The other bits are in `bg-text.md`, `bg-modes.md` and `display-mode-vram.md`. Bit 5 is "H-Blank
interval free" on the GBA; on the DS that meaning moved to bit 23 and bit 5 became the bitmap width.
GBATEK's GBA pages also call bit 6 the OBJ mapping; the DS page puts it in bit 4 and this note
follows the DS page.

MOSAIC bits 8-11 (OBJ horizontal size minus 1) and 12-15 (OBJ vertical size minus 1) are decoded
already (`bg-modes.md`, "Mosaic"); this note gives them their use.

## The object table

OAM holds 128 **entries** of 8 bytes. Entry `n` is at offset `8 * n` of the engine's half and has
three halfwords of its own:

| Offset | Name | Meaning |
|---|---|---|
| `+0` | attr0 | Y, shape of the object, colours, mode, flags |
| `+2` | attr1 | X, flip bits or the affine group, size |
| `+4` | attr2 | tile number, priority, palette |
| `+6` | | not part of the entry: one halfword of an affine parameter group (below) |

### attr0

| Bits | Meaning |
|---|---|
| 0-7 | Y, 0-255 |
| 8 | 1 = affine object (rotation/scaling) |
| 9 | affine object: 1 = double-size; plain object: 1 = **disabled**, not drawn |
| 10-11 | mode: 0 normal, 1 semi-transparent (`TODO(phase4)`, #34), 2 OBJ window (`TODO(phase4)`, #33), 3 bitmap object |
| 12 | mosaic on for this object |
| 13 | 0 = 16 colours of 16 palettes, 1 = 256 colours of one palette (not for a bitmap object, where it should be 0) |
| 14-15 | shape: 0 square, 1 wide, 2 tall, 3 not allowed |

### attr1

| Bits | Meaning |
|---|---|
| 0-8 | X, 0-511 |
| 9-13 | affine object: which of the 32 parameter groups |
| 12 | plain object: horizontal flip |
| 13 | plain object: vertical flip (bits 12-13 are the flips only when bit 8 of attr0 is clear) |
| 14-15 | size, with the shape the table below |

| Size | square | wide | tall |
|---|---|---|---|
| 0 | 8x8 | 16x8 | 8x16 |
| 1 | 16x16 | 32x8 | 8x32 |
| 2 | 32x32 | 32x16 | 16x32 |
| 3 | 64x64 | 64x32 | 32x64 |

### attr2

| Bits | Meaning |
|---|---|
| 0-9 | tile number |
| 10-11 | priority, 0 the highest |
| 12-15 | palette number (16-colour objects); for a bitmap object these four bits are an alpha value (`TODO(phase4)`, #34) |

## Behavior

### Where an object is on the screen

The object is a rectangle of its width `W` by height `H` (the table above). An affine object
with double-size set is drawn in a box of `2W` by `2H`; any other object's box is `W` by `H`. The
box's top-left corner is at the object's X and Y.

- **Y** is 0-255 and the box covers lines `(Y + r) mod 256` for box rows `r`; only lines 0-191 are
  on the screen. A box that runs past line 255 continues at line 0, so it shows at the top of the
  screen. GBATEK says that on the DS a tall object near the bottom appears both at the bottom and
  at the top (so software can switch off the unwanted part), unlike the GBA.
- **X** is a 9-bit signed number: 0-255 as it is, 256-511 as the value minus 512. A box that starts
  left of column 0 shows only its part from column 0 on; one that runs past column 255 is cut.
  (The columns do not wrap.)

### A plain tile object

The object is a grid of 8x8 tiles, `W / 8` across and `H / 8` down. Its pixel at column `c`, row
`r` (before flip) belongs to tile column `c / 8`, tile row `r / 8`, pixel `(c mod 8, r mod 8)` of
that tile. The flips mirror the whole object: horizontal flip takes column `W - 1 - c`, vertical
flip row `H - 1 - r`, so the tiles trade places as well as their pixels.

A tile is 32 bytes at 4 bits a pixel and 64 bytes at 8 bits a pixel, with the pixels inside it laid
out like a BG tile (`bg-text.md`, "Behavior": the left pixel in the low nibble of a byte). The
tile number in attr2 names the object's top-left tile; where the others are depends on the mapping
mode.

**Tile address** is an offset into the engine's OBJ VRAM window (`vram-banks.md`: `0640_0000h`
for engine A, 256 KiB, `0660_0000h` for engine B, 128 KiB).

- **2D mapping** (DISPCNT bit 4 clear). The tile numbers form a matrix 32 wide: one row down is
  `+32` in the tile number. A tile number counts 32 bytes. With 16 colours the object's tile at
  grid `(tc, tr)` has number `tile + tc + 32 * tr`. With 256 colours a tile is two numbers wide, so
  its number is `tile + 2 * tc + 32 * tr` (GBATEK's example: 04h and 06h, then 24h and 26h) and bit
  0 of `tile` is ignored. Address = number * 32.
- **1D mapping** (bit 4 set). The object's first tile is at byte `tile * boundary`, where
  `boundary = 32 << DISPCNT.20-21` (32, 64, 128 or 256 bytes; engine B's window is 128 KiB). The
  other tiles follow it one after another, row after row, each 32 bytes (16 colours) or 64 bytes
  (256 colours): the tile at grid `(tc, tr)` is `tc + tr * (W / 8)` tiles after the first.
  GBATEK's example: a 16x16 object has tiles 04h, 05h, 06h, 07h; with 256 colours 04h, 06h, 08h,
  0Ah (in units of 32 bytes, boundary 32).

"Even if the boundary changes, objects stay composed of 8x8 tiles": the boundary moves only where
the first tile of an object may start.

**Colours.** A pixel value of 0 is transparent in both depths. Otherwise a 16-colour object takes
entry `palette * 16 + value` of the engine's OBJ palette; a 256-colour object takes entry `value`.
The OBJ palette is the second 512 bytes of the engine's palette half (`palette-ram.md`: `0500_0200h`
for engine A, `0500_0600h` for engine B). With DISPCNT bit 31 the palettes come from extended
palette memory (`TODO(phase4)`, #32).

### An affine object

Attr0 bit 8 makes the object affine: its picture is rotated and scaled by one of 32 parameter
groups. The groups sit in the fourth halfword of each entry: group `g` has PA at entry `4g`, PB at
entry `4g + 1`, PC at entry `4g + 2`, PD at entry `4g + 3`, each at offset `+6` of the entry (so
group 0 is at `0700_0006h`, `0700_000Eh`, `0700_0016h`, `0700_001Eh`; group 1 at `0700_0026h`, ...).
Attr1 bits 9-13 select the group. The four numbers are signed 8.8, like the BG's (`bg-modes.md`,
"Reference point and parameters").

The object's own picture is `W` by `H` pixels, the same tile grid as a plain object (the flip bits
do not exist for an affine object). It is drawn into the box of `W` by `H`, or `2W` by `2H` with
double-size, whose centre is the centre of rotation. For a box pixel at column `bx`, row `by`:

- `dx = bx - boxW / 2`, `dy = by - boxH / 2` (the offset from the box centre, `boxW` and `boxH`
  the box's size);
- `u = ((PA * dx + PB * dy) >> 8) + W / 2` and `v = ((PC * dx + PD * dy) >> 8) + H / 2`, an
  arithmetic shift (rounding toward minus infinity);
- the pixel shows the object's picture at `(u, v)` when `0 <= u < W` and `0 <= v < H`, and is
  transparent when it falls outside. So without double-size the corners of a rotated picture are
  cut at the edge of the box, and with double-size they stay when they fit.

The identity is PA = PD = 100h and PB = PC = 0. PA-PD are read when the line is made.

### A bitmap object

Attr0 mode 3 makes a bitmap object. Its pixels are 15-bit direct colour in 16-bit words: bits 0-4
red, 5-9 green, 10-14 blue, and **bit 15 set means opaque, clear means transparent**. There is no
palette. Attr2 bits 12-15 hold an alpha value (`TODO(phase4)`, #34) and are not a palette number.
The object's size and its flip bits (plain) or affine group (affine) work as above; its picture is
`W` by `H` pixels.

Where the pixels are depends on DISPCNT:

| Bit 6 | Bit 5 | Bit 22 | Mapping |
|---|---|---|---|
| 0 | 0 | any | 2D, the source bitmap is 128 pixels wide |
| 0 | 1 | any | 2D, the source bitmap is 256 pixels wide |
| 1 | 0 | 0 | 1D, boundary 128 bytes |
| 1 | 0 | 1 | 1D, boundary 256 bytes (engine A only) |
| 1 | 1 | any | reserved |

- **1D**: the object's first pixel is at byte `tile * boundary` (tile number 0-3FFh) of the OBJ
  VRAM window, and its rows follow with a stride of `W * 2` bytes ("source width = target width").
- **2D**: with `MaskX` = 0Fh (128 wide) or 1Fh (256 wide), the first pixel is at byte
  `(tile & MaskX) * 10h + (tile & ~MaskX) * 80h`, and the rows follow at a stride of the source
  bitmap's width times 2 bytes (256 or 512). The tile number's low bits are the column (in
  8-pixel steps), the rest the row (in 8-line steps).

### Priority

Two kinds of priority decide what is in front:

- **Object against object.** The DS orders objects by a 9-bit number made of attr2's priority
  (bits 10-11, the high part) and the entry number (the low 7 bits); the lowest number is in front.
  So a priority 0 object is in front of a priority 1 object whatever their entries, and among
  equal priorities the lower entry is in front. (On the GBA only the entry number counted.) A
  transparent pixel of an object lets the next one show through.
- **Object against background layer.** An object is in front of a background layer whose priority
  (BGxCNT bits 0-1) is greater than or equal to the object's attr2 priority. So at equal priority
  the object wins. Where several layers qualify, the layer order of `bg-text.md` applies among
  the layers and the object is in front of the ones it beats. The backdrop is behind everything.

The result at a pixel is: the winning object pixel when one is not transparent and not behind
the best background pixel; otherwise the best background pixel; otherwise the backdrop.

### The layer is on when

DISPCNT bit 12 is set and the engine's display mode is the normal picture (mode 1,
`display-mode-vram.md`); with bit 12 clear no object is drawn, whatever OAM holds. An object
with mode 2 (OBJ window) is not drawn (`TODO(phase4)`, #33); one with mode 1 is drawn as a normal
object (`TODO(phase4)`, #34).

### Mosaic

An object with attr0 bit 12 set is mosaic when MOSAIC's OBJ fields are not both 0. With `h` = bits
8-11 plus 1 and `v` = bits 12-15 plus 1, a block of `h` by `v` screen pixels takes the colour of
its upper-left pixel: screen pixel `(x, y)` shows what the object would show at
`(x - x mod h, y - y mod v)`. Blocks are counted from the top-left of the screen, as the background's
(`bg-modes.md`, "Mosaic"). A block whose upper-left pixel is transparent is transparent.

### The line limit

An engine can do only so much object work per line. The budget in cycles is:

- **1210** (304 * 4 - 6) when DISPCNT bit 23 is clear;
- **954** (240 * 4 - 6) when it is set (objects are then processed while the screen is drawn and not
  during H-Blank).

Each object that the line has to look at costs, with `n` the **width in pixels of its box on the
screen** (8-64 for a plain object, 8-64 or 16-128 doubled for an affine one):

- plain: `n` cycles;
- affine: `10 + 2 * n` cycles.

The objects are looked at in entry order, 0 first. When the budget runs out the rest of the
objects are not drawn on this line. GBATEK says that offscreen objects cost as well, and that
those at low entry numbers therefore hurt the most: software should keep the visible ones first or
make the invisible ones 8x8 and not affine.

## Edge cases

- Object 0 is in front of object 1 only when its attr2 priority is not greater; the 9-bit rule
  above, not the entry alone.
- An object with its disable bit set (plain, bit 9) draws nothing. Its bit 9 is the double-size
  flag when bit 8 is set.
- Shape 3 is not allowed: such an object draws nothing.
- A 256-colour tile number in 2D mapping ignores bit 0; in 1D mapping the boundary multiplies it as
  it is.
- X 256 is column -256, so the box is entirely left of the screen (the widest box is 128).
- An object right at the top (Y = 0) and one at Y = 255 (a box row at line 255, then lines 0, 1,
  ...) are both on the screen.
- A palette index 0 pixel of a 16-colour object is transparent whatever its palette number.
- A bitmap object's pixel with bit 15 clear is transparent; `0000h` and `7FFFh` are not special.
- With all objects off-screen the layer is empty and the backgrounds draw as before.

## Unverified

Each item gets `TODO(hw-unverified)` at its code site.

- **OAM power-on contents.** GBATEK does not state them; OAM starts at zero, which is a visible
  8x8 object with tile 0 at (0, 0) for every entry. — `Memory::new` in
  `crates/core/src/bus/memory/mod.rs` — verify by a test ROM on real hardware.
- **The mirror through the 16 MiB block** is GBATEK's general rule, not a statement about OAM, as
  for the palette. — `Memory::build_page_table` — verify likewise.
- **The ARM7 reads 0 and drops writes** at these addresses (nothing is there for it); GBATEK gives
  no value for an unmapped read. — `Memory::build_page_table` — verify likewise.
- **What an object reads past its VRAM window** (an unmapped tile, or tile 1023 with boundary 256
  in engine B's 128 KiB): the pixel is 0, so it is transparent. GBATEK says only that engine B has
  128 KiB. — `obj::tile_value` — verify likewise.
- **Engine B and DISPCNT bit 22** (engine A only): engine B treats it as 0. — `SUB_SPEC` in `gpu2d/spec.rs`
  — verify likewise.
- **X as a signed 9-bit number.** GBATEK says 0-511 and does not say signed; the wrap is what the
  GBA does. — `ObjAttrs::decode` — verify likewise.
- **Vertical wrap.** Two GBATEK pages disagree: "OAM Attributes" says a very large object at
  Y > 128 is shown only at the top, "DS Video OBJs" says the DS shows it at both ends. This note
  follows the DS page: box rows are on line `(Y + r) mod 256`. — `obj::placement` — verify
  likewise.
- **The affine arithmetic.** GBATEK gives no formula for an object; this note uses the centre of
  the box and the BG's 8.8 convention, so the sampling instant (pixel corner or centre) and the
  rounding of `(PA * dx + PB * dy) >> 8` are inferred. — `obj::affine_source` — verify likewise
  with a rotated test pattern.
- **Flip on a bitmap object** is taken to be the same as on a tile object. — `obj::bitmap_color`
  — verify likewise.
- **Whether the budget numbers of the GBA hold on the DS.** 1210 and 954 are GBATEK's GBA values
  (240 visible dots, 308 dots a line); the DS has 256 visible dots and 355 a line, so the real
  numbers are probably different. Also whether a double-size affine object costs `10 + 2 * 2W`.
  GBATEK's table says so ("16..128 pixels"). — `Placement::cost` in `obj.rs` — verify likewise.
- **The cost of an object that is off-screen** horizontally, or **not on the current line**: GBATEK
  says offscreen objects cost and asks whether vertically offscreen ones do too. This model charges
  every object that the line reaches and one whose vertical range does not contain the line costs
  nothing. — `obj::draw_line` — verify likewise.
- **The cost of a disabled object** (attr0 bit 9 on a plain object). Assumed to be 0. —
  `obj::draw_line` — verify likewise.
- **Which object loses when the budget runs out in the middle of one:** the whole object is
  dropped here, and so are all later ones; the hardware may draw part of it. — `obj::draw_line` —
  verify likewise.
- **Bit 23's polarity.** On the GBA the bit is "H-Blank Interval Free": set, OAM may be used in
  H-Blank and the budget is 954. GBATEK names the DS bit "OBJ Processing during H-Blank" and says
  only that it moved from GBA bit 5; read literally that is the opposite polarity. This note takes
  the GBA meaning (set = 954). — `obj::draw_line` — verify likewise.
- **Bit 23's effect beyond the budget.** GBATEK states only the two budgets; whether OAM access
  timing changes the picture is not modelled. — `obj::draw_line` — verify likewise.
- **The mosaic block origin for objects** is the screen's, as for the backgrounds; GBATEK says
  only that the colour is the upper-left covered pixel, and that scrolling or moving the object
  shifts the pick. — `obj::placement` — verify likewise.
- **Mosaic on an affine object**: the block's upper-left pixel is taken on the screen, then
  transformed. — `obj::placement` — verify likewise.
- **OAM writes during a visible line:** a store lands at once and the next line sees it; the
  hardware may block or garble it. — `Machine::render_line` — verify against a trace.
- **The sampling instant** of OAM inside a line is the line's start, as for every register
  (`bg-text.md`, "Unverified"). — `Machine::render_line` — verify against a trace.
- **Tiles 0-511 in bitmap BG modes** (a GBA rule) do not apply: the DS keeps objects in their own
  VRAM windows. — none — noted so nobody adds it.
- **Shape 3 and mode 3 with attr0 bit 13 set:** shape 3 draws nothing and costs no budget; a
  bitmap object with the colour bit set is drawn as a bitmap object regardless. —
  `obj::placement` and `obj::bitmap_color` — verify likewise.
- **DISPCNT bits 5 and 6 both set** (reserved for bitmap objects): a bitmap object draws nothing.
  — `obj::bitmap_color` — verify likewise.
- **The cost of an OBJ-window object** (mode 2): it costs its cycles like any other object though
  it is not drawn yet (#33). — `obj::draw_line` — verify likewise.
- **Semi-transparent and OBJ-window objects** are drawn as normal objects / not drawn until #34
  and #33; bitmap alpha is ignored until #34; extended palettes (bit 31) until #32.

## Evidence

Each test ROM builds its tiles, bitmaps, palettes and OAM from arithmetic, and the expected pixels
are worked out from this note in `crates/runner/tests/`, never read from the emulator: `objtext`
(tile objects: sizes, depths, flips, the mappings, the edges), `objprio` (priority against the
backgrounds and between objects), `objaffine`, `objbmp`, `objmosaic` and `objlimit` (the line
budget, with and without DISPCNT bit 23). Each uses both engines.
