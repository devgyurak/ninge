// SPDX-License-Identifier: Apache-2.0
//! The save chips on the cartridge slot's SPI bus: EEPROM, FLASH and FRAM. Note:
//! `docs/hw/auxspi-saves.md`.
//!
//! A chip is a byte state machine ([`SaveChip`]): the bus hands it one byte at a time and takes one
//! back. It knows nothing of the slot's registers, the clock or the ROM, so the same FLASH model can
//! later serve the firmware chip (#39).

mod chip;

pub use chip::SaveChip;

use serde::{Deserialize, Serialize};

/// The chip fitted to the cartridge. `None` is a card with no save memory: every read is `FFh`;
/// `Auto` is a chip to be found out from its first commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveType {
    /// No save chip.
    #[default]
    None,
    /// EEPROM of 512 bytes (one address byte, page 16).
    Eeprom512,
    /// EEPROM of 8 KiB (two address bytes, page 32).
    Eeprom8K,
    /// EEPROM of 64 KiB (two address bytes, page 128).
    Eeprom64K,
    /// EEPROM of 128 KiB (three address bytes).
    Eeprom128K,
    /// FLASH of 256 KiB.
    Flash256K,
    /// FLASH of 512 KiB.
    Flash512K,
    /// FLASH of 1 MiB.
    Flash1M,
    /// FLASH of 8 MiB.
    Flash8M,
    /// FRAM of 8 KiB (two address bytes, no page limit).
    Fram8K,
    /// FRAM of 32 KiB (two address bytes, no page limit).
    Fram32K,
    /// The chip is not known yet: the first session that writes decides it (`auxspi-saves.md`,
    /// "Auto-detection"). Until then the chip is empty, and `data` has no bytes.
    Auto,
}

/// Every type with its name and its size in bytes (`docs/hw/auxspi-saves.md`, "Chip type").
const TYPES: [(SaveType, &str, usize); 12] = [
    (SaveType::None, "none", 0),
    (SaveType::Eeprom512, "eeprom-512", 512),
    (SaveType::Eeprom8K, "eeprom-8k", 8 * 1024),
    (SaveType::Eeprom64K, "eeprom-64k", 64 * 1024),
    (SaveType::Eeprom128K, "eeprom-128k", 128 * 1024),
    (SaveType::Flash256K, "flash-256k", 256 * 1024),
    (SaveType::Flash512K, "flash-512k", 512 * 1024),
    (SaveType::Flash1M, "flash-1m", 1024 * 1024),
    (SaveType::Flash8M, "flash-8m", 8 * 1024 * 1024),
    (SaveType::Fram8K, "fram-8k", 8 * 1024),
    (SaveType::Fram32K, "fram-32k", 32 * 1024),
    (SaveType::Auto, "auto", 0),
];

impl SaveType {
    /// The name the hosts use for the type, such as `flash-512k`.
    pub fn name(self) -> &'static str {
        self.entry().1
    }

    /// The type called `name`, or `None` when no type has that name.
    pub fn from_name(name: &str) -> Option<Self> {
        TYPES.iter().find(|entry| entry.1 == name).map(|e| e.0)
    }

    /// Bytes of memory the chip has.
    pub fn len(self) -> usize {
        self.entry().2
    }

    /// Whether the type has no memory (`None`).
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The type a save file of `len` bytes belongs to, or `None` when no chip has that size. 8 KiB
    /// is an EEPROM, the commoner part: a FRAM of that size is named with its flag.
    pub fn from_file_len(len: usize) -> Option<Self> {
        TYPES
            .iter()
            .find(|entry| entry.2 == len && entry.0 != Self::Fram8K && !entry.0.is_empty())
            .map(|e| e.0)
    }

    fn entry(self) -> &'static (SaveType, &'static str, usize) {
        // A `SaveType` is always one of the table's rows; the fallback is the `None` row.
        TYPES
            .iter()
            .find(|entry| entry.0 == self)
            .unwrap_or(&TYPES[0])
    }
}

/// Why a save image was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    /// The image is not as long as the chip's memory.
    #[error("a {kind} save is {expected} bytes, the image has {found}", kind = .kind.name())]
    BadLength {
        /// The chip type the image was meant for.
        kind: SaveType,
        /// Bytes the chip holds.
        expected: usize,
        /// Bytes the image has.
        found: usize,
    },
}

#[cfg(test)]
mod tests;
