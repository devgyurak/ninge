// SPDX-License-Identifier: Apache-2.0
//! ROM header parsing and validation. Note: `docs/hw/cart-header.md`.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Bytes of the header the loader keeps (`cart-header.md`, "Layout").
pub const HEADER_LEN: usize = 0x170;

/// Which CPU's binary a field belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    /// The ARM9 binary.
    Arm9,
    /// The ARM7 binary.
    Arm7,
}

impl fmt::Display for Binary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Arm9 => "ARM9",
            Self::Arm7 => "ARM7",
        })
    }
}

/// A header field that can be out of range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// ARM9 ROM offset (0x020).
    Arm9RomOffset,
    /// ARM9 entry address (0x024).
    Arm9Entry,
    /// ARM9 RAM address (0x028).
    Arm9RamAddress,
    /// ARM9 size (0x02C).
    Arm9Size,
    /// ARM7 ROM offset (0x030).
    Arm7RomOffset,
    /// ARM7 entry address (0x034).
    Arm7Entry,
    /// ARM7 RAM address (0x038).
    Arm7RamAddress,
    /// ARM7 size (0x03C).
    Arm7Size,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Arm9RomOffset => "ARM9 ROM offset",
            Self::Arm9Entry => "ARM9 entry address",
            Self::Arm9RamAddress => "ARM9 RAM address",
            Self::Arm9Size => "ARM9 size",
            Self::Arm7RomOffset => "ARM7 ROM offset",
            Self::Arm7Entry => "ARM7 entry address",
            Self::Arm7RamAddress => "ARM7 RAM address",
            Self::Arm7Size => "ARM7 size",
        })
    }
}

/// Why a header was refused (`cart-header.md`, "Refusals").
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    /// The image cannot hold the part of the header the loader keeps.
    #[error("the image is {len} bytes, too short for a {HEADER_LEN}-byte header")]
    ImageTooShort {
        /// The image length.
        len: usize,
    },
    /// A field is outside the range GBATEK allows.
    #[error("{field} {value:#x} is outside the allowed range")]
    FieldOutOfRange {
        /// The field.
        field: Field,
        /// What the header holds.
        value: u32,
    },
    /// A binary has size zero.
    #[error("the {binary} binary has size zero")]
    ZeroSize {
        /// The binary.
        binary: Binary,
    },
    /// A binary runs past the end of the image.
    #[error("the {binary} binary ends at {end:#x}, past the end of the {len}-byte image")]
    PastImageEnd {
        /// The binary.
        binary: Binary,
        /// ROM offset plus size.
        end: u64,
        /// The image length.
        len: usize,
    },
    /// A binary runs past the area boot code can be loaded into.
    #[error("the {binary} binary ends at {end:#x}, past the load area limit {limit:#x}")]
    PastLoadArea {
        /// The binary.
        binary: Binary,
        /// RAM address plus size.
        end: u64,
        /// The end of the load area the RAM address is in.
        limit: u32,
    },
}

/// Where one CPU's binary comes from and goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinarySpec {
    /// Offset of the binary in the image.
    pub rom_offset: u32,
    /// Address the CPU starts at.
    pub entry: u32,
    /// Address the binary is copied to.
    pub ram_address: u32,
    /// Length in bytes.
    pub size: u32,
}

/// The parsed header fields Phase 2 uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// The ARM9 binary.
    pub arm9: BinarySpec,
    /// The ARM7 binary.
    pub arm7: BinarySpec,
    /// The CRC16 stored at 0x15E.
    pub crc: u16,
    /// Whether the stored CRC matches the one computed over 0x000-0x15D. A mismatch does not
    /// refuse the header.
    // TODO(hw-unverified): what real firmware does on a mismatch is not stated; we boot anyway
    // (`docs/hw/cart-header.md`, "Unverified").
    pub crc_ok: bool,
}

// Offsets of the fields Phase 2 reads (`cart-header.md`, "Layout").
const ARM9_ROM_OFFSET: usize = 0x020;
const ARM7_ROM_OFFSET: usize = 0x030;
/// Entry, RAM address and size follow the ROM offset, one word each.
const ENTRY_DELTA: usize = 0x04;
const RAM_ADDRESS_DELTA: usize = 0x08;
const SIZE_DELTA: usize = 0x0C;
const CRC_OFFSET: usize = 0x15E;

// Allowed values (`cart-header.md`, "Allowed values"); both bounds of a range are inclusive.
const ARM9_ROM_OFFSET_MIN: u32 = 0x4000;
/// The ARM9 ROM offset is a multiple of this.
const ARM9_ROM_OFFSET_ALIGN: u32 = 0x1000;
const ARM7_ROM_OFFSET_MIN: u32 = 0x8000;
const MAIN_RAM_MIN: u32 = 0x0200_0000;
/// The last main RAM address boot code can start at, and where a loaded binary must end.
const MAIN_RAM_LOAD_END: u32 = 0x023B_FE00;
const MAIN_RAM_SIZE_MAX: u32 = 0x3B_FE00;
const WRAM_MIN: u32 = 0x037F_8000;
/// The last address of the ARM7 range, and where a binary loaded there must end.
const WRAM_LOAD_END: u32 = 0x0380_7E00;
const WRAM_SIZE_MAX: u32 = 0xFE00;

/// CRC-16/MODBUS: initial value `FFFFh`, reflected polynomial `A001h`, no final xor
/// (`cart-header.md`, "The header CRC16").
// GBATEK's table form, read with a register wider than 16 bits, is this variant; the standard
// check vector and the CRC `ndstool` wrote into our own test ROM agree.
pub fn crc16(data: &[u8]) -> u16 {
    const POLYNOMIAL: u16 = 0xA001;
    data.iter().fold(0xFFFF, |crc, &byte| {
        (0..8).fold(crc ^ u16::from(byte), |crc, _| match crc & 1 {
            0 => crc >> 1,
            _ => (crc >> 1) ^ POLYNOMIAL,
        })
    })
}

/// Reads a little-endian word; zero outside the slice (the callers have checked the length).
fn word(image: &[u8], offset: usize) -> u32 {
    image
        .get(offset..offset.saturating_add(4))
        .and_then(|bytes| bytes.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn in_main_ram(address: u32) -> bool {
    (MAIN_RAM_MIN..=MAIN_RAM_LOAD_END).contains(&address)
}

fn in_wram(address: u32) -> bool {
    (WRAM_MIN..=WRAM_LOAD_END).contains(&address)
}

fn spec(image: &[u8], base: usize) -> BinarySpec {
    BinarySpec {
        rom_offset: word(image, base),
        entry: word(image, base + ENTRY_DELTA),
        ram_address: word(image, base + RAM_ADDRESS_DELTA),
        size: word(image, base + SIZE_DELTA),
    }
}

/// Checks one binary. `arm7` selects the ARM7's wider RAM range and its ROM offset rule; the
/// fields are checked in header order, then the size, then where the binary ends.
fn check(spec: &BinarySpec, binary: Binary, image_len: usize) -> Result<(), HeaderError> {
    let arm7 = binary == Binary::Arm7;
    let (offset_field, entry_field, ram_field, size_field) = match arm7 {
        false => (
            Field::Arm9RomOffset,
            Field::Arm9Entry,
            Field::Arm9RamAddress,
            Field::Arm9Size,
        ),
        true => (
            Field::Arm7RomOffset,
            Field::Arm7Entry,
            Field::Arm7RamAddress,
            Field::Arm7Size,
        ),
    };
    let out_of_range = |field, value| Err(HeaderError::FieldOutOfRange { field, value });
    let offset_ok = match arm7 {
        false => {
            spec.rom_offset >= ARM9_ROM_OFFSET_MIN
                && spec.rom_offset.is_multiple_of(ARM9_ROM_OFFSET_ALIGN)
        }
        true => spec.rom_offset >= ARM7_ROM_OFFSET_MIN,
    };
    if !offset_ok {
        return out_of_range(offset_field, spec.rom_offset);
    }
    let allowed = |address| in_main_ram(address) || (arm7 && in_wram(address));
    if !allowed(spec.entry) {
        return out_of_range(entry_field, spec.entry);
    }
    if !allowed(spec.ram_address) {
        return out_of_range(ram_field, spec.ram_address);
    }
    if spec.size == 0 {
        return Err(HeaderError::ZeroSize { binary });
    }
    // The size limit follows the range the RAM address is in.
    let (size_max, limit) = match in_wram(spec.ram_address) {
        true => (WRAM_SIZE_MAX, WRAM_LOAD_END),
        false => (MAIN_RAM_SIZE_MAX, MAIN_RAM_LOAD_END),
    };
    if spec.size > size_max {
        return out_of_range(size_field, spec.size);
    }
    // TODO(hw-unverified): the load-area end is derived from where GBATEK says boot code can be
    // loaded, not from a rule on the sum (`cart-header.md`, "Unverified").
    let rom_end = u64::from(spec.rom_offset) + u64::from(spec.size);
    if rom_end > image_len as u64 {
        return Err(HeaderError::PastImageEnd {
            binary,
            end: rom_end,
            len: image_len,
        });
    }
    let ram_end = u64::from(spec.ram_address) + u64::from(spec.size);
    if ram_end > u64::from(limit) {
        return Err(HeaderError::PastLoadArea {
            binary,
            end: ram_end,
            limit,
        });
    }
    Ok(())
}

/// Parses and validates the header at the start of `image`.
///
/// Pure: it never panics, whatever the bytes (the image is guest input). A CRC mismatch is
/// recorded in [`Header::crc_ok`] and does not refuse the header (`cart-header.md`, "Behavior").
// TODO(hw-unverified): the image must hold `HEADER_LEN` bytes, our choice for "too short to
// hold a header" (`cart-header.md`, "Unverified").
pub fn parse(image: &[u8]) -> Result<Header, HeaderError> {
    if image.len() < HEADER_LEN {
        return Err(HeaderError::ImageTooShort { len: image.len() });
    }
    let arm9 = spec(image, ARM9_ROM_OFFSET);
    let arm7 = spec(image, ARM7_ROM_OFFSET);
    check(&arm9, Binary::Arm9, image.len())?;
    check(&arm7, Binary::Arm7, image.len())?;
    let crc = image
        .get(CRC_OFFSET..CRC_OFFSET + 2)
        .and_then(|bytes| bytes.try_into().ok())
        .map_or(0, u16::from_le_bytes);
    Ok(Header {
        arm9,
        arm7,
        crc,
        crc_ok: image.get(..CRC_OFFSET).map(crc16) == Some(crc),
    })
}
