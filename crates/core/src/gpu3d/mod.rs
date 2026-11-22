// SPDX-License-Identifier: Apache-2.0
//! The 3D engine stub: DISP3DCNT, the geometry command decoder (GXFIFO and the command ports), the
//! matrix mode and stack pointers, GXSTAT and the FIFO IRQ condition (`docs/hw/gx-stub.md`).
//!
//! Nothing is drawn and no matrix is computed. The model is "instant drain": a command is consumed
//! the moment its last parameter arrives, so the FIFO and the PIPE never hold an entry and GXSTAT
//! always reads empty. A program that waits for the FIFO to drain therefore finishes waiting.
//!
//! Only the ARM9 sees these registers, so [`Gpu3d`] is a plain type implementing `Device<true>`
//! (`AGENTS.md` §4, "Per-CPU devices"): the "seen by one CPU" class. It registers directly with its
//! own port, with no `io/` entrance, as `gpu2d` does (`AGENTS.md` §2, "Placement").
// TODO(phase7): the geometry engine (#48) replaces the instant drain with the real FIFO, the PIPE,
// the command timings, the matrices and the swap at VBlank; the rendering engine is #49-#52
// (`gx-stub.md`, "Scope").

use crate::io::device::{Checked, Cx, Device, Effect, Width, read_bytes, write_bytes};
use crate::io::irq::{GXFIFO_IRQ, Requests};
use serde::{Deserialize, Serialize};

/// DISP3DCNT, the first register of the device, at `0400_0060h` (`gx-stub.md`, "Registers").
pub const DISP3DCNT_ADDR: u32 = 0x0400_0060;
/// Bytes of the DISP3DCNT block: the halfword and two unused bytes, to stay on the 4-byte grid.
pub const DISP3DCNT_BYTES: u32 = 4;
/// RDLINES_COUNT, the first register of the rendering block, at `0400_0320h`.
pub const RENDER_ADDR: u32 = 0x0400_0320;
/// Bytes of the rendering block, `0400_0320h`-`0400_03BFh`.
pub const RENDER_BYTES: u32 = 0xA0;
/// GXFIFO, the first register of the geometry block, at `0400_0400h`.
pub const GEOMETRY_ADDR: u32 = 0x0400_0400;
/// Bytes of the geometry block, up to and including VECMTX_RESULT (`0400_06A3h`).
pub const GEOMETRY_BYTES: u32 = 0x2A4;
/// GXSTAT, at `0400_0600h`.
pub const GXSTAT_ADDR: u32 = GEOMETRY_ADDR + GXSTAT_REL;

/// Where the rendering block starts, in the device's offsets ([`DISP3DCNT_ADDR`] is offset 0).
const RENDER_OFFSET: u32 = RENDER_ADDR - DISP3DCNT_ADDR;
/// Where the geometry block starts, in the device's offsets.
const GEOMETRY_OFFSET: u32 = GEOMETRY_ADDR - DISP3DCNT_ADDR;

// Offsets inside the geometry block.
/// GXFIFO and its mirrors: 16 words, `0400_0400h`-`0400_043Fh`.
const GXFIFO_BYTES: u32 = 0x40;
/// The command ports start here, `0400_0440h`; the port of command `id` is `id * 4` from the block.
const PORTS_REL: u32 = 0x40;
/// The first byte past the ports, `0400_0600h`: GXSTAT.
const GXSTAT_REL: u32 = 0x200;
/// The bytes of GXSTAT.
const GXSTAT_BYTES: u32 = 4;
/// Bytes of a port: one word, so the ID is the offset from the block divided by this.
const PORT_SHIFT: u32 = 2;

/// DISP3DCNT: bits 0-11 and 14 are plain read/write; bits 12-13 are status flags the stub never
/// raises, so they are not stored (`gx-stub.md`, "DISP3DCNT bits").
pub const DISP3DCNT_MASK: u16 = 0x4FFF;
/// RDLINES_COUNT as the stub reads it: the largest value the field holds, as for a renderer that
/// keeps its line buffer full.
// TODO(hw-unverified): the real value depends on the renderer (`gx-stub.md`, "Unverified").
const RDLINES_COUNT: u8 = 46;

/// GXSTAT bit 1: the BOX_TEST result, 1 = parts or all inside the view volume.
const GXSTAT_BOX_RESULT: u32 = 1 << 1;
/// GXSTAT bits 8-12: the low 5 bits of the coordinate stack pointer.
const GXSTAT_COORD_SHIFT: u32 = 8;
const GXSTAT_COORD_MASK: u8 = 0x1F;
/// GXSTAT bit 13: the projection stack pointer.
const GXSTAT_PROJECTION_SHIFT: u32 = 13;
/// GXSTAT bit 15: the stack error flag, write 1 to clear.
const GXSTAT_ERROR: u32 = 1 << 15;
/// GXSTAT bit 25: the FIFO is less than half full. The stub's FIFO is never occupied.
const GXSTAT_FIFO_HALF: u32 = 1 << 25;
/// GXSTAT bit 26: the FIFO is empty.
const GXSTAT_FIFO_EMPTY: u32 = 1 << 26;
/// GXSTAT bits 30-31: the FIFO IRQ mode.
const GXSTAT_IRQ_SHIFT: u32 = 30;

/// The FIFO IRQ modes (`gx-stub.md`, "GXSTAT and the FIFO IRQ"). Mode 3 is reserved and raises
/// nothing.
const IRQ_HALF_FULL: u8 = 1;
const IRQ_EMPTY: u8 = 2;

/// The matrix stack pointers and entry counts (`gx-stub.md`, "Matrix mode and the stack pointers").
/// The coordinate pointer is 6 bits and its stack has 31 valid entries; the projection and texture
/// pointers are 1 bit with one valid entry.
const COORD_POINTER_MASK: u8 = 0x3F;
const COORD_VALID_ENTRIES: u8 = 31;
const SMALL_POINTER_MASK: u8 = 1;
const SMALL_VALID_ENTRIES: u8 = 1;
/// MTX_POP's offset is bits 0-5 as a signed 6-bit value; STORE and RESTORE take bits 0-4.
const POP_OFFSET_BITS: u32 = 6;
const STORE_ADDRESS_MASK: u32 = 0x1F;
/// MTX_MODE keeps bits 0-1.
const MTX_MODE_MASK: u32 = 0b11;
const MODE_PROJECTION: u8 = 0;
const MODE_TEXTURE: u8 = 3;

/// The command IDs the stub gives an effect to; the others are consumed and dropped.
const CMD_MTX_MODE: u8 = 0x10;
const CMD_MTX_PUSH: u8 = 0x11;
const CMD_MTX_POP: u8 = 0x12;
const CMD_MTX_STORE: u8 = 0x13;
const CMD_MTX_RESTORE: u8 = 0x14;
const CMD_BOX_TEST: u8 = 0x70;

/// How many parameter words follow command `id`, or `None` for an ID that is not a command
/// (`gx-stub.md`, "The command set"). `00h` (NOP) is not a command either: it has no port.
// TODO(hw-unverified): 34h (SHININESS) is valid with 32 parameters, as GBATEK's table has it, though
// its sentence on invalid commands stops at 33h (`gx-stub.md`, "Unverified").
const fn parameter_count(id: u8) -> Option<u8> {
    Some(match id {
        0x10 | 0x12..=0x14 => 1,
        0x11 | 0x15 | 0x41 => 0,
        0x16 | 0x18 => 16,
        0x17 | 0x19 => 12,
        0x1A => 9,
        0x1B | 0x1C => 3,
        0x20..=0x22 | 0x24..=0x2B => 1,
        0x23 => 2,
        0x30..=0x33 => 1,
        0x34 => 32,
        0x40 | 0x50 | 0x60 | 0x72 => 1,
        0x70 => 3,
        0x71 => 2,
        _ => return None,
    })
}

/// The stub's 3D engine registers and command decoder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu3d {
    disp3dcnt: u16,
    /// MTX_MODE, 0-3.
    mtx_mode: u8,
    /// The 1-bit projection stack pointer.
    projection_sp: u8,
    /// The 6-bit pointer the coordinate and directional stacks share.
    coordinate_sp: u8,
    /// The 1-bit texture stack pointer.
    texture_sp: u8,
    /// GXSTAT bit 15.
    stack_error: bool,
    /// GXSTAT bits 30-31.
    irq_mode: u8,
    /// GXSTAT bit 1, 0 until a BOX_TEST has run.
    box_result: bool,
    /// The command IDs of the word being decoded that have not run yet, the next in the low byte.
    /// A byte of 0 is NOP and so is everything above the last non-zero byte: 0 means none left.
    pending: u32,
    /// The command waiting for parameters, valid while `owed` is above 0.
    command: u8,
    /// How many parameter words `command` still needs.
    owed: u8,
}

impl Gpu3d {
    /// The registers at power-on: every pointer 0, the error flag clear, the FIFO IRQ off.
    // TODO(hw-unverified): the reset values are the stack pointers' 0 from GBATEK and what an empty
    // FIFO reads; nothing more is known (`gx-stub.md`, "Unverified").
    pub fn new() -> Self {
        Self::default()
    }

    /// DISP3DCNT as stored.
    pub const fn disp3dcnt(&self) -> u16 {
        self.disp3dcnt
    }

    /// MTX_MODE, 0-3.
    pub const fn matrix_mode(&self) -> u8 {
        self.mtx_mode
    }

    /// The projection stack pointer, 0-1.
    pub const fn projection_pointer(&self) -> u8 {
        self.projection_sp
    }

    /// The pointer of the coordinate and directional stacks, 0-63.
    pub const fn coordinate_pointer(&self) -> u8 {
        self.coordinate_sp
    }

    /// The texture stack pointer, 0-1.
    pub const fn texture_pointer(&self) -> u8 {
        self.texture_sp
    }

    /// GXSTAT bit 15, the stack error flag.
    pub const fn stack_error(&self) -> bool {
        self.stack_error
    }

    /// GXSTAT bits 30-31, the FIFO IRQ mode.
    pub const fn irq_mode(&self) -> u8 {
        self.irq_mode
    }

    /// The IF bits the FIFO IRQ holds up: bit 21 while the mode is 1 (less than half full) or 2
    /// (empty), a condition the stub's always-empty FIFO keeps true. `Io` re-raises it after
    /// every write that could make an acknowledge stick, so an acknowledge has no effect while the
    /// condition holds (`gx-stub.md`, "GXSTAT and the FIFO IRQ").
    // TODO(hw-unverified): mode 3 (reserved) raises nothing (`gx-stub.md`, "Unverified").
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "`Io` registers the device and calls this in the wiring PR"
        )
    )]
    pub(crate) fn fifo_irq_asserted(&self) -> Requests {
        match self.irq_mode {
            IRQ_HALF_FULL | IRQ_EMPTY => Requests::for_cpu::<true>(GXFIFO_IRQ),
            _ => Requests::default(),
        }
    }

    /// GXSTAT as a read sees it (`gx-stub.md`, "GXSTAT bits").
    fn gxstat(&self) -> u32 {
        let coordinate = u32::from(self.coordinate_sp & GXSTAT_COORD_MASK) << GXSTAT_COORD_SHIFT;
        let projection = u32::from(self.projection_sp) << GXSTAT_PROJECTION_SHIFT;
        let error = if self.stack_error { GXSTAT_ERROR } else { 0 };
        let box_result = if self.box_result {
            GXSTAT_BOX_RESULT
        } else {
            0
        };
        // Bits 0, 14, 16-24 and 27 read 0 and bits 25-26 read 1: the instant-drain FIFO is empty
        // and nothing is busy.
        box_result
            | coordinate
            | projection
            | error
            | GXSTAT_FIFO_HALF
            | GXSTAT_FIFO_EMPTY
            | u32::from(self.irq_mode) << GXSTAT_IRQ_SHIFT
    }

    /// The byte of the device at `offset`. Everything but DISP3DCNT, RDLINES_COUNT and GXSTAT
    /// reads 0: the write-only registers, the results, RAM_COUNT and the unnamed addresses.
    // TODO(hw-unverified): the write-only registers and the unnamed addresses read 0
    // (`gx-stub.md`, "Unverified").
    fn byte_at(&self, offset: u32) -> u8 {
        let byte = |value: u32, base: u32| (value >> (8 * (offset - base))) as u8;
        match offset {
            0..DISP3DCNT_BYTES => byte(u32::from(self.disp3dcnt), 0),
            RENDER_OFFSET => RDLINES_COUNT,
            _ => match offset.wrapping_sub(GEOMETRY_OFFSET + GXSTAT_REL) {
                0..GXSTAT_BYTES => byte(self.gxstat(), GEOMETRY_OFFSET + GXSTAT_REL),
                _ => 0,
            },
        }
    }

    /// One byte written to DISP3DCNT or GXSTAT; both are byte-merged registers.
    fn write_byte(&mut self, offset: u32, byte: u8) {
        match offset {
            0..DISP3DCNT_BYTES => {
                let shift = 8 * offset;
                let merged =
                    u32::from(self.disp3dcnt) & !(0xFF << shift) | u32::from(byte) << shift;
                self.disp3dcnt = (merged as u16) & DISP3DCNT_MASK;
            }
            _ => self.write_gxstat_byte(offset.wrapping_sub(GEOMETRY_OFFSET + GXSTAT_REL), byte),
        }
    }

    /// Byte `index` (0-3) of GXSTAT written: bit 15 (byte 1, bit 7) clears the error flag and
    /// resets the projection and texture pointers; bits 30-31 (byte 3, bits 6-7) are the FIFO IRQ
    /// mode. Everything else ignores writes.
    // TODO(hw-unverified): a write of 1 to bit 15 also resets the texture pointer ("probably", says
    // GBATEK), and a write of 0 does nothing (`gx-stub.md`, "Unverified").
    fn write_gxstat_byte(&mut self, index: u32, byte: u8) {
        match index {
            1 if byte & (GXSTAT_ERROR >> 8) as u8 != 0 => {
                self.stack_error = false;
                self.projection_sp = 0;
                self.texture_sp = 0;
            }
            3 => self.irq_mode = byte >> (GXSTAT_IRQ_SHIFT - 24),
            _ => {}
        }
    }

    /// A word written to GXFIFO: a parameter when the decoder still owes some, a command word
    /// otherwise (`gx-stub.md`, "GXFIFO: packed and unpacked words").
    fn gxfifo_word(&mut self, word: u32) {
        match self.owed {
            0 => {
                self.pending = word;
                self.run_pending();
            }
            _ => self.parameter(word),
        }
    }

    /// A word written to the port of command `id`: the same as a command word of `id` followed by
    /// one parameter. A port no command uses drops the write.
    // TODO(hw-unverified): a port write while another command waits for parameters abandons it,
    // and a write narrower than a word is one entry (`gx-stub.md`, "Unverified").
    fn port_word(&mut self, id: u8, word: u32) {
        let Some(count) = parameter_count(id) else {
            return;
        };
        if self.owed == 0 || self.command != id {
            self.pending = 0;
            self.command = id;
            self.owed = count;
            if count == 0 {
                self.execute(id, word);
                return;
            }
        }
        self.parameter(word);
    }

    /// One parameter word of the current command; the last one runs it.
    fn parameter(&mut self, word: u32) {
        self.owed = self.owed.saturating_sub(1);
        if self.owed == 0 {
            self.execute(self.command, word);
            self.run_pending();
        }
    }

    /// Runs the IDs of the packed word in order, up to the first that needs parameters. A byte of 0
    /// (NOP) and an ID that is not a command take none.
    fn run_pending(&mut self) {
        while self.owed == 0 && self.pending != 0 {
            let id = self.pending as u8;
            self.pending >>= 8;
            match parameter_count(id) {
                Some(0) => self.execute(id, 0),
                Some(count) => {
                    self.command = id;
                    self.owed = count;
                }
                None => {}
            }
        }
    }

    /// A command whose last parameter (`param`, 0 for none) has arrived. Only the matrix mode, the
    /// stack pointers and the BOX_TEST result have an effect; the rest is consumed and dropped.
    // TODO(phase7): the geometry engine (#48) gives every command its effect.
    fn execute(&mut self, id: u8, param: u32) {
        match id {
            CMD_MTX_MODE => self.mtx_mode = (param & MTX_MODE_MASK) as u8,
            CMD_MTX_PUSH => self.push(),
            CMD_MTX_POP => self.pop(param),
            CMD_MTX_STORE | CMD_MTX_RESTORE => self.store_or_restore(param),
            // TODO(phase7): the result comes from the box and the matrices; the stub answers
            // "inside" so a program keeps its draw path (`gx-stub.md`, "Model").
            CMD_BOX_TEST => self.box_result = true,
            _ => {}
        }
    }

    /// The pointer of the stack the current mode selects, its mask and how many entries are
    /// valid. Modes 1 and 2 share the coordinate stack for the stack commands, and the texture
    /// stack is treated like the projection stack.
    // TODO(hw-unverified): the texture stack's rules are the projection stack's
    // (`gx-stub.md`, "Unverified").
    fn stack(&mut self) -> (&mut u8, u8, u8) {
        match self.mtx_mode {
            MODE_PROJECTION => (
                &mut self.projection_sp,
                SMALL_POINTER_MASK,
                SMALL_VALID_ENTRIES,
            ),
            MODE_TEXTURE => (
                &mut self.texture_sp,
                SMALL_POINTER_MASK,
                SMALL_VALID_ENTRIES,
            ),
            _ => (
                &mut self.coordinate_sp,
                COORD_POINTER_MASK,
                COORD_VALID_ENTRIES,
            ),
        }
    }

    /// Whether the current stack is the coordinate one (modes 1 and 2).
    const fn is_coordinate(&self) -> bool {
        self.mtx_mode != MODE_PROJECTION && self.mtx_mode != MODE_TEXTURE
    }

    /// MTX_PUSH: entry `S` is stored, then `S` moves up; an entry past the valid ones raises the
    /// error flag.
    // TODO(hw-unverified): the pointer moves even when the entry is invalid
    // (`gx-stub.md`, "Unverified").
    fn push(&mut self) {
        let (pointer, mask, valid) = self.stack();
        let entry = *pointer;
        *pointer = entry.wrapping_add(1) & mask;
        self.stack_error |= entry >= valid;
    }

    /// MTX_POP: `S` moves down by the signed 6-bit offset in the parameter (always 1 off the
    /// coordinate stack), then entry `S` is loaded; an invalid entry raises the error flag.
    fn pop(&mut self, param: u32) {
        let offset = match self.is_coordinate() {
            true => sign_extend_6(param),
            false => 1,
        };
        let (pointer, mask, valid) = self.stack();
        *pointer = pointer.wrapping_sub(offset) & mask;
        let entry = *pointer;
        self.stack_error |= entry >= valid;
    }

    /// MTX_STORE and MTX_RESTORE: entry `N` (bits 0-4 of the parameter; 0 off the coordinate
    /// stack) is used and the pointer is left alone; an invalid entry raises the error flag.
    fn store_or_restore(&mut self, param: u32) {
        let entry = match self.is_coordinate() {
            true => (param & STORE_ADDRESS_MASK) as u8,
            false => 0,
        };
        let (_, _, valid) = self.stack();
        self.stack_error |= entry >= valid;
    }
}

/// Bits 0-5 of `value` as a signed number, as the byte a pointer is stepped by (two's complement
/// wraps correctly in the 6-bit pointer).
const fn sign_extend_6(value: u32) -> u8 {
    let shift = 8 - POP_OFFSET_BITS;
    (((value as u8) << shift) as i8 >> shift) as u8
}

impl Checked for Gpu3d {
    /// Every register and pointer holds a value it can reach, and a decoder that waits for
    /// parameters waits for a command that takes that many.
    fn well_formed(&self, _now: u64) -> bool {
        let waiting = match self.owed {
            0 => true,
            owed => parameter_count(self.command).is_some_and(|count| owed <= count),
        };
        self.disp3dcnt & !DISP3DCNT_MASK == 0
            && self.mtx_mode <= MODE_TEXTURE
            && self.projection_sp <= SMALL_POINTER_MASK
            && self.texture_sp <= SMALL_POINTER_MASK
            && self.coordinate_sp <= COORD_POINTER_MASK
            && self.irq_mode <= 3
            && waiting
    }
}

impl Device<true> for Gpu3d {
    fn peek(&self, offset: u32, width: Width, _cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.byte_at(at))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, _cx: Cx<'_>) -> Effect {
        match offset.wrapping_sub(GEOMETRY_OFFSET) {
            // GXFIFO and its mirrors: one write, one word, whatever its width.
            ..GXFIFO_BYTES => {
                self.gxfifo_word(value);
                Effect::default()
            }
            // The command ports, `4 * id` bytes into the block.
            PORTS_REL..GXSTAT_REL => {
                let id = ((offset - GEOMETRY_OFFSET) >> PORT_SHIFT) as u8;
                self.port_word(id, value);
                Effect::default()
            }
            // GXSTAT and DISP3DCNT take a byte at a time; every other register drops the write.
            rel if (GXSTAT_REL..GXSTAT_REL + GXSTAT_BYTES).contains(&rel) => {
                write_bytes(offset, width, value, |at, byte| {
                    self.write_byte(at, byte);
                    Effect::default()
                })
            }
            _ if offset < DISP3DCNT_BYTES => write_bytes(offset, width, value, |at, byte| {
                self.write_byte(at, byte);
                Effect::default()
            }),
            _ => Effect::default(),
        }
    }
}

#[cfg(test)]
mod tests;
