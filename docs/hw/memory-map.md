# Memory map

Sources: GBATEK "DS Memory Maps".
Used by: `crates/core/src/bus/` (planned, #18)   Phase: 0 (regions the smoke ROM loads into); #18 extends it

## Regions (Phase 0 subset)

| Region | Base | Size | Seen by |
|---|---|---|---|
| Instruction TCM | 0x0000_0000 | 32 KiB | ARM9 only; not moveable; can mirror up to 0x0100_0000 |
| Data TCM | 0x0xxx_x000 | 16 KiB | ARM9 only; moveable |
| Main memory | 0x0200_0000 | 4 MiB | ARM9 and ARM7 |
| Shared WRAM | 0x0300_0000 | 0, 16 or 32 KiB | Allocated to ARM9 or ARM7 |
| ARM7 WRAM | 0x0380_0000 | 64 KiB | ARM7 only |

## Behavior

Programs commonly reach shared WRAM through a mirror at 0x037F_8000 on both CPUs; on the ARM7
this is contiguous with ARM7 WRAM, giving one 96 KiB block.

## Phase 0 use

The smoke ROM places ARM9 code at 0x0200_0000 and ARM7 code at 0x0238_0000, both main
memory, so neither depends on the WRAMCNT allocation. Nothing in Phase 0 implements the bus.

## Edge cases

Deferred to #18: mirroring, open bus, WRAMCNT and VRAM bank moves, TCM overlap with main
memory.

## Unverified

- Default Data TCM base — "DS Memory Maps" says only that it is moveable; the value is set by
  software through CP15 — settle in #15 with the CP15 note.
- The ARM7 has no TCM, cache, or CP15 — taken from general knowledge, not cited from the
  section above; cite the source or drop it in #18.
- The allocated shared WRAM is mirrored across the whole WRAM area, and 16 MiB blocks are
  usually mirrored through their unused upper part (TCM and BIOS excluded) — cite the WRAM
  control section in #18.
- Default shared WRAM ownership at reset — not stated in the section read; settle in #18.
- Whether main memory mirrors above 4 MiB inside its 16 MiB block — GBATEK's general rule
  says "usually", not confirmed here; settle in #18.
