# The 3D rasterizer: when it draws, its output buffer, and what it reads

Sources: GBATEK "DS 3D Overview" (Rendering Engine, Scanline Cache vs Framebuffer), "DS 3D I/O
Map", "DS 3D Display Control" (DISP3DCNT), "DS 3D Toon, Edge, Fog, Alpha-Blending,
Anti-Aliasing" (edge marking, fog, anti-aliasing, polygon size, RDLINES_COUNT), "DS 3D Shadow
Polygons" (the stencil buffer), "DS 3D Final 2D Output", "DS 3D Rear-Plane" (CLEAR_COLOR,
CLEAR_DEPTH, the rear bitmaps), "DS Display Capture" (the 15-bit capture data).
Used by: `crates/core/src/gpu3d/`, `crates/core/src/gpu2d/`, `crates/core/src/machine/render.rs`
Phase: 7 (#49-#52). This note settles the frame structure the code seams (#322) are built on; the
pixel rules (coverage, interpolation, texture, blending) are written with #49-#51.

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
