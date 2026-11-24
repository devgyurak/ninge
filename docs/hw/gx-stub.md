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

**Implemented (#43):** DISP3DCNT; the command decoder (GXFIFO, packed and unpacked words, and the
command ports); the matrix mode and the three stack pointers with their error flag; GXSTAT; the FIFO
IRQ condition; the zero reads of the rendering and test registers; ARM9 DMA start mode 7;
POWCNT1 bits 2 and 3 gating the ports.

**A deliberate approximation, not a hardware claim: instant drain.** A command is executed (consumed
and dropped) the moment its last parameter arrives. The FIFO and the PIPE never hold an entry, so a
program can never see a full or half-full FIFO, a busy engine, a busy stack, or a stalled CPU. There
is no per-command cycle cost, no stall on a full FIFO, and SWAP_BUFFERS does not wait for VBlank.
`TODO(phase7)` (#48): `geometry-engine.md` replaces this with the real FIFO, the PIPE, the command
timings and the swap at VBlank.

**Not implemented (`TODO(phase7)`):** matrix arithmetic, vertices and polygons, lighting, textures,
POS_TEST, VEC_TEST and BOX_TEST results, RAM_COUNT and the lock-up SWAP_BUFFERS has after an
incomplete polygon list; all in `geometry-engine.md`. The rendering registers' effect is #49-#52.

## Registers

All of these are ARM9 only. The ARM7 does not see them: the ARM7 has the sound registers at
`0400_0400h` (`sound-registers.md`).

| Address | Name | Width | Access | Stub |
|---|---|---|---|---|
| `0400_0060h` | DISP3DCNT | 16 | R/W | stored, bits 12-13 acknowledge |
| `0400_0320h` | RDLINES_COUNT | 8 | R | a fixed value, see "Unverified" |
| `0400_0330h`-`0400_03BFh` | the rendering registers (EDGE_COLOR, ALPHA_TEST_REF, CLEAR_COLOR, CLEAR_DEPTH, CLRIMAGE_OFFSET, FOG_COLOR, FOG_OFFSET, FOG_TABLE, TOON_TABLE) | 8-32 | W | write dropped, reads 0 |
| `0400_0400h`-`0400_043Fh` | GXFIFO (16 mirrors of one word) | 32 | W | the command decoder |
| `0400_0440h`-`0400_05FFh` | the command ports | 32 | W | the command decoder |
| `0400_0600h` | GXSTAT | 32 | R, bits 30-31 R/W, bit 15 write-1-to-clear | see below |
| `0400_0604h` | RAM_COUNT | 32 | R | reads 0 |
| `0400_0610h` | DISP_1DOT_DEPTH | 16 | W | write dropped |
| `0400_0620h`-`0400_062Fh` | POS_RESULT | 4 words | R | reads 0 |
| `0400_0630h`-`0400_0635h` | VEC_RESULT | 3 halfwords | R | reads 0 |
| `0400_0640h`-`0400_067Fh` | CLIPMTX_RESULT | 16 words | R | reads 0 |
| `0400_0680h`-`0400_06A3h` | VECMTX_RESULT | 9 words | R | reads 0 |

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

Bits 0-11 and 14 are plain read/write (mask `4FFFh`). Bits 12 and 13 are status flags raised by the
rendering engine; there is no rendering engine in the stub, so they are not stored: they read 0 and
a write of 1 to them changes nothing (see "Unverified"). A byte write changes only its byte.

### GXSTAT bits (`0400_0600h`)

| Bits | Meaning | Stub reads |
|---|---|---|
| 0 | BOX_TEST, POS_TEST or VEC_TEST busy | 0 |
| 1 | BOX_TEST result: 1 = parts or all of the box inside the view volume | 0 until a BOX_TEST has run, then 1 (see "Model") |
| 2-7 | Not used | 0 |
| 8-12 | Position/vector stack level: the low 5 bits of the 6-bit coordinate stack pointer | the pointer |
| 13 | Projection stack level (the 1-bit projection pointer) | the pointer |
| 14 | Matrix stack busy (a push or pop is executing) | 0 |
| 15 | Matrix stack overflow/underflow error. Write 1 to clear | the flag |
| 16-23 | The low 8 bits of the number of 40-bit FIFO entries (0-256) | 0 |
| 24 | FIFO full (the top bit of the count, 256 entries) | 0 |
| 25 | FIFO less than half full | 1 |
| 26 | FIFO empty | 1 |
| 27 | Geometry engine busy (a command is executing, or the PIPE or FIFO holds data) | 0 |
| 28-29 | Not used | 0 |
| 30-31 | FIFO IRQ: 0 = never, 1 = less than half full, 2 = empty, 3 = reserved | the value written |

Only bits 30-31 are writable, apart from bit 15: a write of 1 to bit 15 clears the error flag and
also resets the projection stack pointer to 0; GBATEK says it "probably" also resets the texture
stack pointer (see "Unverified"). A write of 0 to bit 15 changes nothing. Every other bit ignores
writes. The register is 32 bits wide; a halfword or byte access sees the matching part and a write
to a part acts on only that part.

Reset value in the stub: `0600_0000h` (bits 25 and 26: less than half full, empty).

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
vector together, 3 = texture. The stub keeps only this number; the matrices themselves are not
computed.

There are three stacks and two pointers. The stub keeps the pointers:

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

GBATEK says entry 31 appears to exist and to be readable and writable, though it raises the flag. The
stub keeps no matrices, so that matters only to the flag.

In projection mode GBATEK is "not sure" whether the ignored parameter of POP, STORE and RESTORE has
to be written to the FIFO; the stub requires it (one parameter word) like any other command with
parameters (see "Unverified").

### GXSTAT and the FIFO IRQ

A GXSTAT read assembles the bits of the table above from the pointers and the error flag. The FIFO
and the engine are never occupied, so bits 0 and 14 read 0, bit 1 reads the stub's BOX_TEST result,
bits 16-24 read 0 (an empty FIFO), bits 25 and 26 read 1, and bit 27 reads 0.

The FIFO IRQ (IF bit 21 on the ARM9) is level driven. With the mode in bits 30-31 set to 1 (less than
half full) or 2 (empty), the IF bit stays set for as long as that condition is true, and an
acknowledge written to IF bit 21 does nothing meanwhile: a handler has to make the condition false
(refill the FIFO) or turn the mode off before its acknowledge counts. Mode 0 raises nothing, and mode 3 is reserved: the stub raises nothing for
it. The stub's FIFO is always empty and always less than half full, so the condition holds for
modes 1 and 2 and never for 0 and 3.

So: when a write sets the mode to 1 or 2, IF bit 21 is set at once. An acknowledge of IF bit 21
while the mode is 1 or 2 leaves it set. A write that sets the mode to 0 or 3 stops further requests;
the bit that is already in IF stays until the program acknowledges it (see "Unverified").

In the model the device holds the bit as a level: while the condition is true it reports IF bit 21 as
held up, and a write that could change the outcome (any write to the geometry block, and any write to
the ARM9 interrupt controller's IE, IME or IF) asks the I/O layer to recheck, which raises the held bit
again. Raising a bit that is already set changes nothing, so the recheck is safe after any write.

### BOX_TEST, POS_TEST and VEC_TEST

The three test commands take their parameters (BOX_TEST 3, POS_TEST 2, VEC_TEST 1) and consume
them. The tests are busy for a while on hardware (GXSTAT bit 0); the stub finishes at once and bit
0 stays 0. POS_RESULT, VEC_RESULT read 0. BOX_TEST sets the result bit (GXSTAT bit 1) to 1,
meaning "parts or all of the box are inside the view volume", so a program that tests a box before
it draws takes its normal path; see "Model".

### Power control: POWCNT1 bits 2 and 3

POWCNT1 (`0400_0304h`, `display-mode-vram.md`) bit 2 powers the 3D rendering engine and bit 3 the
geometry engine. Bit 2 clear makes the ports `0400_0320h`-`0400_03FFh` read-only and bit 3 clear
makes `0400_0400h`-`0400_06FFh` read-only. Read-only means the writes are dropped and the reads
still answer. DISP3DCNT (`0400_0060h`) is not in either range and is always writable.

With bit 3 clear, then, GXFIFO and the command ports drop their writes (no command runs), GXSTAT's
bits 30-31 and the write-1-to-clear of bit 15 are dropped, and a GXSTAT read still reports the state
as it was.

Bit 2 gates the rendering block, whose registers all drop writes in the stub anyway, so clearing it
has no observable effect here; `TODO(phase7)` with the rasterizer (#49-#52).

GBATEK also advises sending one SwapBuffers after powering the two engines on; the stub does not need
it.

### ARM9 DMA start mode 7

A channel whose ARM9 start timing (control bits 27-29) is 7 starts when the geometry command FIFO is
less than half full, and moves 112 words to GXFIFO, or fewer if the count runs out first. GBATEK
gives the usual setup as 32-bit units, a fixed destination `0400_0400h`, repeat off.

The stub's FIFO is always less than half full, so the condition always holds: a channel set to mode 7
starts as soon as it is enabled and moves its words in bursts of 112 (the last burst shorter), one
after the other, until the count is spent. The bursts follow one another with nothing in between
(the FIFO never fills, so no burst waits), and the model's transfers are atomic (`dma.md`,
"Model"), so it moves the whole count in one transfer; the words and their order are the same as in
bursts. Then the end of a transfer works as for any channel (`dma.md`): IF bit 8 + *n* if control
bit 30 is set, and the enable bit clears when repeat is off. Each word goes to the destination
exactly as a CPU write of that word would, so the decoder above sees them in order.

The ARM7 has no start mode 7. A channel of the ARM7 with the same bits does not start.

## Model (not hardware)

- **Instant drain** (see "Scope").
- **The BOX_TEST result.** GXSTAT bit 1 reads 0 until a BOX_TEST has been executed and 1 after it.
  Hardware computes it from the box and the matrices; the stub has no matrices and answers "inside",
  which keeps a program on its draw path. This is a stub value, not a hardware fact. `TODO(phase7)`.
- **The results read 0:** POS_RESULT, VEC_RESULT, CLIPMTX_RESULT, VECMTX_RESULT, RAM_COUNT.
- **The rendering registers drop their writes and read 0.** They are write-only on hardware, and the
  stub has nothing to store them for. `TODO(phase7)` (#49-#52).
- **GXSTAT bit 15 and the pointers** are the only state the commands change: the matrix mode, the
  three pointers, the error flag.

## Edge cases

- A command whose parameters are still being sent when a savestate is taken continues after the load:
  the decoder's state (the IDs of the packed word not yet run, the command, the parameters still
  owed) is part of the state.
- A packed word of four NOPs (`0000_0000h`) does nothing. A word whose first byte is zero and others
  not still runs the others, in order.
- An invalid ID inside a packed word takes no parameters, and the next ID of the word runs.
- A COLOR, VTX_16 or any other command that draws is consumed with its parameters and has no effect.
- SWAP_BUFFERS (one parameter) is consumed with no wait.
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
  stub treats it like the projection stack (1-bit pointer, entry 1 invalid), and a write of 1 to
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
- **Repeat on in DMA mode 7.** GBATEK gives repeat off as the usual setup and says nothing of repeat
  on. The stub starts the channel once, when it is enabled; a repeating channel stays enabled with
  its count reloaded and is not started again, because no trigger follows (the level that starts it
  is only looked at when the channel is enabled). — `io/dma/mod.rs` (`Dma::write_control`).
- **RDLINES_COUNT.** A real value depends on the renderer. The stub reads 46, the largest the field
  can hold (a renderer that always keeps its 48-line buffer full), so a program that checks for
  underflow danger sees none. — `gpu3d/`.
- **DISP3DCNT bits 12 and 13** are not stored and a write of 1 to them does nothing. GBATEK says only
  "underflow/acknowledge". — `gpu3d/` — verify by a test ROM writing 1 to them.
- **Unnamed addresses** between `0320h` and `06A3h` (the gaps in the table above) read 0 and drop
  writes. — `gpu3d/`, `io/map.rs`.
- **A write of 0 to GXSTAT bit 15** changes nothing, and the pointer moves even when a stack
  command reaches an invalid entry. GBATEK states neither. — `gpu3d/`.
- **POWCNT1 bit 3 clear and GXSTAT.** "Read-only" is read as: GXSTAT bits 30-31 and the clear of bit
  15 are dropped and reads still answer. — `gpu3d/`, `io/mod.rs`.
- **The GXSTAT reset value** `0600_0000h` follows from an empty FIFO; the stack pointers start at 0
  (GBATEK). The stub has no cold-boot evidence for the rest. — `gpu3d/`.
