// SPDX-License-Identifier: Apache-2.0
//! Exception entry (#12): SWI, the undefined-instruction space, coprocessor instructions, the
//! abort and interrupt entries, and the vector base. Note: `docs/hw/arm-exceptions-modes.md`.

use super::cp15::UNMAPPED_READ;
use super::psr::MODE_MASK;
use super::psr::{FLAG_F, FLAG_I, FLAG_T};
use super::regs::{LR, Mode, PC};
use super::{Bus, Cpu};

/// The high vector base (`docs/hw/arm-exceptions-modes.md`, "Vector base").
const HIGH_VECTOR_BASE: u32 = 0xFFFF_0000;
/// The coprocessor number of the CP15 system control coprocessor.
const CP15: u32 = 15;
/// CPSR bits 31–28, N Z C V: what an MRC to R15 loads (`docs/hw/cp15.md`, "Access").
const CPSR_FLAGS: u32 = 0xF000_0000;

/// The exceptions a core takes (`docs/hw/arm-exceptions-modes.md`, "Vector table").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exception {
    Reset,
    Undefined,
    SoftwareInterrupt,
    PrefetchAbort,
    DataAbort,
    Irq,
    Fiq,
}

impl Exception {
    /// The vector's offset from the vector base.
    const fn vector(self) -> u32 {
        match self {
            Self::Reset => 0x00,
            Self::Undefined => 0x04,
            Self::SoftwareInterrupt => 0x08,
            Self::PrefetchAbort => 0x0C,
            Self::DataAbort => 0x10,
            Self::Irq => 0x18,
            Self::Fiq => 0x1C,
        }
    }

    /// The mode the exception enters.
    const fn mode(self) -> Mode {
        match self {
            Self::Reset | Self::SoftwareInterrupt => Mode::Supervisor,
            Self::Undefined => Mode::Undefined,
            Self::PrefetchAbort | Self::DataAbort => Mode::Abort,
            Self::Irq => Mode::Irq,
            Self::Fiq => Mode::Fiq,
        }
    }

    /// Whether the entry also sets CPSR bit F; I is set by every entry.
    const fn sets_f(self) -> bool {
        matches!(self, Self::Reset | Self::Fiq)
    }

    /// What is added to R15, as it reads while the exception is taken, to get R14
    /// (`docs/hw/arm-exceptions-modes.md`, "Return address"); `None` when the entry leaves R14
    /// and the SPSR alone.
    // TODO(hw-unverified): the Thumb column is worked out from GBATEK's statement that one return
    // sequence serves both states, and FIQ is given the IRQ value; Reset leaves R14 and the SPSR
    // alone because GBATEK is silent (`docs/hw/arm-exceptions-modes.md`, "Unverified").
    const fn link_offset(self, thumb: bool) -> Option<i32> {
        match (self, thumb) {
            (Self::Reset, _) => None,
            (Self::Undefined | Self::SoftwareInterrupt, false) => Some(-4),
            (Self::Undefined | Self::SoftwareInterrupt, true) => Some(-2),
            (Self::PrefetchAbort | Self::Irq | Self::Fiq, false) => Some(-4),
            (Self::PrefetchAbort | Self::Irq | Self::Fiq, true) => Some(0),
            (Self::DataAbort, false) => Some(0),
            (Self::DataAbort, true) => Some(4),
        }
    }
}

impl<const ARM9: bool> Cpu<ARM9> {
    /// Where the exception vectors start: 0 on the ARM7, and on the ARM9 0 or `FFFF0000h`
    /// according to bit 13 of the CP15 control register (`docs/hw/arm-exceptions-modes.md`,
    /// "Vector base"; `docs/hw/cp15.md`, "C1,C0,0 — control register").
    // The debugger's CPU panel (#56) will show it.
    pub fn vector_base(&self) -> u32 {
        match ARM9 && self.cp15.high_vectors() {
            true => HIGH_VECTOR_BASE,
            false => 0,
        }
    }

    /// Enters `exception` without refilling the pipeline: R14 and the SPSR of the entered mode
    /// get the return address and the old CPSR, the CPSR takes the new mode with T clear, I set
    /// and F set for Reset and FIQ, and R15 becomes the vector. The caller refills from R15.
    pub(super) fn take_exception(&mut self, exception: Exception) {
        let old = self.regs.cpsr();
        let thumb = old & FLAG_T != 0;
        let link = exception
            .link_offset(thumb)
            .map(|offset| self.regs.get(PC).wrapping_add_signed(offset));
        let mode = exception.mode();
        let f = match exception.sets_f() {
            true => FLAG_F,
            false => 0,
        };
        // The CPSR first, so that the bank switch has happened when R14 is written.
        self.regs
            .set_cpsr((old & !(MODE_MASK | FLAG_T)) | mode.bits() | FLAG_I | f);
        if let Some(link) = link {
            self.regs.set_spsr(mode.bank(), old);
            self.regs.set(LR, link);
        }
        self.regs
            .set(PC, self.vector_base().wrapping_add(exception.vector()));
    }

    /// The handler form of an exception entry: it wrote the PC, so the step refills.
    pub(super) fn enter(&mut self, exception: Exception) -> bool {
        self.take_exception(exception);
        true
    }

    /// A coprocessor instruction whose condition holds: true if it entered the Undefined
    /// exception (`docs/hw/arm-exceptions-modes.md`, "Coprocessor instructions").
    // TODO(hw-unverified): every coprocessor instruction traps on the ARM7, whatever the number,
    // which only the suite shows; on the ARM9 that CDP, LDC and STC to CP15 and everything sent to
    // another coprocessor trap is inferred from it (`docs/hw/arm-exceptions-modes.md`, "Unverified").
    // TODO(hw-unverified): MCR and MRC to CP15 run in every mode, User included
    // (`docs/hw/cp15.md`, "Unverified").
    // MCRR and MRRC (ARMv5TE) are stepped past on the ARM9.
    pub(super) fn coprocessor(&mut self, word: u32) -> bool {
        let number = (word >> 8) & 0xF;
        // Bits 27–24 = 1110 with bit 4 set is MCR or MRC; bits 27–21 = 1100010 is MCRR or MRRC.
        let register_transfer = word >> 24 & 0xF == 0xE && word >> 4 & 1 == 1;
        let double_transfer = word >> 21 & 0x7F == 0x62;
        match (
            ARM9 && register_transfer && number == CP15,
            ARM9 && double_transfer,
        ) {
            (true, _) => {
                self.cp15_transfer(word);
                false
            }
            (false, true) => false,
            (false, false) => self.enter(Exception::Undefined),
        }
    }

    /// An MCR or MRC to CP15 on the ARM9 (`docs/hw/cp15.md`, "Access"). Opc1 other than 0 selects
    /// no register.
    // TODO(hw-unverified): an opc1 other than 0 reads 0 and ignores the write, like any unmapped
    // register; and Rd = R15 follows the general coprocessor rules, which GBATEK's CP15 overview
    // contradicts (`docs/hw/cp15.md`, "Unverified").
    fn cp15_transfer(&mut self, word: u32) {
        let opc1 = (word >> 21) & 0x7;
        let load = word >> 20 & 1 == 1;
        let crn = (word >> 16) & 0xF;
        let rd = ((word >> 12) & 0xF) as usize;
        let opc2 = (word >> 5) & 0x7;
        let crm = word & 0xF;
        match (load, opc1) {
            (true, 0) => {
                let value = self.cp15.read(crn, crm, opc2);
                self.write_transfer_target(rd, value);
            }
            (true, _) => self.write_transfer_target(rd, UNMAPPED_READ),
            // An MCR sends R15 as a store does: the instruction address + 12.
            (false, 0) => {
                let value = self.store_operand(rd as u32);
                self.cp15.write(crn, crm, opc2, value);
            }
            (false, _) => {}
        }
    }

    /// Delivers an MRC's value: to the register, or for R15 only its top four bits, to the flags.
    // TODO(hw-unverified): see `Cpu::cp15_transfer`.
    fn write_transfer_target(&mut self, rd: usize, value: u32) {
        match rd == PC {
            true => {
                let cpsr = self.regs.cpsr();
                self.regs
                    .set_cpsr((cpsr & !CPSR_FLAGS) | (value & CPSR_FLAGS));
            }
            false => self.regs.set(rd, value),
        }
    }

    /// Offers pending interrupt lines between two instructions: a FIQ is taken if F is clear, an
    /// IRQ if I is clear, and FIQ has the higher priority. Returns the entry's cost in master cycles if
    /// one was taken, which is an instruction's (`docs/hw/interrupts.md`, "Behavior"), else `None`.
    /// Call it between two instructions, when R15 reads the next instruction's address + 8 (ARM) or
    /// + 4 (Thumb).
    // The interrupt controller (`docs/hw/interrupts.md`) drives the IRQ line; nothing drives FIQ.
    pub fn take_interrupt<B: Bus>(&mut self, bus: &mut B, irq: bool, fiq: bool) -> Option<u64> {
        let cpsr = self.regs.cpsr();
        let exception = match (fiq && cpsr & FLAG_F == 0, irq && cpsr & FLAG_I == 0) {
            (true, _) => Exception::Fiq,
            (false, true) => Exception::Irq,
            (false, false) => return None,
        };
        self.take_exception(exception);
        self.refill(bus);
        Some(Self::INSTRUCTION_COST)
    }

    /// Takes a prefetch abort for the instruction that is executing (R15 = its address + 8).
    // TODO(phase8): the protection unit will raise it; BKPT enters the abort itself.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "nothing calls it until the protection unit checks accesses (phase 8)"
        )
    )]
    pub(crate) fn prefetch_abort<B: Bus>(&mut self, bus: &mut B) {
        self.take_exception(Exception::PrefetchAbort);
        self.refill(bus);
    }

    /// Takes a data abort for the instruction that is executing (R15 = its address + 8).
    // TODO(phase8): nothing raises it yet: the protection unit does not check accesses.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "nothing calls it until the protection unit checks accesses (phase 8)"
        )
    )]
    pub(crate) fn data_abort<B: Bus>(&mut self, bus: &mut B) {
        self.take_exception(Exception::DataAbort);
        self.refill(bus);
    }

    /// Resets the core: Supervisor mode, I and F set, ARM state, and the pipeline refilled from
    /// the reset vector. R14, the SPSR and the other registers are left as they were.
    // TODO(hw-unverified): see `Exception::link_offset`; CP15, the vector base included, is left
    // alone, as GBATEK says nothing about a reset changing it (`docs/hw/cp15.md`, "Unverified").
    pub fn reset<B: Bus>(&mut self, bus: &mut B) {
        self.take_exception(Exception::Reset);
        self.refill(bus);
    }
}
