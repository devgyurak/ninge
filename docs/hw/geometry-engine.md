# The geometry engine: command FIFO, matrices, vertices, clipping, lighting, tests

Sources: GBATEK "DS 3D Overview", "DS 3D I/O Map", "DS 3D Display Control", "DS 3D Geometry
Commands", "DS 3D Matrix Load/Multiply", "DS 3D Matrix Types", "DS 3D Matrix Stack", "DS 3D Matrix
Examples" (the fixed-point paragraphs), "DS 3D Polygon Attributes", "DS 3D Polygon Definitions by
Vertices", "DS 3D Polygon Light Parameters", "DS 3D Texture Coordinates", "DS 3D Status", "DS 3D
Tests", "DS DMA Transfers" (the start-timing table).
Used by: `crates/core/src/gpu3d/`, `crates/core/src/io/dma/`, `crates/core/src/debug/`
Phase: 7 (#48). The rasterizer, textures and the per-pixel effects are #49-#52 and are not here.

The 3D hardware has two halves. The **geometry engine** (this note) takes commands, runs the
matrix and vector arithmetic, builds polygons and stores them. The **rendering engine** (#49)
draws what the geometry engine stored. `gx-stub.md` describes the register-level front of the
geometry engine that Phase 5 built: the register map, the command decoder, the ports, the matrix
mode and the stack pointers. This note describes everything behind that front. Where the two
disagree about a *timing or a result*, this note wins; the decoder, the ports, the register widths
and the POWCNT1 gating stay as `gx-stub.md` has them.

How the work is split across PRs of #48 (each PR flips the sections it names from "model" to
"implemented"):

| PR | Sections |
|---|---|
| B | the FIFO and PIPE, command timing, GXSTAT, the FIFO IRQ, DMA mode 7, the full-FIFO stall, SWAP_BUFFERS waiting |
| C | matrices, stacks, CLIPMTX_RESULT, VECMTX_RESULT |
| D | VTX commands, polygon assembly, clipping, Polygon RAM and Vertex RAM, RAM_COUNT, the overflow flag |
| E | lighting, TEXCOORD and the texture matrix |
| F | POS_TEST, VEC_TEST, BOX_TEST |

## Registers

Only the registers whose behavior changes are listed; the others are in `gx-stub.md`.

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0400h`-`0400_043Fh` | GXFIFO | 32 | W | - |
| `0400_0440h`-`0400_05FFh` | the command ports | 32 | W | - |
| `0400_0600h` | GXSTAT | 32 | R, bits 30-31 R/W, bit 15 write-1-to-clear | `0600_0000h` |
| `0400_0604h` | RAM_COUNT | 32 | R | 0 |
| `0400_0620h`-`0400_062Fh` | POS_RESULT (x, y, z, w) | 4 words | R | 0 |
| `0400_0630h`-`0400_0635h` | VEC_RESULT (x, y, z) | 3 halfwords | R | 0 |
| `0400_0640h`-`0400_067Fh` | CLIPMTX_RESULT | 16 words | R | identity, see "Matrices" |
| `0400_0680h`-`0400_06A3h` | VECMTX_RESULT | 9 words | R | see "Matrices" |
| `0400_0060h` | DISP3DCNT | 16 | R/W | 0 |

### GXSTAT (`0400_0600h`)

| Bits | Meaning |
|---|---|
| 0 | A BOX_TEST, POS_TEST or VEC_TEST is executing |
| 1 | BOX_TEST result: 1 = the box is partly or wholly inside the view volume |
| 8-12 | Coordinate/vector stack level, low 5 bits of the 6-bit pointer |
| 13 | Projection stack level |
| 14 | Matrix stack busy: a PUSH, POP, STORE or RESTORE is executing |
| 15 | Matrix stack error flag. Write 1 to clear (and reset the projection pointer) |
| 16-23 | Low 8 bits of the number of FIFO entries |
| 24 | FIFO full (the entry count is 256) |
| 25 | FIFO less than half full (the entry count is below 128) |
| 26 | FIFO empty |
| 27 | Engine busy: a command is executing, or the PIPE or the FIFO holds an entry |
| 30-31 | FIFO IRQ mode: 0 never, 1 less than half full, 2 empty, 3 reserved |

Bits 2-7 and 28-29 read 0. Bits 8-15 and 30-31 are as in `gx-stub.md`. "The FIFO count" below
means the number of entries in the FIFO proper; the PIPE is not counted (GBATEK: data can still
be in the PIPE when the FIFO count is 0, which is why bit 27 exists).

### RAM_COUNT (`0400_0604h`)

| Bits | Meaning |
|---|---|
| 0-11 | Polygons now held in Polygon RAM (0-2048) |
| 16-28 | Vertices now held in Vertex RAM (0-6144) |

Bits 12-15 and 29-31 read 0. The counts describe the buffer the geometry engine is filling.
After a SWAP_BUFFERS the counters are cleared 10 cycles after the next VBlank begins (GBATEK). The
model clears them when SWAP_BUFFERS ends, 784 master cycles after VBlank begins (see "Unverified").

### DISP3DCNT bits 12 and 13

Bit 13 is the Polygon RAM / Vertex RAM overflow flag. The geometry engine sets it when it has to
drop a polygon or a vertex because a RAM is full. A write of 1 to the bit clears it. Bit 12 (the
colour-buffer underflow flag) belongs to the rasterizer (#49); until then it reads 0. Both are
status flags and are not plain read/write bits, as in `gx-stub.md`.

## Behavior

### Units of time

Every cycle count below is in 33.51 MHz units, the bus clock. The scheduler counts master cycles at
the ARM9 clock, which is twice the bus clock (`scheduler.md`), so a count of *n* here is *2n*
master cycles.

### The FIFO and the PIPE

The command stream is stored as **entries**. An entry is a command ID with one 32-bit parameter,
40 bits in all. A command with *N* parameters takes *N* entries (the ID rides with the first
parameter); a command with no parameters takes one entry. A packed word is first unpacked into
its commands, so what is queued is the same whatever way the program sent it.

There are two queues in front of the engine: the **FIFO**, 256 entries, and the **PIPE**, 4
entries. GBATEK gives the movement rules:

- A new entry goes straight into the PIPE when the FIFO is empty and the PIPE is not full. In every
  other case it goes into the FIFO.
- When the PIPE holds fewer than 3 entries, 2 entries move from the FIFO to the PIPE (fewer if the
  FIFO holds fewer).
- The engine takes its entries from the PIPE.

GXSTAT reports the FIFO count only. Bit 27 says that something is executing or queued anywhere.

### Executing a command

The engine runs one command at a time. The table gives the execution time of each command in bus
cycles (GBATEK's own table, restated). A time marked `*` is the base; see below it.

| Command | Cycles | Command | Cycles |
|---|---|---|---|
| MTX_MODE | 1 | COLOR | 1 |
| MTX_PUSH | 17 | NORMAL | 9 to 12 * |
| MTX_POP | 36 | TEXCOORD | 1 |
| MTX_STORE | 17 | VTX_16 | 9 |
| MTX_RESTORE | 36 | VTX_10, VTX_XY, VTX_XZ, VTX_YZ, VTX_DIFF | 8 |
| MTX_IDENTITY | 19 | POLYGON_ATTR | 1 |
| MTX_LOAD_4x4 | 34 | TEXIMAGE_PARAM | 1 |
| MTX_LOAD_4x3 | 30 | PLTT_BASE | 1 |
| MTX_MULT_4x4 | 35 * | DIF_AMB | 4 |
| MTX_MULT_4x3 | 31 * | SPE_EMI | 4 |
| MTX_MULT_3x3 | 28 * | LIGHT_VECTOR | 6 |
| MTX_SCALE | 22 | LIGHT_COLOR | 1 |
| MTX_TRANS | 22 * | SHININESS | 32 |
| BEGIN_VTXS | 1 | END_VTXS | 1 |
| SWAP_BUFFERS | see below | VIEWPORT | 1 |
| BOX_TEST | 103 | POS_TEST | 9 |
| VEC_TEST | 5 | NOP | 0 |

- MTX_MULT_4x4, MTX_MULT_4x3, MTX_MULT_3x3 and MTX_TRANS take 30 more cycles in matrix mode 2,
  because they run on the position matrix and then on the vector matrix. (GBATEK marks MULT_* and
  TRANS with `*` and states the +30 for mode 2 without saying which of the starred commands it
  covers; MTX_SCALE is not starred and does not touch the vector matrix, so it takes no extra.) See
  "Unverified".
- NORMAL takes 9 cycles plus one per enabled light, capped by GBATEK's own range 9 to 12; see
  "Unverified" for the count-versus-range puzzle.
- SWAP_BUFFERS is not executed until the next VBlank (line 192 begins) and the engine is held until
  then; it then takes 392 cycles. The wait is the engine's, not the CPU's.
- An invalid ID costs nothing and takes no parameters (`gx-stub.md`).

The engine begins a command when all of its entries are present in the PIPE and the FIFO. It
removes them at that moment (the FIFO count drops then) and the command **takes effect when its
time is up**, so a register read during the execution still sees the state from before it.

### The FIFO IRQ, the DMA start and the stall

- **IRQ.** The level condition is as in `gx-stub.md` ("GXSTAT and the FIFO IRQ"), with the real
  FIFO count: mode 1 is true while the count is below 128, mode 2 while it is 0. IF bit 21 is held
  while the condition is true.
- **DMA mode 7.** An ARM9 channel with start timing 7 starts whenever the FIFO becomes less than
  half full, and then moves 112 words, or fewer if its count runs out, to GXFIFO. A channel with
  repeat on starts again at the next time the condition becomes true; one with repeat off stays
  finished. The model moves the 112 words as one atomic transfer (`dma.md`), so a burst may push
  the count past 128 and up to 256 or beyond; what does not fit is covered by the next item.
- **A write to a full FIFO.** GBATEK says the CPU is frozen for as long as the FIFO is full, and
  that this includes the bus (no DMA, no interrupt, no ARM7 access in that time). The model: the
  ARM9 write that finds no space runs the engine forward until one entry is free, and the ARM9 is
  **held** for the engine cycles that took: the machine records "ARM9 held until *T*" (state, saved
  with the machine) and the run loop skips the ARM9's clock to *T*, or to the next event if that
  comes first, without running an instruction, so the line, timer, DMA, sample and IRQ events keep
  firing on time during the hold. No separate event holds the CPU. This is deterministic because
  the engine's clock is a function of the queue alone. The ARM7 is not frozen (see "Unverified").
  A DMA word that finds the FIFO full does the same and holds the ARM9 with it, as GBATEK's "DMA
  Overkill" paragraph says the DMA and the CPU are paused together.

### Matrices

All matrices are 4x4 with signed 32-bit elements in 1.19.12 fixed point (12 fraction bits); `1.0`
is `1000h`. There are four current matrices: projection, position (model-view), vector
(directional), texture. The clip matrix is derived.

| Mode | Selects | Notes |
|---|---|---|
| 0 | projection | the stack has one entry |
| 1 | position | stack commands also act on the vector matrix |
| 2 | position and vector together | every command acts on both, except MTX_SCALE (position only) |
| 3 | texture | one entry |

The loads put the given elements in place and fill the rest from the identity matrix, as the
shapes in GBATEK show:

- 4x4: 16 values in order, row by row.
- 4x3: 12 values fill the first three columns; the fourth column is `0, 0, 0, 1.0`.
- 3x3: 9 values fill the upper-left 3x3; the rest is the identity.
- Scale: the three values go on the diagonal, the fourth diagonal element is `1.0`.
- Translate: the three values go in the bottom row, the diagonal is `1.0`.

**Multiply order.** The multiply commands compute *C = M x C*, where *M* is the matrix built from
the parameters and *C* the current matrix; the product follows the usual rule *c_yx = a_y1 b_1x +
a_y2 b_2x + a_y3 b_3x + a_y4 b_4x* with *A = M* and *B = C*.

**Precision.** Each product is the sum of four full products with a 24-bit fraction; the sum is
shifted right by 12 once, at the end. The shift is arithmetic (it rounds toward negative infinity).
The sum must not be cut short in between. The model sums in 64 bits, wrapping, and a 32-bit result
that overflows wraps too (see "Unverified").

**The stack commands** are in `gx-stub.md` ("Matrix mode and the stack pointers"): the pointer
arithmetic and the error flag are unchanged. What the real model adds is the matrices themselves:
the stack holds a matrix in each valid entry, PUSH writes the current matrix at the pointer, POP
loads from the entry the pointer reaches, STORE and RESTORE address an entry by number. Entry 31
exists and can be written and read although it raises the error flag (GBATEK). The projection and
texture stacks each have one entry, 0; GBATEK says the upper half of every stack mirrors the lower,
so entry 1 is entry 0 and an access to it sets the flag and uses entry 0 (see "Unverified"). The vector stack moves with the coordinate stack and has the same size.

**The clip matrix** is *position x projection*, kept up to date whenever either of them changes
(GBATEK). CLIPMTX_RESULT reads it. VECMTX_RESULT reads the vector matrix; GBATEK says the fourth row
and column hold "any values" and only the upper 3x3 is defined. A read while the engine is busy
(GXSTAT bit 27) is not defined by GBATEK, which tells the program to wait.

**Reset.** All matrices start as the identity (see "Unverified"; GBATEK is silent).

### Vertices

The VTX commands set the current vertex position (16-bit signed, 1.3.12) and then process it:

- VTX_16: X and Y in the first parameter, Z in the second.
- VTX_10: three 10-bit fields, each with a 6-bit fraction. A field value `v` becomes `v << 6`
  (the stored position still has 12 fraction bits).
- VTX_XY, VTX_XZ, VTX_YZ: replace two of the three, keep the third from the previous vertex.
- VTX_DIFF: three signed 10-bit fields, each added to the previous coordinate. GBATEK gives the
  field a 9-bit fraction that is divided by 8 and sign-extended to the position's 12-bit fraction;
  that is the same number as the field itself read in 12-bit fraction units, so each coordinate
  gains `sign_extend_10(field)` and the range is about +-0.125. The sum is a 16-bit value.

Every VTX command ends by transforming the vertex: *(xx, yy, zz, ww) = (x, y, z, 1.0) x ClipMatrix*,
and storing it in Vertex RAM. Screen position is derived at render time by the rasterizer:
*screen_x = (xx+ww) x viewport_width / (2 ww) + viewport_x1*, the same for y. A VTX command that
completes a polygon also writes an entry in Polygon RAM.

VTX commands are only valid between BEGIN_VTXS and END_VTXS; outside them the model keeps the
position update but builds no polygon (see "Unverified"). END_VTXS has no effect on hardware; it
may be repeated inside a list.

POLYGON_ATTR is stored but takes effect only at the next BEGIN_VTXS. The Light bits take effect
only when NORMAL runs.

### Primitives and strips

BEGIN_VTXS bits 0-1 choose the primitive:

| Value | Primitive | Vertices |
|---|---|---|
| 0 | separate triangles | 3 per triangle |
| 1 | separate quads | 4 per quad |
| 2 | triangle strip | 3 for the first, then 1 per further triangle |
| 3 | quad strip | 4 for the first, then 2 per further quad |

Vertex order within a polygon (the order the rasterizer sees):

- Separate triangle: 0, 1, 2. Separate quad: 0, 1, 2, 3.
- Triangle strip: triangle *k* uses vertices *k*, *k+1*, *k+2*; odd triangles use clockwise order,
  so they are emitted as *k+1*, *k*, *k+2* to keep the same facing.
- Quad strip: quad *k* uses vertices 2k, 2k+1, 2k+3, 2k+2 (the "up-down" order).

A triangle with twice the same vertex is a line segment, always rendered. Crossed quads and quads
with an angle over 180 degrees give unspecified results (GBATEK) and the model draws what the
order says.

A list that is not a multiple of the primitive's size leaves the last partial polygon unfinished.
See "SWAP_BUFFERS" for what an unfinished list does at the swap.

### Clipping

A polygon is clipped against the six sides of the view volume. In clip coordinates a vertex is
inside when *-w <= x <= w*, *-w <= y <= w*, *-w <= z <= w* (the model's reading; GBATEK states only
"the 6 sides").

- A vertex that lies outside a side is replaced by two new vertices on the edges that join it to
  its neighbours. Each clipped vertex therefore costs one Vertex RAM entry less and two more.
- A polygon wholly outside is not stored in either RAM. GBATEK notes the exception that polygons
  exactly one pixel below or to the right of the lower and right edges are stored by accident (see
  "Unverified").
- A polygon from a strip that gets clipped becomes a separate polygon, which costs more Vertex RAM.
- POLYGON_ATTR bit 12 selects what happens to a polygon that crosses the **far** plane: 1 renders
  it clipped; 0 hides it entirely.
- POLYGON_ATTR bit 13 and DISP_1DOT_DEPTH decide what happens to polygons the size of one dot; that
  is a rendering decision (#49) and the geometry engine only stores the bit.

### Polygon RAM, Vertex RAM and the double buffer

There are two sets of Polygon RAM (2048 polygons) and Vertex RAM (6144 vertices). The geometry
engine fills one while the rendering engine reads the other. SWAP_BUFFERS exchanges them and empties
the one the geometry engine gets back. A polygon entry holds its attributes (the POLYGON_ATTR in
force at BEGIN_VTXS, the texture parameters and palette base at the time) and pointers to its
vertices; a vertex holds its clip coordinates, colour and texture coordinates.

When a RAM is full the polygon that needs room is dropped and DISP3DCNT bit 13 is set (the model;
GBATEK names only the flag; see "Unverified").

SWAP_BUFFERS also carries two parameter bits that apply to commands *after* it: bit 0 (translucent
polygons sorted manually) and bit 1 (depth by W instead of Z). The rendering registers (DISP3DCNT
and `0400_0330h`-`0400_03BFh`) are not swapped.

### What the rasterizer reads

The set the last SWAP_BUFFERS handed on (`Gpu3d::render_ram`): its polygons in the order they were
completed, each with its attributes (POLYGON_ATTR as of BEGIN_VTXS, TEXIMAGE_PARAM, PLTT_BASE) and
3 to 10 indices into the same set's vertices, in drawing order; each vertex has clip coordinates
(1.19.12), an RGB555 colour and S, T (1.11.4). Screen coordinates are derived from the VIEWPORT
value (`Gpu3d::viewport`) at render time, as the formula above says. Vertex RAM is filled in the
order the vertices arrived, a vertex shared by a strip once.

### SWAP_BUFFERS

The command is held until line 192 begins (VBlank), the engine counts as busy during the wait, and
the CPU is not halted. At the VBlank: the buffers swap, the counters in RAM_COUNT are cleared 10
cycles later, and the command's own 392 cycles run. A SWAP_BUFFERS with an unfinished polygon list
(for example a triangle with two vertices) locks the 3D hardware: only 2D video keeps working, GXSTAT
bit 27 stays set for ever, and nothing the program does recovers it (GBATEK). The model: the engine
stays busy and executes nothing more. A savestate carries the lock-up.

### Lighting

NORMAL calculates the vertex colour from the light and material registers; to the rest of the
hardware the colour is the same as one set by COLOR.

State: four lights (a direction vector and a colour each), the material (diffuse, ambient, specular,
emission colours), the shininess table (128 entries of 8 bits), the current normal.

- **LIGHT_VECTOR** (direction, bits 30-31 the light number). Components are 1.0.9 signed fixed
  point. The vector is multiplied by the vector matrix as it arrives, and the product is the stored
  light direction. A program that wants the light unrotated loads the identity into the vector
  matrix first (mode 2).
- **LIGHT_COLOR** (RGB555, bits 30-31 the light number).
- **DIF_AMB**: diffuse RGB555 in bits 0-14, ambient RGB555 in bits 16-30. Bit 15 set also sets the
  current vertex colour to the diffuse colour, exactly as COLOR would.
- **SPE_EMI**: specular RGB555 in bits 0-14, emission RGB555 in bits 16-30, bit 15 selects the
  shininess table (0 = a linear ramp is used instead of the table).
- **SHININESS**: 32 words, entries 0-3 in the first word, 124-127 in the last; 8 bits each, an
  unsigned 0.8 fraction.
- **NORMAL**: the normal (1.0.9 per component) goes through the vector matrix and the vertex colour
  is recalculated for each light enabled in POLYGON_ATTR bits 0-3, as below. The 5-bit colour fed to
  the pipeline is expanded to 6 bits first as `x*2 + (x+31)/32`, i.e. 0 stays 0 and every other
  value becomes `2x+1` (GBATEK's COLOR paragraph).

GBATEK gives the order of the work in "Internal Operation on Normal Command" and "... on
Light_Vector Command", which this note restates:

1. In texture transformation mode 2 the texture coordinates are calculated from the normal **as it
   arrives**, before the vector matrix turns it (see "Texture coordinates").
2. The normal is multiplied by the vector matrix.
3. The colour starts as the emission colour. For each enabled light *i*, per colour channel:
   - diffuse level = `max(0, -(L_i . N))`;
   - shininess level = `max(0, (-H_i) . N)` **squared**, and, when the table is enabled, the
     table entry it selects replaces the level;
   - colour += specular x light colour x shininess level;
   - colour += diffuse x light colour x diffuse level;
   - colour += ambient x light colour.
4. LIGHT_VECTOR stores `L_i` = the vector times the vector matrix and `H_i = (L_i + S) / 2`, where
   the line of sight `S` is always `(0, 0, -1.0)`: the hardware cannot normalize, so it leaves out
   the projection matrix, and specular reflection does not work when the projection matrix is
   rotated.

GBATEK does not give the fixed-point precision and rounding of these products and of the squaring,
how a squared level selects a table entry, or how the clamp to 5 bits is made; those are in
"Unverified". Lighting is calculated for the front face only: the back side takes the same colour.

### Texture coordinates

TEXCOORD gives a 16-bit signed (S, T) pair, each 1.11.4. TEXIMAGE_PARAM bits 30-31 select the
transformation mode:

| Mode | When it runs | Result |
|---|---|---|
| 0 | at TEXCOORD | S', T' = S, T |
| 1 | at TEXCOORD | (S, T, 1/16, 1/16) times the first two columns of the texture matrix |
| 2 | at NORMAL | the normal as it arrives (before the vector matrix), (Nx, Ny, Nz, 1.0), times the first two columns, the bottom row replaced by (S, T) |
| 3 | at each VTX | (Vx, Vy, Vz, 1.0) times the first two columns, the bottom row replaced by (S, T) |

Operand formats: matrix 1.19.12, vertex 1.3.12, normal 1.0.9, the constant 1.0 a single integer
bit, 1/16 four fraction bits, S/T 1.11.4. The product is shifted right to leave 4 fraction bits and
**masked to 16 bits** (GBATEK: "clipped to that size"). The transformed pair is stored in the vertex.

(TEXIMAGE_PARAM and PLTT_BASE are stored with the polygon; what they mean is the texture unit's,
#50.)

### POS_TEST, VEC_TEST and BOX_TEST

- **POS_TEST** takes (x, y, z) as 1.3.12 and multiplies (x, y, z, 1.0) by the clip matrix. The
  four results (x, y, z, w) are 1.19.12 and are read from POS_RESULT after bit 0 of GXSTAT clears.
  It also **overwrites the current vertex position** (the next VTX_XY and its kin are relative to
  the tested position). It takes 9 cycles.
- **VEC_TEST** takes (x, y, z) as 1.0.9 and multiplies (x, y, z, 0) by the vector matrix. The
  three results are read from VEC_RESULT, 16 bits each: a 4-bit sign (0000 or 1111, the
  sign extension) over 12 fraction bits, so a value of 1.0 or more overflows (1.0 reads as -1.0).
  It takes 5 cycles.
- **BOX_TEST** takes a corner (x, y, z) and three offsets, 1.3.12 each, so the sums must not exceed
  the 16-bit range. It tests the six faces of the box against the view volume and sets GXSTAT bit
  1 when any face is wholly or partly inside; it is 0 when all are outside. A box that surrounds
  the whole view volume gives 0 (only the faces are tested). It takes 103 cycles. GBATEK asks for
  the far-plane and one-dot polygon bits in POLYGON_ATTR to be set first and a dummy BEGIN/END to
  apply them; the model ignores that precondition (see "Unverified").

While one of the three runs, GXSTAT bit 0 reads 1 (and bit 27). None is allowed between BEGIN_VTXS
and END_VTXS, except POS_TEST (GBATEK: "anywhere, except within strips, huh?").

## Edge cases

- A command whose entries never all arrive (a program stops writing mid-command) keeps the engine
  waiting; the entries it has sit in the PIPE and FIFO, so GXSTAT bit 27 reads 1 (it is set while
  either holds data) and the engine does nothing.
- NOP and an invalid ID queue no entry (see "Unverified"), so a packed word of NOPs leaves the FIFO
  as it was.
- An odd value is never an error to the host: stack pointers wrap by their width, a matrix product
  wraps in 32 bits, a full RAM drops the polygon, a bad ID is ignored.
- A savestate taken mid-command keeps the queue, the PIPE, the command in execution and the time it
  ends; nothing is recomputed on load (`savestate/mod.rs`).
- Reading CLIPMTX_RESULT or VECMTX_RESULT while a command that changes the matrix is executing
  returns the matrix from before that command.
- A write to the FIFO that holds the ARM9 may hold it for a whole frame when the engine is waiting
  for VBlank (its events still fire); a program that fills the FIFO behind a SWAP_BUFFERS therefore waits.
- The FIFO count reaches 256 and bit 24 is set; the low 8 bits (bits 16-23) then read 0.

## Model (not hardware)

These are decisions that GBATEK does not state and that the emulator needs. Each is written in the
PR that implements it. None claims to be how the silicon works.

- **Command start.** A command starts when all its entries are queued, takes them at once and has
  its effect at the end. (PR B)
- **PIPE refill.** The 2-entry refill happens at the moment the engine takes entries, with no
  delay. (PR B)
- **The stall.** The ARM9 is held for the engine time; see "The FIFO IRQ, the DMA start and the
  stall". (PR B)
- **Lighting rounding**: the precision of the products, the squaring and the table index. (PR E; see
  "Unverified")
- **Clipping interpolation** and the guard band. (PR D; see "Unverified")
- **Polygon RAM layout.** A polygon is a fixed-size record: attributes, texture parameters, the
  vertex count and the first vertex index, with the vertices in Vertex RAM in order. The renderer
  reads the buffer the last swap made current. (PR D)

## Unverified

Each item carries `TODO(hw-unverified)` at the site that implements it, in the PR that adds the
code. "Verify by" is a test ROM on hardware or a comparison with the reference emulator used as an
oracle.

- **Command start and the entry timing.** GBATEK does not say when a command's entries leave the
  FIFO, nor whether a long command starts before its last parameter arrives. The model takes all
  entries at the start. — `gpu3d/` (the engine) — verify by reading GXSTAT bits 16-26 between two
  writes of one MTX_LOAD_4x4.
- **A command abandoned midway.** When another port is written while a command waits for
  parameters, the model drops the entries the abandoned command already queued (the engine takes a
  command only when all its entries are there, so they would otherwise block the queue for ever).
  GBATEK says nothing of an abandoned command. — `gpu3d/` (`Gpu3d::abandon`) — verify by writing
  part of a MTX_LOAD_4x4 to its port and then another port's command, and reading GXSTAT's busy bit
  and FIFO count.
- **A write that can never complete.** A writer stuck on a FIFO the engine can never drain would
  hang the hardware; the model drops the entry instead. Not reachable until SWAP_BUFFERS locks the
  hardware up (`geometry-engine.md`, "SWAP_BUFFERS"). — `gpu3d/` (`Gpu3d::enqueue`).
- **The ARM9 hold.** A write to a full FIFO holds the ARM9 until the engine took an entry; the
  model skips its clock to that time (events still fire in between) and the engine state is
  advanced inline at the write, so for the span of the hold a register read by another path
  (a DMA at `now`) sees the engine already at the end of it. A DMA burst that finds the FIFO full
  holds the ARM9 the same way (GBATEK, "GXFIFO DMA Overkill": "the DMA (and CPU) paused"). —
  `machine/events.rs` (`run_arm9_until`, `run_dma`).
- **The swap's wait.** SWAP_BUFFERS' own time is "until VBlank plus 392 cycles"; see the item on
  its timing. — `gpu3d/` (`Gpu3d::start`).
- **NOP and invalid IDs queue no entry.** GBATEK says a no-parameter command takes one entry and
  says nothing of NOP or of an invalid ID. — `gpu3d/` (`Gpu3d::run_pending`) — verify by counting
  GXSTAT's FIFO count after a word of NOPs behind a busy engine.
- **PIPE refill timing.** The 2-entry move is modelled as instant. — `gpu3d/`.
- **FIFO entry for a command with parameters**: whether the command ID takes space of its own or
  rides with the first parameter, as the "40-bit entry" suggests. The model uses *N* entries. —
  `gpu3d/`.
- **Whether the engine runs during the ARM9 stall in the real order** and whether the ARM7 and DMA
  stop (GBATEK says the bus is frozen, including for the ARM7). The model charges the ARM9 only. —
  `gpu3d/`, `machine/`.
- **DMA mode 7 with repeat on.** Restart at each "less than half" level is the natural reading and
  answers the stub's Unverified item on a repeating channel. — `io/dma/mod.rs`.
- **The pause between bursts.** A burst is one atomic transfer and the next one starts after the
  words of the last have gone by at one bus cycle each, so a channel that never fills the FIFO does
  not restart at the very same time for ever; the time the next burst is due is saved with the
  channel. A word that finds the FIFO full holds the ARM9 (see the ARM9 hold). —
  `io/mod.rs` (`Io::dma_finished`), `io/dma/mod.rs`.
- **Cycle counts starred in GBATEK** (MULT and TRANS) and the +30 of mode 2: which commands it
  covers. NORMAL's 9..12 range for 0..4 lights, which is five settings for four timings. The model
  takes 9 plus the light count, capped at 12. — `gpu3d/`.
- **SWAP_BUFFERS timing**: the wait is "until VBlank (scanline 192)" plus 392 cycles; whether the
  392 starts at the line's start, and how a swap issued during VBlank behaves. The model waits for
  the next line-192 start, which is the next frame's when the command begins inside VBlank (lines
  192-262); GBATEK says only "until next VBlank". — `gpu3d/`.
- **VIEWPORT, TEXIMAGE_PARAM and PLTT_BASE** are stored and copied into each polygon; what they do
  is the rasterizer's and the texture unit's (#49, #50). POLYGON_ATTR bit 13 (1-dot polygons) and
  DISP_1DOT_DEPTH are not applied by the geometry engine. — `gpu3d/polygon.rs`.
- **Lock-up on an unfinished list**: exactly which lists count as unfinished (fewer vertices than
  the primitive needs for its *first* polygon, or any partial polygon at the end). The model locks
  up on any partial polygon. — `gpu3d/`.
- **Matrix overflow.** The width of the accumulator and what an element does that does not fit 32
  bits after the shift. The model sums four products in a wrapping 64-bit accumulator (so a guest's
  extreme values never panic the core) and wraps the shifted result to 32 bits. — `gpu3d/matrix.rs`
  (`multiply`).
- **The coordinate stack's storage.** The 6-bit pointer's upper half mirrors the lower, so the model
  keeps 32 entries and an access to entry *e* uses slot *e* mod 32, error flag or not; the
  directional stack moves with it. — `gpu3d/matrix.rs` (`Matrices::store`).
- **VECMTX_RESULT layout.** The nine words are the vector matrix's upper 3x3 row by row, taken from
  GBATEK's "3x3 matrix format"; the fourth row and column are not read. — `gpu3d/matrix.rs`
  (`Matrices::vector_word`).
- **Matrix reset state.** The model starts all matrices as the identity. — `gpu3d/`.
- **Stack entries beyond the valid ones** and the texture stack. Entry 31 exists; entry 1 of the
  projection and texture stacks mirrors entry 0 (GBATEK's "upper half mirrors the lower"), and the
  texture stack follows the projection stack's rules, which GBATEK does not give ("one..none?"). —
  `gpu3d/matrix.rs` (`Matrices::store`).
- **VTX_DIFF** field format. The reading above (the field added as a sign-extended 12-bit-fraction
  count) follows from GBATEK's "divided by 8" and its stated range of +-0.125; the text does not
  say it in one line. What happens when the sum leaves the 16-bit range is not stated; the model
  wraps. — `gpu3d/`.
- **VTX commands outside BEGIN/END.** The model updates the position and builds nothing. —
  `gpu3d/`.
- **Clipping.** The inside test uses *-w <= x,y,z <= w*; GBATEK names the six sides only. The
  interpolation of colour and texture coordinates for a new vertex, the rounding of the new
  position, and the handling of *w <= 0* are not stated. The model interpolates every attribute
  linearly in clip space along the edge, `a + (b - a) x da / (da - db)` in 128-bit integers, and
  the division truncates toward zero. The "one pixel below
  or right of the edge" exception is not modelled. — `gpu3d/`.
- **Strip clipping**: a clipped polygon in a strip becomes separate polygons and the strip
  continues from the unclipped vertices. The model stores the clipped polygon's corners afresh and
  stores the strip's own vertices again when a later polygon uses them. — `gpu3d/polygon.rs`.
- **The order of a clipped polygon's corners** (where the list starts after the six planes). The
  model clips plane by plane in the order left, right, bottom, top, near, far, each pass starting
  at the second corner of the first edge, so the first corner moves on by one for every plane.
  GBATEK says only that vertices outside a side are replaced by two new ones and nothing about the
  order, so the whole order is the model's choice. — `gpu3d/polygon.rs` (`clip`).
- **RAM_COUNT and the swap's instant.** GBATEK clears the counters 10 cycles after VBlank; the model
  swaps the sets and clears them when SWAP_BUFFERS' own execution ends, 784 master cycles after
  VBlank. Where a swap and a VBlank clear coincide is not modelled. — `gpu3d/polygon.rs` (`swap`).
- **RAM overflow behavior**: what is dropped (the polygon, the vertex, or both), and when bit 13 is
  set. The model drops the whole polygon that does not fit and sets the flag. — `gpu3d/`.
- **The precision of the lighting products.** GBATEK gives the equation (see "Lighting") but not
  the widths: the model keeps the normal and the light as 1.0.9, takes a dot product as an
  18-bit-fraction sum shifted right by 9 and clamped to `0..=1.0`, squares the shininess level
  (`level * level >> 9`), indexes the 128-entry table by the squared level divided by 4, and forms
  each colour term as an integer product divided by `31 x 512`, `31 x 256` or `31` with every
  division truncating. The rounding of each step and the table index are open. — `gpu3d/`
  (the lighting function) — verify by an oracle trace of vertex colours for one light and a table
  of normals.
- **Shininess table off**: "linear increasing numbers" is taken as entry *i* = *i*·2 for *i* in
  0..127. — `gpu3d/lighting.rs` (`Lighting::shininess`).
- **The model's terms in detail.** Diffuse = `max(0, -(N . L)) >> 9` (of 512); specular dot =
  `max(0, -(N . H)) >> 9` with `H = (L + (0, 0, -1.0)) / 2`, shininess level = dot squared (`dot x dot
  >> 9`), table index = level / 4 (the linear ramp, with the table off, is twice that index); per channel
  `diffuse_reflection x light x diffuse / (31 x 512) + specular_reflection x light x shininess /
  (31 x 256) + ambient_reflection x light / 31`, summed over the enabled lights with the emission and
  clamped to 31; every division truncates. LIGHT_VECTOR and NORMAL are multiplied by the vector
  matrix at the time they run (a light is not turned again when the matrix later changes). Lights
  are taken from POLYGON_ATTR as latched at BEGIN_VTXS. — `gpu3d/lighting.rs` (`vertex_color`).
- **NORMAL's cycles** are 9 bus cycles plus one for each enabled light up to 12, a reading of
  GBATEK's "9..12 for 0..4 lights" that has four settings for five light counts. —
  `gpu3d/cycles.rs`.
- **Texture matrix modes.** Mode 1 is GBATEK's formula with the two `1/16` constants as a raw 1
  and one shift right by 12 (S, T have 4 fraction bits, the matrix 12). Modes 2 and 3 replace the
  matrix's bottom row by S and T, whose units GBATEK does not give; the model shifts the vector
  terms right by 17 (a normal, 9 fraction bits) or 20 (a position, 12) to leave 4 fraction bits and
  adds S and T as they are, masked to 16 bits. Mode 2 runs at NORMAL (lit or not), mode 3 at each
  vertex; the mode is read from TEXIMAGE_PARAM when the command runs. Mode 2 takes the normal as
  it arrives, before the vector matrix (GBATEK's "Internal Operation on Normal Command"). —
  `gpu3d/polygon.rs` (`map_texcoord`, `texture_from`).
- **POS_TEST and the current vertex**: that it replaces the position without touching the
  colour. — `gpu3d/`.
- **BOX_TEST** (the interpretation of the three offsets as the box's size, "presumably" in GBATEK;
  the face test; the result when the box surrounds the view volume; the precondition on POLYGON_ATTR
  bits 12-13). — `gpu3d/`.
- **VEC_TEST result width**: a 12-bit fraction with a 4-bit sign copy. — `gpu3d/`.
- **GXSTAT bit 0 and the tests**: the busy flag covers the whole test's time. — `gpu3d/`.
- **The matrix stack busy bit** (GXSTAT bit 14) is modelled as set during PUSH, POP, STORE and
  RESTORE and clear otherwise. — `gpu3d/`.
