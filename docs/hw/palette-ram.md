# Palette RAM

Sources: GBATEK "DS Memory Maps" (NDS9 Memory Map); GBATEK "DS Memory Control - VRAM" (the "Other
Video RAM" table: the four 512-byte palette tables, and the notes on write widths); GBATEK "LCD
Color Palettes" (the color format and transparent color 0).
Used by: `crates/core/src/bus/memory/`   Phase: 3 (#28); the engines that read it are `bg-text.md`
(engine A, Phase 3) and Phase 4 (engine B, sprites)

The palette RAM is a small memory that holds the colors the 2D engines look up. It is not part of
the VRAM banks and has no control register: it is always there.

## Layout

2 KiB at `0500_0000h`, four 512-byte tables of 256 colors each:

| Address | Table |
|---|---|
| `0500_0000h` | engine A, BG palette |
| `0500_0200h` | engine A, OBJ palette |
| `0500_0400h` | engine B, BG palette |
| `0500_0600h` | engine B, OBJ palette |

A color is 16 bits: red in bits 0-4, green in 5-9, blue in 10-14, bit 15 not part of the color.
The tables are plain bytes to the bus; what each index means (4bpp: 16 colors per palette number,
8bpp: 256 colors, index 0 transparent) belongs to the engine (`bg-text.md`).

## Behavior

- **Who sees it.** The ARM9 only. The ARM7's address map has no palette RAM.
- **Mirroring.** The 2 KiB repeat through the whole 16 MiB block `0500_0000h`-`05FF_FFFFh`: the
  byte used is `(addr - 0500_0000h) mod 2 KiB` (GBATEK's general rule for a block that holds
  less than 16 MiB; see "Unverified").
- **Widths.** Reads work at every width. The CPU can store with halfword and word accesses only;
  a byte store is ignored and changes nothing (the rule in `vram-banks.md`, "Behavior", which
  applies to palette RAM and OAM as well). A word or halfword store goes to the aligned address
  (`arm9-memory-map.md`, "Alignment").
- **Power-on contents** are not stated; see "Unverified".

## Edge cases

- A byte store at any address of the block leaves the palette unchanged.
- The last byte before `0600_0000h` is palette RAM; `0600_0000h` is not.
- Engine B's half is separate storage: a store to `0500_0400h` does not change `0500_0000h`.

## Unverified

- **Power-on contents.** GBATEK does not state them; the palette starts at zero. —
  `Memory::new` in `crates/core/src/bus/memory/mod.rs` — verify by a test ROM on real hardware.
- **The mirror through the 16 MiB block** is GBATEK's general rule, not a statement about the
  palette. — `Memory::build_page_table` and `Memory::region_bytes` — verify likewise.
- **The ARM7 reads 0 and drops writes** at these addresses (nothing is there for it); GBATEK
  gives no value for an unmapped read. — `Memory::build_page_table` — verify likewise.
- **Byte stores** are ignored on the strength of the one note in `vram-banks.md`; it is worded
  for VRAM, palette RAM and OAM together. — `Memory::store` — verify likewise.
