// SPDX-License-Identifier: Apache-2.0
//! The register file: current registers, banked copies, CPSR and the five SPSRs
//! (`docs/hw/cpu-registers.md`).

use serde::{Deserialize, Serialize};

/// CPSR mode-bit field, M4–M0 (`docs/hw/cpu-registers.md`, "Mode bits").
const MODE_MASK: u32 = 0x1F;
/// CPSR value after power-on: Supervisor mode, IRQ and FIQ disabled, ARM state. GBATEK
/// "ARM CPU Exceptions" states the mode, I = 1, F = 1 and T = 0 for a reset.
// TODO(hw-unverified): the sources do not state R0–R14 or N, Z, C, V after a reset; they are
// zero here. Verify with a test ROM or a trace against the reference emulator.
const POWER_ON_CPSR: u32 = 0xD3;

/// An operating mode (`docs/hw/cpu-registers.md`, "Mode bits").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Non-privileged mode, `10000`.
    User,
    /// Fast interrupt, `10001`.
    Fiq,
    /// Interrupt, `10010`.
    Irq,
    /// Supervisor, `10011`.
    Supervisor,
    /// Abort, `10111`.
    Abort,
    /// Undefined instruction, `11011`.
    Undefined,
    /// Privileged User mode, `11111`.
    System,
}

impl Mode {
    /// Decodes the low five CPSR bits; `None` for a value that is no mode of this CPU,
    /// including the old 26-bit modes (`docs/hw/cpu-registers.md`).
    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits & MODE_MASK {
            0x10 => Some(Self::User),
            0x11 => Some(Self::Fiq),
            0x12 => Some(Self::Irq),
            0x13 => Some(Self::Supervisor),
            0x17 => Some(Self::Abort),
            0x1B => Some(Self::Undefined),
            0x1F => Some(Self::System),
            _ => None,
        }
    }

    /// The mode's value in the CPSR's low five bits.
    pub const fn bits(self) -> u32 {
        match self {
            Self::User => 0x10,
            Self::Fiq => 0x11,
            Self::Irq => 0x12,
            Self::Supervisor => 0x13,
            Self::Abort => 0x17,
            Self::Undefined => 0x1B,
            Self::System => 0x1F,
        }
    }

    /// The register bank this mode uses; User and System share one.
    pub fn bank(self) -> Bank {
        match self {
            Self::User | Self::System => Bank::UserSystem,
            Self::Fiq => Bank::Fiq,
            Self::Irq => Bank::Irq,
            Self::Supervisor => Bank::Supervisor,
            Self::Abort => Bank::Abort,
            Self::Undefined => Bank::Undefined,
        }
    }
}

/// A set of banked registers. The first five variants are the modes that own an SPSR, in the
/// order the JSON suite lists them; their value doubles as the SPSR index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bank {
    /// FIQ: owns R8–R14.
    Fiq = 0,
    /// Supervisor: owns R13–R14.
    Supervisor = 1,
    /// Abort: owns R13–R14.
    Abort = 2,
    /// IRQ: owns R13–R14.
    Irq = 3,
    /// Undefined: owns R13–R14.
    Undefined = 4,
    /// User and System: owns R13–R14, plus the R8–R12 every mode but FIQ shares.
    UserSystem = 5,
}

/// All registers of one CPU as plain data.
///
/// Decided in #76. `visible` holds what the current bank sees; `stored` holds, per bank, the
/// copies of R8–R14 that are not currently visible. An entry of `stored` is meaningful only
/// for the registers its bank owns (see [`Registers::owner`]); the others stay zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registers {
    visible: [u32; 16],
    stored: [[u32; 7]; 6],
    cpsr: u32,
    spsr: [u32; 5],
    bank: Bank,
}

impl Registers {
    /// Power-on state: all registers zero, Supervisor mode, interrupts disabled.
    pub fn new() -> Self {
        Self {
            visible: [0; 16],
            stored: [[0; 7]; 6],
            cpsr: POWER_ON_CPSR,
            spsr: [0; 5],
            bank: Bank::Supervisor,
        }
    }

    /// Which bank holds register `index` (8–14) for `bank`: R8–R12 are the FIQ bank's own or
    /// the User/System copy shared by every other mode; R13–R14 belong to the mode's bank.
    fn owner(bank: Bank, index: usize) -> Bank {
        match (index <= 12, bank) {
            (true, Bank::Fiq) => Bank::Fiq,
            (true, _) => Bank::UserSystem,
            (false, bank) => bank,
        }
    }

    /// The bank whose registers are currently visible.
    pub fn bank(&self) -> Bank {
        self.bank
    }

    /// Reads a register as the current mode sees it. Only the low four bits of `index` count.
    pub fn get(&self, index: usize) -> u32 {
        self.visible[index & 0xF]
    }

    /// Writes a register as the current mode sees it. Only the low four bits of `index` count.
    pub fn set(&mut self, index: usize, value: u32) {
        self.visible[index & 0xF] = value;
    }

    /// Reads register `index` (0–15) as `bank` sees it, whether or not `bank` is active.
    pub fn banked(&self, bank: Bank, index: usize) -> u32 {
        let index = index & 0xF;
        match (8..=14).contains(&index) && Self::owner(bank, index) != Self::owner(self.bank, index)
        {
            true => self.stored[Self::owner(bank, index) as usize][index - 8],
            false => self.visible[index],
        }
    }

    /// Writes register `index` (0–15) as `bank` sees it, whether or not `bank` is active.
    pub fn set_banked(&mut self, bank: Bank, index: usize, value: u32) {
        let index = index & 0xF;
        match (8..=14).contains(&index) && Self::owner(bank, index) != Self::owner(self.bank, index)
        {
            true => self.stored[Self::owner(bank, index) as usize][index - 8] = value,
            false => self.visible[index] = value,
        }
    }

    /// The CPSR exactly as last written.
    pub fn cpsr(&self) -> u32 {
        self.cpsr
    }

    /// Writes the CPSR. If the mode bits name a mode, the register banks switch to it; if they
    /// name none, the bits are kept and the banks stay as they were.
    pub fn set_cpsr(&mut self, value: u32) {
        self.cpsr = value;
        // TODO(hw-unverified): an unknown mode value leaves the bank unchanged; see
        // `docs/hw/cpu-registers.md`, "Unverified".
        if let Some(mode) = Mode::from_bits(value) {
            self.switch_bank(mode.bank());
        }
    }

    /// Whether the visible bank is the one the CPSR's mode bits name. A CPSR that names no mode
    /// leaves the bank free (`set_cpsr`). A savestate that decodes to anything else is corrupt.
    pub(crate) fn is_consistent(&self) -> bool {
        Mode::from_bits(self.cpsr).is_none_or(|mode| mode.bank() == self.bank)
    }

    /// Test helper: makes the visible bank disagree with the CPSR mode, as only a damaged
    /// savestate could.
    #[cfg(test)]
    pub(crate) fn break_bank_for_test(&mut self) {
        self.bank = Bank::Fiq;
        self.cpsr = Mode::User.bits();
    }

    /// The SPSR of `bank`, or `None` for User/System, which have none.
    // TODO(hw-unverified): what a read of the SPSR gives in User/System mode is not stated by
    // the sources (`docs/hw/cpu-registers.md`, "Unverified"); modeled as having none.
    pub fn spsr(&self, bank: Bank) -> Option<u32> {
        self.spsr.get(bank as usize).copied()
    }

    /// Writes the SPSR of `bank`; a write to User/System is ignored because they have none.
    // TODO(hw-unverified): see `Registers::spsr`.
    pub fn set_spsr(&mut self, bank: Bank, value: u32) {
        if let Some(slot) = self.spsr.get_mut(bank as usize) {
            *slot = value;
        }
    }

    /// Makes `target` the visible bank: each register whose owner changes is saved to its old
    /// owner's storage and loaded from the new owner's.
    fn switch_bank(&mut self, target: Bank) {
        for index in 8..=14 {
            let (old, new) = (Self::owner(self.bank, index), Self::owner(target, index));
            if old != new {
                self.stored[old as usize][index - 8] = self.visible[index];
                self.visible[index] = self.stored[new as usize][index - 8];
            }
        }
        self.bank = target;
    }
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests_consistent {
    use super::*;

    #[test]
    fn given_power_on_registers_when_checked_then_they_are_consistent() {
        assert!(Registers::new().is_consistent());
    }

    #[test]
    fn given_a_cpsr_written_with_irq_mode_when_checked_then_the_banks_follow_and_it_is_consistent()
    {
        let mut regs = Registers::new();
        regs.set_cpsr(0x12);
        assert!(regs.is_consistent());
    }

    #[test]
    fn given_irq_mode_bits_but_the_user_bank_when_checked_then_it_is_inconsistent() {
        let mut regs = Registers::new();
        regs.cpsr = 0x12;
        regs.bank = Bank::UserSystem;
        assert!(!regs.is_consistent());
    }

    #[test]
    fn given_mode_bits_naming_no_mode_when_checked_then_any_bank_is_consistent() {
        let mut regs = Registers::new();
        regs.cpsr = 0x00;
        regs.bank = Bank::Fiq;
        assert!(regs.is_consistent());
    }
}
