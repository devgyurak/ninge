// SPDX-License-Identifier: Apache-2.0
//! The ARM9's and ARM7's views of memory: main RAM, the tightly coupled memories and VRAM bank A as LCDC
//! (`docs/hw/arm9-memory-map.md`, `docs/hw/vram-banks.md`).

mod memory;
mod view;

pub use memory::{Memory, Region, VramBankState};
pub use view::{Arm7View, Arm9View, TcmConfig};

#[cfg(test)]
mod tests;
