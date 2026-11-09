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

// TODO(hw-unverified): bit 7 ARM7-only and bit 13 on both CPUs follow GBATEK's table as read
// (`interrupts.md`, "Unverified").
/// The IE and IF bits that exist on the ARM9 (`interrupts.md`, "IE and IF bits").
pub const VALID_BITS_ARM9: u32 = 0x003F_3F7F;
/// The IE and IF bits that exist on the ARM7 (`interrupts.md`, "IE and IF bits").
pub const VALID_BITS_ARM7: u32 = 0x01DF_3FFF;

/// Which CPU a controller belongs to: it decides the valid bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrqCpu {
    /// The ARM9.
    Arm9,
    /// The ARM7.
    Arm7,
}

/// IME, IE and IF of one CPU. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptController {
    cpu: IrqCpu,
    ime: bool,
    // `pub(crate)` so the memory's well-formedness test can corrupt it.
    pub(crate) ie: u32,
    if_: u32,
}

impl InterruptController {
    /// A controller in its power-on state: everything 0.
    // TODO(hw-unverified): the reset values are not stated (`interrupts.md`, "Unverified").
    pub fn new(cpu: IrqCpu) -> Self {
        Self {
            cpu,
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

    /// Whether this controller belongs to `cpu` and holds only bits that exist there. A savestate
    /// that decodes to anything else is corrupt (`savestate`).
    pub(crate) fn is_well_formed(&self, cpu: IrqCpu) -> bool {
        let outside = !self.valid_bits();
        self.cpu == cpu && (self.ie | self.if_) & outside == 0
    }

    /// The IE and IF bits that exist on this CPU.
    fn valid_bits(&self) -> u32 {
        match self.cpu {
            IrqCpu::Arm9 => VALID_BITS_ARM9,
            IrqCpu::Arm7 => VALID_BITS_ARM7,
        }
    }

    /// Latches the requests in `mask` into IF, whatever IME and IE say (`interrupts.md`,
    /// "Behavior"). A bit the CPU does not have is dropped.
    // TODO(hw-unverified): a bit that does not exist is dropped (`interrupts.md`, "Unverified").
    // The sources that call it: IPC (#24), timers (#25), DMA (#26), the LCD (#104).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no interrupt source exists until #24-#26 and #104"
        )
    )]
    pub(crate) fn request(&mut self, mask: u32) {
        self.if_ |= mask & self.valid_bits();
    }

    /// The byte of the register at `addr`, if one of the three holds it.
    pub(crate) fn read_byte(&self, addr: u32) -> Option<u8> {
        let (base, shift) = Self::locate(addr)?;
        let value = match base {
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
            IME_ADDR => {
                if shift == 0 {
                    self.ime = part & 1 != 0;
                }
            }
            // TODO(hw-unverified): a bit that does not exist is dropped (`interrupts.md`, "Unverified").
            IE_ADDR => {
                let byte_mask = 0xFF << shift;
                self.ie = (self.ie & !byte_mask) | (part & byte_mask & self.valid_bits());
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

#[cfg(test)]
mod tests;
