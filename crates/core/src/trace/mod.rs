// SPDX-License-Identifier: Apache-2.0
//! The instruction trace: [`TraceRecord`], its text form, and the [`TraceSink`] a traced run
//! hands each record to (`docs/contributing/trace-oracle.md`, "The trace format").
//!
//! The format lives in the core because the runner writes it and the diff tool reads it, and the
//! runner must not depend on the diff tool. It is pure: types, encode (`Display`) and parse, no
//! IO. A sink receives records by static dispatch; the debugger's trace panel (#56-#59) can be one.

use crate::bus::{BIOS7_BASE, BIOS7_SIZE, BIOS9_BASE, BIOS9_SIZE};
use crate::cpu::Cpu;
use core::fmt;
use core::ops::Range;
use thiserror::Error;

/// The first line of a trace: the format name and its version. A reader refuses another version.
pub const HEADER: &str = "ninge-trace 1";

/// CPSR bit 5, the Thumb state bit.
const FLAG_T: u32 = 1 << 5;
/// Registers in a record, r0 through r15.
const REGISTERS: usize = 16;
/// Columns of a line: cpu, step, pc, opcode, sixteen registers, cpsr.
const COLUMNS: usize = 4 + REGISTERS + 1;
/// Hex digits of every number column but the opcode and the cpu.
const WIDTH: usize = 8;
/// Hex digits of a Thumb opcode.
const THUMB_WIDTH: usize = 4;
/// R15 reads this far past the instruction in ARM state, and half that in Thumb state
/// (`docs/hw/arm-branch-pipeline.md`).
const ARM_AHEAD: u32 = 8;
const THUMB_AHEAD: u32 = 4;

/// Which CPU executed an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TraceCpu {
    /// The ARM9, `9` in the text form.
    Arm9,
    /// The ARM7, `7` in the text form.
    Arm7,
}

/// The opcode of an executed instruction, in the width of its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opcode {
    /// A 32-bit ARM opcode: 8 hex digits.
    Arm(u32),
    /// A 16-bit Thumb opcode: 4 hex digits.
    Thumb(u16),
}

/// One executed instruction: the registers **before** it ran, as the current mode sees them.
/// `regs[15]` is the address of the instruction itself, not the pipeline value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceRecord {
    /// The CPU that ran it.
    pub cpu: TraceCpu,
    /// How many instructions that CPU had recorded before this one.
    pub step: u32,
    /// The opcode at `regs[15]`.
    pub opcode: Opcode,
    /// r0 through r15 before the instruction.
    pub regs: [u32; REGISTERS],
    /// The CPSR before the instruction; its T bit says which [`Opcode`] width the line has.
    pub cpsr: u32,
}

impl TraceRecord {
    /// The address of the instruction, which is `regs[15]`.
    pub fn pc(&self) -> u32 {
        self.regs[REGISTERS - 1]
    }
}

impl fmt::Display for TraceRecord {
    /// The line of the text form, without a newline.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cpu = match self.cpu {
            TraceCpu::Arm9 => '9',
            TraceCpu::Arm7 => '7',
        };
        write!(f, "{cpu} {:08x} {:08x} ", self.step, self.pc())?;
        match self.opcode {
            Opcode::Arm(word) => write!(f, "{word:08x}")?,
            Opcode::Thumb(half) => write!(f, "{half:04x}")?,
        }
        self.regs.iter().try_for_each(|r| write!(f, " {r:08x}"))?;
        write!(f, " {:08x}", self.cpsr)
    }
}

/// Why one line did not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum LineError {
    /// The line does not have the 21 columns.
    #[error("expected {COLUMNS} columns")]
    Columns,
    /// The cpu column is neither `9` nor `7`.
    #[error("the cpu column is not 9 or 7")]
    Cpu,
    /// A number column is not lowercase hex of its fixed width.
    #[error("column `{0}` is not fixed-width lowercase hex")]
    Number(&'static str),
    /// The pc column differs from r15.
    #[error("the pc column differs from r15")]
    Pc,
    /// The opcode width disagrees with the CPSR's Thumb bit.
    #[error("the opcode width disagrees with the CPSR Thumb bit")]
    State,
}

/// Why a whole trace did not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TraceError {
    /// The first line is not [`HEADER`], or the text is empty.
    #[error("the first line is not `{HEADER}`")]
    Header,
    /// A line is malformed; `line` counts from 1.
    #[error("line {line}: {error}")]
    Line {
        /// The 1-based line number.
        line: usize,
        /// What is wrong with it.
        error: LineError,
    },
}

/// Parses one fixed-width lowercase hex column.
fn hex(token: &str, name: &'static str, width: usize) -> Result<u32, LineError> {
    let digits = token
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    match digits && token.len() == width {
        true => u32::from_str_radix(token, 16).map_err(|_| LineError::Number(name)),
        false => Err(LineError::Number(name)),
    }
}

/// Parses one record line of the text form.
///
/// # Errors
///
/// A [`LineError`] for a wrong column count, a column that is not fixed-width lowercase hex, a
/// pc column that is not r15, or an opcode width that disagrees with the CPSR's Thumb bit.
pub fn parse_line(line: &str) -> Result<TraceRecord, LineError> {
    let tokens: Vec<&str> = line.split(' ').collect();
    let [cpu, step, pc, opcode, rest @ ..] = tokens.as_slice() else {
        return Err(LineError::Columns);
    };
    if tokens.len() != COLUMNS {
        return Err(LineError::Columns);
    }
    let cpu = match *cpu {
        "9" => TraceCpu::Arm9,
        "7" => TraceCpu::Arm7,
        _ => return Err(LineError::Cpu),
    };
    let step = hex(step, "step", WIDTH)?;
    let pc = hex(pc, "pc", WIDTH)?;
    let mut regs = [0; REGISTERS];
    for (slot, token) in regs.iter_mut().zip(rest) {
        *slot = hex(token, "register", WIDTH)?;
    }
    let cpsr = hex(rest[REGISTERS], "cpsr", WIDTH)?;
    if pc != regs[REGISTERS - 1] {
        return Err(LineError::Pc);
    }
    let thumb = cpsr & FLAG_T != 0;
    let opcode = match (thumb, opcode.len()) {
        (false, WIDTH) => Opcode::Arm(hex(opcode, "opcode", WIDTH)?),
        (true, THUMB_WIDTH) => {
            // The width check above bounds the value to 16 bits.
            Opcode::Thumb(hex(opcode, "opcode", THUMB_WIDTH)? as u16)
        }
        _ => return Err(LineError::State),
    };
    Ok(TraceRecord {
        cpu,
        step,
        opcode,
        regs,
        cpsr,
    })
}

/// Parses a whole trace: the [`HEADER`] line, then record lines; `#` comments and blank lines are
/// skipped.
///
/// # Errors
///
/// [`TraceError::Header`] if the first line is not the header, [`TraceError::Line`] for the first
/// malformed record.
pub fn parse_trace(text: &str) -> Result<Vec<TraceRecord>, TraceError> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, HEADER)) => {}
        _ => return Err(TraceError::Header),
    }
    lines
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(index, line)| {
            parse_line(line).map_err(|error| TraceError::Line {
                line: index + 1,
                error,
            })
        })
        .collect()
}

/// Where a traced run hands each record.
pub trait TraceSink {
    /// Receives one executed instruction.
    fn record(&mut self, record: &TraceRecord);
}

/// A sink that drops every record: what a run that wants only the traced loop's behavior uses.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullSink;

impl TraceSink for NullSink {
    fn record(&mut self, _record: &TraceRecord) {}
}

impl TraceSink for Vec<TraceRecord> {
    fn record(&mut self, record: &TraceRecord) {
        self.push(*record);
    }
}

/// Which CPUs a [`Tracer`] records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceCpus {
    /// Only the ARM9.
    Arm9,
    /// Only the ARM7.
    Arm7,
    /// Both.
    Both,
}

/// What a traced run carries: the sink, which CPUs to record, and each CPU's step counter. The
/// counters live here, not in the machine, so a trace can span several frames and the machine's
/// savestate does not change.
#[derive(Clone, Debug)]
pub struct Tracer<S> {
    sink: S,
    cpus: TraceCpus,
    steps: [u32; 2],
}

impl<S: TraceSink> Tracer<S> {
    /// A tracer that records `cpus` into `sink`, counting steps from 0.
    pub fn new(sink: S, cpus: TraceCpus) -> Self {
        Self {
            sink,
            cpus,
            steps: [0; 2],
        }
    }

    /// The sink, to read what was recorded.
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// The sink, by value.
    pub fn into_sink(self) -> S {
        self.sink
    }

    /// How many instructions of `cpu` were recorded.
    pub fn steps(&self, cpu: TraceCpu) -> u32 {
        self.steps[cpu as usize]
    }

    /// Records the instruction at the head of `cpu`'s pipeline, before it executes. No record for
    /// a CPU the tracer skips, or for an address in that CPU's BIOS area: BIOS execution leaves no
    /// line on either side (`trace-oracle.md`, "BIOS collapsing").
    pub(crate) fn record<const ARM9: bool>(&mut self, cpu: &Cpu<ARM9>) {
        let id = if ARM9 { TraceCpu::Arm9 } else { TraceCpu::Arm7 };
        let wanted = matches!(
            (self.cpus, id),
            (TraceCpus::Both, _)
                | (TraceCpus::Arm9, TraceCpu::Arm9)
                | (TraceCpus::Arm7, TraceCpu::Arm7)
        );
        let regs = cpu.regs();
        let cpsr = regs.cpsr();
        let thumb = cpsr & FLAG_T != 0;
        // R15 reads ahead of the instruction by the pipeline depth of its state.
        let ahead = if thumb { THUMB_AHEAD } else { ARM_AHEAD };
        let pc = regs.get(REGISTERS - 1).wrapping_sub(ahead);
        if !wanted || in_bios::<ARM9>(pc) {
            return;
        }
        let word = cpu.pipeline()[0];
        let opcode = match thumb {
            // A Thumb opcode sits in the pipeline's low halfword.
            true => Opcode::Thumb(word as u16),
            false => Opcode::Arm(word),
        };
        let mut all = std::array::from_fn(|i| regs.get(i));
        all[REGISTERS - 1] = pc;
        let record = TraceRecord {
            cpu: id,
            step: self.steps[id as usize],
            opcode,
            regs: all,
            cpsr,
        };
        self.sink.record(&record);
        self.steps[id as usize] = self.steps[id as usize].wrapping_add(1);
    }
}

impl TraceCpu {
    /// The addresses of this CPU's BIOS area (`docs/hw/hle-swi.md`, "The BIOS area"): execution
    /// there leaves no trace line, on either side of a comparison. Defined once here so the
    /// oracle adapter and the traced run agree.
    pub fn bios_area(self) -> Range<u32> {
        let (base, size) = match self {
            Self::Arm9 => (BIOS9_BASE, BIOS9_SIZE),
            Self::Arm7 => (BIOS7_BASE, BIOS7_SIZE),
        };
        base..base + size
    }
}

/// Whether `pc` lies in CPU `ARM9`'s BIOS area.
fn in_bios<const ARM9: bool>(pc: u32) -> bool {
    let cpu = if ARM9 { TraceCpu::Arm9 } else { TraceCpu::Arm7 };
    cpu.bios_area().contains(&pc)
}

#[cfg(test)]
mod tests;
