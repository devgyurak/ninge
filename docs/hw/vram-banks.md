# VRAM banks (A-I: registers, LCDC mode, the BG, OBJ, extended and ARM7 mappings)

Sources: GBATEK "DS Memory Control - VRAM"; GBATEK "DS Memory Maps" (NDS9 Memory Map).
Used by: `crates/core/src/bus/`   Phase: 2 (#18, bank A in LCDC mode), 3 (#28, bank A as engine A
BG), 4 (#35: all nine banks; LCDC mode, the BG and OBJ mappings of both engines, and the ARM7's
mapping of banks C and D with VRAMSTAT)

The DS has nine VRAM banks, A to I (656 KiB in all). Each has its own control register on the
ARM9 side. This note grows with #35. It models every bank, its register, the plain CPU (LCDC)
mapping, the BG and OBJ mappings of both 2D engines, and the ARM7's own mapping of banks C and D.
The mappings only the display controller sees are decoded but not read by the CPU: the texture ones
are the renderer's `TODO(phase7)`; the extended-palette ones are read by the 2D engines
(`extended-palettes.md`, #32).

## The banks

One 656 KiB buffer holds all nine banks, in alphabetical order: bank X starts at the sum of the
sizes of the banks before it. The same offsets give the LCDC addresses (`0680_0000h` + offset).

| Bank | Size | Offset in the buffer | LCDC address (MST 0) |
|---|---|---|---|
| A | 128 KiB | `00000h` | `0680_0000h`-`0681_FFFFh` |
| B | 128 KiB | `20000h` | `0682_0000h`-`0683_FFFFh` |
| C | 128 KiB | `40000h` | `0684_0000h`-`0685_FFFFh` |
| D | 128 KiB | `60000h` | `0686_0000h`-`0687_FFFFh` |
| E | 64 KiB | `80000h` | `0688_0000h`-`0688_FFFFh` |
| F | 16 KiB | `90000h` | `0689_0000h`-`0689_3FFFh` |
| G | 16 KiB | `94000h` | `0689_4000h`-`0689_7FFFh` |
| H | 32 KiB | `98000h` | `0689_8000h`-`0689_FFFFh` |
| I | 16 KiB | `A0000h` | `068A_0000h`-`068A_3FFFh` |

The nine LCDC windows are contiguous: bank X ends exactly where bank X+1 begins, and `068A_4000h`
is the first address past bank I.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0240h` | VRAMCNT_A (ARM9) | 8 | W | 0 (see "Unverified") |
| `0400_0241h` | VRAMCNT_B (ARM9) | 8 | W | 0 |
| `0400_0242h` | VRAMCNT_C (ARM9) | 8 | W | 0 |
| `0400_0243h` | VRAMCNT_D (ARM9) | 8 | W | 0 |
| `0400_0244h` | VRAMCNT_E (ARM9) | 8 | W | 0 |
| `0400_0245h` | VRAMCNT_F (ARM9) | 8 | W | 0 |
| `0400_0246h` | VRAMCNT_G (ARM9) | 8 | W | 0 |
| `0400_0248h` | VRAMCNT_H (ARM9) | 8 | W | 0 |
| `0400_0249h` | VRAMCNT_I (ARM9) | 8 | W | 0 |

`0400_0247h` is WRAMCNT (`wramcnt.md`), not a VRAM register.

### VRAMCNT bits (all nine)

| Bits | Meaning |
|---|---|
| 0-2 | MST: how the bank is used. Bit 2 is not used by banks A, B, H and I |
| 3-4 | OFS: offset 0-3. Not used by banks E, H and I |
| 5-6 | not used |
| 7 | enable: 0 = disabled, 1 = enabled |

A bit a bank does not use is dropped on a write, so the register keeps only the bits in this
table: A and B keep `9Bh`, C, D, F and G keep `9Fh`, E keeps `87h`, H and I keep `83h`.

### VRAMSTAT (ARM7)

The ARM7 has no VRAMCNT registers. Its register at `0400_0240h` is VRAMSTAT, 8 bits, read-only:

| Bits | Meaning |
|---|---|
| 0 | bank C is enabled and in the ARM7 mapping (enable bit 1 and MST 2) |
| 1 | bank D is enabled and in the ARM7 mapping |
| 2-7 | not used, always 0 |

It does not show OFS. A write to it from the ARM7 changes nothing. (The byte at `0400_0241h` on the
ARM7 is WRAMSTAT, `wramcnt.md`.)

## Behavior

Each bank's MST field picks where it appears. The mapping depends on the whole register: a bank
is visible only while its enable bit is 1, and only at the one place its MST names. Writing a
VRAMCNT register can therefore move a bank in or out of the address space, and the lookup
structure derived from the mapping is rebuilt on every write. Only the VRAM block of the address
space (`0600_0000h`-`06FF_FFFFh`) depends on these registers, so only that block is rebuilt. A
bank's contents stay as they are when it moves: the same bytes are seen at the new place.

### Where each bank goes

OFS.0 is OFS bit 0 and OFS.1 is OFS bit 1 (VRAMCNT bits 3 and 4). "+x" is the offset inside the
window below.

Seen by the ARM9 CPU, at these windows:

| Window | Address | Size | Contents |
|---|---|---|---|
| LCDC | `0680_0000h` | 656 KiB | every bank, MST 0 (the table under "The banks") |
| engine A BG | `0600_0000h` | up to 512 KiB | A, B, C, D (MST 1): `+20000h * OFS`; E (MST 1): `+0`; F, G (MST 1): `+4000h * OFS.0 + 10000h * OFS.1` |
| engine A OBJ | `0640_0000h` | up to 256 KiB | A, B (MST 2): `+20000h * OFS.0`; E (MST 2): `+0`; F, G (MST 2): `+4000h * OFS.0 + 10000h * OFS.1` |
| engine B BG | `0620_0000h` | up to 128 KiB | C (MST 4): `+0`; H (MST 1): `+0`; I (MST 1): `+8000h` |
| engine B OBJ | `0660_0000h` | up to 128 KiB | D (MST 4): `+0`; I (MST 2): `+0` |

Seen by the display controller only (not in any CPU address space; "slot" is the unit the
renderer picks by):

| Use | Bank, MST | Slot |
|---|---|---|
| texture image | A, B, C, D (MST 3) | slot OFS (0-3) |
| texture palette | E (MST 3) | slots 0-3 |
| | F, G (MST 3) | slot `OFS.0 + 4 * OFS.1` (0, 1, 4 or 5) |
| engine A BG extended palette | E (MST 4) | slots 0-3 (only the lower 32 KiB used) |
| | F, G (MST 4) | slots 0-1 for OFS 0, slots 2-3 for OFS 1 |
| engine A OBJ extended palette | F, G (MST 5) | slot 0 |
| engine B BG extended palette | H (MST 2) | slots 0-3 |
| engine B OBJ extended palette | I (MST 3) | slot 0 |

Seen by the ARM7 CPU, in its own table:

| Window | Address | Size | Contents |
|---|---|---|---|
| ARM7 VRAM | `0600_0000h` | up to 256 KiB | C, D (MST 2): `+20000h * OFS.0` |

A bank in this mapping is the ARM7's work RAM: the ARM9 does not see it (its LCDC address and
every window are empty for it) and the 2D engines do not read it. The ARM7 sees no other VRAM.

A bank with any other MST (for example MST 5 on a bank that is not F or G) is mapped nowhere.
For the F and G extended-palette rows the register's OFS.1 is not used; for A, B in OBJ mode
and for the ARM7 mapping GBATEK says OFS.1 must be zero, and only OFS.0 counts here.

The CPU reads and writes a CPU-visible mapping like any other memory; the display controller
reads the same bytes. A bank in an extended-palette or texture mapping is where the CPU does not
see it: its LCDC address and every window are empty for it, and to change such a bank software
switches it to MST 0 first. Its contents stay as they are when it moves.

### Overlap

Two enabled banks can name overlapping addresses (two banks at the same BG offset, E under A,
C over H and I). GBATEK does not say what the CPU sees. Here the bank that is earlier in the
alphabet wins, for reads and writes alike, over every address they share; the bank behind it
holds its bytes but the CPU cannot reach them while the earlier one covers them. When the earlier
bank is disabled or moves, the later one shows through at once.

### Display mode 2 reads banks A-D

The VRAM display mode of engine A (`display-mode-vram.md`) shows the bank DISPCNT bits 18-19
name: 0 to 3 for bank A to D. The bank is read through its LCDC mapping, so it must be enabled
with MST 0 to show; a bank that is not is drawn as zeros (`display-mode-vram.md`, "Unverified").

### The 2D engines' view of a window

A renderer reads a window by offset from the window's start, not by CPU address: engine A's text
backgrounds count their character and screen bases from `0600_0000h` (`bg-text.md`, "Memory").
The four windows are engine A BG (512 KiB), engine A OBJ (256 KiB), engine B BG (128 KiB) and
engine B OBJ (128 KiB). A read at an offset finds what the CPU would read at the window's address
plus that offset, and reads 0 where no bank holds it or past the window's size, so a renderer
and the CPU never disagree about what the window contains.

The CPU can store to VRAM, palette RAM (`palette-ram.md`) and OAM with halfword and word accesses
only; a byte store is dropped and changes nothing (GBATEK "DS Memory Control - VRAM", the notes
under the bank table). The one exception is the ARM7's own work-RAM mapping of banks C and D:
the ARM7 can store a byte there (GBATEK: the reason is the GBA mode, where two 128 KiB banks stand in
for the GBA's 256 KiB work RAM). Reads work in any width. The write rule applies to every window
the ARM9 sees.

## Evidence

The `memmap` test ROM (`testroms/memmap/`) enables bank A with MST 0, stores a word and a
halfword at `0680_0000h` and reads them back, stores a byte over a word and reads the word
(slots 28-30), then disables the bank and reads (slot 31). Its last group moves the bank to MST 1:
the halfword stored in LCDC mode is still there at `0600_0004h` (slot 41), a word stored and read
back in the BG window (42), a byte store there ignored (43), OFS 1 moves the window to
`0602_0000h` (44) and the LCDC address is empty (45). `crates/runner/tests/memmap.rs` holds the
expected values. The `bgtext` ROM (`bg-text.md`) draws from the BG window.

The `vrambanks` test ROM (`testroms/vrambanks/`) enables each of the nine banks with MST 0 in
turn, writes a different marker word at the first and last word of its LCDC window, and reads
back the markers and the words just outside the window. Its slots are listed at the top of
`arm9.s`; `crates/runner/tests/vrambanks.rs` computes the expected values from this note alone.
Its later groups set every MST and OFS that maps a bank into one of the four windows, read the
first and last word of the window mapping and the words just outside it with every other bank off,
overlap two or three banks, check that a byte store to each window is ignored, that a bank keeps
its bytes when it moves between LCDC, BG and OBJ mappings, and that a bank in an extended-palette
or texture mode leaves the LCDC address and all four windows empty.

The ARM7 half of the same ROM (`arm7.s`) and the ARM9 take turns through IPCSYNC (`ipc.md`). In
each step the ARM9 sets banks C and D, and the ARM7 reads VRAMSTAT and the words at the ARM7
window's start, end and just past it, stores a word, a byte and a halfword there, and records
what it found at `0200_5000h`. The steps cover MST 2 at OFS 0 and 1, OFS.1 set, both banks on at
the same offset and side by side, and banks that are in another mode or disabled. The ARM9 then
switches bank C to LCDC and reads the words the ARM7 stored: the same bytes.

## Edge cases

- A byte write to any bank leaves the bank unchanged.
- A disabled bank is not there: reads give the unmapped value and writes are ignored
  (`arm9-memory-map.md`).
- Each bank occupies exactly its window (its size at the address the tables give); the address
  just below and just above are the neighbours', or empty. The address past bank I in the LCDC
  area (`068A_4000h`) is unmapped.
- Disabling bank X unmaps its window at once and leaves its neighbours as they are.
- A VRAMCNT write rebuilds only the VRAM block of the page table; the result is the same as
  rebuilding the whole table.
- Engine A BG offset 0 is preceded by palette RAM (`05FF_FFFFh`).
- In the ARM7 mapping a bank occupies exactly 128 KiB at `+20000h * OFS.0`; C and D at the same
  offset overlap and C wins, at different offsets they sit side by side.
- The ARM7's bytes in C and D are the same bytes the ARM9 sees in LCDC mode; moving a bank
  between the two mappings does not clear it.
- VRAMSTAT shows a bank in the ARM7 mapping only: C or D in MST 0, 1, 3 or 4, or disabled with
  MST 2, reads 0 there.
- Changing MST or OFS moves a bank without clearing it; a bank sits in at most one place.
- A bank in an extended-palette or texture mapping, or with an MST the tables do not define, is
  in no window and not in the LCDC area.
- A word or halfword write that spans several VRAMCNT registers writes each byte to the register
  at its own address; `0400_0247h` inside the span is WRAMCNT.

## Unverified

- **Reset value of the VRAMCNT registers.** GBATEK gives none; each starts at 0, so every bank is
  disabled and unmapped until software enables it. — `Memory::new` in
  `crates/core/src/bus/memory/mod.rs` — verify by a test ROM on real hardware.
- **Reads of the VRAMCNT registers** return 0: they are write-only and GBATEK does not say what
  a read gives. — `Memory::read_control` — verify likewise.
- **A disabled bank, or one with an MST that is not 0 here,** is unmapped at the LCDC address
  (reads 0, writes ignored). GBATEK describes the mapping only for the enabled case. —
  `Memory::build_vram_pages` — verify likewise. The `memmap` ROM records the disabled-bank read
  (slot 31); running that ROM on hardware settles it.
- **Mirrors of the LCDC window** inside its 16 MiB block (`0600_0000h` block) are not modeled:
  the BG/OBJ windows and the LCDC area share a block and GBATEK's mirror rule is not specific.
  An address inside the block that no bank holds reads 0 and a write is dropped. —
  `Memory::build_vram_pages` — verify likewise.
- **Mirrors inside a window.** The BG and OBJ windows are wider than the banks that fill them
  (engine A BG is 2 MiB of address space for at most 512 KiB), and nothing is mirrored: an address
  no bank holds is unmapped (reads 0, writes dropped), and so is a renderer offset past the
  window. — `Memory::build_vram_pages`, `VramWindow::byte` — verify likewise.
- **Overlapping banks.** The earlier bank in the alphabet wins both reads and writes where two
  enabled banks cover the same address; GBATEK is silent. — `Memory::build_vram_pages` — verify
  likewise.
- **An MST the tables do not define** (C-G with MST 5-7 apart from F and G with MST 5, A, B with
  MST 4 or more, and so on) maps nowhere. — `vram::mapping` — verify likewise.
- **OFS.1 where it must be zero** (A, B in OBJ mode; the ARM7 mapping) and **in F, G's
  extended-palette modes**: only OFS.0 is used. — `vram::mapping` — verify likewise.
- **Byte stores to BG and OBJ VRAM** are dropped like every other VRAM byte store, on the strength of the
  one note under GBATEK's bank table; the note is worded for VRAM in general and does not
  single out the BG window. — `Memory::store` — verify likewise.
- **The ARM7 window's mirrors.** Nothing repeats the ARM7's 256 KiB through the rest of
  `0600_0000h`-`06FF_FFFFh`; an address no bank holds reads 0 and a write is dropped. —
  `Memory::build_vram_pages` — verify likewise.
- **Overlap of C and D in the ARM7 mapping** follows the same rule as the ARM9 windows: C wins.
  — `Memory::build_vram_pages` — verify likewise.
- **Reads of VRAMSTAT above bit 1** are 0, as GBATEK says ("always zero"); a write from the ARM7
  is ignored, which GBATEK does not state. — `Memory::read_control` — verify likewise.
- **Unused bits of a register** (MST bit 2 for A, B, H, I; OFS for E, H, I; bits 5-6) are
  dropped on a write. — `Memory::write_vramcnt` — verify likewise.
