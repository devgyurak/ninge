// SPDX-License-Identifier: Apache-2.0
//! What the cartridge returns for a read: the chip ID and where a data read starts, computed
//! from the ROM image alone. Note: `docs/hw/cart-protocol.md`.
//!
//! Everything here is a pure function of the image, so the device (`io::cart`) stays free of
//! ROM bytes: it asks these for each byte it hands out.

/// One MiB, the smallest chip the chip ID can name (`cart-protocol.md`, "The chip ID").
const MIB: usize = 1 << 20;
/// The largest chip the one-byte size field can name: 128 MiB gives size byte `7Fh`.
const MAX_SIZE_BYTE: u8 = 0x7F;
/// Chip ID byte 1, the manufacturer. GBATEK lists `C2h` among the known IDs and fixes no value.
// TODO(hw-unverified): the manufacturer byte is our choice (`cart-protocol.md`, "Unverified").
const MANUFACTURER: u8 = 0xC2;
/// Chip ID byte 4, bit 7: the newer protocol variant of the bigger ROMs.
const VARIANT_BIT: u8 = 0x80;
/// Chips larger than this have the protocol variant bit set.
// TODO(hw-unverified): GBATEK says "up to 64MB" and "64MB or more" for the two variants; the
// model sets the bit only above 64 MiB (`cart-protocol.md`, "Unverified").
const VARIANT_ABOVE: usize = 64 * MIB;

/// The data below this ROM offset cannot be read by `B7`; the command redirects it
/// (`cart-protocol.md`, "Commands").
const READABLE_FROM: u32 = 0x8000;
/// The redirect keeps this many low address bits.
const REDIRECT_MASK: u32 = 0x1FF;
/// Data wraps inside a 4 KiB block of the ROM.
pub(crate) const BLOCK_BYTES: u32 = 0x1000;

/// The byte a read returns where the chip holds nothing of the image.
// TODO(hw-unverified): GBATEK describes the unused ROM bytes as `FFh`-padded; the model reads
// `FFh` between the image and the chip end (`cart-protocol.md`, "Unverified").
const UNUSED: u8 = 0xFF;

/// The header byte that names the chip: the device capacity, `128 KiB << n` (`cart-header.md`).
const CAPACITY_OFFSET: usize = 0x14;
/// The largest `n` the model shifts by: 128 KiB `<<` 10 is 128 MiB, the largest chip the size byte
/// can name, so a larger guest value saturates there instead of overflowing the shift.
// TODO(hw-unverified): a guest capacity above 10 is treated as 10 (`cart-protocol.md`, "Unverified").
const CAPACITY_SHIFT_MAX: u32 = 10;
/// The unit of the device capacity.
const CAPACITY_UNIT: usize = 128 * 1024;

/// The chip size the model gives the image `rom`: the larger of the header's device capacity and
/// the image length rounded up to a power of two, at least 1 MiB. The header keeps a trimmed
/// image reporting its real chip; the length keeps a wrong header from hiding image data behind
/// the mirror.
// TODO(hw-unverified): taking the larger of the two, and the 1 MiB floor, are the model's choice;
// GBATEK names the capacity field but not how a card uses it (`cart-protocol.md`, "Unverified").
fn chip_bytes(rom: &[u8]) -> usize {
    let n = rom.get(CAPACITY_OFFSET).copied().map_or(0, u32::from);
    let capacity = CAPACITY_UNIT << n.min(CAPACITY_SHIFT_MAX);
    capacity.max(rom.len().next_power_of_two()).max(MIB)
}

/// The chip ID of the image `rom`, as the 32-bit data word: the first received byte in bits 0-7
/// (`cart-protocol.md`, "The chip ID").
pub(crate) fn chip_id(rom: &[u8]) -> u32 {
    let chip = chip_bytes(rom);
    let size_byte = u8::try_from(chip / MIB - 1).map_or(MAX_SIZE_BYTE, |n| n.min(MAX_SIZE_BYTE));
    let flags = if chip > VARIANT_ABOVE { VARIANT_BIT } else { 0 };
    u32::from_le_bytes([MANUFACTURER, size_byte, 0, flags])
}

// TODO(hw-unverified): the mirror is by the model's chip size (`cart-protocol.md`, "Unverified").
/// Where a `B7` read of ROM offset `address` starts inside the chip: the address mirrors into the
/// chip, then an offset below `8000h` is redirected to `8000h + (offset & 1FFh)`
/// (`cart-protocol.md`, "Commands").
pub(crate) fn start_of(address: u32, rom: &[u8]) -> u32 {
    let in_chip = address & u32::try_from(chip_bytes(rom) - 1).unwrap_or(u32::MAX);
    if in_chip < READABLE_FROM {
        READABLE_FROM + (in_chip & REDIRECT_MASK)
    } else {
        in_chip
    }
}

/// The byte of the chip at `offset`: the image's byte, or `FFh` past the image.
pub(crate) fn byte_at(rom: &[u8], offset: u32) -> u8 {
    rom.get(offset as usize).copied().unwrap_or(UNUSED)
}

#[cfg(test)]
mod tests;
