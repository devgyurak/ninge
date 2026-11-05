# VRAM banks (bank A, LCDC mode)

Sources: GBATEK "DS Memory Control - VRAM"; GBATEK "DS Memory Maps" (NDS9 Memory Map).
Used by: `crates/core/src/bus/`   Phase: 2 (#18, bank A in LCDC mode only); #35 (Phase 4) extends it

The DS has nine VRAM banks, A to I (656 KiB in all). Each has a control register at the ARM9
side. This note grows with #35; Phase 2 models bank A only, and only its plain CPU (LCDC) mapping.
The other banks, and every other MST value, are `TODO(phase4)` (#35).

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

Bank A is 128 KiB. With MST = 0 it is mapped for plain ARM9 access ("LCDC mode") at
`0680_0000h`-`0681_FFFFh`. The other MST values map it into the BG-VRAM, OBJ-VRAM and texture
areas (`TODO(phase4)`); in Phase 2 only MST = 0 shows bank A to the CPU.

The mapping depends on the whole register: the bank is visible at its LCDC address only while the
enable bit is 1 and MST is 0. Writing VRAMCNT_A can therefore move the bank in or out of the
address space, and the lookup structure derived from the mapping is rebuilt on every write.

The CPU can store to VRAM, palette RAM and OAM with halfword and word accesses only; a byte
store is dropped and changes nothing (GBATEK "DS Memory Control - VRAM", the notes under the
bank table). The one exception is the ARM7's own work-RAM mapping of banks C and D, which is
Phase 3-4. Reads work in any width. The write rule applies to the LCDC window too.

## Edge cases

- A byte write to bank A leaves the bank unchanged.
- A disabled bank is not there: reads give the unmapped value and writes are ignored
  (`arm9-memory-map.md`).
- The bank occupies exactly `0680_0000h`-`0681_FFFFh` in the LCDC area; the addresses just below
  and above are bank neighbours (B starts at `0682_0000h`), which are unmapped in Phase 2.

## Unverified

- **Reset value of VRAMCNT_A.** GBATEK gives none; the register starts at 0, so bank A is
  disabled and unmapped until software enables it. — `Memory::new` in
  `crates/core/src/bus/memory.rs` — verify by a test ROM on real hardware.
- **Reads of VRAMCNT_A** return 0: the register is write-only and GBATEK does not say what a
  read gives. — `Memory::read_slow` — verify likewise.
- **A disabled bank, or one with MST other than 0,** is unmapped at the LCDC address (reads 0,
  writes ignored). GBATEK describes the mapping only for the enabled case. —
  `Memory::build_page_table` — verify likewise.
- **Mirrors of the LCDC window** inside its 16 MiB block (`0600_0000h` block) are not modeled:
  the BG/OBJ windows and the LCDC area share a block and GBATEK's mirror rule is not specific.
  — `Memory::build_page_table` — verify likewise.
- **Bit 2 of the register** is dropped on a write (not used by bank A). —
  `Memory::write_vramcnt_a` — verify likewise.
