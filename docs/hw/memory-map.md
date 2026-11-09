# Memory map

Sources: GBATEK "DS Memory Maps".
Used by: `crates/core/src/bus/`   Phase: 0 (regions the smoke ROM loads into); #18 (Phase 2) adds the ARM9 view

## Regions (Phase 0 subset)

| Region | Base | Size | Seen by |
|---|---|---|---|
| Instruction TCM | 0x0000_0000 | 32 KiB | ARM9 only; not moveable; can mirror up to 0x0100_0000; sizes and enable bits in `cp15.md` |
| Data TCM | 0x0xxx_x000 | 16 KiB | ARM9 only; moveable; base, size and enable bits in `cp15.md` |
| Main memory | 0x0200_0000 | 4 MiB | ARM9 and ARM7 |
| Shared WRAM | 0x0300_0000 | 0, 16 or 32 KiB | Allocated to ARM9 or ARM7 |
| ARM7 WRAM | 0x0380_0000 | 64 KiB | ARM7 only |

## Behavior

Programs commonly reach shared WRAM through a mirror at 0x037F_8000 on both CPUs; on the ARM7
this is contiguous with ARM7 WRAM, giving one 96 KiB block when all 32 KiB of shared WRAM are
allocated to the ARM7.

## Phase 0 use

The smoke ROM places ARM9 code at 0x0200_0000 and ARM7 code at 0x0238_0000, both main
memory, so neither depends on the WRAMCNT allocation. Nothing in Phase 0 implements the bus.

## Edge cases

The ARM9 view (main RAM mirroring, TCM placement and priority, unmapped addresses) is in
`arm9-memory-map.md`; VRAM bank A as LCDC is in `vram-banks.md`. The shared WRAM split and the ARM7 view are in
`wramcnt.md` (#22); the other VRAM banks are deferred (#35).

## Unverified

- Default Data TCM base — moved to `arm9-memory-map.md`, "Unverified": GBATEK names `027C0000h`
  as the default but not whether it is a reset value; CP15 starts at zero (`cp15.md`).
- The ARM7 has no TCM, cache, or CP15 — GBATEK "DS Memory Control - Cache and TCM" says so ("The
  NDS7 doesn't have any TCM, Cache, or CP15"); verified.
- Shared WRAM mirroring and ownership — moved to `wramcnt.md`; the reset value is Unverified there.
- Whether main memory mirrors above 4 MiB inside its 16 MiB block — moved to
  `arm9-memory-map.md`: GBATEK's general rule ("typically" mirrored) is applied and recorded as
  Unverified there.
