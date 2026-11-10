// SPDX-License-Identifier: Apache-2.0
//! The ARM CPU shared by both processors: `Cpu<false>` is the ARM7TDMI, `Cpu<true>` the
//! ARM946E-S. Phase 1 grows it instruction group by group (#8–#13).

mod alu;
mod arm;
mod arm_transfer;
mod armv5;
mod bus;
mod cp15;
mod decode;
mod disasm;
mod disasm_thumb;
mod exception;
mod multiply_carry;
mod psr;
mod regs;
mod shifter;
mod thumb;
mod thumb_decode;

pub use bus::Bus;
pub use cp15::{Cp15, Tcm};
pub use disasm::{ArmDisassembly, disassemble_arm};
pub use disasm_thumb::{ThumbDisassembly, disassemble_thumb};
pub use regs::{Bank, Mode, Registers};

pub(crate) use psr::FLAG_T;
pub(crate) use regs::{LR, PC, SP};
use serde::{Deserialize, Serialize};

/// Master cycles one ARM9 instruction costs (`scheduler.md`, "Cost per instruction").
// TODO(hw-unverified): ignores waitstates and multi-cycle instructions (`scheduler.md`, "Unverified").
pub const ARM9_INSTRUCTION_COST: u64 = 1;
/// Master cycles one ARM7 instruction costs (`scheduler.md`, "Cost per instruction").
// TODO(hw-unverified): ignores waitstates and multi-cycle instructions (`scheduler.md`, "Unverified").
pub const ARM7_INSTRUCTION_COST: u64 = 2;

/// One ARM core. `ARM9` selects the ARMv5TE behavior over ARMv4T at compile time.
///
/// Decided in #76.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cpu<const ARM9: bool> {
    regs: Registers,
    /// The two opcodes fetched ahead of execution: `[0]` runs next
    /// (`docs/hw/arm-branch-pipeline.md`).
    pipeline: [u32; 2],
    /// The system control coprocessor. The ARM7 has none: it traps before reaching this
    /// (`docs/hw/arm-exceptions-modes.md`, "Coprocessor instructions"), so the field stays at
    /// its power-on value there.
    cp15: Cp15,
}

impl<const ARM9: bool> Cpu<ARM9> {
    /// Whether decoded state keeps the invariants the core relies on (the bank matches the
    /// CPSR mode). A savestate that decodes to anything else is corrupt.
    pub(crate) fn is_well_formed(&self) -> bool {
        self.regs.is_consistent()
    }

    /// A CPU in its power-on state.
    pub fn new() -> Self {
        Self {
            regs: Registers::new(),
            pipeline: [0; 2],
            cp15: Cp15::new(),
        }
    }

    /// The register file.
    pub fn regs(&self) -> &Registers {
        &self.regs
    }

    /// The register file, for the host and the test harness to set up state.
    pub fn regs_mut(&mut self) -> &mut Registers {
        &mut self.regs
    }

    /// The system control coprocessor's state.
    // The debugger's CPU panel (#56) will show it.
    pub fn cp15(&self) -> &Cp15 {
        &self.cp15
    }

    /// The prefetch pipeline: the opcode that executes next, then the one after it
    /// (`docs/hw/arm-branch-pipeline.md`).
    // The debugger's CPU panel (#56) will show it.
    pub fn pipeline(&self) -> [u32; 2] {
        self.pipeline
    }

    /// Sets the prefetch pipeline, for the host and the test harness to set up state.
    pub fn set_pipeline(&mut self, pipeline: [u32; 2]) {
        self.pipeline = pipeline;
    }

    /// Starts execution at `entry` as if branched to: R15 is set and the pipeline refilled
    /// (`docs/hw/direct-boot.md`, "The pipeline"); a narrow entry point for direct boot, so
    /// `refill` itself stays private to `cpu/`.
    pub(crate) fn start_at<B: Bus>(&mut self, bus: &mut B, entry: u32) {
        self.regs.set(15, entry);
        self.refill(bus);
    }

    /// Writes a CP15 register as an MCR would, for a test to set the machine up.
    #[cfg(test)]
    pub(crate) fn write_cp15(&mut self, crn: u32, crm: u32, opc2: u32, value: u32) {
        self.cp15.write(crn, crm, opc2, value);
    }

    /// Master cycles one instruction costs this CPU, which is also what an interrupt entry costs.
    pub(crate) const INSTRUCTION_COST: u64 = match ARM9 {
        true => ARM9_INSTRUCTION_COST,
        false => ARM7_INSTRUCTION_COST,
    };

    /// Executes one instruction, in the state the CPSR's T bit selects (ARM when clear, Thumb
    /// when set). Returns its cost in master cycles (`docs/hw/scheduler.md`, "Cost per
    /// instruction").
    #[must_use]
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> u64 {
        match self.regs.cpsr() & FLAG_T != 0 {
            true => self.step_thumb(bus),
            false => self.step_arm(bus),
        }
        Self::INSTRUCTION_COST
    }
}

impl<const ARM9: bool> Default for Cpu<ARM9> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
