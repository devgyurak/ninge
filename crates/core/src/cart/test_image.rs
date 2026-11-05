// SPDX-License-Identifier: Apache-2.0
//! A synthetic ROM image builder for tests: a valid header (`docs/hw/cart-header.md`) with
//! overridable fields, a correct CRC, and recognizable filler. Nothing here is real data.
//!
//! The CRC is computed by `reference_crc16` below, written from the algorithm in the note, so
//! the tests never depend on the `crc16` they check.

use super::{BinarySpec, HEADER_LEN};

/// The largest image the builder allocates, so an absurd header field cannot allocate gigabytes.
const MAX_AUTO_LEN: u64 = 0x44_0000;
/// Header bytes before the reserved area that no field covers; see `header_byte`.
const CRC_OFFSET: usize = 0x15E;

/// A synthetic image description. Build it with [`Image::valid`], change fields, then
/// [`Image::build`].
#[derive(Clone, Debug)]
pub(crate) struct Image {
    /// The ARM9 binary's header fields.
    pub arm9: BinarySpec,
    /// The ARM7 binary's header fields.
    pub arm7: BinarySpec,
    /// Opcodes/bytes placed at the start of the ARM9 binary; the rest is filler.
    pub arm9_code: Vec<u8>,
    /// Bytes placed at the start of the ARM7 binary; the rest is filler.
    pub arm7_code: Vec<u8>,
    /// The CRC to store, or `None` for the correct one.
    pub crc: Option<u16>,
    /// The image length, or `None` for "just big enough" (at least 0x200).
    pub len: Option<usize>,
}

/// Filler for header bytes no field covers (offset `i`).
pub(crate) fn header_byte(i: usize) -> u8 {
    (i.wrapping_mul(7).wrapping_add(3)) as u8
}

/// Filler at byte `j` of the ARM9 binary.
pub(crate) fn arm9_byte(j: usize) -> u8 {
    (j as u8) ^ 0xA9
}

/// Filler at byte `j` of the ARM7 binary.
pub(crate) fn arm7_byte(j: usize) -> u8 {
    (j as u8) ^ 0x7A
}

/// The CRC in GBATEK's table form: a 32-bit register and the constants shifted by `7 - j`
/// (`cart-header.md`, "The header CRC16"). Written differently from `crc16` on purpose.
pub(crate) fn reference_crc16(data: &[u8]) -> u16 {
    const TABLE: [u32; 8] = [
        0xC0C1, 0xC181, 0xC301, 0xC601, 0xCC01, 0xD801, 0xF001, 0xA001,
    ];
    let mut crc = 0xFFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for (j, constant) in TABLE.iter().enumerate() {
            let carry = crc & 1 != 0;
            crc >>= 1;
            if carry {
                crc ^= constant << (7 - j);
            }
        }
    }
    (crc & 0xFFFF) as u16
}

/// Little-endian words as bytes.
pub(crate) fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

impl Image {
    /// A valid image: ARM9 at 0x4000 (0x100 bytes, RAM and entry 0x0200_0000), ARM7 at 0x8000
    /// (0x80 bytes, RAM and entry 0x0238_0000), correct CRC.
    pub(crate) fn valid() -> Self {
        Self {
            arm9: BinarySpec {
                rom_offset: 0x4000,
                entry: 0x0200_0000,
                ram_address: 0x0200_0000,
                size: 0x100,
            },
            arm7: BinarySpec {
                rom_offset: 0x8000,
                entry: 0x0238_0000,
                ram_address: 0x0238_0000,
                size: 0x80,
            },
            arm9_code: Vec::new(),
            arm7_code: Vec::new(),
            crc: None,
            len: None,
        }
    }

    /// The bytes of the image.
    pub(crate) fn build(&self) -> Vec<u8> {
        let end = |spec: &BinarySpec| u64::from(spec.rom_offset) + u64::from(spec.size);
        let auto = end(&self.arm9)
            .max(end(&self.arm7))
            .clamp(0x200, MAX_AUTO_LEN);
        let len = self.len.unwrap_or(auto as usize);
        // Build at full size, then cut or pad to `len`, so a short image is a prefix of a
        // complete one.
        let full = len.max(auto as usize).max(0x200);
        let mut image = vec![0u8; full];
        image[HEADER_LEN..0x200].fill(0xEE);
        (0..HEADER_LEN).for_each(|i| image[i] = header_byte(i));
        Self::place(&mut image, &self.arm7, &self.arm7_code, arm7_byte);
        Self::place(&mut image, &self.arm9, &self.arm9_code, arm9_byte);
        let fields = [
            (0x20, self.arm9.rom_offset),
            (0x24, self.arm9.entry),
            (0x28, self.arm9.ram_address),
            (0x2C, self.arm9.size),
            (0x30, self.arm7.rom_offset),
            (0x34, self.arm7.entry),
            (0x38, self.arm7.ram_address),
            (0x3C, self.arm7.size),
        ];
        fields
            .iter()
            .for_each(|&(at, value)| image[at..at + 4].copy_from_slice(&value.to_le_bytes()));
        let crc = self
            .crc
            .unwrap_or_else(|| reference_crc16(&image[..CRC_OFFSET]));
        image[CRC_OFFSET..CRC_OFFSET + 2].copy_from_slice(&crc.to_le_bytes());
        image.truncate(len);
        image
    }

    /// Writes a binary's bytes where its ROM offset says, as far as the buffer reaches.
    fn place(image: &mut [u8], spec: &BinarySpec, code: &[u8], filler: fn(usize) -> u8) {
        let size = (spec.size as usize).min(image.len());
        (0..size).for_each(|j| {
            let at = spec.rom_offset as usize + j;
            let byte = code.get(j).copied().unwrap_or_else(|| filler(j));
            if let Some(slot) = image.get_mut(at) {
                *slot = byte;
            }
        });
    }
}
