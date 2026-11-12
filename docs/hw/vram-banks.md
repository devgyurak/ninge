# VRAM banks (A-I: registers and LCDC mode; bank A as engine A BG)

Sources: GBATEK "DS Memory Control - VRAM"; GBATEK "DS Memory Maps" (NDS9 Memory Map).
Used by: `crates/core/src/bus/`   Phase: 2 (#18, bank A in LCDC mode), 3 (#28, bank A as engine A
BG), 4 (#35: all nine banks; LCDC mode first, the BG and OBJ mappings and the ARM7 mapping follow)

The DS has nine VRAM banks, A to I (656 KiB in all). Each has its own control register on the
ARM9 side. This note grows with #35. The first step models every bank, its register and the plain
CPU (LCDC) mapping, plus bank A as engine A BG VRAM (MST 1, the one other mapping Phase 3 had).
The other mappings (BG, OBJ and the ARM7's) are `TODO(phase4)` (#35).

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

The register at `0400_0240h` on the ARM7 is VRAMSTAT (read-only status of banks C and D); the
ARM7 view is `TODO(phase4)` (#35).

## Behavior

Each bank's MST field picks where it appears. The mapping depends on the whole register: a bank
is visible only while its enable bit is 1, and only at the one place its MST names. Writing a
VRAMCNT register can therefore move a bank in or out of the address space, and the lookup
structure derived from the mapping is rebuilt on every write. Only the VRAM block of the address
space (`0600_0000h`-`06FF_FFFFh`) depends on these registers, so only that block is rebuilt. A
bank's contents stay as they are when it moves: the same bytes are seen at the new place.

For every bank MST 0 is LCDC mode, plain CPU access at the address in the table above. The
other MST values are `TODO(phase4)` (#35), except bank A's MST 1 below.

| Bank A MST | Where bank A is | Step |
|---|---|---|
| 0 | LCDC: `0680_0000h`-`0681_FFFFh`, plain ARM9 access | LCDC |
| 1 | engine A BG VRAM: `0600_0000h + OFS * 20000h`, 128 KiB | Phase 3 |
| 2, 3 | engine A OBJ VRAM; texture slot | `TODO(phase4)` (#35), `TODO(phase7)` |

### Display mode 2 reads banks A-D

The VRAM display mode of engine A (`display-mode-vram.md`) shows the bank DISPCNT bits 18-19
name: 0 to 3 for bank A to D. The bank is read through its LCDC mapping, so it must be enabled
with MST 0 to show; a bank that is not is drawn as zeros (`display-mode-vram.md`, "Unverified").

### Engine A BG VRAM (MST 1)

The engine A BG VRAM window is `0600_0000h`-`061F_FFFFh` (2 MiB of address space; the banks that
can fill it add up to 512 KiB). Bank A with MST 1 sits at `0600_0000h + OFS * 20000h` where OFS
is VRAMCNT_A bits 3-4 (0-3), so it covers one of four 128 KiB slots of the first 512 KiB. The
CPU reads and writes it there like any other memory; engine A reads the same bytes as tile data
and background maps (`bg-text.md`). The BG screen and character base offsets of engine A count
from `0600_0000h`, the start of this window (`bg-text.md`, "Memory"); bank A is the only bank that
exists in Phase 3, so a background that points outside its slot reads what no bank holds (zero,
`bg-text.md`, "Unverified").

The CPU can store to VRAM, palette RAM (`palette-ram.md`) and OAM with halfword and word accesses
only; a byte store is dropped and changes nothing (GBATEK "DS Memory Control - VRAM", the notes
under the bank table). The one exception is the ARM7's own work-RAM mapping of banks C and D,
which is Phase 3-4. Reads work in any width. The write rule applies to the LCDC window and to the
engine A BG window alike.

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

## Edge cases

- A byte write to any bank leaves the bank unchanged.
- A disabled bank is not there: reads give the unmapped value and writes are ignored
  (`arm9-memory-map.md`).
- Each bank occupies exactly its LCDC window; the address just below and just above are its
  neighbours' (bank X+1 starts where X ends), and the address past bank I (`068A_4000h`) is
  unmapped.
- Disabling bank X unmaps its window at once and leaves its neighbours as they are.
- A VRAMCNT write rebuilds only the VRAM block of the page table; the result is the same as
  rebuilding the whole table.
- With MST 1 bank A occupies exactly the 128 KiB slot OFS names in the BG window; the address
  just before it (for OFS 0, `05FF_FFFFh`) is palette RAM and the address just after is
  unmapped until bank B or another bank joins (`TODO(phase4)`).
- Changing MST or OFS moves a bank without clearing it; the LCDC address and the BG slot never
  hold bank A at the same time.
- A word or halfword write that spans several VRAMCNT registers writes each byte to the register
  at its own address; `0400_0247h` inside the span is WRAMCNT.

## Unverified

- **Reset value of the VRAMCNT registers.** GBATEK gives none; each starts at 0, so every bank is
  disabled and unmapped until software enables it. — `Memory::new` in
  `crates/core/src/bus/memory/mod.rs` — verify by a test ROM on real hardware.
- **Reads of the VRAMCNT registers** return 0: they are write-only and GBATEK does not say what
  a read gives. — `Memory::read_control` — verify likewise.
- **A disabled bank, or one with an MST this step does not map,** is unmapped at the LCDC address
  (reads 0, writes ignored). GBATEK describes the mapping only for the enabled case. —
  `Memory::build_vram_pages` — verify likewise. The `memmap` ROM records the disabled-bank read
  (slot 31); running that ROM on hardware settles it.
- **Mirrors of the LCDC window** inside its 16 MiB block (`0600_0000h` block) are not modeled:
  the BG/OBJ windows and the LCDC area share a block and GBATEK's mirror rule is not specific.
  An address inside the block that no bank holds reads 0 and a write is dropped. —
  `Memory::build_vram_pages` — verify likewise.
- **Mirrors of the engine A BG window.** The 2 MiB window is wider than the 512 KiB the banks can
  fill, and nothing is mirrored here: an address outside the bank's slot is unmapped (reads 0,
  writes dropped). — `Memory::build_vram_pages` — verify likewise.
- **Byte stores to BG VRAM** are dropped like every other VRAM byte store, on the strength of the
  one note under GBATEK's bank table; the note is worded for VRAM in general and does not
  single out the BG window. — `Memory::store` — verify likewise.
- **Unused bits of a register** (MST bit 2 for A, B, H, I; OFS for E, H, I; bits 5-6) are
  dropped on a write. — `Memory::write_vramcnt` — verify likewise.
