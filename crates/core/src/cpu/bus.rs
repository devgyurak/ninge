// SPDX-License-Identifier: Apache-2.0
//! The memory interface the CPU is generic over.

/// What a CPU reads and writes through. The CPU is monomorphized over its bus, so there is no
/// dynamic dispatch on the hot path (AGENTS.md §2 rule 3).
///
/// Code fetches and data accesses are separate methods: the ARM9 has separate instruction and
/// data paths, and the JSON suite harness treats them differently. Every method takes
/// `&mut self` so a bus can log or step its own state without interior mutability. Addresses
/// are not checked for alignment here; what the hardware does with an odd address belongs to
/// the instruction semantics (#8–#13).
///
/// Decided in #76.
pub trait Bus {
    /// Fetches a 32-bit ARM opcode from `addr`.
    fn fetch32(&mut self, addr: u32) -> u32;
    /// Fetches a 16-bit Thumb opcode from `addr`.
    fn fetch16(&mut self, addr: u32) -> u16;
    /// Reads a byte of data.
    fn read8(&mut self, addr: u32) -> u8;
    /// Reads a halfword of data.
    fn read16(&mut self, addr: u32) -> u16;
    /// Reads a word of data.
    fn read32(&mut self, addr: u32) -> u32;
    /// Writes a byte of data.
    fn write8(&mut self, addr: u32, value: u8);
    /// Writes a halfword of data.
    fn write16(&mut self, addr: u32, value: u16);
    /// Writes a word of data.
    fn write32(&mut self, addr: u32, value: u32);
}
