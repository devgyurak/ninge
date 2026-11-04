// SPDX-License-Identifier: Apache-2.0
//! The ARM CPU shared by both processors: `Cpu<false>` is the ARM7TDMI, `Cpu<true>` the
//! ARM946E-S. Phase 1 grows it instruction group by group (#8–#13).

mod alu;
mod arm;
mod arm_transfer;
mod bus;
mod decode;
mod psr;
mod regs;
mod shifter;

pub use bus::Bus;
pub use regs::{Bank, Mode, Registers};

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
}

impl<const ARM9: bool> Cpu<ARM9> {
    /// A CPU in its power-on state.
    pub fn new() -> Self {
        Self {
            regs: Registers::new(),
            pipeline: [0; 2],
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

    /// Executes one instruction.
    pub fn step<B: Bus>(&mut self, bus: &mut B) {
        self.step_arm(bus);
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
