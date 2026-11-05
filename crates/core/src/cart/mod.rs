// SPDX-License-Identifier: Apache-2.0
//! The cartridge: the ROM header (`docs/hw/cart-header.md`). The cartridge protocol and the
//! AUXSPI save arrive in later phases.

mod header;

pub use header::{Binary, BinarySpec, Field, HEADER_LEN, Header, HeaderError, crc16, parse};

#[cfg(test)]
pub(crate) mod test_image;
#[cfg(test)]
mod tests;
