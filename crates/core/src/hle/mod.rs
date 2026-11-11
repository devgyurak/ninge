// SPDX-License-Identifier: Apache-2.0
//! High-level emulation of what the BIOS and firmware do for the cartridge's code: direct boot
//! (`docs/hw/direct-boot.md`) and the BIOS vectors, SWIs and IRQ dispatcher (`docs/hw/hle-swi.md`).

mod bios;
mod direct_boot;
mod swi;

pub use bios::SwiRecord;
pub(crate) use bios::{BiosState, enter};
pub(crate) use direct_boot::boot;

#[cfg(test)]
mod tests;
