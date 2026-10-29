# Memory map

Sources: GBATEK "DS Memory Maps".
Used by: `crates/core/src/bus/`   Phase: 0 (regions the smoke ROM loads into); #18 extends it

## Regions (Phase 0 subset)

| Region | Base | Size | Seen by |
|---|---|---|---|
| Instruction TCM | 0x0000_0000 | 32 KiB | ARM9 only; not moveable; can mirror up to 0x0100_0000 |
| Data TCM | 0x0xxx_x000 | 16 KiB | ARM9 only; moveable (default base 0x027C_0000) |
| Main memory | 0x0200_0000 | 4 MiB | ARM9 and ARM7 |
| Shared WRAM | 0x0300_0000 | 0, 16 or 32 KiB | Allocated to ARM9 or ARM7 |
| ARM7 WRAM | 0x0380_0000 | 64 KiB | ARM7 only |

The ARM7 has no TCM, cache, or CP15.

## Behavior

The allocated shared WRAM (16 or 32 KiB) is mirrored across the whole WRAM area. Programs
commonly reach shared WRAM at 0x037F_8000 on both CPUs; on the ARM7 this is contiguous with
ARM7 WRAM. Blocks of 16 MiB holding valid memory are usually mirrored through their unused
upper part; TCM and BIOS are not mirrored.

## Phase 0 use

The smoke ROM places ARM9 code at 0x0200_0000 and ARM7 code at 0x0238_0000, both main
memory, so neither depends on the WRAMCNT allocation. Nothing in Phase 0 implements the bus.

## Edge cases

Deferred to #18: mirroring, open bus, WRAMCNT and VRAM bank moves, TCM overlap with main
memory.

## Unverified

- Default shared WRAM ownership at reset — not stated in the section read; settle in #18.
- Whether main memory mirrors above 4 MiB inside its 16 MiB block — GBATEK's general rule
  says "usually", not confirmed here; settle in #18.
