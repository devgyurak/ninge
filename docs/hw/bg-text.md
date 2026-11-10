# Engine A text backgrounds

Sources: GBATEK "DS 2D Video - LCD Control" (DISPCNT, the BG modes); GBATEK "DS 2D Video - BG
Control" (BGxCNT, BGxHOFS/VOFS); GBATEK "DS 2D Video - Text Background Screen / BG Tile Map"
(screen entries and the tile layout); GBATEK "DS 2D Video - Color Palettes" and "DS 2D Video -
Color Format"; GBATEK "DS 2D Video - Display Priority".
Used by: `crates/core/src/gpu2d/`   Phase: 3 (#28, engine A, standard palettes); engine B,
sprites, extended palettes, mosaic, windows and blending are Phase 4

A text background is a grid of 8x8-pixel tiles. A map says which tile goes in each cell, the
tile data says what each tile looks like, and the palette RAM (`palette-ram.md`) says what the
tile's pixel values mean. Engine A draws up to four of them and puts the backdrop color behind
them.

## Registers

Engine A's registers sit in the ARM9's I/O block. All are halfword or word aligned; a byte write
changes that byte alone.

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0000h` | DISPCNT | 32 | R/W | 0 (`display-mode-vram.md`) |
| `0400_0008h` | BG0CNT | 16 | R/W | 0 (see "Unverified") |
| `0400_000Ah` | BG1CNT | 16 | R/W | 0 |
| `0400_000Ch` | BG2CNT | 16 | R/W | 0 |
| `0400_000Eh` | BG3CNT | 16 | R/W | 0 |
| `0400_0010h` | BG0HOFS | 16 | W | 0 |
| `0400_0012h` | BG0VOFS | 16 | W | 0 |
| `0400_0014h` / `0016h` | BG1HOFS / BG1VOFS | 16 | W | 0 |
| `0400_0018h` / `001Ah` | BG2HOFS / BG2VOFS | 16 | W | 0 |
| `0400_001Ch` / `001Eh` | BG3HOFS / BG3VOFS | 16 | W | 0 |

### DISPCNT bits used here

| Bits | Meaning |
|---|---|
| 0-2 | BG mode, 0-6 (7 is not defined) |
| 3 | BG0 is 2D (0) or 3D (1) |
| 7 | forced blank (stored, not acted on: `display-mode-vram.md`, "Unverified") |
| 8-11 | BG0-BG3 on: bit 8 + n shows background n |
| 16-17 | display mode: 0 off (white), 1 graphics, 2 VRAM, 3 main memory; this note is mode 1 |
| 24-26 | character base: tile data starts `N * 64 KiB` into BG VRAM |
| 27-29 | screen base: maps start `N * 64 KiB` into BG VRAM |
| 30 | BG extended palettes |

The other bits (OBJ, windows, OBJ mappings, OBJ extended palettes, VRAM block, 31) belong to
Phase 4 and are stored and ignored.

### BGxCNT bits

| Bits | Meaning |
|---|---|
| 0-1 | priority 0-3; 0 is in front |
| 2-5 | character base block: tile data starts `N * 16 KiB` past the DISPCNT character base |
| 6 | mosaic |
| 7 | colors: 0 = 16 colors in 16 palettes (4 bits per pixel), 1 = 256 colors (8 bits per pixel) |
| 8-12 | screen base block: the map starts `N * 2 KiB` past the DISPCNT screen base |
| 13 | extended palette slot (BG0, BG1) / area overflow (affine); stored, not acted on |
| 14-15 | screen size (text): 0 = 256x256, 1 = 512x256, 2 = 256x512, 3 = 512x512 pixels |

### BGxHOFS / BGxVOFS

Bits 0-8 are the scroll in pixels; bits 9-15 are not used and are dropped. The registers are
write-only.

## Memory

All bases count from `0600_0000h`, the start of engine A's BG VRAM window (`vram-banks.md`,
"Engine A BG VRAM (MST 1)"):

- tile data address = `DISPCNT.char_base * 64 KiB + BGxCNT.char_block * 16 KiB`
- map address = `DISPCNT.screen_base * 64 KiB + BGxCNT.screen_block * 2 KiB`

Colors come from the engine A BG palette, the first 512 bytes of the palette RAM (`0500_0000h`):
256 colors of 16 bits (`palette-ram.md`, "Layout").

## Behavior

### Which layers exist

The BG mode says what each of the four layers is:

| Mode | BG0 | BG1 | BG2 | BG3 |
|---|---|---|---|---|
| 0 | text (or 3D) | text | text | text |
| 1 | text (or 3D) | text | text | affine |
| 2 | text (or 3D) | text | affine | affine |
| 3 | text (or 3D) | text | text | extended |
| 4 | text (or 3D) | text | affine | extended |
| 5 | text (or 3D) | text | extended | extended |
| 6 | text (or 3D) | none | large bitmap | none |

Phase 3 draws the text layers only. An affine, extended or large-bitmap layer, and a layer
that is none, draws nothing (`TODO(phase4)`). BG0 with DISPCNT bit 3 set is the 3D engine's
picture, which draws nothing yet (`TODO(phase7)`). A layer is drawn only when its DISPCNT bit
8 + n is set and the display mode is 1; mode 1 is the only one that shows backgrounds.

### The picture of one layer at one pixel

For the pixel (x, y) of the screen (y is the line, 0-191) of layer n:

1. The point in the layer is `px = (x + HOFS) mod W`, `py = (y + VOFS) mod H`, where W x H is
   the size BGxCNT bits 14-15 pick (256 or 512 wide, 256 or 512 high). A scroll value is 9
   bits, and the layer wraps around at its own size.
2. The cell is `(px / 8, py / 8)`. A map is made of blocks of 32x32 cells (2 KiB, one cell is
   one halfword, row by row). A 256x256 layer is one block. A 512x256 layer is two blocks side
   by side (block `cx / 32`), 256x512 two on top of each other (block `cy / 32`), 512x512 four,
   in the order top-left, top-right, bottom-left, bottom-right (block `(cy / 32) * 2 + cx / 32`).
   Within a block the cell is `(cy mod 32) * 32 + (cx mod 32)`, each block following the
   previous one in memory.
3. The screen entry (16 bits) is: bits 0-9 tile number, bit 10 flip horizontally, bit 11 flip
   vertically, bits 12-15 palette number.
4. The pixel inside the tile is `(px mod 8, py mod 8)`; a horizontal flip mirrors the column
   (`7 - column`), a vertical flip the row.
5. Tile data: a 4 bits per pixel tile is 32 bytes, 4 bytes a row, the left pixel in the low
   nibble of each byte; a tile of 8 bits per pixel is 64 bytes, 8 bytes a row, one byte a pixel.
   Tile `t` is at `tile data address + t * 32` (or `t * 64`).
6. The pixel value 0 is transparent in both modes. Otherwise the color is the BG palette
   entry `palette number * 16 + value` (4 bits per pixel) or `value` (8 bits per pixel; the
   palette number is not used).
7. A color is 15 bits, as in `display-mode-vram.md`, "Colour expansion".

### Putting the layers together

At a pixel the layer in front wins: the one with the lowest BGxCNT priority, and for equal
priorities the lower layer number (BG0 over BG1 over BG2 over BG3). The first layer in that
order whose pixel is not transparent gives the color. Where every layer is transparent, off or
not drawn, the pixel is the backdrop: BG palette entry 0.

### Lines

The picture is made one line at a time, at the start of each visible line (`display-timing.md`),
from the registers and memory as they are then. A write that lands between two lines therefore
shows from the next line on: the foundation for HBlank effects (`TODO(phase4)` for the rest of
the engine's per-line state).

## Edge cases

- A tile number past the end of the data, or a base block beyond what the mapped banks hold,
  reads memory that is not there: the pixel value is 0, so the pixel is transparent.
- Scroll values near the layer size wrap: HOFS 511 on a 512-wide layer shows column 511 first,
  then column 0.
- A 256-wide layer wraps at 256 even though the scroll register holds 9 bits: HOFS 256 equals 0.
- Two layers with the same priority: the lower layer number is in front.
- An enabled layer in a mode where it is not a text layer (an affine layer in mode 1's BG3)
  draws nothing; the layer behind it shows through.
- The same tile data can serve several layers and several palette numbers.

## Unverified

- **Reset values of BGxCNT, BGxHOFS and BGxVOFS** are not stated; they start at 0. —
  `Gpu2d::new` in `crates/core/src/gpu2d/mod.rs` — verify by a test ROM on real hardware.
- **Reads of BGxHOFS and BGxVOFS** return 0: GBATEK marks them write-only and does not say what
  a read gives. — `Gpu2d::read_byte` — verify likewise.
- **BGxCNT bit 13** is kept as written even for a text layer, where it matters only with
  extended palettes. — `Gpu2d::write_byte` — verify likewise.
- **Reads past the mapped banks.** A base block that points beyond bank A's 128 KiB slot reads
  0 (`vram-banks.md`, "Unverified"). — `BgVram::byte` — verify likewise.
- **Mode 7** (not defined) draws no layer. — `text::layer_kind` — verify likewise.
- **The sampling instant of a line** is its start; real hardware reads the registers while the
  line is being drawn. A write inside the visible part of a line is a half-line effect that this
  model rounds to the next line. — `Machine::render_line` — verify against a trace.
- **Extended palettes** (DISPCNT bit 30) are not modeled: an 8 bits per pixel layer uses the
  standard palette whatever the bit says (`TODO(phase4)`). — `text::pixel` — verify likewise.
- **Forced blank** and **master brightness** do not change the picture (`display-mode-vram.md`,
  "Unverified"; `TODO(phase4)`).
- **Mosaic** (BGxCNT bit 6) is stored and ignored (`TODO(phase4)`).

## Evidence

The `bgtext` test ROM (`testroms/bgtext/`) builds its tile data, maps and palette from simple
arithmetic, then enables several layers with different priorities, sizes, scroll values, flips
and color depths; the expected pixels are worked out from this note in
`crates/runner/tests/bgtext.rs`, never read back from the emulator. The libnds console ROM
(`testroms/console/`) uses one 4 bits per pixel 256x256 layer at screen block 31.
