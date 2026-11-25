# Display capture and the main-memory display (engine A)

Sources: GBATEK "DS Video Capture and Main Memory Display Mode" (`4000064h` DISPCAPCNT,
`4000068h` DISP_MMEM_FIFO); "DS Video Display System Block Diagram" (where the capture unit taps
the picture); "DS Video BG Modes / Control" (DISPCNT, display modes 0-3, the VRAM read block);
"DS DMA Transfers" (the ARM9 start modes 3 and 4); "DS Memory Control - VRAM" (LCDC mapping).
Used by: `crates/core/src/machine/render.rs`, `crates/core/src/gpu2d/`,
`crates/core/src/io/dma/`   Phase: 4 (#36)

Both features belong to engine A alone. Capture copies engine A's finished line (or a VRAM
bitmap, or the FIFO, or a blend of two of them) into an LCDC-mapped VRAM bank, so a later frame
can show it. The main-memory display streams a 256x192 bitmap from main RAM to the screen or to
capture, through a small FIFO that a DMA channel fills. Issue #36 covers both: they are the last
Phase 4 items, and the FIFO is capture's second source.

## Where the capture unit sits

In the block diagram engine A's layering output (BG, OBJ and the 3D layer) goes two ways: to
"Select Video Input and Master Bright A", and to the capture unit's "Select Capture Source". So
**source A is the layered picture whatever display mode engine A shows**, before the master
brightness (`blending.md`, "Master brightness"). The unit's other source, B, is a VRAM block
(the same block display mode 2 would show) or the main-memory display FIFO. Its output is
written to one of VRAM A-D.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0064h` | DISPCAPCNT | 32 | R/W | 0 (see "Unverified") |
| `0400_0068h` | DISP_MMEM_FIFO | 32 | W (reads 0, see "Unverified") | empty |

### DISPCAPCNT bits

| Bits | Meaning |
|---|---|
| 0-4 | EVA, the blend factor of source A (0..16) |
| 8-12 | EVB, the blend factor of source B (0..16) |
| 16-17 | VRAM write block: 0-3 = VRAM A-D (it must be in LCDC mode) |
| 18-19 | VRAM write offset: value n is `n * 8000h` bytes |
| 20-21 | capture size: 0 = 128x128, 1 = 256x64, 2 = 256x128, 3 = 256x192 |
| 24 | source A: 0 = the graphics screen (BG + 3D + OBJ), 1 = the 3D screen |
| 25 | source B: 0 = VRAM, 1 = the main-memory display FIFO |
| 26-27 | VRAM read offset: value n is `n * 8000h` bytes |
| 29-30 | capture source: 0 = A, 1 = B, 2 and 3 = A and B blended |
| 31 | enable (writing 1) / busy (reading 1) |

Bits 5-7, 13-15, 22-23 and 28 are unused: they read 0 and a write to them is dropped. GBATEK
prints the offset values as `0=00000h, 0=08000h, 0=10000h, 0=18000h`; this note reads that as the
typo it looks like, for the values 0, 1, 2, 3 (offsets `0`, `8000h`, `10000h`, `18000h`).

The VRAM **read block** is not in this register: it is DISPCNT bits 18-19, the same field display
mode 2 uses (`display-mode-vram.md`). The register accepts byte, halfword and word accesses.

## The capture pipeline

**Enable and busy.** Writing bit 31 as 1 asks for a capture; it begins at the next line 0, and
the bit reads 1 until the hardware clears it at line 192, whatever the capture size. A capture
therefore runs for the visible lines of one frame; a size smaller than 256x192 stops writing after
its last row and stays busy until line 192.

**Per line.** For each visible line `y` of a running capture, the unit makes `width` pixels, for
`y` below the size's height, and writes them into the write block. The line it taps is the one
engine A made for that line, before the master brightness.

**Sizes.** The sizes below 256x192 capture the upper-left part of the screen: pixel `(x, y)` of
the capture is pixel `(x, y)` of the line, for `x < width` and `y < height`.

**Where it goes.** The halfword for `(x, y)` goes to byte offset
`(write_offset * 8000h + (y * width + x) * 2) & 1FFFFh` of the write block: the offset wraps to
the start of the 128 KiB bank (the register note says offsets "wrap to 00000h when exceeding
1FFFFh"; this note applies the wrap to every halfword, so a capture that starts near the end
continues at the start of the bank). The row stride is the capture width (see "Unverified").

**Colour.** Capture data is 15-bit colour. The halfword it writes is the colour in bits 0-14 and
the alpha in bit 15. A source's alpha is 1 for every pixel of the 2D graphics screen (see
"Unverified"), and for a VRAM source it is bit 15 of the halfword that is read.

| Capture source | Stored pixel |
|---|---|
| A | A's colour and A's alpha |
| B | B's colour and B's alpha |
| A and B blended | per colour channel `(A * alphaA * EVA + B * alphaB * EVB) / 16`, alpha = `(alphaA and EVA > 0) or (alphaB and EVB > 0)` |

In the blend, `alphaA` and `alphaB` are 0 or 1, and EVA and EVB are used only in this source. The
blend is done on each of red, green and blue (5 bits each).

**Source B from VRAM.** The pixel is the halfword at byte offset
`(read_offset * 8000h + (y * width + x) * 2) & 1FFFFh` of the read block, where `read_offset` is
DISPCAPCNT bits 26-27, and it **is 0 in VRAM display mode (display mode 2)**: GBATEK says the
read offset is ignored (zero) there. A read block that is not in LCDC mode reads as 0.

**Source B from the FIFO.** The pixel is the next pixel of the main-memory display (below), for
`x < width` of the line; the FIFO delivers a whole 256-pixel line per line whatever the size.

**Source A from the 3D screen** (bit 24 = 1) is the 3D layer's line (`rasterizer.md`, "The output"):
colour with alpha 1 where the pixel's 5-bit alpha is above 0, a zero pixel elsewhere. Until the
rasterizer draws (#49) the line is transparent, so source A is a zero pixel in that case.

## The main-memory display

DISP_MMEM_FIFO takes a 15-bit-colour 256x192 bitmap from main RAM by DMA, to engine A's screen
(display mode 3) or to the capture unit (source B = FIFO). GBATEK describes the transfer: DMA
start mode "main memory display", 32-bit units, a word count of 4 (so a request is 4 words = 8
pixels), the destination `DISP_MMEM_FIFO`, the source in main RAM. "Transfer starts at next
frame." Bit 15 of each pixel is unused.

**Model.** A line is 256 pixels = 128 words = 32 requests of 4 words. The model keeps the FIFO as a
queue of 15-bit pixels with room for one line (256). A channel in start mode 4 gets 32 requests at
once, in one atomic transfer of `32 * count` words (`count` is 4 for the layout above), raised
once per line for the **next** line: at the start of every visible line but the last, and at the
start of the last line of the frame, so line 0 already finds its pixels. A word written to
DISP_MMEM_FIFO (by the DMA unit or by the CPU) pushes its two pixels, low halfword first; a pixel
that does not fit is dropped.

At the start of each visible line, before anything draws, the display takes one line, 256
pixels, out of the FIFO whether or not anything shows it, and a pixel that is not there reads as 0
(black). That line is what display mode 3 shows and what capture reads as source B = FIFO.

A channel enabled with start mode 4 does not take part in the lines of the frame it was enabled
in: it starts at the last line of that frame, the first request of the next one ("Transfer starts at
next frame"), so the next frame's line 0 finds its pixels. Enabling it again (the enable bit going
from 0 to 1) makes it wait again, and reloads the source address, which is how a program starts each
frame at the top of its bitmap: the channel only repeats, so its source runs on past the bitmap
after one frame.

**Display mode 3** shows that line, the same way display mode 2 shows a VRAM line: 15-bit colour,
then the master brightness and the host conversion (`blending.md`, "Master brightness").

## ARM9 DMA start modes 3 and 4

| Value | Start |
|---|---|
| 3 | synchronize to the start of display: the channel starts at the start of line 0 |
| 4 | main-memory display: 32 requests per line, as above |

Both are raised by the machine's line start (`dma.md`, "Starting"), like the VBlank and HBlank
starts. Mode 3 is raised once per frame (see "Unverified"). The machine begins in the middle of the
first frame, so line 0 of that frame is not a line start and starts nothing; the first line 0 that
does is the next frame's.

## Edge cases

- **Offsets** wrap at 128 KiB for every halfword read or written (`1FFFFh`).
- **The 128-wide size** has a row stride of 128 pixels (32 KiB for 128 rows, which is also the offset
  step).
- **A capture enabled mid-frame** waits for the next line 0, and the bit reads 1 meanwhile.
- **Busy clears at line 192** for every size, including a size that stopped writing earlier.
- **The write block not in LCDC mode** (or disabled): nothing is written.
- **EVA or EVB above 16:** the factor is limited to 16; a blend that adds up past 31 saturates at 31.
- **EVA = 0** removes A from the blend (and its alpha from the result), likewise EVB.
- **Source A in display mode 2:** it is still the composed layers, not the VRAM picture on screen.
- **A write to bit 31 as 0 during a capture** stops it.

## Not implemented yet

- The rasterizer that fills the 3D line (#49): source A from the 3D screen and BG0 as the 3D layer
  read a transparent line until then (`TODO(phase7)`).
- VRAM wait states from the capture unit's writes. The timing is approximate (`scheduler.md`).

## Unverified

GBATEK does not say the following; each is a model, marked `TODO(hw-unverified)` at its code site.

- **EVA or EVB above 16, and the sum past 16:** factors limited to 16, each channel saturated at 31.
- **The alpha of a 2D graphics-screen pixel:** 1 for every pixel. The alpha of a FIFO pixel: 1.
- **The alpha of a 3D-screen pixel:** 1 when its 5-bit alpha is above 0, else a zero pixel.
- **BG0HOFS and the captured 3D screen:** the 3D screen that source A takes is the renderer's line
  before BG0's horizontal scroll, which only the composed BG0 applies (`bg-modes.md`, "Edge
  cases"). GBATEK says the scroll applies to the 3D layer as a BG0 and does not say whether the
  capture's 3D source sees it. — `crates/core/src/gpu2d/capture.rs` (`source_a`) — verify by a test
  ROM that scrolls BG0 and captures the 3D screen.
- **The row stride of the 128-wide size**, for both the write and the VRAM source-B read: 128 pixels,
  not 256.
- **A write block not in LCDC mode:** nothing is written. A read block not in LCDC mode: 0.
- **Clearing bit 31 during a capture:** it stops the capture. **Changing DISPCAPCNT mid-frame:** the
  fields are read at each line; only the enable is latched at line 0.
- **Read and write in the same bank on the same line:** the line's source B is read in full before
  its capture is written (the "trail" use GBATEK describes needs it).
- **Source A with display mode 0, or the screen forced blank:** still the composed layers.
- **The FIFO:** its depth (one line here), what an underrun shows (black), that the unused FIFO line is
  dropped at every visible line, the request rate inside a line (all at the line's start), that a
  request for line `y + 1` is raised at line `y`, that a halfword write pushes one pixel and a byte write
  nothing, that a read gives 0, and the behavior when display mode 3 is off but source B = FIFO (the
  FIFO is drained the same).
- **"Synchronize to start of display" (start mode 3):** once per frame, at line 0. GBATEK names the
  mode but not how often it fires.
- **The DISPCAPCNT reset value:** 0.
- **The unused DISPCAPCNT bits** (5-7, 13-15, 22-23, 28) read 0 and drop a write; GBATEK only says
  "not used".
- **The wait of a start-mode-4 channel:** one enabled mid-frame waits for the last line of that frame
  (GBATEK: "Transfer starts at next frame"), and enabling it again waits again and reloads its source.
- **A start-mode-4 channel without repeat:** it moves `32 * count` words in one transfer and then
  disables, raising its IRQ (if enabled) once per line, not once per request. GBATEK's generic repeat
  rule would suggest one request of `count` units.
