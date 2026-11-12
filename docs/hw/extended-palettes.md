# Extended palettes (BG and OBJ, both engines)

Sources: GBATEK "DS Video Extended Palettes" (what the palettes are and which layers use them);
GBATEK "DS Video BG Modes / Control" (DISPCNT bits 30 and 31); GBATEK "LCD I/O BG Control"
(BGxCNT bit 13 for BG0 and BG1 on the DS); GBATEK "DS Memory Control - VRAM" (which bank backs
which slot, and that such a bank is not on the CPU bus); GBATEK "LCD OBJ - OAM Attributes" (the
palette number of an object).
Used by: `crates/core/src/gpu2d/`, `crates/core/src/bus/memory/`   Phase: 4 (#32)

The standard palettes (`palette-ram.md`) hold 256 colours for the BG and 256 for the OBJ of each
engine. An *extended* palette is 16 palettes of 256 colours, so a 256-colour layer or object can
pick one of 16 palettes the way a 16-colour one does. It lives in VRAM banks that the CPU cannot
see while they are used that way. This note says which pixels read it, where it is, and what the
renderer does when it is not there. `vram-banks.md` says which bank is in which mapping;
`bg-text.md`, `bg-modes.md` and `obj.md` say how a pixel's palette number and value are found.

## Registers

| Register | Bit | Meaning | Engines |
|---|---|---|---|
| DISPCNT | 30 | BG extended palettes on | both |
| DISPCNT | 31 | OBJ extended palettes on | both |
| BG0CNT, BG1CNT | 13 | which slot BG0 / BG1 read: 0 = its own (0 / 1), 1 = slot 2 / 3 | both |

BGxCNT bit 13 of BG2 and BG3 is the area-overflow bit of an affine layer (`bg-modes.md`); on those
two layers it has nothing to do with slots.

## Where the palette is

Each engine has **four BG slots**, each 8 KiB, and **one OBJ slot** of 8 KiB. A slot holds 16
palettes of 256 colours: the colour of palette `p`, index `i` is the little-endian 16-bit value
(`palette-ram.md`: bits 0-4 red, 5-9 green, 10-14 blue) at byte `slot start + p * 512 + i * 2`.

A bank backs slots when its VRAMCNT puts it in one of these mappings (`vram-banks.md`, "Where each
bank goes"):

| Slot of | Bank, MST | Slots it backs |
|---|---|---|
| engine A BG | E (MST 4) | slots 0-3: its first 8 KiB, second 8 KiB, ...; only the lower 32 KiB of the bank is used |
| | F, G (MST 4) | OFS.0 = 0: slots 0 and 1 (the bank's first and second 8 KiB); OFS.0 = 1: slots 2 and 3 |
| engine A OBJ | F, G (MST 5) | slot 0: the first 8 KiB of the bank; the rest is not used |
| engine B BG | H (MST 2) | slots 0-3: its four 8 KiB |
| engine B OBJ | I (MST 3) | slot 0: the first 8 KiB of the bank; the rest is not used |

A bank in such a mapping is not on the CPU bus (`vram-banks.md`, "Behavior"). Software fills it
while it is in LCDC mode (MST 0) and then switches the MST; the bytes stay as they are.

## Behavior

### Background layers (DISPCNT bit 30 set)

Only a **256-colour layer whose map entries are 16 bits** reads the extended palette:

- a text layer with BGxCNT bit 7 set (8 bits per pixel), whose entry is the 16-bit entry of
  `bg-text.md`;
- an extended layer in its tile format with 16-bit entries (`bg-modes.md`, "Extended layers"),
  BG2 or BG3.

The entry's bits 12-15 (the palette number, which a 256-colour layer does not use otherwise) pick
the palette; the pixel value is the index. The slot is the layer number (BG0-BG3 read slots 0-3),
except that BG0 reads slot 2 and BG1 reads slot 3 when their BGxCNT bit 13 is set.

Everything else keeps the standard palette: a 16-colour text layer, an affine layer with 8-bit
map entries, a 256-colour bitmap, a direct-colour bitmap (it has no palette), the large bitmap
and the backdrop (colour 0 of standard palette 0). With bit 30 clear no layer reads an extended
palette and bit 13 of BG0CNT and BG1CNT changes nothing.

### Objects (DISPCNT bit 31 set)

A **256-colour tile object**, plain or affine, reads the OBJ slot: attr2's palette number (bits
12-15, which a 256-colour object does not use otherwise) picks the palette and the pixel value
is the index. A 16-colour object keeps the standard OBJ palette (`obj.md`, "A plain tile
object"), and a bitmap object has no palette at all. With bit 31 clear the standard palette is
used as before.

### Transparency

A pixel value of 0 is transparent in every palette, standard or extended (`palette-ram.md`,
`bg-text.md`). That is decided by the value before the palette is read, so an empty slot (below)
never makes a pixel opaque by itself.

### Which bank backs a slot

The renderer finds a slot's bank from the nine VRAMCNT registers when the line is made:

- a bank backs a slot only while it is enabled and in the mapping of the table above for that
  engine and kind; a bank in LCDC mode, in a BG or OBJ window mapping or disabled backs nothing;
- when two banks back the same slot (E with F or G, or F with G) the **earlier letter** wins, as
  for the CPU windows (`vram-banks.md`, "Overlap");
- a slot that no bank backs is empty.

## Edge cases

- DISPCNT bit 30 set with no bank in an extended-palette mapping: every 256-colour pixel with a
  non-zero value reads colour 0 (black, not transparent).
- Two layers can read the same slot (BG0 with BGxCNT bit 13 set and BG2 both read slot 2): a slot
  is not owned by a layer, it is just 8 KiB of palettes.
- A bank moved out of the mapping (back to MST 0 or to a window) stops backing its slot at the next
  line; its bytes are unchanged.
- F and G in OBJ mode back one slot of 8 KiB; the upper 8 KiB of the bank is not read.
- Engine B's slots come from H and I only; engine A's from E, F and G only.

## Unverified

Each item gets `TODO(hw-unverified)` at its code site.

- **A slot no bank backs.** GBATEK does not say what a pixel reads when the enable bit is set but
  the bank is missing; the colour reads as 0, as an unbacked VRAM window does (`vram-banks.md`,
  "Unverified"). A value of 0 stays transparent either way. — `ExtPalette::color` in
  `crates/core/src/bus/memory/` — verify by a test ROM on real hardware.
- **Two banks on one slot.** The earlier letter wins, as for the CPU windows; GBATEK says nothing
  about the display side. — `ext_slots` in `crates/core/src/bus/memory/vram.rs` — verify likewise.
- **BGxCNT bit 13 with DISPCNT bit 30 clear** is taken to do nothing. — `TextBg::decode` in
  `gpu2d/text.rs` — verify likewise.
- **The 16-bit tile format of an extended layer with bit 30 clear** reads the standard palette by the
  value alone; this is `bg-modes.md`'s "Unverified" item, marked at `affine::pixel`.
- **A text layer in 16-colour mode with bit 30 set** is taken to keep the standard palette, as
  GBATEK's list says. — `text::pixel` — verify likewise.
- **When the slots are read.** The banks are read when the line is made, like every other register
  and memory (`bg-text.md`, "Unverified"); the hardware reads them while it draws. —
  `Machine::render_line` (the marker next to the `ext_*_palette` calls) — verify against a trace.

## Test ROM

The pictures are worked out from this note in `crates/runner/tests/`, never read from the
emulator:

- `bgextpal` (#32, PR 2): engine A takes slots from bank E and from bank F/G, engine B from bank
  H. BG0 and BG1 with BGxCNT bit 13 clear and set, a text layer in 8 bits per pixel, an extended
  tile layer with 16-bit entries, layers that stay on the standard palette (an affine layer with
  8-bit entries, a 256-colour bitmap) and DISPCNT bit 30 clear.
- `objextpal` (#32, PR 3): engine A through bank F or G (MST 5), engine B through bank I (MST 3);
  a plain and an affine 256-colour object with different palette numbers, a 16-colour object on the
  standard palette beside them, and DISPCNT bit 31 clear.

Banks in these mappings are invisible to the CPU, so each ROM maps the bank as LCDC (MST 0), fills
it, and then writes the extended-palette MST.
