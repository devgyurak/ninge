# VRAM banks (bank A: LCDC mode and engine A BG)

Sources: GBATEK "DS Memory Control - VRAM"; GBATEK "DS Memory Maps" (NDS9 Memory Map).
Used by: `crates/core/src/bus/`   Phase: 2 (#18, bank A in LCDC mode), 3 (#28, bank A as engine A
BG); #35 (Phase 4) extends it

The DS has nine VRAM banks, A to I (656 KiB in all). Each has a control register at the ARM9
side. This note grows with #35; Phases 2-3 model bank A only, in two mappings: the plain CPU
(LCDC) mapping and the engine A background mapping (MST 1, below). The other banks, and the
other MST values (OBJ, texture), are `TODO(phase4)` (#35).

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0240h` | VRAMCNT_A (ARM9) | 8 | W | 0 (see "Unverified") |

### VRAMCNT_A bits

| Bits | Meaning |
|---|---|
| 0-1 | MST: how the bank is used (bit 2 is unused by banks A, B, H, I) |
| 2 | not used by bank A |
| 3-4 | OFS: offset 0-3, used by the BG, OBJ and texture mappings |
| 5-6 | not used |
| 7 | enable: 0 = disabled, 1 = enabled |

The register at the same address on the ARM7 is VRAMSTAT (read-only status of banks C and D);
the ARM7 view is Phase 3.

## Behavior

Bank A is 128 KiB. Its MST field picks where it appears:

| MST | Where bank A is | Phase |
|---|---|---|
| 0 | LCDC: `0680_0000h`-`0681_FFFFh`, plain ARM9 access | 2 |
| 1 | engine A BG VRAM: `0600_0000h + OFS * 20000h`, 128 KiB | 3 |
| 2 | engine A OBJ VRAM (`0640_0000h + OFS.0 * 20000h`) | `TODO(phase4)` (#35) |
| 3 | texture slot OFS | `TODO(phase7)` |

The mapping depends on the whole register: the bank is visible only while the enable bit is 1,
and only at the one place MST names. Writing VRAMCNT_A can therefore move the bank in or out of
the address space, and the lookup structure derived from the mapping is rebuilt on every write.
The bank's contents stay as they are when it moves: the same 128 KiB are seen at the new place.

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

## Edge cases

- A byte write to bank A leaves the bank unchanged.
- A disabled bank is not there: reads give the unmapped value and writes are ignored
  (`arm9-memory-map.md`).
- The bank occupies exactly `0680_0000h`-`0681_FFFFh` in the LCDC area; the addresses just below
  and above are bank neighbours (B starts at `0682_0000h`), which are unmapped in Phase 2.
- With MST 1 the bank occupies exactly the 128 KiB slot OFS names in the BG window; the address
  just before it (for OFS 0, `05FF_FFFFh`) is palette RAM and the address just after is
  unmapped until bank B or another bank joins in Phase 4.
- Changing MST or OFS moves the bank without clearing it; the LCDC address and the BG slot never
  hold the bank at the same time.

## Unverified

- **Reset value of VRAMCNT_A.** GBATEK gives none; the register starts at 0, so bank A is
  disabled and unmapped until software enables it. — `Memory::new` in
  `crates/core/src/bus/memory/mod.rs` — verify by a test ROM on real hardware.
- **Reads of VRAMCNT_A** return 0: the register is write-only and GBATEK does not say what a
  read gives. — `read_registers` in `bus/view.rs` — verify likewise.
- **A disabled bank, or one with MST other than 0,** is unmapped at the LCDC address (reads 0,
  writes ignored). GBATEK describes the mapping only for the enabled case. —
  `Memory::build_page_table` — verify likewise. The `memmap` ROM records the disabled-bank read
  (slot 31); running that ROM on hardware settles it. MST values other than 0 are not exercised.
- **Mirrors of the LCDC window** inside its 16 MiB block (`0600_0000h` block) are not modeled:
  the BG/OBJ windows and the LCDC area share a block and GBATEK's mirror rule is not specific.
  — `Memory::build_page_table` — verify likewise.
- **Mirrors of the engine A BG window.** The 2 MiB window is wider than the 512 KiB the banks can
  fill, and nothing is mirrored here: an address outside the bank's slot is unmapped (reads 0,
  writes dropped). — `Memory::build_page_table` — verify likewise.
- **Byte stores to BG VRAM** are dropped like every other VRAM byte store, on the strength of the
  one note under GBATEK's bank table; the note is worded for VRAM in general and does not
  single out the BG window. — `Memory::store` — verify likewise.
- **Bit 2 of the register** is dropped on a write (not used by bank A). —
  `Memory::write_vramcnt_a` — verify likewise.
