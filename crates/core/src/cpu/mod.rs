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
mod exception;
mod psr;
mod regs;
mod shifter;
mod thumb;
mod thumb_decode;

pub use bus::Bus;
pub use cp15::{Cp15, Tcm};
pub use disasm::{ArmDisassembly, disassemble_arm};
pub use regs::{Bank, Mode, Registers};

use psr::FLAG_T;
use serde::{Deserialize, Serialize};

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
    // TODO(phase2): direct boot (#17) fills the pipeline from the entry point.
    pub fn set_pipeline(&mut self, pipeline: [u32; 2]) {
        self.pipeline = pipeline;
    }

    /// Executes one instruction, in the state the CPSR's T bit selects (ARM when clear, Thumb
    /// when set).
    pub fn step<B: Bus>(&mut self, bus: &mut B) {
        match self.regs.cpsr() & FLAG_T != 0 {
            true => self.step_thumb(bus),
            false => self.step_arm(bus),
        }
    }
}

impl<const ARM9: bool> Default for Cpu<ARM9> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_arm;

#[cfg(test)]
mod tests_transfer;

#[cfg(test)]
mod tests_branch;

#[cfg(test)]
mod tests_exception;

#[cfg(test)]
mod tests_thumb;

#[cfg(test)]
mod tests_armv5;

#[cfg(test)]
mod tests_cp15;

#[cfg(test)]
mod tests_disasm;
