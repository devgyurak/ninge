// SPDX-License-Identifier: Apache-2.0
//! The cartridge: the ROM header (`docs/hw/cart-header.md`) and what a ROM read returns
//! (`docs/hw/cart-protocol.md`). The register device that speaks the protocol is
//! `io::cart`; the AUXSPI save arrives with #38.

mod header;
pub(crate) mod protocol;

pub(crate) use header::crc16_update;
pub use header::{Binary, BinarySpec, Field, HEADER_LEN, Header, HeaderError, crc16, parse};

#[cfg(test)]
pub(crate) mod test_image;
#[cfg(test)]
mod tests;
