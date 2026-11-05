// SPDX-License-Identifier: Apache-2.0
//! High-level emulation of what the BIOS and firmware do for the cartridge's code: direct boot
//! (`docs/hw/direct-boot.md`). HLE SWIs arrive in Phase 3.

mod direct_boot;

pub(crate) use direct_boot::boot;

#[cfg(test)]
mod tests;
