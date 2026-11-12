// SPDX-License-Identifier: Apache-2.0
//! The two 2D graphics engines and their display stage: display off, the VRAM display
//! (`docs/hw/display-mode-vram.md`), the text backgrounds (`docs/hw/bg-text.md`) and engine B
//! (`docs/hw/bg-modes.md`).
//!
//! The picture is made one line at a time ([`Gpu2d::render_line_a`], [`Gpu2d::render_line_b`]),
//! from the registers and memory as they are when the line is asked for. No allocation, formatting
//! or collection happens while pixels are made.
//!
//! Both engines are wired to the ARM9 alone, so `Gpu2d` is a plain type with no CPU const generic:
//! the "seen by one CPU" class of `AGENTS.md` §4 ("Per-CPU devices"). It owns two plain
//! [`Engine`]s, one per register block, and what only engine A has lives here (the display modes 2
//! and 3) or in [`EngineSpec`].

mod affine;
mod engine;
mod kind;
mod spec;
mod text;

use crate::io::device::{Device, Effect, Width, read_bytes, write_bytes};
use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};
pub use affine::{AffineBg, AffineFormat};
use engine::{
    AFFINE_OFFSET, AFFINE_REGS_BYTES as ENGINE_AFFINE_REGS_BYTES,
    BG_REGS_BYTES as ENGINE_BG_REGS_BYTES, BGCNT_OFFSET, Engine,
};
use serde::{Deserialize, Serialize};
pub(crate) use spec::EngineInputs;
use spec::{EngineSpec, MAIN_SPEC, SUB_SPEC};
pub use text::TextBg;

/// DISPCNT of engine A, at `0400_0000h` (`display-mode-vram.md`, "Registers").
pub const DISPCNT_A_ADDR: u32 = 0x0400_0000;
/// Engine B's register block is engine A's plus this (`bg-modes.md`, "Registers").
const ENGINE_B_OFFSET: u32 = 0x1000;
/// DISPCNT of engine B, at `0400_1000h`.
pub const DISPCNT_B_ADDR: u32 = DISPCNT_A_ADDR + ENGINE_B_OFFSET;
/// POWCNT1, at `0400_0304h` (`display-mode-vram.md`, "Registers").
pub const POWCNT1_ADDR: u32 = 0x0400_0304;
/// POWCNT1's offset in the register space of [`Gpu2d`], which starts at DISPCNT.
const POWCNT1_OFFSET: u32 = POWCNT1_ADDR - DISPCNT_A_ADDR;
/// BG0CNT, at `0400_0008h`: BG0CNT-BG3CNT, then BG0HOFS, BG0VOFS, BG1HOFS, ... BG3VOFS
/// (`bg-text.md`, "Registers"). They are one block of the map, 24 bytes from here.
pub const BG_REGS_ADDR: u32 = DISPCNT_A_ADDR + BGCNT_OFFSET;
/// Engine B's BG control block, at `0400_1008h`.
pub const BG_REGS_B_ADDR: u32 = BG_REGS_ADDR + ENGINE_B_OFFSET;
/// The bytes of that block: four BGxCNT (2 bytes) and four scroll pairs (4 bytes).
pub const BG_REGS_BYTES: u32 = ENGINE_BG_REGS_BYTES;
/// BG2PA, at `0400_0020h`: the transform registers of BG2 and BG3, 32 bytes from here
/// (`bg-modes.md`, "Registers").
pub const AFFINE_REGS_ADDR: u32 = DISPCNT_A_ADDR + AFFINE_OFFSET;
/// Engine B's transform registers, at `0400_1020h`.
pub const AFFINE_REGS_B_ADDR: u32 = AFFINE_REGS_ADDR + ENGINE_B_OFFSET;
/// The bytes of that block: PA-PD, BGxX and BGxY of two layers.
pub const AFFINE_REGS_BYTES: u32 = ENGINE_AFFINE_REGS_BYTES;
/// The xRGB8888 value of a white pixel.
pub const WHITE: u32 = 0x00FF_FFFF;
/// The xRGB8888 value of a black pixel.
pub const BLACK: u32 = 0;

/// DISPCNT bits 16-17: the display mode.
const DISPCNT_MODE_SHIFT: u32 = 16;
const DISPCNT_MODE_MASK: u32 = 0x3;
/// Display mode 0: display off, a white screen.
const MODE_OFF: u32 = 0;
/// Display mode 1: the layers (here, the text backgrounds) over the backdrop.
const MODE_GRAPHICS: u32 = 1;
/// Display mode 2: the bitmap read from a VRAM block.
const MODE_VRAM: u32 = 2;
/// DISPCNT bits 18-19: the VRAM block mode 2 reads.
const DISPCNT_BLOCK_SHIFT: u32 = 18;
const DISPCNT_BLOCK_MASK: u32 = 0x3;
/// POWCNT1 bit 9: engine B (2D) power.
const POWCNT1_ENGINE_B: u16 = 1 << 9;
/// POWCNT1 bit 15: display swap. Set sends engine A to the upper screen.
const POWCNT1_SWAP: u16 = 1 << 15;
/// POWCNT1 after direct boot: LCD, both 2D engines and both 3D units on, engine A on top.
// TODO(hw-unverified): a choice made for lack of evidence; the smoke golden depends on bit 15
// (`display-mode-vram.md`, "Unverified").
const POWCNT1_RESET: u16 = 0x820F;

/// Colour channel mask: five bits per channel in the 15-bit colour.
const CHANNEL_MASK: u16 = 0x1F;
/// Bit offsets of green and blue in the 15-bit colour; red is at bit 0.
const GREEN_SHIFT: u32 = 5;
const BLUE_SHIFT: u32 = 10;

/// One of the two 2D engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineId {
    /// The main engine, registers at `0400_0000h`.
    A,
    /// The sub engine, registers at `0400_1000h`.
    B,
}

impl EngineId {
    /// What this engine lacks (`bg-modes.md`, "Engine B").
    const fn spec(self) -> &'static EngineSpec {
        match self {
            Self::A => &MAIN_SPEC,
            Self::B => &SUB_SPEC,
        }
    }
}

/// Both engines' registers and POWCNT1 (`display-mode-vram.md`, "Registers").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu2d {
    a: Engine,
    b: Engine,
    powcnt1: u16,
}

/// Where an offset in the register space of [`Gpu2d`] lands.
#[derive(Clone, Copy)]
enum Route {
    /// This byte of POWCNT1.
    Powcnt1(u32),
    /// This offset in the register block of an engine.
    Engine(EngineId, u32),
}

/// Widens a 5-bit channel to 8 bits by repeating its top bits. A host-side choice, not a
/// hardware fact (`display-mode-vram.md`, "Colour expansion"; #68).
fn expand_channel(channel: u16) -> u32 {
    let c = u32::from(channel & CHANNEL_MASK);
    (c << 3) | (c >> 2)
}

/// A 15-bit colour as xRGB8888. Bit 15 is not part of the colour.
fn expand_color(halfword: u16) -> u32 {
    (expand_channel(halfword) << 16)
        | (expand_channel(halfword >> GREEN_SHIFT) << 8)
        | expand_channel(halfword >> BLUE_SHIFT)
}

/// `reg` with its byte `index` replaced by `value`.
fn with_byte(reg: u32, index: u32, value: u8) -> u32 {
    let shift = index * 8;
    (reg & !(0xFF << shift)) | (u32::from(value) << shift)
}

impl Gpu2d {
    /// Registers in their power-on state.
    // TODO(hw-unverified): POWCNT1 starts at `820Fh`, not stated by GBATEK
    // (`display-mode-vram.md`, "Unverified"); the engines' registers start at 0 likewise.
    pub fn new() -> Self {
        Self {
            a: Engine::new(),
            b: Engine::new(),
            powcnt1: POWCNT1_RESET,
        }
    }

    fn engine(&self, which: EngineId) -> &Engine {
        match which {
            EngineId::A => &self.a,
            EngineId::B => &self.b,
        }
    }

    /// Layer `layer`'s registers decoded (`layer` is taken modulo 4) as engine `which` sees them. A
    /// debugger's BG panel (#58) shows it.
    pub fn text_bg(&self, which: EngineId, layer: usize) -> TextBg {
        let engine = self.engine(which);
        engine.text_bg(which.spec().dispcnt(engine.dispcnt()), layer)
    }

    /// Affine layer `layer` (2 or 3; others are taken modulo 2) as engine `which` sees it, with
    /// its internal reference point. A debugger's BG panel (#58) shows it.
    pub fn affine_bg(&self, which: EngineId, layer: usize) -> AffineBg {
        let engine = self.engine(which);
        engine.affine_bg(which.spec().dispcnt(engine.dispcnt()), layer)
    }

    /// A visible line has been drawn: both engines' affine reference points step down one line
    /// (`bg-modes.md`, "When the picture is made").
    pub(crate) fn end_line(&mut self) {
        self.a.end_line();
        self.b.end_line();
    }

    /// VBlank starts: both engines' affine reference points are loaded from their registers.
    pub(crate) fn vblank_started(&mut self) {
        self.a.vblank_started();
        self.b.vblank_started();
    }

    /// DISPCNT of engine A.
    pub fn dispcnt_a(&self) -> u32 {
        self.a.dispcnt()
    }

    /// DISPCNT of engine B.
    pub fn dispcnt_b(&self) -> u32 {
        self.b.dispcnt()
    }

    /// POWCNT1.
    pub fn powcnt1(&self) -> u16 {
        self.powcnt1
    }

    /// Whether POWCNT1 bit 15 sends engine A to the upper screen.
    // TODO(hw-unverified): bits 0 (LCD enable) and 1 (engine A enable) have no effect on the
    // picture (`display-mode-vram.md`, "Unverified").
    // TODO(phase7): the other POWCNT1 bits arrive with the 3D units.
    pub fn engine_a_on_top(&self) -> bool {
        self.powcnt1 & POWCNT1_SWAP != 0
    }

    /// Whether POWCNT1 bit 9 powers engine B (`bg-modes.md`, "Engine B").
    pub fn engine_b_on(&self) -> bool {
        self.powcnt1 & POWCNT1_ENGINE_B != 0
    }

    /// Where `offset` in the register space (which starts at DISPCNT of engine A) lands.
    fn route(offset: u32) -> Route {
        let powcnt1_index = offset.wrapping_sub(POWCNT1_OFFSET);
        if powcnt1_index < 2 {
            Route::Powcnt1(powcnt1_index)
        } else if offset >= ENGINE_B_OFFSET {
            Route::Engine(EngineId::B, offset - ENGINE_B_OFFSET)
        } else {
            Route::Engine(EngineId::A, offset)
        }
    }

    /// The byte at `offset` in the register space; 0 where no register is.
    fn read_byte(&self, offset: u32) -> u8 {
        match Self::route(offset) {
            Route::Powcnt1(byte) => (self.powcnt1 >> (byte * 8)) as u8,
            Route::Engine(which, at) => self.engine(which).read_byte(at),
        }
    }

    /// Writes one byte of a register; an offset that is no register is not ours and is ignored,
    /// and so is a write to engine B's registers while POWCNT1 bit 9 is clear: the ports of a
    /// powered-off engine are read-only (`bg-modes.md`, "Engine B").
    // TODO(hw-unverified): POWCNT1's undefined bits are kept as written
    // (`display-mode-vram.md`, "Unverified").
    fn write_byte(&mut self, offset: u32, value: u8) {
        match Self::route(offset) {
            Route::Powcnt1(byte) => {
                self.powcnt1 = with_byte(u32::from(self.powcnt1), byte, value) as u16;
            }
            Route::Engine(EngineId::A, at) => self.a.write_byte(at, value),
            Route::Engine(EngineId::B, at) => {
                if self.engine_b_on() {
                    self.b.write_byte(at, value);
                }
            }
        }
    }

    /// Draws line `y` (0-191) of engine A's picture into `out` (`SCREEN_WIDTH` pixels), given the
    /// bytes of banks A-D where the CPU sees them as LCDC VRAM and what the engine reads
    /// ([`EngineInputs`]). A line past the last visible one is not drawn.
    // TODO(phase4): mode 1 completes (OBJ, windows, blending, extended palettes), and mode 3
    // draws the main-memory bitmap.
    // TODO(phase4): master brightness applies to the output of modes 1-3 (#34).
    // TODO(hw-unverified): DISPCNT bit 7 (forced blank) is stored but ignored
    // (`display-mode-vram.md`, "Unverified").
    // TODO(hw-unverified): a bank that is disabled or not in LCDC mode is drawn as zeros, i.e.
    // black (`display-mode-vram.md`, "Unverified").
    pub(crate) fn render_line_a(
        &self,
        y: usize,
        lcdc: [Option<&[u8]>; 4],
        inputs: &EngineInputs<'_>,
        out: &mut [u32],
    ) {
        if y >= SCREEN_HEIGHT {
            return;
        }
        let dispcnt = inputs.spec.dispcnt(self.a.dispcnt());
        let mode = (dispcnt >> DISPCNT_MODE_SHIFT) & DISPCNT_MODE_MASK;
        let block = (dispcnt >> DISPCNT_BLOCK_SHIFT) & DISPCNT_BLOCK_MASK;
        if mode == MODE_OFF {
            out.fill(WHITE);
        } else if mode == MODE_GRAPHICS {
            self.a.render_line(y, inputs, out);
        } else if mode == MODE_VRAM {
            out.fill(BLACK);
            let row = lcdc
                .get(block as usize)
                .copied()
                .flatten()
                .and_then(|bank| bank.get(y * SCREEN_WIDTH * 2..(y + 1) * SCREEN_WIDTH * 2))
                .unwrap_or(&[]);
            let (pairs, _) = row.as_chunks::<2>();
            out.iter_mut().zip(pairs).for_each(|(pixel, &bytes)| {
                *pixel = expand_color(u16::from_le_bytes(bytes));
            });
        } else {
            out.fill(BLACK);
        }
    }

    /// Draws line `y` of engine B's picture into `out`. Engine B has the display modes 0 (white)
    /// and 1 (the layers) only; a powered-off engine is black (`bg-modes.md`, "Engine B").
    // TODO(hw-unverified): what the screen of a powered-off engine shows (`bg-modes.md`,
    // "Unverified").
    pub(crate) fn render_line_b(&self, y: usize, inputs: &EngineInputs<'_>, out: &mut [u32]) {
        if y >= SCREEN_HEIGHT {
            return;
        }
        let dispcnt = inputs.spec.dispcnt(self.b.dispcnt());
        let mode = (dispcnt >> DISPCNT_MODE_SHIFT) & DISPCNT_MODE_MASK;
        if !self.engine_b_on() {
            out.fill(BLACK);
        } else if mode == MODE_GRAPHICS {
            self.b.render_line(y, inputs, out);
        } else {
            out.fill(WHITE);
        }
    }
}

/// Both engines are wired to the ARM9 alone (`AGENTS.md` §4, "Per-CPU devices"). Their registers
/// are bytes; a wider access is split into them.
impl Device<true> for Gpu2d {
    fn peek(&self, offset: u32, width: Width, _now: u64) -> u32 {
        read_bytes(offset, width, |at| self.read_byte(at))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, _now: u64) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte(at, byte);
            Effect::default()
        })
    }
}

impl Default for Gpu2d {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
