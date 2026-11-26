// SPDX-License-Identifier: Apache-2.0
//! The geometry engine: DISP3DCNT, the command decoder (GXFIFO and the command ports), the
//! 256-entry FIFO and 4-entry PIPE, the engine's command timing, the matrices and their stacks,
//! vertices and polygons with their clipping and the two sets of Polygon RAM and Vertex RAM,
//! lighting, texture coordinates, the three test commands, GXSTAT and the FIFO IRQ
//! (`docs/hw/geometry-engine.md`, `docs/hw/gx-stub.md`).
//!
//! The engine takes one command at a time from the queue, holds for the command's cycles and then
//! applies its effect: each command's end is a scheduler event of this device (slot 0). A writer that
//! finds the FIFO full waits for the engine, and the engine cycles that takes are charged to it
//! (`Effect::stall`). Nothing is drawn: the polygons the last SWAP_BUFFERS handed on are what the
//! rendering engine will read.
//!
//! Only the ARM9 sees these registers, so [`Gpu3d`] is a plain type implementing `Device<true>`
//! (`AGENTS.md` §4, "Per-CPU devices"): the "seen by one CPU" class. It registers directly with its
//! own port, with no `io/` entrance, as `gpu2d` does (`AGENTS.md` §2, "Placement").
// The rasterizer (`raster`) draws the polygons and the palette textures; the other texture formats
// and the rear bitmap are #50, toon, edge, fog and AA are #51 and the parallel executor is #52.

mod cycles;
mod executor;
mod lighting;
mod matrix;
mod polygon;
mod queue;
mod raster;
mod render;

use crate::io::device::{
    Checked, Cx, Device, Effect, Fired, Signal, Signals, SlotMask, StateTimes, Width, read_bytes,
    write_bytes,
};
use crate::io::irq::{GXFIFO_IRQ, Requests};
use crate::io::timing::next_vblank_start;
use cycles::command_cycles;
pub use executor::{ScanlineExecutor, SequentialExecutor};
use lighting::unpack;
use matrix::{IDENTITY, Matrices, Matrix, Target, shape};
use polygon::Geometry;
pub use polygon::{MAX_POLYGON_VERTICES, POLYGON_RAM, Polygon, Ram, VERTEX_RAM, Vertex};
use queue::{Entry, FIFO_CAPACITY, HALF_FULL, Queue};
pub(crate) use raster::Frame as Frame3d;
pub use render::{RenderRegs, Scene};
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
/// DISP_1DOT_DEPTH, `0400_0610h`, in bytes into the geometry block, and its size.
const ONE_DOT_REL: u32 = 0x210;
const ONE_DOT_BYTES: u32 = 2;
/// The first byte past the ports, `0400_0600h`: GXSTAT.
const GXSTAT_REL: u32 = 0x200;
/// The bytes of GXSTAT.
const GXSTAT_BYTES: u32 = 4;
/// RAM_COUNT, `0400_0604h`, one word.
const RAM_COUNT_REL: u32 = 0x204;
const RAM_COUNT_END: u32 = RAM_COUNT_REL + 4;
/// POS_RESULT, `0400_0620h`, four words; VEC_RESULT, `0400_0630h`, three halfwords.
const POS_RESULT_REL: u32 = 0x220;
const VEC_RESULT_REL: u32 = 0x230;
const VEC_RESULT_END: u32 = VEC_RESULT_REL + 6;
/// CLIPMTX_RESULT, `0400_0640h`, 16 words; VECMTX_RESULT, `0400_0680h`, 9 words.
const CLIPMTX_REL: u32 = 0x240;
const VECMTX_REL: u32 = 0x280;
/// Bytes of a port: one word, so the ID is the offset from the block divided by this.
const PORT_SHIFT: u32 = 2;

/// DISP3DCNT: bits 0-11 and 14 are plain read/write; bits 12-13 are status flags, not stored here.
/// Bit 13 (Polygon/Vertex RAM overflow) is raised by the geometry engine and a write of 1
/// acknowledges it; bit 12 (the colour buffer's underflow) is the rasterizer's (#49) and reads 0
/// until then (`geometry-engine.md`, "DISP3DCNT bits 12 and 13").
// TODO(hw-unverified): bit 13 sets on an overflow and clears by a write of 1, bit 12 reads 0 and a
// write of 1 to it does nothing (`gx-stub.md`, "Unverified").
pub const DISP3DCNT_MASK: u16 = 0x4FFF;
/// RDLINES_COUNT as it reads until the rasterizer (#49) exists: the largest value the field holds, as for a renderer that
/// keeps its line buffer full.
// TODO(hw-unverified): the real value depends on the renderer (`gx-stub.md`, "Unverified").
const RDLINES_COUNT: u8 = 46;

/// GXSTAT bit 0: a BOX_TEST, POS_TEST or VEC_TEST is executing.
const GXSTAT_TEST_BUSY: u32 = 1;
/// GXSTAT bit 1: the BOX_TEST result, 1 = parts or all inside the view volume.
const GXSTAT_BOX_RESULT: u32 = 1 << 1;
/// GXSTAT bits 8-12: the low 5 bits of the coordinate stack pointer.
const GXSTAT_COORD_SHIFT: u32 = 8;
const GXSTAT_COORD_MASK: u8 = 0x1F;
/// GXSTAT bit 13: the projection stack pointer.
const GXSTAT_PROJECTION_SHIFT: u32 = 13;
/// GXSTAT bit 14: a MTX_PUSH, MTX_POP, MTX_STORE or MTX_RESTORE is executing.
const GXSTAT_STACK_BUSY: u32 = 1 << 14;
/// GXSTAT bit 15: the stack error flag, write 1 to clear.
const GXSTAT_ERROR: u32 = 1 << 15;
/// GXSTAT bits 16-23: the low 8 bits of the FIFO's entry count.
const GXSTAT_COUNT_SHIFT: u32 = 16;
const GXSTAT_COUNT_MASK: u32 = 0xFF;
/// GXSTAT bit 24: the FIFO is full (the entry count is 256).
const GXSTAT_FIFO_FULL: u32 = 1 << 24;
/// GXSTAT bit 25: the FIFO is less than half full.
const GXSTAT_FIFO_HALF: u32 = 1 << 25;
/// GXSTAT bit 26: the FIFO is empty.
const GXSTAT_FIFO_EMPTY: u32 = 1 << 26;
/// GXSTAT bit 27: the engine is busy: a command is executing, or the PIPE or FIFO holds data.
const GXSTAT_BUSY: u32 = 1 << 27;
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
const MODE_SIMULTANEOUS: u8 = 2;
const MODE_TEXTURE: u8 = 3;

/// The command IDs the decoder and the engine give an effect to.
const CMD_MTX_MODE: u8 = 0x10;
const CMD_MTX_PUSH: u8 = 0x11;
const CMD_MTX_POP: u8 = 0x12;
const CMD_MTX_STORE: u8 = 0x13;
const CMD_MTX_RESTORE: u8 = 0x14;
const CMD_MTX_IDENTITY: u8 = 0x15;
const CMD_MTX_LOAD_4X4: u8 = 0x16;
const CMD_MTX_LOAD_4X3: u8 = 0x17;
const CMD_MTX_MULT_4X4: u8 = 0x18;
const CMD_MTX_MULT_4X3: u8 = 0x19;
const CMD_MTX_MULT_3X3: u8 = 0x1A;
const CMD_MTX_SCALE: u8 = 0x1B;
const CMD_MTX_TRANS: u8 = 0x1C;
const CMD_COLOR: u8 = 0x20;
const CMD_NORMAL: u8 = 0x21;
const CMD_TEXCOORD: u8 = 0x22;
const CMD_VTX_16: u8 = 0x23;
const CMD_VTX_DIFF: u8 = 0x28;
const CMD_POLYGON_ATTR: u8 = 0x29;
const CMD_TEXIMAGE_PARAM: u8 = 0x2A;
const CMD_PLTT_BASE: u8 = 0x2B;
const CMD_DIF_AMB: u8 = 0x30;
const CMD_SPE_EMI: u8 = 0x31;
const CMD_LIGHT_VECTOR: u8 = 0x32;
const CMD_LIGHT_COLOR: u8 = 0x33;
const CMD_SHININESS: u8 = 0x34;
const CMD_BEGIN_VTXS: u8 = 0x40;
const CMD_SWAP_BUFFERS: u8 = 0x50;
const CMD_VIEWPORT: u8 = 0x60;
/// COLOR's parameter: RGB555.
const COLOR_MASK: u32 = 0x7FFF;
/// DISP3DCNT bit 13: Polygon/Vertex RAM overflow, write 1 to acknowledge.
const DISP3DCNT_OVERFLOW: u32 = 1 << 13;
const CMD_BOX_TEST: u8 = 0x70;
const CMD_POS_TEST: u8 = 0x71;
const CMD_VEC_TEST: u8 = 0x72;

/// The engine's one event slot: the end of the command it executes.
const ENGINE_SLOT: SlotMask = 1;
/// The most parameters a command takes: SHININESS has 32.
const MAX_PARAMS: usize = 32;

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

/// The geometry engine's registers, command decoder, queue and state.
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
    /// POS_RESULT: the clip coordinates of the last POS_TEST, 1.19.12.
    pos_result: [i32; 4],
    /// VEC_RESULT: the last VEC_TEST, three 16-bit values with 12 fraction bits.
    vec_result: [u16; 3],
    /// The command IDs of the word being decoded that have not run yet, the next in the low byte.
    /// A byte of 0 is NOP and so is everything above the last non-zero byte: 0 means none left.
    pending: u32,
    /// The command waiting for parameters, valid while `owed` is above 0.
    command: u8,
    /// How many parameter words `command` still needs.
    owed: u8,
    /// The FIFO and the PIPE, in front of the engine.
    queue: Queue,
    /// The rendering registers, stored for the rasterizer (`geometry-engine.md`, "The rendering
    /// registers").
    render: RenderRegs,
    /// The matrices and their stacks (`geometry-engine.md`, "Matrices").
    matrices: Matrices,
    /// The vertex attributes, the list being assembled and the two sets of Polygon and Vertex RAM
    /// (`geometry-engine.md`, "Vertices").
    geometry: Geometry,
    /// SWAP_BUFFERS met an unfinished polygon list: the 3D hardware is locked up and nothing runs
    /// again (`geometry-engine.md`, "SWAP_BUFFERS").
    locked: bool,
    /// The command the engine executes, if any (`geometry-engine.md`, "Executing a command").
    running: Option<Running>,
}

/// The command the engine is executing: its effect is applied when `ends` is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Running {
    /// The command ID.
    command: u8,
    /// The command's parameters in order, the first `entries` of them; a command with none has one
    /// entry of 0.
    params: [u32; MAX_PARAMS],
    /// The master time the command ends.
    ends: u64,
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

    /// Entries in the FIFO (GXSTAT's count), 0-256.
    pub fn fifo_len(&self) -> usize {
        self.queue.fifo_len()
    }

    /// Entries in the PIPE, 0-4.
    pub fn pipe_len(&self) -> usize {
        self.queue.pipe_len()
    }

    /// The `index`th queued entry as `(command, parameter)`, the PIPE first: for the debugger.
    pub fn queued(&self, index: usize) -> Option<(u8, u32)> {
        self.queue
            .get(index)
            .map(|entry| (entry.command, entry.param))
    }

    /// The command the engine is executing and the master time it ends: for the debugger.
    pub fn running(&self) -> Option<(u8, u64)> {
        self.running.map(|running| (running.command, running.ends))
    }

    /// Polygon RAM and Vertex RAM the geometry engine is filling: for the debugger.
    pub fn geometry_ram(&self) -> &Ram {
        self.geometry.filling()
    }

    /// What the last SWAP_BUFFERS handed on: the set of RAM the rasterizer (#49) draws and the
    /// swap's two parameter bits.
    pub fn scene(&self) -> Scene<'_> {
        Scene {
            ram: self.geometry.rendering(),
            swap_bits: self.geometry.swap_bits,
        }
    }

    /// The rendering registers as written: the rasterizer's inputs besides the scene.
    pub fn render_regs(&self) -> &RenderRegs {
        &self.render
    }

    /// DISP3DCNT bit 13: a polygon or vertex was dropped because a RAM was full.
    pub fn ram_overflow(&self) -> bool {
        self.geometry.overflow
    }

    /// VIEWPORT as written.
    pub fn viewport(&self) -> u32 {
        self.geometry.viewport
    }

    /// Whether SWAP_BUFFERS locked the 3D hardware up.
    pub fn locked_up(&self) -> bool {
        self.locked
    }

    /// The projection matrix, 16 elements row by row in 1.19.12: for the debugger.
    pub fn projection_matrix(&self) -> &[i32; 16] {
        &self.matrices.projection
    }

    /// The position (model-view) matrix.
    pub fn position_matrix(&self) -> &[i32; 16] {
        &self.matrices.position
    }

    /// The directional vector matrix.
    pub fn vector_matrix(&self) -> &[i32; 16] {
        &self.matrices.vector
    }

    /// The texture matrix.
    pub fn texture_matrix(&self) -> &[i32; 16] {
        &self.matrices.texture
    }

    /// The clip matrix: the position matrix times the projection matrix.
    pub fn clip_matrix(&self) -> &[i32; 16] {
        &self.matrices.clip
    }

    /// Whether a command is executing or the PIPE or FIFO holds data: GXSTAT bit 27.
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.queue.len() > 0
    }

    /// GXSTAT as a read sees it (`geometry-engine.md`, "GXSTAT").
    fn gxstat(&self) -> u32 {
        let coordinate = u32::from(self.coordinate_sp & GXSTAT_COORD_MASK) << GXSTAT_COORD_SHIFT;
        let projection = u32::from(self.projection_sp) << GXSTAT_PROJECTION_SHIFT;
        let error = if self.stack_error { GXSTAT_ERROR } else { 0 };
        let box_result = if self.box_result {
            GXSTAT_BOX_RESULT
        } else {
            0
        };
        let executing = |ids: &[u8]| {
            self.running
                .is_some_and(|running| ids.contains(&running.command))
        };
        // TODO(hw-unverified): bit 0 covers the whole time a test executes and bit 14 the time a
        // push, pop, store or restore executes (`geometry-engine.md`, "Unverified").
        let test_busy = if executing(&[CMD_BOX_TEST, CMD_POS_TEST, CMD_VEC_TEST]) {
            GXSTAT_TEST_BUSY
        } else {
            0
        };
        let stack_busy = if executing(&[CMD_MTX_PUSH, CMD_MTX_POP, CMD_MTX_STORE, CMD_MTX_RESTORE])
        {
            GXSTAT_STACK_BUSY
        } else {
            0
        };
        let len = self.queue.fifo_len();
        let count = (len as u32 & GXSTAT_COUNT_MASK) << GXSTAT_COUNT_SHIFT;
        let full = if len == FIFO_CAPACITY {
            GXSTAT_FIFO_FULL
        } else {
            0
        };
        let half = if len < HALF_FULL { GXSTAT_FIFO_HALF } else { 0 };
        let empty = if len == 0 { GXSTAT_FIFO_EMPTY } else { 0 };
        let busy = if self.busy() { GXSTAT_BUSY } else { 0 };
        test_busy
            | box_result
            | coordinate
            | projection
            | stack_busy
            | error
            | count
            | full
            | half
            | empty
            | busy
            | u32::from(self.irq_mode) << GXSTAT_IRQ_SHIFT
    }

    /// The byte of the device at `offset`: DISP3DCNT, RDLINES_COUNT, GXSTAT and the result registers
    /// (RAM_COUNT, POS and VEC, the clip and vector matrices) hold state; the write-only registers
    /// and the unnamed addresses read 0.
    // TODO(hw-unverified): the write-only registers and the unnamed addresses read 0
    // (`gx-stub.md`, "Unverified").
    fn byte_at(&self, offset: u32) -> u8 {
        let byte = |value: u32, base: u32| (value >> (8 * (offset - base))) as u8;
        match offset {
            0..DISP3DCNT_BYTES => byte(self.disp3dcnt_read(), 0),
            RENDER_OFFSET => RDLINES_COUNT,
            _ => match offset.wrapping_sub(GEOMETRY_OFFSET + GXSTAT_REL) {
                0..GXSTAT_BYTES => byte(self.gxstat(), GEOMETRY_OFFSET + GXSTAT_REL),
                _ => self.matrix_byte(offset.wrapping_sub(GEOMETRY_OFFSET)),
            },
        }
    }

    /// DISP3DCNT as a read sees it: the stored bits and the overflow flag (bit 13).
    fn disp3dcnt_read(&self) -> u32 {
        let overflow = if self.geometry.overflow {
            DISP3DCNT_OVERFLOW
        } else {
            0
        };
        u32::from(self.disp3dcnt) | overflow
    }

    /// The byte of CLIPMTX_RESULT or VECMTX_RESULT at `rel` bytes into the geometry block, 0 where
    /// neither is (`geometry-engine.md`, "Matrices"). The matrices are read as they stand: a command
    /// that is still executing has not changed them yet.
    fn matrix_byte(&self, rel: u32) -> u8 {
        let word_byte = |base: u32, word: u32| (word >> (8 * ((rel - base) % 4))) as u8;
        match rel {
            POS_RESULT_REL..VEC_RESULT_REL => word_byte(
                POS_RESULT_REL,
                self.pos_result[((rel - POS_RESULT_REL) / 4) as usize] as u32,
            ),
            VEC_RESULT_REL..VEC_RESULT_END => {
                let at = rel - VEC_RESULT_REL;
                (self.vec_result[(at / 2) as usize] >> (8 * (at % 2))) as u8
            }
            RAM_COUNT_REL..RAM_COUNT_END => word_byte(RAM_COUNT_REL, self.geometry.ram_count()),
            CLIPMTX_REL..VECMTX_REL => word_byte(
                CLIPMTX_REL,
                self.matrices.clip_word(((rel - CLIPMTX_REL) / 4) as usize),
            ),
            VECMTX_REL..GEOMETRY_BYTES => word_byte(
                VECMTX_REL,
                self.matrices.vector_word(((rel - VECMTX_REL) / 4) as usize),
            ),
            _ => 0,
        }
    }

    /// One byte written to DISP3DCNT or GXSTAT; both are byte-merged registers.
    fn write_byte(&mut self, offset: u32, byte: u8) {
        match offset {
            0..DISP3DCNT_BYTES => {
                // A write of 1 to bit 13 (byte 1, bit 5) acknowledges the overflow flag.
                if offset == 1 && u32::from(byte) << 8 & DISP3DCNT_OVERFLOW != 0 {
                    self.geometry.overflow = false;
                }
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
    /// otherwise (`gx-stub.md`, "GXFIFO: packed and unpacked words"). `at` is the master time of
    /// the write and moves on when the write waits for room.
    // TODO(hw-unverified): a GXFIFO word while a port command owes parameters is that command's
    // parameter (`gx-stub.md`, "Unverified").
    fn gxfifo_word(&mut self, word: u32, at: &mut u64) {
        match self.owed {
            0 => {
                self.pending = word;
                self.run_pending(at);
            }
            _ => self.parameter(word, at),
        }
    }

    /// A word written to the port of command `id`: the same as a command word of `id` followed by
    /// one parameter. A port no command uses drops the write.
    // TODO(hw-unverified): a port write while another command waits for parameters abandons it,
    // and a write narrower than a word is one entry (`gx-stub.md`, "Unverified").
    fn port_word(&mut self, id: u8, word: u32, at: &mut u64) {
        let Some(count) = parameter_count(id) else {
            return;
        };
        if self.owed == 0 || self.command != id {
            self.abandon();
            self.pending = 0;
            self.command = id;
            self.owed = count;
            if count == 0 {
                self.enqueue(Entry::new(id, 0), at);
                return;
            }
        }
        self.parameter(word, at);
    }

    /// Drops the command that waits for parameters, with the entries it already queued: the engine
    /// takes a command only when all its entries are there, so they would otherwise stay in the
    /// queue for ever and shift every later command boundary.
    // TODO(hw-unverified): an abandoned command's entries are taken back (`geometry-engine.md`,
    // "Unverified").
    fn abandon(&mut self) {
        let count = parameter_count(self.command).map_or(0, usize::from);
        let queued = count.saturating_sub(usize::from(self.owed));
        // After a lock-up entries that found the FIFO full were dropped, so the count of queued ones
        // is not known; the queue is frozen then and keeps what it has.
        if self.owed > 0 && !self.locked {
            self.queue.drop_newest(queued);
        }
        self.owed = 0;
    }

    /// One parameter word of the current command, queued as an entry; the last one frees the
    /// decoder for the rest of its packed word.
    fn parameter(&mut self, word: u32, at: &mut u64) {
        self.owed = self.owed.saturating_sub(1);
        self.enqueue(Entry::new(self.command, word), at);
        if self.owed == 0 {
            self.run_pending(at);
        }
    }

    /// Runs the IDs of the packed word in order, up to the first that needs parameters. A byte of 0
    /// (NOP) and an ID that is not a command take none and queue nothing; a command with no
    /// parameters queues one entry.
    // TODO(hw-unverified): the last non-zero command of a packed word may have no parameters;
    // the hardware's behavior is not stated (`gx-stub.md`, "Unverified").
    // TODO(hw-unverified): NOP and an invalid ID take no queue entry (`geometry-engine.md`,
    // "Unverified").
    fn run_pending(&mut self, at: &mut u64) {
        while self.owed == 0 && self.pending != 0 {
            let id = self.pending as u8;
            self.pending >>= 8;
            match parameter_count(id) {
                Some(0) => self.enqueue(Entry::new(id, 0), at),
                Some(count) => {
                    self.command = id;
                    self.owed = count;
                }
                None => {}
            }
        }
    }

    /// Queues `entry` at master time `at`. A full FIFO holds the writer: the engine runs on until it
    /// takes an entry, `at` moves to that time, and the difference is what the write stalls
    /// (`geometry-engine.md`, "The FIFO IRQ, the DMA start and the stall").
    // TODO(hw-unverified): a writer stuck on a FIFO the engine can never drain would hang the
    // hardware; the entry is dropped here (`geometry-engine.md`, "Unverified").
    fn enqueue(&mut self, entry: Entry, at: &mut u64) {
        self.advance(*at);
        while self.queue.fifo_full() {
            let Some(running) = self.running.filter(|_| !self.locked) else {
                return;
            };
            *at = (*at).max(running.ends);
            self.advance(running.ends);
        }
        self.queue.push(entry);
        self.start(*at);
    }

    /// Runs the engine up to master time `time`: every command that ends by then takes effect, and
    /// the next starts where it ended.
    fn advance(&mut self, time: u64) {
        while let Some(running) = self
            .running
            .filter(|running| running.ends <= time && !self.locked)
        {
            self.running = None;
            self.execute(running.command, &running.params);
            self.start(running.ends);
        }
    }

    /// Starts the next command at master time `at` if the engine is idle and the queue holds all
    /// its entries: they leave the queue now and the command ends after its cycles.
    // TODO(hw-unverified): a command starts only when all its entries are queued, and takes them at
    // once (`geometry-engine.md`, "Unverified").
    fn start(&mut self, at: u64) {
        if self.running.is_some() {
            return;
        }
        let Some(front) = self.queue.front() else {
            return;
        };
        let entries = parameter_count(front.command).map_or(1, |count| usize::from(count.max(1)));
        if self.queue.len() < entries {
            return;
        }
        let mut params = [0; MAX_PARAMS];
        params
            .iter_mut()
            .take(entries)
            .for_each(|slot| *slot = self.queue.pop().map_or(0, |entry| entry.param));
        // SWAP_BUFFERS is held until VBlank begins; the engine counts as busy meanwhile.
        // TODO(hw-unverified): the wait ends at the first line-192 start at or after the command
        // begins, the next frame's inside VBlank (`geometry-engine.md`, "Unverified").
        let begins = match front.command {
            CMD_SWAP_BUFFERS => at.max(next_vblank_start(at)),
            _ => at,
        };
        self.running = Some(Running {
            command: front.command,
            params,
            ends: begins.saturating_add(command_cycles(
                front.command,
                self.mtx_mode,
                self.geometry.enabled_lights(),
            )),
        });
    }

    /// A command that has ended, with its parameters (`params`, one 0 for none): its effect takes
    /// place now. The rendering registers are not commands and are not here.
    fn execute(&mut self, id: u8, params: &[u32; MAX_PARAMS]) {
        let param = params[0];
        match id {
            CMD_MTX_MODE => self.mtx_mode = (param & MTX_MODE_MASK) as u8,
            CMD_MTX_PUSH => self.push(),
            CMD_MTX_POP => self.pop(param),
            CMD_MTX_STORE | CMD_MTX_RESTORE => self.store_or_restore(id, param),
            CMD_MTX_IDENTITY => self.matrices.load(self.target(true), &IDENTITY),
            CMD_MTX_LOAD_4X4 | CMD_MTX_LOAD_4X3 => {
                self.matrices.load(self.target(true), &shape(id, params));
            }
            CMD_MTX_MULT_4X4 | CMD_MTX_MULT_4X3 | CMD_MTX_MULT_3X3 | CMD_MTX_TRANS => {
                self.matrices
                    .multiply_by(self.target(true), &shape(id, params));
            }
            // MTX_SCALE leaves the vector matrix alone even in mode 2, so a light vector keeps its
            // length (`geometry-engine.md`, "Matrices").
            CMD_MTX_SCALE => self
                .matrices
                .multiply_by(self.target(false), &shape(id, params)),
            CMD_COLOR => self.geometry.color = (param & COLOR_MASK) as u16,
            CMD_NORMAL => {
                self.geometry
                    .normal(param, &self.matrices.vector, &self.matrices.texture);
            }
            CMD_TEXCOORD => self.geometry.texcoord(param, &self.matrices.texture),
            CMD_DIF_AMB => self.geometry.diffuse_ambient(param),
            CMD_SPE_EMI => self.geometry.specular_emission(param),
            CMD_LIGHT_VECTOR => self.geometry.light_vector(param, &self.matrices.vector),
            CMD_LIGHT_COLOR => self.geometry.light_color(param),
            CMD_SHININESS => self.geometry.shininess(params),
            CMD_POLYGON_ATTR => self.geometry.write_attr(param),
            CMD_TEXIMAGE_PARAM => self.geometry.texture = param,
            CMD_PLTT_BASE => self.geometry.palette = param,
            CMD_BEGIN_VTXS => self.geometry.begin(param),
            CMD_VTX_16..=CMD_VTX_DIFF => {
                self.geometry
                    .vertex(id, params, &self.matrices.clip, &self.matrices.texture);
            }
            CMD_VIEWPORT => self.geometry.viewport = param,
            CMD_SWAP_BUFFERS => self.swap_buffers(param),
            CMD_BOX_TEST => {
                self.box_result = self.geometry.box_test(params, &self.matrices.clip);
            }
            CMD_POS_TEST => {
                self.pos_result = self.geometry.position_test(params, &self.matrices.clip);
            }
            CMD_VEC_TEST => self.vec_result = vector_test(param, &self.matrices.vector),
            _ => {}
        }
    }

    /// SWAP_BUFFERS has waited for VBlank and run: the two sets of RAM exchange roles, or, with an
    /// unfinished polygon list, the hardware locks up and the command never ends.
    fn swap_buffers(&mut self, param: u32) {
        if self.geometry.unfinished() {
            self.locked = true;
            self.running = Some(Running {
                command: CMD_SWAP_BUFFERS,
                params: [0; MAX_PARAMS],
                ends: crate::scheduler::MAX_TIME,
            });
        } else {
            self.geometry.swap(param);
        }
    }

    /// The matrices a matrix command reaches in the current mode. `with_vector` says whether mode
    /// 2 includes the vector matrix (every command but MTX_SCALE).
    const fn target(&self, with_vector: bool) -> Target {
        match self.mtx_mode {
            MODE_PROJECTION => Target::Projection,
            MODE_TEXTURE => Target::Texture,
            mode => Target::Coordinate {
                vector: with_vector && mode == MODE_SIMULTANEOUS,
            },
        }
    }

    /// The matrices a stack command reaches: modes 1 and 2 both act on the position and the vector
    /// stack.
    const fn stack_target(&self) -> Target {
        match self.mtx_mode {
            MODE_PROJECTION => Target::Projection,
            MODE_TEXTURE => Target::Texture,
            _ => Target::Coordinate { vector: true },
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
    // TODO(hw-unverified): entries 1Fh and above of the coordinate stack are all invalid, read
    // literally from GBATEK (`gx-stub.md`, "Unverified").
    fn push(&mut self) {
        let (pointer, mask, valid) = self.stack();
        let entry = *pointer;
        *pointer = entry.wrapping_add(1) & mask;
        self.stack_error |= entry >= valid;
        self.matrices.store(self.stack_target(), entry);
    }

    /// MTX_POP: `S` moves down by the signed 6-bit offset in the parameter (always 1 off the
    /// coordinate stack), then entry `S` is loaded; an invalid entry raises the error flag.
    // TODO(hw-unverified): in projection and texture mode the ignored parameter is still one FIFO
    // entry, so POP, STORE and RESTORE take one parameter word there (`gx-stub.md`, "Unverified").
    fn pop(&mut self, param: u32) {
        let offset = match self.is_coordinate() {
            true => sign_extend_6(param),
            false => 1,
        };
        let (pointer, mask, valid) = self.stack();
        *pointer = pointer.wrapping_sub(offset) & mask;
        let entry = *pointer;
        self.stack_error |= entry >= valid;
        self.matrices.restore(self.stack_target(), entry);
    }

    /// MTX_STORE and MTX_RESTORE: entry `N` (bits 0-4 of the parameter; 0 off the coordinate
    /// stack) is used and the pointer is left alone; an invalid entry raises the error flag.
    fn store_or_restore(&mut self, id: u8, param: u32) {
        let entry = match self.is_coordinate() {
            true => (param & STORE_ADDRESS_MASK) as u8,
            false => 0,
        };
        let (_, _, valid) = self.stack();
        self.stack_error |= entry >= valid;
        match id {
            CMD_MTX_STORE => self.matrices.store(self.stack_target(), entry),
            _ => self.matrices.restore(self.stack_target(), entry),
        }
    }
}

/// VEC_TEST: the vector (1.0.9) through the vector matrix, as 12-bit fractions in 16 bits whose top
/// four bits copy the sign: a value of 1.0 or more overflows (1.0 reads as -1.0). The sum of the
/// products has 21 fraction bits (9 and 12) and keeps 12, so the low bits of the result are real.
// TODO(hw-unverified): the result's width and sign extension (`geometry-engine.md`, "Unverified").
fn vector_test(param: u32, vector: &Matrix) -> [u16; 3] {
    let v = unpack(param);
    std::array::from_fn(|c| {
        let sum = (0..3).fold(0i64, |sum, k| {
            sum.wrapping_add(i64::from(v[k]).wrapping_mul(i64::from(vector[4 * k + c])))
        });
        // 21 fraction bits to 12, then the 13-bit signed value sign-extended to 16 bits.
        let twelve = (sum >> 9) as i32;
        (((twelve << 19) >> 19) as i16) as u16
    })
}

/// Bits 0-5 of `value` as a signed number, as the byte a pointer is stepped by (two's complement
/// wraps correctly in the 6-bit pointer).
const fn sign_extend_6(value: u32) -> u8 {
    let shift = 8 - POP_OFFSET_BITS;
    (((value as u8) << shift) as i8 >> shift) as u8
}

impl Gpu3d {
    /// Puts a clip matrix in that is not the product of the two it comes from, to build a corrupt
    /// state in a test.
    #[cfg(test)]
    pub(crate) fn scramble_clip_for_test(&mut self) {
        self.matrices.scramble_clip_for_test();
    }
}

impl Checked for Gpu3d {
    /// A load decoded Polygon RAM and Vertex RAM with exactly their length, and a command must never
    /// allocate (`AGENTS.md` §2 rule 3): the room for a full set is reserved once. The clip matrix is
    /// a product of two saved matrices, so it is recomputed, never trusted.
    fn restore(&mut self, _now: u64) {
        self.geometry.reserve_ram();
        self.matrices.restore_clip();
    }

    /// Every register and pointer holds a value it can reach, a decoder that waits for parameters
    /// waits for a command that takes that many, the queue is whole and the command the engine
    /// executes is one the decoder queues, due no later than the model's last time.
    fn well_formed(&self, _times: StateTimes) -> bool {
        let waiting = match self.owed {
            0 => true,
            owed => parameter_count(self.command).is_some_and(|count| owed <= count),
        };
        let executing = self.running.is_none_or(|running| {
            parameter_count(running.command).is_some() && running.ends <= crate::scheduler::MAX_TIME
        });
        self.disp3dcnt & !DISP3DCNT_MASK == 0
            && self.mtx_mode <= MODE_TEXTURE
            && self.projection_sp <= SMALL_POINTER_MASK
            && self.texture_sp <= SMALL_POINTER_MASK
            && self.coordinate_sp <= COORD_POINTER_MASK
            && self.irq_mode <= 3
            && waiting
            && self.queue.well_formed()
            && self.geometry.well_formed()
            && self.render.well_formed()
            && (self.running.is_some() || !self.locked)
            && executing
    }
}

impl Device<true> for Gpu3d {
    fn peek(&self, offset: u32, width: Width, _cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.byte_at(at))
    }

    /// The FIFO IRQ is a level: IF bit 21 is held up while the mode is 1 and the FIFO is less than
    /// half full, or the mode is 2 and the FIFO is empty. A write to this block or to the interrupt
    /// controller, and the end of a command, ask `Io` to recheck, which raises the bit again after
    /// an acknowledge, so an acknowledge has no effect while the condition holds. No other path can
    /// make the condition true or clear an IF bit (`geometry-engine.md`, "The FIFO IRQ, the DMA
    /// start and the stall").
    // TODO(hw-unverified): a bit already set stays after the mode goes to 0 until it is
    // acknowledged (`gx-stub.md`, "Unverified").
    // TODO(hw-unverified): mode 3 (reserved) raises nothing (`gx-stub.md`, "Unverified").
    fn held_requests(&self) -> Requests {
        let len = self.queue.fifo_len();
        match self.irq_mode {
            IRQ_HALF_FULL if len < HALF_FULL => Requests::for_cpu::<true>(GXFIFO_IRQ),
            IRQ_EMPTY if len == 0 => Requests::for_cpu::<true>(GXFIFO_IRQ),
            _ => Requests::default(),
        }
    }

    /// The FIFO is less than half full: a DMA channel in start mode 7 takes a burst
    /// (`geometry-engine.md`, "The FIFO IRQ, the DMA start and the stall").
    fn held_signals(&self) -> Signals {
        if self.queue.fifo_len() < HALF_FULL {
            Signals::of(Signal::GeometryFifoHalf)
        } else {
            Signals::default()
        }
    }

    /// Every write asks `Io` to recheck what the device holds, as the one that sets the mode may
    /// make the FIFO IRQ's condition true ([`Device::held_requests`]).
    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        Effect {
            recheck_held_requests: true,
            ..self.write_registers(offset, width, value, cx)
        }
    }

    const SLOTS: u8 = 1;

    /// The engine's slot is due when the command it executes ends.
    fn slot_due(&self, _slot: u8) -> Option<u64> {
        self.running
            .filter(|_| !self.locked)
            .map(|running| running.ends)
    }

    /// The command ended: its effect is applied, the next command starts, and the levels the FIFO
    /// feeds (the IRQ and the DMA start) are asked again.
    fn slot_fire(&mut self, _slot: u8, now: u64) -> Fired {
        self.advance(now);
        Fired {
            effect: Effect {
                reschedule: ENGINE_SLOT,
                signals: self.held_signals(),
                recheck_held_requests: true,
                ..Effect::default()
            },
            transfer: None,
        }
    }
}

impl Gpu3d {
    /// A write that queues entries at master time `now`: its effect replaces the engine's event,
    /// as a command may have started, and carries the cycles a full FIFO held the writer.
    fn queued_write(&mut self, now: u64, write: impl FnOnce(&mut Self, &mut u64)) -> Effect {
        let mut at = now;
        write(self, &mut at);
        Effect {
            reschedule: ENGINE_SLOT,
            stall: at.saturating_sub(now),
            ..Effect::default()
        }
    }

    /// The register write itself, without the recheck request (`write`).
    fn write_registers(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        // POWCNT1 bit 3 clear makes the whole geometry block read-only: GXFIFO, the ports and
        // GXSTAT drop their writes (`gx-stub.md`, "Power control").
        // TODO(hw-unverified): GXSTAT's R/W bits count as read-only too (`gx-stub.md`,
        // "Unverified").
        if offset.wrapping_sub(GEOMETRY_OFFSET) < GEOMETRY_BYTES && !cx.lines.geometry_powered {
            return Effect::default();
        }
        match offset.wrapping_sub(GEOMETRY_OFFSET) {
            // GXFIFO and its mirrors: one write, one word, whatever its width.
            // TODO(hw-unverified): all 16 words mirror GXFIFO and a narrow write is one entry
            // (`gx-stub.md`, "Unverified").
            ..GXFIFO_BYTES => self.queued_write(cx.now, |gpu, at| gpu.gxfifo_word(value, at)),
            // The command ports, `4 * id` bytes into the block. A port no command uses drops the
            // write (`port_word`).
            // TODO(hw-unverified): writes to unused ports are dropped (`gx-stub.md`, "Unverified").
            PORTS_REL..GXSTAT_REL => {
                let id = ((offset - GEOMETRY_OFFSET) >> PORT_SHIFT) as u8;
                self.queued_write(cx.now, |gpu, at| gpu.port_word(id, value, at))
            }
            // GXSTAT and DISP3DCNT take a byte at a time; every other register drops the write.
            rel if (GXSTAT_REL..GXSTAT_REL + GXSTAT_BYTES).contains(&rel) => {
                write_bytes(offset, width, value, |at, byte| {
                    self.write_byte(at, byte);
                    Effect::default()
                })
            }
            // DISP_1DOT_DEPTH: not routed through GXFIFO, so a write takes effect at once and
            // applies to polygons still in the FIFO (`rasterizer.md`, "Which pixels a polygon
            // covers").
            rel if (ONE_DOT_REL..ONE_DOT_REL + ONE_DOT_BYTES).contains(&rel) => {
                write_bytes(offset, width, value, |at, byte| {
                    self.render
                        .write_one_dot_byte(at - (GEOMETRY_OFFSET + ONE_DOT_REL), byte);
                    Effect::default()
                })
            }
            _ if offset < DISP3DCNT_BYTES => write_bytes(offset, width, value, |at, byte| {
                self.write_byte(at, byte);
                Effect::default()
            }),
            // The rendering block, unless POWCNT1 bit 2 has made it read-only.
            // TODO(hw-unverified): the power gate (`gx-stub.md`, "Power control").
            _ if (RENDER_OFFSET..RENDER_OFFSET + RENDER_BYTES).contains(&offset)
                && cx.lines.rendering_powered =>
            {
                write_bytes(offset, width, value, |at, byte| {
                    self.render.write_byte(at - RENDER_OFFSET, byte);
                    Effect::default()
                })
            }
            _ => Effect::default(),
        }
    }
}

#[cfg(test)]
mod tests;
