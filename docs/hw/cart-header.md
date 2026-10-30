# Cartridge header

Sources: GBATEK "DS Cartridge Header".
Used by: `crates/core/src/cart/` (planned, #17: header parse, direct boot)   Phase: 0 (fields the smoke ROM relies on); #17 extends it

## Layout (Phase 0 subset)

The header sits at the start of the ROM image. Only the fields that place the two CPU
binaries and guard the header are listed here; the rest are added with #17.

| Offset | Name | Width | Meaning |
|---|---|---|---|
| 0x020 | ARM9 ROM offset | 32 | Where the ARM9 binary starts in the image. |
| 0x024 | ARM9 entry address | 32 | Address the ARM9 starts executing at. |
| 0x028 | ARM9 RAM address | 32 | Address the ARM9 binary is copied to. |
| 0x02C | ARM9 size | 32 | Byte length of the ARM9 binary. |
| 0x030 | ARM7 ROM offset | 32 | Where the ARM7 binary starts in the image. |
| 0x034 | ARM7 entry address | 32 | Address the ARM7 starts executing at. |
| 0x038 | ARM7 RAM address | 32 | Address the ARM7 binary is copied to. |
| 0x03C | ARM7 size | 32 | Byte length of the ARM7 binary. |
| 0x15E | Header CRC16 | 16 | CRC-16 over bytes 0x000 to 0x15D. |

## Allowed values (GBATEK)

| Field | Allowed |
|---|---|
| ARM9 ROM offset | 0x4000 or higher, aligned to 0x1000 |
| ARM9 entry / RAM address | 0x0200_0000 to 0x023B_FE00 |
| ARM9 size | at most 0x3B_FE00 |
| ARM7 ROM offset | 0x8000 or higher |
| ARM7 entry / RAM address | 0x0200_0000 to 0x023B_FE00, or 0x037F_8000 to 0x0380_7E00 (shared WRAM mirror + ARM7 WRAM) |
| ARM7 size | at most 0x3B_FE00 (main RAM) or 0xFE00 (ARM7 WRAM) |

See `memory-map.md` for the regions these ranges fall in.

## Behavior

Direct boot (no BIOS, no firmware) copies each binary from its ROM offset to its RAM address
for its size, then starts each CPU at its entry address. The header CRC16 exists so a loader
can reject a damaged header.

## Phase 0 use

The smoke test ROM is packed by `ndstool`, which fills these fields and computes the CRC16.
Nothing in Phase 0 parses the header; the table fixes the facts the ROM's addresses rely on.
The smoke ROM uses ARM9 load = entry = 0x0200_0000 and ARM7 load = entry = 0x0238_0000, both
in main memory and inside the ranges above.

## Edge cases

Not covered in Phase 0 (#17): misaligned or out-of-range fields, size zero, CRC mismatch.

## Unverified

- Whether direct boot should validate the header CRC16 or ignore it — decide in #17; verify
  with a test ROM carrying a bad CRC.
