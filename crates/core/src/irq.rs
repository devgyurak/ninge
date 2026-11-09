// SPDX-License-Identifier: Apache-2.0
//! One CPU's interrupt controller: IME, IE and IF (`docs/hw/interrupts.md`).
//!
//! Each CPU owns one; they sit at the same addresses and never see each other.

use serde::{Deserialize, Serialize};

/// IME, at `0400_0208h` (`interrupts.md`, "Registers").
pub const IME_ADDR: u32 = 0x0400_0208;
/// IE, at `0400_0210h` (`interrupts.md`, "Registers").
pub const IE_ADDR: u32 = 0x0400_0210;
/// IF, at `0400_0214h` (`interrupts.md`, "Registers").
pub const IF_ADDR: u32 = 0x0400_0214;

/// IF bit 0: LCD VBlank (`interrupts.md`, "IE and IF bits").
pub const VBLANK_IRQ: u32 = 1 << 0;
/// IF bit 1: LCD HBlank.
pub const HBLANK_IRQ: u32 = 1 << 1;
/// IF bit 2: LCD VCount match.
pub const VCOUNT_IRQ: u32 = 1 << 2;

/// IF bit 16: IPC sync, from the other CPU's IPCSYNC bit 13 (`ipc.md`, "Behavior").
pub const IPCSYNC_IRQ: u32 = 1 << 16;
/// IF bit 17: IPC send FIFO empty (`ipc.md`, "Interrupts").
pub const SEND_EMPTY_IRQ: u32 = 1 << 17;
/// IF bit 18: IPC receive FIFO not empty (`ipc.md`, "Interrupts").
pub const RECV_NOT_EMPTY_IRQ: u32 = 1 << 18;

// TODO(hw-unverified): bit 7 ARM7-only and bit 13 on both CPUs follow GBATEK's table as read
// (`interrupts.md`, "Unverified").
/// The IE and IF bits that exist on the ARM9 (`interrupts.md`, "IE and IF bits").
pub const VALID_BITS_ARM9: u32 = 0x003F_3F7F;
/// The IE and IF bits that exist on the ARM7 (`interrupts.md`, "IE and IF bits").
pub const VALID_BITS_ARM7: u32 = 0x01DF_3FFF;

/// IME, IE and IF of one CPU: the ARM9's when `ARM9` is true, the ARM7's otherwise. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptController<const ARM9: bool> {
    ime: bool,
    ie: u32,
    if_: u32,
}

impl<const ARM9: bool> InterruptController<ARM9> {
    /// A controller in its power-on state: everything 0.
    // TODO(hw-unverified): the reset values are not stated (`interrupts.md`, "Unverified").
    pub fn new() -> Self {
        Self {
            ime: false,
            ie: 0,
            if_: 0,
        }
    }

    /// IME bit 0.
    pub fn ime(&self) -> bool {
        self.ime
    }

    /// IE, the enabled sources.
    pub fn ie(&self) -> u32 {
        self.ie
    }

    /// IF, the latched requests.
    pub fn if_(&self) -> u32 {
        self.if_
    }

    /// Whether the controller asserts the CPU's IRQ line: `IME && (IE & IF) != 0`
    /// (`interrupts.md`, "Behavior"). The CPU's own I flag is the CPU's to check.
    pub fn line(&self) -> bool {
        self.ime && self.ie & self.if_ != 0
    }

    /// Whether IE and IF hold only bits that exist on this CPU. A savestate that decodes to
    /// anything else is corrupt (`savestate`).
    pub(crate) fn is_well_formed(&self) -> bool {
        (self.ie | self.if_) & !Self::valid_bits() == 0
    }

    /// A controller with IE set as given, unmasked, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn with_unchecked_ie(ie: u32) -> Self {
        Self { ie, ..Self::new() }
    }

    /// The IE and IF bits that exist on this CPU.
    fn valid_bits() -> u32 {
        match ARM9 {
            true => VALID_BITS_ARM9,
            false => VALID_BITS_ARM7,
        }
    }

    /// Latches the requests in `mask` into IF, whatever IME and IE say (`interrupts.md`,
    /// "Behavior"). A bit the CPU does not have is dropped.
    // TODO(hw-unverified): a bit that does not exist is dropped, and IF latches whatever IE and
    // IME say (`interrupts.md`, "Unverified").
    // The LCD (#104) and IPC (#24) call it; timers (#25) and DMA (#26) follow.
    pub(crate) fn request(&mut self, mask: u32) {
        self.if_ |= mask & Self::valid_bits();
    }

    /// The byte of the register at `addr`, if one of the three holds it.
    pub(crate) fn read_byte(&self, addr: u32) -> Option<u8> {
        let (base, shift) = Self::locate(addr)?;
        let value = match base {
            // TODO(hw-unverified): IME bits 1-31 read 0 (`interrupts.md`, "Unverified").
            IME_ADDR => u32::from(self.ime),
            IE_ADDR => self.ie,
            _ => self.if_,
        };
        Some((value >> shift) as u8)
    }

    /// Writes a byte to the register at `addr`; a byte no register holds is ignored. IF takes a
    /// 1 as "clear this bit" (`interrupts.md`, "Behavior").
    pub(crate) fn write_byte(&mut self, addr: u32, value: u8) {
        let Some((base, shift)) = Self::locate(addr) else {
            return;
        };
        let part = u32::from(value) << shift;
        match base {
            // Only bit 0 of the register exists, and it sits in byte 0.
            // TODO(hw-unverified): the other IME bits are dropped and read 0
            // (`interrupts.md`, "Unverified").
            IME_ADDR => {
                if shift == 0 {
                    self.ime = part & 1 != 0;
                }
            }
            // TODO(hw-unverified): a bit that does not exist is dropped
            // (`interrupts.md`, "Unverified").
            IE_ADDR => {
                let byte_mask = 0xFF << shift;
                self.ie = (self.ie & !byte_mask) | (part & byte_mask & Self::valid_bits());
            }
            _ => self.if_ &= !part,
        }
    }

    /// Which register holds `addr`, and the bit position of its byte within the 32-bit register.
    fn locate(addr: u32) -> Option<(u32, u32)> {
        [IME_ADDR, IE_ADDR, IF_ADDR].into_iter().find_map(|base| {
            let offset = addr.checked_sub(base).filter(|&o| o < 4)?;
            Some((base, offset * 8))
        })
    }
}

impl<const ARM9: bool> Default for InterruptController<ARM9> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
