# The 3D rasterizer: when it draws, its output buffer, and what it reads

Sources: GBATEK "DS 3D Overview" (Rendering Engine, Scanline Cache vs Framebuffer), "DS 3D I/O
Map", "DS 3D Display Control" (DISP3DCNT), "DS 3D Toon, Edge, Fog, Alpha-Blending,
Anti-Aliasing" (edge marking, fog, anti-aliasing, polygon size, RDLINES_COUNT), "DS 3D Shadow
Polygons" (the stencil buffer), "DS 3D Final 2D Output", "DS 3D Rear-Plane" (CLEAR_COLOR,
CLEAR_DEPTH, the rear bitmaps), "DS Display Capture" (the 15-bit capture data).
Used by: `crates/core/src/gpu3d/`, `crates/core/src/gpu2d/`, `crates/core/src/machine/render.rs`
Phase: 7 (#49-#52). This note settles the frame structure the code seams (#322) are built on and
the pixel rules of #49 (coverage, colour, depth, translucency, shadows); texture, toon, edge, fog
and anti-aliasing are written with #50-#51.

## When the hardware draws

The rendering engine works scanline by scanline and keeps up with the display. GBATEK says it
starts 48 lines ahead of the first visible line, while the VBlank is still running (RDLINES_COUNT
puts the start at line 214), and writes into a small cache of up to 48 finished lines. The lines
go out to engine A as BG0 from that cache once the display period begins (after line 262). There
is no full 192-line framebuffer on the hardware: a line is drawn, waits in the cache, and is read
out. If the engine falls behind the display the cache drains; DISP3DCNT bit 12 reports an
underflow and RDLINES_COUNT the smallest fill of the previous frame (minus 2).

When the geometry side hands over no new data, the same image is drawn again, frame after frame.
The set a SWAP_BUFFERS handed on is drawn until the next swap.

### The model

The model draws a frame's 3D lines from one `Scene` (`geometry-engine.md`, "What the rasterizer
reads") and the rendering registers, plus the texture slots and the line number. It does not model
the 48-line cache or the underflow, which are timing (Phase 8): whatever is drawn is ready when
the 2D line that shows it is composed. Whether the lines are drawn one at a time as the 2D line
needs them or all 192 together at a fixed point of the frame cannot be seen by a program that
keeps the scene and the registers intact from the start of drawing to the end of the display
(GBATEK asks for exactly that; the registers "must be kept intact during rendering"). For such a
program the two are identical in output. A program that writes a rendering register while the
frame is on screen would see the difference (a per-line renderer picks the write up from the next
line, a batch renderer not until the next frame); the model takes the batch and leaves that case
under "Unverified". The choice is the project's (see the decision recorded in the PR that adds the
seams).

## The output: a 256x192 colour and alpha line buffer

For each line the rasterizer produces 256 pixels of colour with a 5-bit alpha, which the 2D
engine takes as BG0. GBATEK says the 5-bit colours of a vertex (the COLOR command) are expanded
internally to 6 bits with *X = X x 2 + 1* for X > 0, and gives the texture-blending formulas for
6-bit values; it also calls the 3D image an 18-bit one where it says that capture keeps only 15
bits ("Capture data is 15bit color depth (even when capturing 18bit 3D-images)"). So the engine
computes at 6 bits per channel, at least for vertex colours and blending. GBATEK does not say what
the rest of the pipeline does (the clear, edge and fog colours, the toon table) or how the colour
leaves the 3D unit towards the 2D engine; the model's rasterizer computes at 6 bits and narrows
each channel to the 15-bit colour of `Pixel3d` at its output, a choice of the model that the
15-bit capture makes plausible and nothing states (see "Unverified"). A pixel nothing covers shows
the rear-plane (CLEAR_COLOR and CLEAR_DEPTH, or the rear bitmaps in texture slots 2 and 3 when
DISP3DCNT bit 14 is set). The alpha is what the 2D engine's blending uses for the 3D layer
(blending with BG0 as 1st target via the 3D alpha values; `blending.md`). Beyond the colour and
alpha, the engine needs per pixel while it draws: depth, the polygon ID, the fog flag and the edge
flag (below), and the stencil bit that shadow polygons use (below); these are working state of a
frame, not the output.

## The stencil buffer

A shadow is drawn in two passes of the same volume (GBATEK, "DS 3D Shadow Polygons"). The first,
a shadow polygon with polygon ID 0, draws nothing to the colour buffer and sets flags in a stencil
buffer where the volume is in front of what was drawn. The second, with a non-zero ID, draws to
the colour buffer only where those flags are zero, and resets the flags it checks; it also draws
only where its ID differs from the ID already in the pixel. So drawing order matters, and the
rasterizer keeps one more per-pixel bit for the frame, with the same lifetime as depth and ID.
How this reads in the line-by-line model is #49's; the buffer exists in the model either way.

## Edge marking and fog read neighbours

GBATEK describes edge marking as applied once the opaque polygons have been drawn, and adds that
it reportedly waits until the translucent ones are drawn too. Either way it is a pass after the
polygons, not part of drawing one. A pixel flagged as an edge gets the edge colour when at least
one of its four neighbours (up, down, left, right) has a different polygon ID and the edge's depth
is less than that neighbour's depth. So a line's edge pixels depend on the lines above and below
it. Anti-aliasing makes the edges of opaque polygons translucent; GBATEK does not say it reads
neighbours, and the model treats it as needing the same second pass until #49 shows otherwise (see
"Unverified"). GBATEK describes fog as a lookup by the pixel's own depth, so the model takes it to need no
neighbour.

Consequence for the model: a rasterizer that "writes only its own line" is not enough when edge
marking is on. The model draws all polygons of the frame into depth, ID and edge-flag buffers,
then resolves edges and fog into the colour and alpha buffer, so the resolve comes after the
translucent polygons as well (the "reportedly" reading; see "Unverified"). For the line
parallelism #52 wants, the per-line work is independent only after the neighbours' ID and depth
lines exist: the pass is two phases, draw then resolve.

## Where the buffer lives and whether it is saved

Not settled by the hardware: the maintainer decides it, and the PR that adds the seams (#322, part
2) records the answer. The facts it rests on: the hardware keeps no whole frame, so nothing of the
3D output is visible to a program except through the 2D layer and capture; a frame is a pure
function of the scene, the registers and the texture memory; and a savestate can be taken at any
instruction boundary a host stops at (#322), in the middle of a frame, not only between
frames, so a buffer that is not saved must be rebuilt from those three inputs on load.

## From clip coordinates to the screen

GBATEK gives the mapping for a vertex with clip coordinates (xx, yy, zz, ww) and the VIEWPORT in
force (`geometry-engine.md`, "VIEWPORT"): *screen_x = (xx + ww) x width / (2 x ww) + x1* and
*screen_y = (yy + ww) x height / (2 x ww) + y1*. The origin of the viewport is the lower left, so
the 2D line of a screen y is *191 - y*. GBATEK does not say how *width* and *height* follow from
X1, X2, Y1, Y2, in what fixed-point precision the division runs, or how it rounds; it only notes a
vertex may land one pixel beyond X2 or Y1 "due to rounding". The model computes the division in
integers with a fractional part (a model choice, `TODO(hw-unverified)`), takes *width = X2 - X1 +
1* and *height = Y2 - Y1 + 1*, and clips the polygon's pixels to the viewport rectangle and to the
256x192 screen. The viewport limits where polygons draw; it does not move or size the rear plane.

A polygon has 3 to 10 vertices after clipping (`geometry-engine.md`). The model fills it as a
convex polygon, one scanline at a time, between its left and right edge on that line.

## Which pixels a polygon covers

GBATEK states these rules and no more:

- The hardware rounds the width and height of every polygon up to at least 1, so a polygon of
  0x0, 1x0, 0x1 and 1x1 dots all draw as one dot. Only the 0x0 case is subject to the 1-dot depth
  check (POLYGON_ATTR bit 13 and DISP_1DOT_DEPTH, a W comparison on the vertices; the dot takes
  the colour, depth and texture of the first vertex).
- A polygon with an alpha of 0 is a wire frame: only the lines along its edges draw, at a fixed
  alpha of 31, and the inside is transparent. A line segment (a triangle with two equal vertices)
  always draws; it has no front or back.
- Some polygons leave out their **lower and right edges**, to save work where polygons join:
  opaque polygons that are not wire frames and have neither edge marking nor anti-aliasing on,
  and every polygon with a vertical right edge (line segments excepted), and translucent polygons
  when DISP3DCNT bit 3 (blending) is off. Every other polygon draws all its edges. Turning edge
  marking on makes polygons full size even if nothing is marked.
- A pixel is drawn only if its alpha passes the test: greater than 0, or greater than
  ALPHA_TEST_REF when DISP3DCNT bit 2 is on. A pixel with alpha 0 changes no buffer at all.

GBATEK does not give the exact rule that decides whether a pixel centre lies inside an edge (the
top-left convention, or a different one), the slope arithmetic, or what makes an edge "right" and
"lower" on a slanted polygon. The model's rule, chosen so that the stated facts hold and marked
`TODO(hw-unverified)`: a pixel on scanline *y* is covered from the first whole x at or right of the
left edge's crossing to the last whole x left of the right edge's crossing, the right end
excluded for the polygons above that drop their right and lower edges, and included for the rest.
The rule is compared with a reference emulator binary as a black box before a golden frame fixes
it (see the PR that adds it); that comparison is a record of agreement, not a hardware fact.

### Facing

POLYGON_ATTR bit 6 (render the back surface) and bit 7 (render the front surface) pick which side
draws. The geometry engine stores the vertices in the order they arrived, anti-clockwise for a
front face as GBATEK defines it, with strips alternating (`geometry-engine.md`, "Primitives and
strips"). A polygon is front facing when its vertices run anti-clockwise as seen on screen with y
up. A front-facing polygon draws only if bit 7 is set, a back-facing one only if bit 6 is. A line segment ignores both bits.
A polygon of zero area has no side; GBATEK does not say what it counts as (a model choice, see
"Unverified").

## Colour along a polygon

A vertex colour is the 5-bit RGB of the COLOR command (or the lit colour), expanded to 6 bits with
*X x 2 + 1* for X > 0 and 0 for 0. GBATEK does not describe the interpolation across a polygon at
all. The model interpolates the 6-bit channels linearly across the polygon in perspective-correct
fashion, using 1/W as the weight (the model's choice; the exact precision and rounding are not stated), in integer fixed point with the
rounding stated in the code, and takes a texture-less pixel's colour from that. All of it is
`TODO(hw-unverified)` and settled by the comparison above. With no texture and no lighting the
vertex colour is the pixel colour. The decal (polygon mode 1) and toon or highlight (mode 2)
polygons draw the interpolated vertex colour in this model until texture and toon shading exist
(#50, #51).

## Depth

Each pixel has a 24-bit depth. SWAP_BUFFERS bit 1 chooses what it holds: 0 the Z value of the
clip coordinates, 1 the W value (GBATEK warns that W does not work with orthogonal projections).
CLEAR_DEPTH's 15 bits expand to 24 as *X x 200h + ((X + 1) / 8000h) x 1FFh*, so 7FFFh becomes
FFFFFFh.

A pixel passes the depth test when it is **less** than the stored depth (closer), or, with
POLYGON_ATTR bit 14 set, when it is **equal** within a tolerance of 200h either way (so it passes
when the two differ by at most 200h; GBATEK says "+/-200h" and warns that near polygons may
match by accident). A pixel that passes and is opaque writes its depth, its polygon ID and its fog
flag (POLYGON_ATTR bit 15). The depth of the polygon's interior at a pixel is the interpolation of
its vertices' depth; GBATEK does not state the precision or how Z and W map into the 24 bits
(`TODO(hw-unverified)`; the model interpolates Z linearly in screen space and W as its
reciprocal's inverse, in integers, mapped onto 0..FFFFFFh).

POLYGON_ATTR bit 12 decides what happens to a polygon that crosses the **far** plane: 1 draws the
clipped part, 0 hides the polygon. The geometry engine applies it (`geometry-engine.md`,
"Clipping"); the rasterizer sees only polygons that remain.

## Translucency, draw order and the rear plane

A polygon with alpha 31 is opaque, 1 to 30 translucent, 0 a wire frame. Draw order:

1. Opaque polygons, in the order they were completed.
2. Translucent polygons, last. SWAP_BUFFERS bit 0 picks the order: 0 sorts them automatically by
   Y (GBATEK says only "Y-sorting"; the model sorts by the smallest screen Y of a polygon's
   vertices, ties kept in completion order, and marks it `TODO(hw-unverified)`), 1 keeps the order
   of completion.

Blending a translucent pixel into the colour buffer (DISP3DCNT bit 3 on), per channel with 5-bit
alpha *A* of the polygon and 5-bit-scale colours, GBATEK gives:
*(poly x (A + 1) + buffer x (31 - A)) / 32*, and for alpha *max(A, buffer alpha)*. The blend is
skipped, and the polygon's colour and alpha overwrite the buffer, when blending is off, when the
pixel is opaque, and when the buffer's alpha is 0 (a rear plane with alpha 0). GBATEK states the
formula for 5-bit colours and the engine computes at 6 bits; the model applies the same
formula to the 6-bit channels and narrows at the output (see "Unverified").

A translucent pixel writes its depth only when POLYGON_ATTR bit 11 is 1 ("set new depth"); with 0
it keeps the old depth. A translucent polygon does not draw over a pixel that holds the same
polygon ID that an earlier translucent polygon wrote there (GBATEK states the "draw only if the ID differs" rule for shadow polygons and says the ID is "used
for translucent, shadow and edge marking"; it does not state it for ordinary translucent polygons,
so that extension is the model's). The model: a translucent
pixel is skipped when the buffer's stored polygon ID equals its own and the buffer pixel already
came from a translucent polygon, `TODO(hw-unverified)`.

### The rear plane

With DISP3DCNT bit 14 clear the buffers start each frame as: colour from CLEAR_COLOR bits 0-14,
alpha from bits 16-20, fog flag from bit 15 and polygon ID from bits 24-29; depth from CLEAR_DEPTH
(above). A pixel nothing draws on keeps the CLEAR_COLOR colour and alpha (alpha 0 lets the 2D
layers behind show). The bitmap rear plane (bit 14) is #50's.

## Shadow polygons

Polygon mode 3 is a shadow polygon (GBATEK "DS 3D Shadow Polygons"). They need two passes of the
same volume, with automatic sorting off so the order holds.

- **Mask pass:** mode 3, polygon ID 0, alpha 1 to 30, back surface rendered and front surface
  hidden (bit 6 set, bit 7 clear). Nothing is drawn to the colour buffer. Where the polygon
  passes its depth test (it is the back side, in front of what has been drawn), the pixel's
  stencil flag is set.
- **Render pass:** mode 3, polygon ID not 0, alpha 1 to 30, front surface rendered; GBATEK also
  sets the back surface to render and marks that "what/why?" (it is under "Unverified"). A pixel is drawn to the colour buffer, blended as a
  translucent pixel with the polygon's alpha, only if its stencil flag is 0 and its polygon ID
  differs from the ID in the pixel's buffer. The pass clears the stencil flag when it checks it.
  The pixel's fog flag becomes the old flag ANDed with the polygon's.

Shadow polygons are translucent by their alpha (1 to 30), so they draw after the opaque ones even
when the order is manual. The stencil is one bit per pixel and is reset to 0 at the start of each
frame (GBATEK does not say; a model choice, `TODO(hw-unverified)`). Whether a render-pass pixel
that fails the stencil or the ID test writes depth is not stated; the model says it does not. Whether
the mask pass writes depth is not stated either; the model says it does not (`TODO(hw-unverified)`
for both).

## Edge cases

- DISP3DCNT bit 0 (texture) and bit 14 (rear bitmap) change what is read; the slots must be mapped
  to texture VRAM (`vram-banks.md`); what an unmapped slot reads is #50's.
- The 3D layer is BG0 of engine A only, and it is shown only in the display mode that shows BG0
  with the 3D bit of DISPCNT set (`bg-modes.md`).

## Unverified

- Whether anti-aliasing reads neighbouring pixels, as edge marking does. — `gpu3d/` — verify by
  a test ROM.
- Whether the hardware's 48-line cache and underflow need modelling for any program-visible
  behaviour other than RDLINES_COUNT and DISP3DCNT bit 12. — `gpu3d/` — verify by a test ROM that
  loads the renderer.
- Whether edge marking really waits for the translucent polygons (GBATEK "reportedly"). The model
  resolves edges after all polygons until #49 has a test ROM. — `gpu3d/` — verify by a test ROM.
- Where the 3D colour narrows from 6 to 5 bits per channel on its way to the 2D engine, and how
  (dropping the low bit is the obvious reading). GBATEK states the 15-bit depth only for capture.
  The model narrows at the rasterizer's output; whether the 2D engine's blend sees 6 bits is not
  known. — `gpu2d/` — verify by a test ROM that blends 3D with a 2D layer and by one that
  captures it.
- Whether the 6-bit expansion (*X = X x 2 + 1*) applies to the colours the rasterizer takes from
  registers (clear, edge, fog, toon table) as it does to a vertex colour. — `gpu3d/` — verify by a
  test ROM.
- What an edge pixel at the screen border is compared with. GBATEK says edges "seem to be
  rendered in respect to the rear-plane's polygon_id entry"; the model does that. — `gpu3d/` —
  verify by a test ROM.
- What a write to a rendering register while the frame is displayed does to the lines not yet
  shown; the model draws the whole frame from the registers at one point. — `gpu3d/` — verify by a
  test ROM.
- The coverage rule at an edge (top-left or another), the slope and division precision, and how a
  slanted polygon's "lower and right" edges are told apart. The model's rule is stated under "Which
  pixels a polygon covers". — `gpu3d/` — verify by a test ROM drawing polygons on pixel grids.
- The interpolation precision and rounding of colour, depth and W, whether colour is perspective
  correct, and how Z and W map into 24 bits. — `gpu3d/` — verify by a test ROM that reads back
  captured pixels.
- The Y-sort key of translucent polygons and how ties order. — `gpu3d/` — verify by a test ROM.
- Whether a translucent pixel is skipped when the buffer holds the same polygon ID from a
  translucent polygon, and whether the rule applies when the earlier pixel was opaque. — `gpu3d/`
  — verify by a test ROM.
- Whether the shadow render pass draws the back surface too (GBATEK's "what/why?"), the facing of
  a polygon of zero area, and the viewport width and height (X2 - X1 + 1 or not). —
  `gpu3d/` — verify by a test ROM.
- Whether the shadow mask pass or a failed render pass writes depth, and the stencil buffer's
  lifetime (cleared each frame in the model). — `gpu3d/` — verify by a test ROM.
