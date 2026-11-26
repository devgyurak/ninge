# Textures: formats, sampling, blending and the rear bitmap

Sources: GBATEK "DS 3D Texture Attributes" (TEXCOORD, TEXIMAGE_PARAM, PLTT_BASE), "DS 3D Texture
Formats", "DS 3D Texture Coordinates", "DS 3D Texture Blending", "DS 3D Display Control"
(DISP3DCNT), "DS 3D Rear-Plane" (the rear bitmaps, CLRIMAGE_OFFSET), "DS Memory Control: VRAM"
(the texture image and palette slots).
Used by: `crates/core/src/gpu3d/raster/`, `crates/core/src/bus/memory/vram.rs` (`Textures`)
Phase: 7 (#50). The coordinate transformation modes are the geometry engine's and already done
(`geometry-engine.md`, #48); this note is the rasterizer's side: what a texel is, how a pixel finds
one, and how it joins the vertex colour. Toon and highlight shading (polygon mode 2), edge marking,
fog and anti-aliasing are #51.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_0488 | TEXCOORD (command 22h) | 32 | W | - |
| 0x0400_04A8 | TEXIMAGE_PARAM (command 2Ah) | 32 | W | - |
| 0x0400_04AC | PLTT_BASE (command 2Bh) | 32 | W | - |
| 0x0400_0356 | CLRIMAGE_OFFSET | 16 | W | 0 |
| 0x0400_0060 | DISP3DCNT | 16 | R/W | 0 |

The three commands go through the command FIFO. TEXCOORD is per vertex: the S and T in force (after
the transformation mode) are stored with each vertex. TEXIMAGE_PARAM and PLTT_BASE are per polygon:
a polygon keeps the values in force when it completes (`geometry-engine.md`), and they may change
between polygons but not inside a strip.

### TEXIMAGE_PARAM bits

| Bits | Meaning |
|---|---|
| 0-15 | Offset of the texture data in the image space, in units of 8 bytes (0 to 512 KiB, across all four 128 KiB slots) |
| 16 | Repeat in S (0 clamp, 1 repeat) |
| 17 | Repeat in T |
| 18 | Flip in S: mirror every second repetition (needs repeat) |
| 19 | Flip in T (needs repeat) |
| 20-22 | S size: *8 << n* texels, 8 to 1024 |
| 23-25 | T size: *8 << n* texels |
| 26-28 | Format: 0 no texture, 1 A3I5, 2 4-colour, 3 16-colour, 4 256-colour, 5 4x4 compressed, 6 A5I3, 7 direct |
| 29 | Colour 0 of the 4-, 16- and 256-colour formats: 0 shown, 1 transparent |
| 30-31 | Coordinate transformation mode (`geometry-engine.md`) |

### PLTT_BASE bits

Bits 0-12 are the palette base: in units of **8 bytes** for the 4-colour format and **16 bytes**
for every other palette format. The direct format does not use it. The palette space is 96 KiB
(six 16 KiB slots) of 15-bit colours, bit 15 unused.

### DISP3DCNT and the rear plane

Bit 0 turns texturing on; bit 14 turns the rear bitmap on. CLRIMAGE_OFFSET holds the bitmap's X
scroll in bits 0-7 and its Y scroll in bits 8-15 (GBATEK's text says bits 8-14 for Y and in the
same line "0..255"; 8 bits are needed for that range, see "Unverified"). The scroll registers are
also reachable as the upper half of CLEAR_DEPTH, which is the same halfword (GBATEK: CLEAR_DEPTH
bits 16-31 are CLRIMAGE_OFFSET).

## Behavior

### A texel's address and colour

A texture is a row-major bitmap of *S size x T size* texels starting at the byte offset
*TEXIMAGE_PARAM offset x 8* in the image space. The image space is the four texture slots one
after another (slot *n* starts at *n x 128 KiB*); a texture may straddle a slot boundary because
the address is just an offset into that space.

For the texel at column *x* and row *t* (after the coordinate step below) the texel index is
*i = t x S size + x*. Per format:

| Format | Texel layout | Colour | Alpha (5 bit) |
|---|---|---|---|
| 1 A3I5 | 8 bits at byte *i*: bits 0-4 index, bits 5-7 alpha *a* | palette[index] | *a x 4 + a / 2* (so 0 stays 0 and 7 becomes 31) |
| 2 4-colour | 2 bits; the first texel of a byte is in its low bits (byte *i / 4*, shift *2 x (i mod 4)*) | palette[index] | 31, or 0 for index 0 when TEXIMAGE_PARAM bit 29 is set |
| 3 16-colour | 4 bits, first texel in the low nibble (byte *i / 2*, shift *4 x (i mod 2)*) | palette[index] | 31, or 0 for index 0 with bit 29 |
| 4 256-colour | 8 bits at byte *i* | palette[index] | 31, or 0 for index 0 with bit 29 |
| 5 4x4 compressed | see below | see below | 31, or 0 for a transparent texel |
| 6 A5I3 | 8 bits: bits 0-2 index, bits 3-7 alpha | palette[index] | the 5 bits as they are |
| 7 direct | 16 bits at halfword *i*: 15-bit colour, bit 15 alpha | the colour | 31 if bit 15 is set, else 0 |

`palette[k]` is the 15-bit colour at byte *PLTT_BASE x unit + 2 x k* of the palette space. Bit 29
affects only formats 2, 3 and 4; A3I5 and A5I3 take their transparency from the alpha field.

### 4x4 compressed texels

The texture is a grid of 4x4-texel blocks, laid out row by row of blocks, *S size / 4* blocks per
row. Block *b* is 32 bits at *offset + 4 x b* in slot 0 or slot 2: four bytes, the top row of
texels first, each byte holding a row with the leftmost texel in its low two bits.

Each block has 16 bits of palette information in **slot 1**: bits 0-13 are a palette offset in
units of 4 bytes, bits 14-15 a mode. The address is related to the block's: for texture data in
slot 0 at image offset *a*, the halfword is at *a / 2* inside slot 1 (image offset *128 KiB + a /
2*); for data in slot 2 at slot-relative offset *a*, at *a / 2 + 64 KiB* inside slot 1. The block's
colours come from the palette space starting at byte *PLTT_BASE x 16 + palette offset x 4*, and
the 2-bit texel value picks among them by mode:

| Texel | Mode 0 | Mode 1 | Mode 2 | Mode 3 |
|---|---|---|---|---|
| 0 | colour 0 | colour 0 | colour 0 | colour 0 |
| 1 | colour 1 | colour 1 | colour 1 | colour 1 |
| 2 | colour 2 | (colour 0 + colour 1) / 2 | colour 2 | (5 x colour 0 + 3 x colour 1) / 8 |
| 3 | transparent | transparent | colour 3 | (3 x colour 0 + 5 x colour 1) / 8 |

The mixes are per 5-bit channel. Modes 1 and 3 use two palette colours, so their palette data is
half as large. The largest 4x4 texture is 1024x512 or 512x1024, which fills slot 0 or 2 and half
of slot 1; 1024x1024 cannot exist because slots 0 and 2 are not adjacent.

### From S and T to a texel

TEXCOORD gives S and T as 1.11.4 fixed point, in texels (1.0 is one texel). The model
interpolates them across the polygon in perspective-correct fashion, with the same 1/W weight as
the colour (`rasterizer.md`, "Colour along a polygon"), keeps extra fractional bits during the
interpolation and takes the integer texel as the **floor** of the interpolated S or T (the value
shifted right by 4). Then, per axis with size *n*:

- **Clamp** (repeat bit clear): the texel is held to *0 .. n - 1*, so the edge texels repeat
  outwards. GBATEK: "the texture coordinates are clipped to MinMax(0, Size-1)".
- **Repeat** (repeat set, flip clear): the texel is taken modulo *n*, so a negative coordinate
  wraps from the far edge.
- **Repeat and flip**: the texel is taken modulo *2n*; the second half mirrors, so texel *k* of
  *n .. 2n - 1* is texel *2n - 1 - k*. GBATEK: "flip each 2nd texture"; flip needs repeat, so
  with repeat clear the flip bit has no effect.

### Blending the texel with the vertex colour

GBATEK writes the formulas for 6-bit channels, 5-bit values widened as *X = X x 2 + 1* for
*X > 0*. A texel colour and alpha are widened the same way (*Rt, Gt, Bt, At*), as are the vertex
colour and alpha (*Rv, Gv, Bv, Av*). For a format with no alpha GBATEK says to take *At* as solid
(31), and for a 1-bit alpha *At = A x 31*.

- **Modulation** (polygon mode 0): *R = ((Rt + 1) x (Rv + 1) - 1) / 64*, likewise G and B, and
  *A = ((At + 1) x (Av + 1) - 1) / 64*. The result is dimmer unless both factors are 63.
- **Decal** (polygon mode 1): *R = (Rt x At + Rv x (63 - At)) / 64*, likewise G and B, except that
  *At = 0* gives the vertex colour and *At = 31* (the opaque value) gives the texel colour; the
  alpha is *Av*. *At* is only the ratio of texture to vertex colour.
- **Toon and highlight** (mode 2) are #51.
- **No texture** (format 0), or DISP3DCNT bit 0 clear: the vertex colour and the polygon alpha,
  untouched.

GBATEK notes a wire frame always has *Av = 31*, and can still be made translucent by a texture
with *At < 31*.

### Texture alpha, translucency and the buffers

What GBATEK does not say, and the model decides (all of it under "Unverified"):

- The pixel's **final alpha** (the blend's *A*, narrowed to 5 bits) takes the place of the
  polygon alpha everywhere `rasterizer.md` uses an alpha: an alpha of 0 hides the pixel (it
  changes no buffer), the alpha test compares it with ALPHA_TEST_REF, 31 is an opaque pixel and
  1 to 30 a translucent one that blends with the buffer (DISP3DCNT bit 3) and writes depth only
  when POLYGON_ATTR bit 11 is set. A colour-0 pixel made transparent, or a texel with alpha 0,
  has alpha 0 in modulation and is therefore hidden; in decal mode the alpha is *Av*, so the
  pixel stays and shows the vertex colour.
- **Draw order** is a property of the polygon, not of the pixel. A polygon is drawn with the
  translucent ones when its POLYGON_ATTR alpha is 1 to 30, **or** its texture format is A3I5 or
  A5I3, the two formats whose texels can hold a partial alpha. A polygon with a direct or palette
  texture stays among the opaque ones: its texels are fully opaque or fully hidden, so no pixel of
  it blends. A shadow polygon stays translucent as before.
- The translucent-pixel rules of `rasterizer.md` (the same-ID skip, the blend, the depth write)
  apply to a pixel of final alpha 1 to 30, whatever polygon it came from; a pixel of alpha 31
  takes the opaque path and writes depth, ID and the translucent flag of "opaque".
- A pixel's alpha for the output narrows from the 6-bit result to 5 bits by dropping the low bit.

### The rear bitmap

With DISP3DCNT bit 14 set, the rear plane comes from two 256x256 bitmaps of 16-bit entries, each
a whole 128 KiB slot: the **colour** bitmap in texture slot 2 (image offset 0x40000) and the
**depth** bitmap in slot 3 (0x60000).

- Colour entry: bits 0-14 colour, bit 15 alpha (0 transparent, 1 solid; 0 and 31 as a 5-bit alpha).
- Depth entry: bits 0-14 depth, expanded to 24 bits as for CLEAR_DEPTH (`rasterizer.md`, "Depth");
  bit 15 is the fog flag the pixel starts with (stored for #51, which implements fog).
- The polygon ID of every rear pixel still comes from CLEAR_COLOR bits 24-29.
- Screen pixel (*x*, *line*) reads entry (*(x + X offset) mod 256*, *(line + Y offset) mod 256*),
  row-major, 2 bytes per entry. The visible part is 256x192; the bottom 64 rows are off screen
  unless the Y offset scrolls them in, and the bitmap wraps at the right and bottom edges.
- The viewport does not move or size the bitmap.

## Edge cases

- **An unmapped slot reads 0**, in the image space and the palette space (`vram-banks.md`). A
  texel read from one is index 0 or colour 0, so a palette format with bit 29 clear draws palette
  colour 0 (also 0 when the palette is unmapped: black) at alpha 31; with bit 29 set it is
  transparent; A3I5, A5I3 and direct texels are alpha 0 and so hidden in modulation. A bitmap rear
  plane with an unmapped slot 2 is colour 0 with alpha 0 (transparent); with an unmapped slot 3 it
  is depth 0 (the nearest) and no fog.
- An image or palette offset past the 512 KiB / 96 KiB space reads 0 (`Textures` does this).
- A texture that straddles a slot boundary reads across it. If the two slots are backed by
  different banks, each byte comes from the bank behind its own slot.
- A PLTT_BASE past the 96 KiB palette space (it has 13 bits, so up to 128 KiB) reads colour 0 for
  the entries beyond it (`Textures::palette_color`).
- A 4x4 texture whose data is in slot 1 or 3 has no defined palette halfword (the formula needs
  slot 0 or 2); the model reads the palette information as 0 (mode 0, offset 0).
- Texture size is *8 << n* with *n* up to 7, so 1024 texels is the largest. A 4x4 texture whose
  block data starts in slot 0 or 2 but runs on into slot 1 or 3 (an offset near the end of the
  slot) has a palette halfword only for the blocks in slot 0 or 2; the blocks past the boundary read
  it as 0 (mode 0, offset 0), as for any block in slot 1 or 3. Block data at or past 512 KiB reads 0.
- S and T are 16-bit values: a transformation that overflows has already wrapped
  (`geometry-engine.md`); the rasterizer sees the wrapped value.

## Unverified

Everything GBATEK does not state. Each item gets `TODO(hw-unverified)` at its code site.

- The precision and rounding of S and T interpolation, and whether it is perspective correct (the
  model: yes, as colour, floor to the texel). — `gpu3d/raster/` — verify by a test ROM that draws a
  texture with a known pattern at a steep angle and reads the pixels back by capture.
- The exact sample point inside a pixel and the texel at an exact edge (S exactly at the size, or a
  negative coordinate under each wrap mode); the model floors (*S >> 4*) and wraps as above. —
  `gpu3d/raster/` — verify by a test ROM at exact coordinates.
- Rounding of modulation and decal (GBATEK's integer division is truncating; the model truncates)
  and the mixed 5-/6-bit notation of the decal's *At = 0* and *At = 31* special cases (the model
  tests the 5-bit value). — `gpu3d/raster/` — verify by a test ROM with known colours.
- Narrowing of the 6-bit result to 5 bits for colour and alpha (the model drops the low bit). —
  `gpu3d/raster/` — verify as `rasterizer.md` does.
- What a decal pixel with a transparent texel does (the model: alpha *Av*, vertex colour). —
  `gpu3d/raster/` — verify by a test ROM.
- Whether the polygon is ordered with the translucent polygons when its texture is A3I5 or A5I3,
  and whether a direct or palette texture with transparent texels counts. — `gpu3d/raster/` —
  verify by a test ROM that draws such a polygon after an opaque one at a nearer depth.
- That the texel alpha reaches the alpha test, the same-ID rule and the depth write as the
  polygon's alpha does. — `gpu3d/raster/` — verify by a test ROM.
- The mixed interpolation of 4x4 modes 1 and 3 (the model: per 5-bit channel, truncating), and the
  palette halfword of a 4x4 texture in slot 1 or 3 (the model: 0). — `gpu3d/raster/` — verify by a
  test ROM.
- Whether the flip bit with repeat clear does anything (the model: nothing). — `gpu3d/raster/` —
  verify by a test ROM.
- CLRIMAGE_OFFSET's Y field width (bits 8-14 per one GBATEK line, 0..255 per the next; the model
  uses bits 8-15) and what X, Y offsets beyond the bitmap do (the model: modulo 256). —
  `gpu3d/raster/` — verify by a test ROM with a marked bitmap row.
- What the rear bitmap does with an unmapped slot (the model: reads 0). — `gpu3d/raster/` — verify
  by a test ROM with one slot unmapped.
- That a shadow polygon (mode 3) draws no texture (the model: its pixels are the vertex colour).
  — `gpu3d/raster/setup.rs` — verify by a test ROM with a textured shadow polygon.
- That a 4x4 block past a slot boundary has no palette halfword (the model: 0). —
  `gpu3d/raster/texture.rs` — verify by a test ROM with a texture at the end of slot 0.
- GBATEK's CLRIMAGE_OFFSET text swaps "row" and "column" (X is called the upper row, Y the left
  column); the model takes X as the horizontal and Y as the vertical scroll. — `gpu3d/raster/` —
  verify by a test ROM with a marked bitmap row and column.
- Whether a texture slot backed by two banks reads the earlier letter (`vram-banks.md`). —
  `bus/memory/vram.rs` — verify by a test ROM on hardware.
