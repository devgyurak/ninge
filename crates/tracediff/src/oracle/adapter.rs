// SPDX-License-Identifier: Apache-2.0
//! The oracle adapter: one CPU of a stub as a stream of [`TraceRecord`]s.
//!
//! Execution inside the CPU's BIOS area is collapsed: the adapter steps until the PC leaves the
//! area and records nothing, and it never reads memory there, so no BIOS byte leaves the oracle
//! (`docs/contributing/trace-oracle.md`, "BIOS collapsing"). ninge's HLE BIOS executes no
//! instruction in the area, which is also what makes the two streams comparable.

use std::fmt;
use std::io::{Read, Write};

use ninge_core::trace::{Opcode, TraceCpu, TraceRecord};

use super::client::{GdbClient, Stop};
use crate::rsp::RspError;

/// CPSR bit 5, the Thumb state bit.
const FLAG_T: u32 = 1 << 5;
/// Registers r0 through r15, the first words of the `g` block.
const GPRS: usize = 16;
/// The most steps one BIOS call may take before the adapter gives up: a BIOS that never returns
/// would otherwise hang the tool.
const MAX_BIOS_STEPS: u32 = 1 << 24;

/// Where the registers sit in the stub's `g` packet, in 32-bit words. r0 through r15 are the
/// first sixteen; the stub decides where the CPSR is.
// TODO(hw-unverified): the default is the common GDB ARM layout, to be confirmed against the
// stub by the step-0 probe (`trace-oracle.md`, "Oracle probe findings").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterLayout {
    /// The word index of the CPSR in the `g` packet.
    pub cpsr: usize,
}

impl Default for RegisterLayout {
    fn default() -> Self {
        Self { cpsr: GPRS }
    }
}

/// Why the adapter could not produce a record.
#[derive(Debug)]
pub enum AdapterError {
    /// The stub or the stream failed.
    Rsp(RspError),
    /// The `g` packet is too short for the layout.
    RegisterCount {
        /// How many words the packet had.
        words: usize,
        /// How many the layout needs.
        needed: usize,
    },
    /// The CPU ran [`MAX_BIOS_STEPS`] steps in its BIOS area without leaving it.
    StuckInBios,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rsp(error) => write!(f, "{error}"),
            Self::RegisterCount { words, needed } => {
                write!(
                    f,
                    "the stub sent {words} register words, the layout needs {needed}"
                )
            }
            Self::StuckInBios => write!(
                f,
                "the CPU stayed in its BIOS area for {MAX_BIOS_STEPS} steps"
            ),
        }
    }
}

impl std::error::Error for AdapterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rsp(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RspError> for AdapterError {
    fn from(error: RspError) -> Self {
        Self::Rsp(error)
    }
}

/// One CPU of the oracle, stepped one recorded instruction at a time.
#[derive(Debug)]
pub struct OracleCpu<T> {
    client: GdbClient<T>,
    cpu: TraceCpu,
    layout: RegisterLayout,
    step: u32,
    exited: bool,
    bios_limit: u32,
}

impl<T: Read + Write> OracleCpu<T> {
    /// Wraps `client`, a connection to `cpu`'s stub. The client must have been built with `cpu`'s
    /// BIOS area forbidden; [`OracleCpu::connect`] does that.
    pub fn new(client: GdbClient<T>, cpu: TraceCpu, layout: RegisterLayout) -> Self {
        Self {
            client,
            cpu,
            layout,
            step: 0,
            exited: false,
            bios_limit: MAX_BIOS_STEPS,
        }
    }

    /// Wraps `stream` in a client that forbids reads of `cpu`'s BIOS area.
    pub fn connect(stream: T, cpu: TraceCpu, layout: RegisterLayout) -> Self {
        Self::new(GdbClient::new(stream, vec![cpu.bios_area()]), cpu, layout)
    }

    /// Sets how many steps one BIOS call may take before [`AdapterError::StuckInBios`]; the
    /// default is [`MAX_BIOS_STEPS`]. A test lowers it.
    #[must_use]
    pub fn with_bios_limit(mut self, steps: u32) -> Self {
        self.bios_limit = steps;
        self
    }

    /// The connection, for breakpoints and resets.
    pub fn client(&mut self) -> &mut GdbClient<T> {
        &mut self.client
    }

    /// How many records this CPU has produced.
    pub fn steps(&self) -> u32 {
        self.step
    }

    /// The next instruction the CPU executes, as a record of the state before it, and steps the
    /// oracle past it. `None` once the target has exited.
    ///
    /// # Errors
    ///
    /// An [`AdapterError`] for a stream or stub failure, a `g` packet shorter than the layout, or
    /// a BIOS call that never returns.
    pub fn next_record(&mut self) -> Result<Option<TraceRecord>, AdapterError> {
        let bios = self.cpu.bios_area();
        let mut in_bios = 0;
        while !self.exited {
            let words = self.client.registers()?;
            // The registers r0-r15 are always read, so a layout with the CPSR among them still needs 16.
            let needed = self.layout.cpsr.saturating_add(1).max(GPRS);
            if words.len() < needed {
                return Err(AdapterError::RegisterCount {
                    words: words.len(),
                    needed,
                });
            }
            let pc = words[GPRS - 1];
            if bios.contains(&pc) {
                // Collapsed: step without reading the area.
                in_bios += 1;
                if in_bios > self.bios_limit {
                    return Err(AdapterError::StuckInBios);
                }
                self.exited = self.client.step()? == Stop::Exited;
                continue;
            }
            let cpsr = words[self.layout.cpsr];
            let opcode = match cpsr & FLAG_T != 0 {
                true => {
                    let bytes = self.client.read_memory(pc, 2)?;
                    Opcode::Thumb(u16::from_le_bytes([bytes[0], bytes[1]]))
                }
                false => {
                    let bytes = self.client.read_memory(pc, 4)?;
                    Opcode::Arm(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                }
            };
            let mut regs = [0; GPRS];
            regs.copy_from_slice(&words[..GPRS]);
            let record = TraceRecord {
                cpu: self.cpu,
                step: self.step,
                opcode,
                regs,
                cpsr,
            };
            self.step = self.step.wrapping_add(1);
            self.exited = self.client.step()? == Stop::Exited;
            return Ok(Some(record));
        }
        Ok(None)
    }
}
