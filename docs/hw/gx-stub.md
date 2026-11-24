# The 3D engine stub: geometry command ports, GXSTAT, matrix stack pointers, DMA start mode 7

Sources: GBATEK "DS 3D I/O Map", "DS 3D Display Control", "DS 3D Geometry Commands", "DS 3D Matrix
Load/Multiply" (MTX_MODE), "DS 3D Matrix Stack", "DS 3D Status", "DS 3D Tests", "DS DMA Transfers"
(the start-timing table), "DS Interrupts" (IF bit 21), "DS Power Control" (POWCNT1).
Used by: `crates/core/src/gpu3d/`, `crates/core/src/io/map.rs`, `crates/core/src/io/dma/`
Phase: 5 (#43). Everything past the stub is Phase 7 (#48 geometry engine, #49-#52 rendering, #53
debugger panels). The geometry engine itself is described in `geometry-engine.md`; this note keeps
the register front (the decoder, the ports, GXSTAT's layout, the stack pointers) and the stub model
that the PRs of #48 replace one by one.

The ARM9 programs of a commercial game start the 3D hardware long before they draw anything: they
write display lists to the command FIFO (by DMA, by `STM`, or by the command ports), poll GXSTAT
until the FIFO has drained, and read the matrix stack levels back. If those registers do not exist,
GXSTAT reads 0, "FIFO empty" is never seen and the program spins for ever. This note describes the
registers a program can see and the smallest model that lets such a program run: commands are
accepted and dropped, nothing is drawn.

## Scope

This note is the register front of the geometry engine: the registers, the command decoder (GXFIFO,
packed and unpacked words, the command ports), the matrix mode and the stack pointers with their
error flag, GXSTAT's layout and POWCNT1 bits 2 and 3 gating the ports. What the engine does with the
commands (the FIFO and the PIPE, timing, matrices, polygons, lighting, the tests, DMA start mode 7)
is in `geometry-engine.md`, which wins where the two differ about a timing or a result.

The rendering registers' effect is #49-#52 (`TODO(phase7)`): they drop their writes and read 0
here.

## Registers

All of these are ARM9 only. The ARM7 does not see them: the ARM7 has the sound registers at
`0400_0400h` (`sound-registers.md`).

| Address | Name | Width | Access | Behavior |
|---|---|---|---|---|
| `0400_0060h` | DISP3DCNT | 16 | R/W | stored, bits 12-13 acknowledge |
| `0400_0320h` | RDLINES_COUNT | 8 | R | a fixed value, see "Unverified" |
| `0400_0330h`-`0400_03BFh` | the rendering registers (EDGE_COLOR, ALPHA_TEST_REF, CLEAR_COLOR, CLEAR_DEPTH, CLRIMAGE_OFFSET, FOG_COLOR, FOG_OFFSET, FOG_TABLE, TOON_TABLE) | 8-32 | W | write dropped, reads 0 |
| `0400_0400h`-`0400_043Fh` | GXFIFO (16 mirrors of one word) | 32 | W | the command decoder |
| `0400_0440h`-`0400_05FFh` | the command ports | 32 | W | the command decoder |
| `0400_0600h` | GXSTAT | 32 | R, bits 30-31 R/W, bit 15 write-1-to-clear | see below |
| `0400_0604h` | RAM_COUNT | 32 | R | the Polygon and Vertex RAM counts (`geometry-engine.md`) |
| `0400_0610h` | DISP_1DOT_DEPTH | 16 | W | write dropped |
| `0400_0620h`-`0400_062Fh` | POS_RESULT | 4 words | R | the last POS_TEST (`geometry-engine.md`) |
| `0400_0630h`-`0400_0635h` | VEC_RESULT | 3 halfwords | R | the last VEC_TEST (`geometry-engine.md`) |
| `0400_0640h`-`0400_067Fh` | CLIPMTX_RESULT | 16 words | R | the clip matrix (`geometry-engine.md`) |
| `0400_0680h`-`0400_06A3h` | VECMTX_RESULT | 9 words | R | the vector matrix (`geometry-engine.md`) |

GBATEK heads GXFIFO as "mirrored up to 400043Fh?" with a question mark, and the text under it relies
on the mirror for `STRD` and `STM` (see "Unverified"). The geometry block therefore ends at
`0400_06A3h`; every other address between `0400_0320h` and `0400_06A3h` that no row names (for
example `0400_0324h`-`0400_032Fh`, `0400_03C0h`-`0400_03FFh`, `0400_0608h`-`0400_060Fh`) reads 0 and
drops writes.

### DISP3DCNT bits (`0400_0060h`)

| Bits | Meaning |
|---|---|
| 0 | Texture mapping |
| 1 | Shading: 0 = toon, 1 = highlight |
| 2 | Alpha test |
| 3 | Alpha blending |
| 4 | Anti-aliasing |
| 5 | Edge marking |
| 6 | Fog: 0 = colour and alpha, 1 = alpha only |
| 7 | Fog master enable |
| 8-11 | Fog depth shift |
| 12 | Colour buffer RDLINES underflow: 1 = underflow, write 1 to acknowledge |
| 13 | Polygon/vertex RAM overflow: 1 = overflow, write 1 to acknowledge |
| 14 | Rear-plane mode: 0 = blank, 1 = bitmap |
| 15-31 | Not used, read 0 |

Bits 0-11 and 14 are plain read/write (mask `4FFFh`). Bits 12 and 13 are status flags, not stored
bits: bit 13 is raised by the geometry engine when a Polygon or Vertex RAM is full and a write of 1
clears it (`geometry-engine.md`, "DISP3DCNT bits 12 and 13"); bit 12 is the rendering engine's
(#49) and reads 0 until it exists. A byte write changes only its byte.

### GXSTAT bits (`0400_0600h`)

The bits are listed in `geometry-engine.md`, "GXSTAT". This note owns the writable ones: bits
30-31, the FIFO IRQ mode (0 = never, 1 = less than half full, 2 = empty, 3 = reserved), and bit 15,
the matrix stack error flag (write 1 to clear). Bits 8-13 are the stack levels, the low 5 bits of the
6-bit coordinate pointer and the 1-bit projection pointer.

Only bits 30-31 are writable, apart from bit 15: a write of 1 to bit 15 clears the error flag and
also resets the projection stack pointer to 0; GBATEK says it "probably" also resets the texture
stack pointer (see "Unverified"). A write of 0 to bit 15 changes nothing. Every other bit ignores
writes. The register is 32 bits wide; a halfword or byte access sees the matching part and a write
to a part acts on only that part.

Reset value: `0600_0000h` (bits 25 and 26: less than half full, empty).

### Reads of the write-only registers

GXFIFO, the command ports, the rendering registers and DISP_1DOT_DEPTH are write-only; a read of
one returns 0 here (see "Unverified").

## Behavior

### The command set

A command is an 8-bit ID. The table lists every valid ID, its command port (`0400_0400h` + 4 * ID,
which is how the ports are spaced) and how many 32-bit parameters follow it.

| ID | Name | Parameters |
|---|---|---|
| 00h | NOP (padding in a packed word) | 0 (no port) |
| 10h | MTX_MODE | 1 |
| 11h | MTX_PUSH | 0 |
| 12h | MTX_POP | 1 |
| 13h | MTX_STORE | 1 |
| 14h | MTX_RESTORE | 1 |
| 15h | MTX_IDENTITY | 0 |
| 16h | MTX_LOAD_4x4 | 16 |
| 17h | MTX_LOAD_4x3 | 12 |
| 18h | MTX_MULT_4x4 | 16 |
| 19h | MTX_MULT_4x3 | 12 |
| 1Ah | MTX_MULT_3x3 | 9 |
| 1Bh | MTX_SCALE | 3 |
| 1Ch | MTX_TRANS | 3 |
| 20h | COLOR | 1 |
| 21h | NORMAL | 1 |
| 22h | TEXCOORD | 1 |
| 23h | VTX_16 | 2 |
| 24h | VTX_10 | 1 |
| 25h | VTX_XY | 1 |
| 26h | VTX_XZ | 1 |
| 27h | VTX_YZ | 1 |
| 28h | VTX_DIFF | 1 |
| 29h | POLYGON_ATTR | 1 |
| 2Ah | TEXIMAGE_PARAM | 1 |
| 2Bh | PLTT_BASE | 1 |
| 30h | DIF_AMB | 1 |
| 31h | SPE_EMI | 1 |
| 32h | LIGHT_VECTOR | 1 |
| 33h | LIGHT_COLOR | 1 |
| 34h | SHININESS | 32 |
| 40h | BEGIN_VTXS | 1 |
| 41h | END_VTXS | 0 |
| 50h | SWAP_BUFFERS | 1 |
| 60h | VIEWPORT | 1 |
| 70h | BOX_TEST | 3 |
| 71h | POS_TEST | 2 |
| 72h | VEC_TEST | 1 |

Any other ID is invalid: the hardware ignores it and, as GBATEK says testing confirmed, it fetches
no parameters. GBATEK's sentence on invalid commands lists `30h..33h`, which leaves out SHININESS
(`34h`) that its own table has with 32 parameters; the stub follows the table (see "Unverified").

### GXFIFO: packed and unpacked words

A word written to GXFIFO is either a command word or a parameter. The decoder knows which: when it
still waits for parameters of the current command, the word is a parameter; otherwise it is a
command word.

A command word carries up to four command IDs, byte 0 first. A byte of `00h` is NOP: nothing
executes and no parameters follow. An *unpacked* command is the same word with bytes 1-3 zero. After
a command word the decoder takes the IDs in order; for each ID with parameters it waits for that
many parameter words (the next words written) before it moves to the next ID; an ID with no
parameters, a NOP and an invalid ID go straight to the next. When all four are done the next word is
a command word again.

So the parameters of all the commands in a packed word follow the word, in command order.
GBATEK also draws layouts for commands 5-6 and 7-9 after the first four; they are the same format
again, a command word (with its upper bytes zero) followed by the parameters of its commands.

GBATEK says "the top-most packed command MUST have parameters". What the hardware does when the last
non-zero command of a word has none is not stated (see "Unverified"); the stub treats it like any
other command with no parameters.

GXFIFO is mirrored every 4 bytes up to `0400_043Fh` (16 words), so `STRD` and `STM` can send one
unpacked command and its parameters in one instruction: each word goes to the next mirror in
ascending order and each is one write of GXFIFO.

### The command ports

`0400_0440h`-`0400_05FFh`. A write to the port of command `C` is the same as a command word `C`
followed by one parameter, for each of the `N` parameters: a command with `N` parameters takes `N`
writes to its port, and a command with no parameters takes one write whose value is a dummy (the
value is ignored). The first write of a series is the command and its first parameter; the decoder
accepts the writes one at a time until the `N`th arrives.

Ports not assigned to a valid command (the gaps in the table, for example `0400_0474h`-`0400_047Fh`)
drop the write (see "Unverified"). A write to a port while the decoder waits for parameters of a
different command abandons that command and starts the port's (see "Unverified"). Two cases follow
from the decoder having one "command waiting for parameters" state shared by GXFIFO and the ports:
a GXFIFO word that arrives while a port command still owes parameters is taken as that command's
parameter, and a port write with the same ID as a command in progress from GXFIFO continues that
command; when it completes, the rest of the packed word it came from still runs (see "Unverified").

### Matrix mode and the stack pointers

MTX_MODE takes bits 0-1 of its parameter: 0 = projection, 1 = position (modelview), 2 = position and
vector together, 3 = texture. The matrices themselves are in `geometry-engine.md`.

There are three stacks and two pointers; this note keeps the pointers:

| Stack | Valid entries | Pointer | Selected by |
|---|---|---|---|
| Projection | 0 (one entry) | 1 bit (0-1) | mode 0 |
| Coordinate and directional | 0-30 (31 entries) | 6 bits (0-63), shared by both stacks; GXSTAT shows the low 5 bits | mode 1 or 2 |
| Texture | one entry at most | 1 bit | mode 3 |

All pointers start at 0. The four stack commands act on the stack of the current mode, with one
exception: in mode 1 they act on both the coordinate and the directional stack, as in mode 2, so
the stub treats modes 1 and 2 alike for them.

- **MTX_PUSH.** The current matrix is stored at entry `S`, then `S = S + 1`.
- **MTX_POP.** `S = S - N`, then the current matrix is loaded from entry `S`. `N` is bits 0-5 of the
  parameter as a signed 6-bit value (usually +1). In projection mode (and, in the stub, the
  texture mode) the parameter is ignored and `N` is always 1.
- **MTX_STORE.** The current matrix is stored at entry `N`, `N` = bits 0-4 of the parameter. The
  pointer is unchanged. In projection mode the parameter is ignored and the entry is 0.
- **MTX_RESTORE.** The current matrix is loaded from entry `N`, the same `N` and the same rules.

The pointer arithmetic is modulo the pointer width: 1 bit for the projection and texture pointers, 6
bits for the coordinate pointer. The upper half of the coordinate range (20h-3Fh) mirrors the lower
half (0-1Fh).

**The error flag (GXSTAT bit 15).** It is set when a stack command reaches an invalid entry: entry 1
for the projection stack, entries 1Fh-3Fh for the coordinate stack. For PUSH the entry is `S` before
the increment; for POP it is `S` after the subtraction; for STORE and RESTORE it is `N`. So a second
PUSH in projection mode, a PUSH with the coordinate pointer at 31 or above, a POP below 0, and a
STORE or RESTORE with `N` = 31 all set it. The flag stays set until a program writes 1 to GXSTAT bit
15. In the stub the pointer moves whether or not the entry was valid (see "Unverified").

GBATEK says entry 31 appears to exist and to be readable and writable, though it raises the flag
(`geometry-engine.md`, "Matrices", keeps a matrix there).

In projection mode GBATEK is "not sure" whether the ignored parameter of POP, STORE and RESTORE has
to be written to the FIFO; the stub requires it (one parameter word) like any other command with
parameters (see "Unverified").

### GXSTAT and the FIFO IRQ

A GXSTAT read assembles its bits from the engine's state (`geometry-engine.md`, "GXSTAT").

The FIFO IRQ (IF bit 21 on the ARM9) is level driven. With the mode in bits 30-31 set to 1 (less than
half full) or 2 (empty), the IF bit stays set for as long as that condition is true, and an
acknowledge written to IF bit 21 does nothing meanwhile: a handler has to make the condition false
(refill the FIFO) or turn the mode off before its acknowledge counts. Mode 0 raises nothing, and mode 3 is reserved: nothing is raised for it. The conditions are the real
FIFO count's: below 128 entries for mode 1, none for mode 2.

So: when a write sets the mode to 1 or 2, IF bit 21 is set at once. An acknowledge of IF bit 21
while the mode is 1 or 2 leaves it set. A write that sets the mode to 0 or 3 stops further requests;
the bit that is already in IF stays until the program acknowledges it (see "Unverified").

In the model the device holds the bit as a level: while the condition is true it reports IF bit 21 as
held up, and a write that could change the outcome (any write to the geometry block, and any write to
the ARM9 interrupt controller's IE, IME or IF) asks the I/O layer to recheck, which raises the held bit
again. Raising a bit that is already set changes nothing, so the recheck is safe after any write.

### BOX_TEST, POS_TEST and VEC_TEST

They take 3, 2 and 1 parameters; what they compute is in `geometry-engine.md`.

### Power control: POWCNT1 bits 2 and 3

POWCNT1 (`0400_0304h`, `display-mode-vram.md`) bit 2 powers the 3D rendering engine and bit 3 the
geometry engine. Bit 2 clear makes the ports `0400_0320h`-`0400_03FFh` read-only and bit 3 clear
makes `0400_0400h`-`0400_06FFh` read-only. Read-only means the writes are dropped and the reads
still answer. DISP3DCNT (`0400_0060h`) is not in either range and is always writable.

With bit 3 clear, then, GXFIFO and the command ports drop their writes (no command runs), GXSTAT's
bits 30-31 and the write-1-to-clear of bit 15 are dropped, and a GXSTAT read still reports the state
as it was.

Bit 2 gates the rendering block. While the block drops its writes (before the registers are stored,
see `geometry-engine.md`, "The rendering registers") clearing it has no observable effect;
`TODO(phase7)` with the rasterizer (#49-#52).

GBATEK also advises sending one SwapBuffers after powering the two engines on; the stub does not need
it.

### ARM9 DMA start mode 7

A channel whose ARM9 start timing (control bits 27-29) is 7 takes bursts of 112 words into GXFIFO
each time the geometry FIFO is less than half full, in `geometry-engine.md`, "The FIFO IRQ, the DMA
start and the stall". The ARM7 has no start mode 7: a channel of the ARM7 with the same bits does
not start.

## Model (not hardware)

- **The rendering registers drop their writes and read 0.** They are write-only on hardware, and
  nothing stores them yet. `TODO(phase7)` (#49-#52).
- **GXSTAT bit 15 and the pointers** are the state the decoder's stack commands change here: the
  matrix mode, the three pointers, the error flag (the matrices themselves are in
  `geometry-engine.md`).

## Edge cases

- A command whose parameters are still being sent when a savestate is taken continues after the load:
  the decoder's state (the IDs of the packed word not yet run, the command, the parameters still
  owed) is part of the state.
- A packed word of four NOPs (`0000_0000h`) does nothing. A word whose first byte is zero and others
  not still runs the others, in order.
- An invalid ID inside a packed word takes no parameters, and the next ID of the word runs.
- What a command does with its parameters, and when, is in `geometry-engine.md`.
- A write of any width to a port or GXFIFO is one entry (see "Unverified").
- A guest value never panics: every pointer wraps by its width and every unknown ID is ignored.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware, or a comparison with the reference emulator used as an oracle.

- **SHININESS (34h).** GBATEK's table gives it 32 parameters; its sentence on invalid commands lists
  `30h..33h` as the valid range. The stub follows the table: 34h is valid with 32 parameters. —
  `gpu3d/` (the parameter-count table) — verify by a test ROM that sends 34h and then a GXFIFO word.
- **A packed word whose last command has no parameters.** GBATEK says the top-most packed command
  must have parameters. What the hardware does otherwise is not stated; the stub runs it like any
  command with no parameters. — `gpu3d/` (the decoder) — verify likewise.
- **GXFIFO mirrors.** GBATEK marks the mirror range `400h-43Fh` with a question mark but relies on
  it for `STRD`/`STM`. The stub mirrors all 16 words. — `gpu3d/`, `io/map.rs`.
- **Writes to ports no command uses** are dropped, and **reads of the write-only registers** (GXFIFO,
  the ports, the rendering registers, DISP_1DOT_DEPTH) return 0. GBATEK states neither. —
  `gpu3d/` — verify by reading them on hardware.
- **A port write while another command is mid-way**, whether from GXFIFO or from another port. The
  stub abandons the old command and starts the port's, except for the same ID, which continues it
  (the rest of its packed word then still runs). A GXFIFO word that arrives while a port command
  owes parameters is that command's parameter. GBATEK states none of this. — `gpu3d/`.
- **Writes narrower than a word** to GXFIFO and the ports are each one entry with the value written;
  GBATEK does not say. — `gpu3d/`.
- **A dummy parameter in projection mode.** GBATEK is unsure whether POP, STORE and RESTORE in
  projection mode take a parameter word into the FIFO. The stub requires it. — `gpu3d/`.
- **The texture stack.** GBATEK's table says "one..none?" and gives no error rule or GXSTAT bits. The
  model treats it like the projection stack (1-bit pointer; entry 1 mirrors entry 0 and raises the
  error flag, `geometry-engine.md`, "Matrices"), and a write of 1 to
  GXSTAT bit 15 resets it too ("probably", says GBATEK). — `gpu3d/` — verify by a test ROM pushing
  twice in mode 3.
- **The invalid entries of the coordinate stack.** GBATEK says "entries 1 or 1Fh..3Fh" and also that
  the upper half mirrors the lower half, which would make 20h-3Eh valid. The stub reads the list
  literally: an access to entry 1Fh or above sets the flag. The pointer moves even then. The literal
  reading is chosen because the same sentence names entry 1 of the projection stack, which is
  certainly an error, so the sentence is a list of error entries and not a statement about
  mirroring. — `gpu3d/`.
- **IF bit 21 after the mode goes to 0.** GBATEK does not say plainly whether a bit already set
  stays until acknowledged. The stub leaves it set. — `gpu3d/` (`Gpu3d::held_requests`).
- **Mode 3 of the FIFO IRQ** ("reserved") raises nothing. — `gpu3d/`.
- **RDLINES_COUNT.** A real value depends on the renderer. The stub reads 46, the largest the field
  can hold (a renderer that always keeps its 48-line buffer full), so a program that checks for
  underflow danger sees none. — `gpu3d/`.
- **DISP3DCNT bits 12 and 13.** GBATEK says only "underflow/acknowledge". The model: bit 13 is
  set by an overflow and cleared by a write of 1, bit 12 reads 0 (and a write of 1 to it does
  nothing) until the rasterizer exists. — `gpu3d/` — verify by a test ROM writing 1 to them.
- **Unnamed addresses** between `0320h` and `06A3h` (the gaps in the table above) read 0 and drop
  writes. — `gpu3d/`, `io/map.rs`.
- **A write of 0 to GXSTAT bit 15** changes nothing, and the pointer moves even when a stack
  command reaches an invalid entry. GBATEK states neither. — `gpu3d/`.
- **POWCNT1 bit 3 clear and GXSTAT.** "Read-only" is read as: GXSTAT bits 30-31 and the clear of bit
  15 are dropped and reads still answer. — `gpu3d/`, `io/mod.rs`.
- **The GXSTAT reset value** `0600_0000h` follows from an empty FIFO; the stack pointers start at 0
  (GBATEK). The stub has no cold-boot evidence for the rest. — `gpu3d/`.
