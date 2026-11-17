// SPDX-License-Identifier: Apache-2.0
//! The cartridge: the ROM header (`docs/hw/cart-header.md`) and what a ROM read returns
//! (`docs/hw/cart-protocol.md`). The register device that speaks the protocol is
//! `io::cart`; the save chips on its SPI bus are `save` (`docs/hw/auxspi-saves.md`).

mod header;
pub(crate) mod protocol;
pub mod save;

pub(crate) use header::crc16_update;
pub use header::{Binary, BinarySpec, Field, HEADER_LEN, Header, HeaderError, crc16, parse};
pub use save::{SaveChip, SaveError, SaveType, resolve_save};

#[cfg(test)]
pub(crate) mod test_image;
#[cfg(test)]
mod tests;
