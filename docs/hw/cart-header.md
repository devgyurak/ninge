# Cartridge header

Sources: GBATEK "DS Cartridge Header"; GBATEK "BIOS Misc Functions" (GetCRC16); GBATEK "NDS BIOS RAM
Usage" (where the header lands in RAM).
Used by: `crates/core/src/cart/header.rs`, `crates/core/src/hle/direct_boot.rs`   Phase: 2 (#17)

## Layout

The header sits at the start of the ROM image. The first `170h` bytes are the part the loader keeps
in RAM (`direct-boot.md`); the rest up to `200h` is reserved, zero, and not stored. The fields
below are the ones Phase 2 reads; the others (title, game code, file tables, icon, secure area
and logo checksums) are not parsed yet.

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

All fields are little-endian.

## Allowed values (GBATEK)

| Field | Allowed |
|---|---|
| ARM9 ROM offset | 0x4000 or higher, aligned to 0x1000 |
| ARM9 entry / RAM address | 0x0200_0000 to 0x023B_FE00 |
| ARM9 size | at most 0x3B_FE00 |
| ARM7 ROM offset | 0x8000 or higher |
| ARM7 entry / RAM address | 0x0200_0000 to 0x023B_FE00, or 0x037F_8000 to 0x0380_7E00 (shared WRAM mirror + ARM7 WRAM) |
| ARM7 size | at most 0x3B_FE00 (main RAM) or 0xFE00 (the 0x037F_8000 range) |

Both bounds of a range are inclusive, as GBATEK writes them. See `memory-map.md` for the regions.
GBATEK's "NDS BIOS RAM Usage" says the boot code is loaded into `0200_0000h`-`023B_FDFFh` (main
RAM, both CPUs) or `037F_8000h`-`0380_7DFFh` (ARM7 only); the top of main RAM is left to the
header copy and system data.

## The header CRC16

GBATEK describes the algorithm as the BIOS function GetCRC16: start from an initial value
(usually `FFFFh`), and for each byte xor it into the low bits of the running value and shift
eight times, xoring in a constant when a bit falls out. The header checksum is this CRC with
initial value `FFFFh` over bytes `000h`-`15Dh`, stored at `15Eh`.

GBATEK's pseudo-code combines a table of eight constants with a shift of `7 - j`. The shifted
constant has up to 24 significant bits, so the running value has to be read as a register wider
than 16 bits (our inference from the constant widths; GBATEK does not state the register size)
and must not be cut to 16 bits after each
shift: the bits above bit 15 come down in the later shifts. Read that way, the algorithm is
CRC-16/MODBUS (initial value `FFFFh`, reflected polynomial `A001h`, no final xor, check value
`4B37h` over the ASCII text "123456789"). Cutting the value to 16 bits at every shift gives
`D851h` instead, which is a misreading. The model implements CRC-16/MODBUS; the test helper
follows the table form with a wide register, so the two are independent. The eight constants are the
single-bit entries of the byte table for the reflected polynomial `A001h`.

## Behavior

Direct boot (no BIOS, no firmware) copies each binary from its ROM offset to its RAM address for
its size, then starts the ARM9 at its entry address (`direct-boot.md`).

The CRC is computed and the result recorded in the parsed header (`crc_ok`). A mismatch does
not stop the boot: the header is accepted and the machine boots anyway. This is a host-side
choice, not a hardware fact (see "Unverified").

## Refusals

A header is refused, with a variant that names the field and the value, when:

- the image is shorter than `170h` bytes (it cannot hold the part of the header the loader keeps);
- the ARM9 ROM offset is below `4000h` or not a multiple of `1000h`;
- the ARM7 ROM offset is below `8000h`;
- an entry or RAM address is outside its range above;
- a size is zero (both binaries), or larger than its limit above;
- a binary runs past the end of the image (`ROM offset + size` beyond the image length);
- a binary runs past the loadable area (`RAM address + size` beyond `023B_FE00h` in main RAM, or
  beyond `0380_7E00h` in the ARM7 range).

Nothing is clamped, masked or guessed. An ARM7 destination in the `037F_8000h` range is accepted
and parsed; it is not placed in Phase 2 (`TODO(phase3)`, #22).

## Edge cases

- A bad CRC does not make parsing fail; only `crc_ok` is false.
- Arithmetic on guest fields uses widened integers, so `offset + size` cannot wrap.

## Unverified

- **What real firmware does on a CRC mismatch.** GBATEK says the header CRC is verified (the RAM
  copy at `027FF808h` is labelled "verified") but not what happens on a mismatch. The model
  boots anyway. — `crates/core/src/cart/header.rs` (`Header::crc_ok`) — verify by a test ROM
  with a bad CRC on hardware.
- **Minimum image length.** `170h` (the stored part of the header) is our choice for "too short
  to hold a header"; GBATEK does not give a minimum. — `parse` in `cart/header.rs` — verify likewise.
- **The load-area end** (`023B_FE00h`, `0380_7E00h`) is derived from GBATEK's statement of where
  boot code can be loaded, not from a rule on the sum of address and size. — `check` in `cart/header.rs` — verify
  likewise.
