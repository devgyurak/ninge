// SPDX-License-Identifier: Apache-2.0
//! The ARM9's and ARM7's views of the bus: main RAM, the tightly coupled memories, VRAM bank A as LCDC
//! and the shared WRAM in `Memory`, the registers in `Io` (`docs/hw/arm9-memory-map.md`,
//! `docs/hw/vram-banks.md`, `docs/hw/wramcnt.md`).

mod memory;
mod view;

#[cfg(test)]
pub use memory::Region;
pub(crate) use memory::{BANK_COUNT, BIOS7_BASE, BIOS9_BASE, ExtPalette, VRAMCNT_A_ADDR};
pub use memory::{
    ExtPaletteBanks, Memory, VramBankState, VramMapping, VramWindow, VramWindowKind, WramShare,
    WramState,
};
pub(crate) use view::peek;
pub use view::{Arm7View, Arm9View, TcmConfig};

#[cfg(test)]
pub(crate) mod bench;
#[cfg(test)]
mod tests;
