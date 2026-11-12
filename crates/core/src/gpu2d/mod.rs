// SPDX-License-Identifier: Apache-2.0
//! Graphics engine A's display registers and its picture: display off, the VRAM display
//! (`docs/hw/display-mode-vram.md`) and the text backgrounds (`docs/hw/bg-text.md`).
//!
//! The picture is made one line at a time ([`Gpu2d::render_line_a`]), from the registers and
//! memory as they are when the line is asked for. No allocation, formatting or collection
//! happens while pixels are made.
//!
//! Engine A is wired to the ARM9 alone, so `Gpu2d` is a plain type with no CPU const generic: the
//! "seen by one CPU" class of `AGENTS.md` §4 ("Per-CPU devices").

mod text;

use crate::bus::BgVram;
use crate::io::device::{Device, Effect, Width, read_bytes, write_bytes};
use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};
use serde::{Deserialize, Serialize};
pub use text::TextBg;

/// DISPCNT of engine A, at `0400_0000h` (`display-mode-vram.md`, "Registers").
pub const DISPCNT_A_ADDR: u32 = 0x0400_0000;
/// POWCNT1, at `0400_0304h` (`display-mode-vram.md`, "Registers").
pub const POWCNT1_ADDR: u32 = 0x0400_0304;
/// POWCNT1's offset in the register space of [`Gpu2d`], which starts at DISPCNT.
const POWCNT1_OFFSET: u32 = POWCNT1_ADDR - DISPCNT_A_ADDR;
/// BG0CNT, at `0400_0008h`: BG0CNT-BG3CNT, then BG0HOFS, BG0VOFS, BG1HOFS, ... BG3VOFS
/// (`bg-text.md`, "Registers"). They are one block of the map, 24 bytes from here.
pub const BG_REGS_ADDR: u32 = 0x0400_0008;
/// The bytes of that block: four BGxCNT (2 bytes) and four scroll pairs (4 bytes).
pub const BG_REGS_BYTES: u32 = 24;
/// The offsets, in the register space of [`Gpu2d`], of the BGxCNT run and of the scroll run.
const BGCNT_OFFSET: u32 = BG_REGS_ADDR - DISPCNT_A_ADDR;
const SCROLL_OFFSET: u32 = BGCNT_OFFSET + 8;
const SCROLL_END: u32 = BGCNT_OFFSET + BG_REGS_BYTES;
/// The scroll registers hold 9 bits (`bg-text.md`, "BGxHOFS / BGxVOFS").
const SCROLL_MASK: u32 = 0x1FF;
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

/// Engine A's display registers (`display-mode-vram.md`, "Registers").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu2d {
    dispcnt_a: u32,
    powcnt1: u16,
    // BG0CNT-BG3CNT and the nine-bit scrolls of BG0-BG3 (`bg-text.md`, "Registers").
    bgcnt: [u16; 4],
    bghofs: [u16; 4],
    bgvofs: [u16; 4],
}

/// A register of [`Gpu2d`] and which byte of it an offset falls on.
#[derive(Clone, Copy)]
enum Field {
    Dispcnt,
    Powcnt1,
    BgCnt(usize),
    BgHofs(usize),
    BgVofs(usize),
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
    // TODO(hw-unverified): DISPCNT starts at 0 and POWCNT1 at `820Fh`, neither stated by
    // GBATEK (`display-mode-vram.md`, "Unverified"); BGxCNT and the scrolls start at 0 likewise
    // (`bg-text.md`, "Unverified").
    pub fn new() -> Self {
        Self {
            dispcnt_a: 0,
            powcnt1: POWCNT1_RESET,
            bgcnt: [0; 4],
            bghofs: [0; 4],
            bgvofs: [0; 4],
        }
    }

    /// Layer `layer`'s registers decoded (`layer` is taken modulo 4). A debugger's BG panel (#58)
    /// shows it.
    pub fn text_bg(&self, layer: usize) -> TextBg {
        let n = layer & 3;
        TextBg::decode(
            self.dispcnt_a,
            self.bgcnt[n],
            self.bghofs[n],
            self.bgvofs[n],
        )
    }

    /// DISPCNT of engine A.
    pub fn dispcnt_a(&self) -> u32 {
        self.dispcnt_a
    }

    /// POWCNT1.
    pub fn powcnt1(&self) -> u16 {
        self.powcnt1
    }

    /// Whether POWCNT1 bit 15 sends engine A to the upper screen.
    // TODO(hw-unverified): bits 0 (LCD enable) and 1 (engine A enable) have no effect on the
    // picture (`display-mode-vram.md`, "Unverified").
    // TODO(phaseN): the other POWCNT1 bits arrive with their units (3D: phase 7, engine B:
    // phase 4).
    pub fn engine_a_on_top(&self) -> bool {
        self.powcnt1 & POWCNT1_SWAP != 0
    }

    /// The register that holds `offset` and the byte of it that `offset` is.
    fn locate(offset: u32) -> Option<(Field, u32)> {
        let powcnt1_index = offset.wrapping_sub(POWCNT1_OFFSET);
        if offset < 4 {
            Some((Field::Dispcnt, offset))
        } else if (BGCNT_OFFSET..SCROLL_OFFSET).contains(&offset) {
            let at = offset - BGCNT_OFFSET;
            Some((Field::BgCnt((at / 2) as usize), at % 2))
        } else if (SCROLL_OFFSET..SCROLL_END).contains(&offset) {
            // Each layer has HOFS in the first halfword of its four bytes and VOFS in the second.
            let at = offset - SCROLL_OFFSET;
            let layer = (at / 4) as usize;
            Some(match at % 4 {
                byte @ 0..=1 => (Field::BgHofs(layer), byte),
                byte => (Field::BgVofs(layer), byte - 2),
            })
        } else if powcnt1_index < 2 {
            Some((Field::Powcnt1, powcnt1_index))
        } else {
            None
        }
    }

    /// The byte at `offset` in the register space, which starts at DISPCNT; 0 where no register
    /// is.
    // TODO(hw-unverified): the scroll registers are write-only and read 0 (`bg-text.md`,
    // "Unverified").
    fn read_byte(&self, offset: u32) -> u8 {
        let Some((field, byte)) = Self::locate(offset) else {
            return 0;
        };
        let value = match field {
            Field::Dispcnt => self.dispcnt_a,
            Field::Powcnt1 => u32::from(self.powcnt1),
            Field::BgCnt(n) => u32::from(self.bgcnt[n & 3]),
            Field::BgHofs(_) | Field::BgVofs(_) => 0,
        };
        (value >> (byte * 8)) as u8
    }

    /// Writes one byte of a register; an offset that is no register is not ours and is ignored.
    // TODO(hw-unverified): POWCNT1's undefined bits are kept as written
    // (`display-mode-vram.md`, "Unverified"); BGxCNT bit 13 is kept as written
    // (`bg-text.md`, "Unverified").
    fn write_byte(&mut self, offset: u32, value: u8) {
        let Some((field, byte)) = Self::locate(offset) else {
            return;
        };
        match field {
            Field::Dispcnt => self.dispcnt_a = with_byte(self.dispcnt_a, byte, value),
            Field::Powcnt1 => self.powcnt1 = with_byte(u32::from(self.powcnt1), byte, value) as u16,
            Field::BgCnt(n) => {
                self.bgcnt[n & 3] = with_byte(u32::from(self.bgcnt[n & 3]), byte, value) as u16;
            }
            Field::BgHofs(n) => {
                self.bghofs[n & 3] =
                    (with_byte(u32::from(self.bghofs[n & 3]), byte, value) & SCROLL_MASK) as u16;
            }
            Field::BgVofs(n) => {
                self.bgvofs[n & 3] =
                    (with_byte(u32::from(self.bgvofs[n & 3]), byte, value) & SCROLL_MASK) as u16;
            }
        }
    }

    /// Draws line `y` (0-191) of engine A's picture into `out` (`SCREEN_WIDTH` pixels), given the
    /// bytes of banks A-D where the CPU sees them as LCDC VRAM, engine A's BG VRAM, and the
    /// palette RAM. A line past the last visible one is not drawn.
    // TODO(phase4): mode 1 completes (OBJ, affine and extended layers, windows, blending, extended
    // palettes, mosaic), and mode 3 draws the main-memory bitmap.
    // TODO(phase4): master brightness applies to the output of modes 1-3 (#34).
    // TODO(hw-unverified): DISPCNT bit 7 (forced blank) is stored but ignored
    // (`display-mode-vram.md`, "Unverified").
    // TODO(hw-unverified): a bank that is disabled or not in LCDC mode is drawn as zeros, i.e.
    // black (`display-mode-vram.md`, "Unverified").
    pub(crate) fn render_line_a(
        &self,
        y: usize,
        lcdc: [Option<&[u8]>; 4],
        vram: &BgVram<'_>,
        palette: &[u8],
        out: &mut [u32],
    ) {
        if y >= SCREEN_HEIGHT {
            return;
        }
        let mode = (self.dispcnt_a >> DISPCNT_MODE_SHIFT) & DISPCNT_MODE_MASK;
        let block = (self.dispcnt_a >> DISPCNT_BLOCK_SHIFT) & DISPCNT_BLOCK_MASK;
        if mode == MODE_OFF {
            out.fill(WHITE);
        } else if mode == MODE_GRAPHICS {
            self.render_graphics_line(y, vram, palette, out);
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

    /// Display mode 1, line `y`: each pixel is the front-most text layer that is not transparent
    /// there, or the backdrop, palette entry 0 (`bg-text.md`, "Putting the layers together").
    fn render_graphics_line(&self, y: usize, vram: &BgVram<'_>, palette: &[u8], out: &mut [u32]) {
        let palette = palette.get(..text::BG_PALETTE_BYTES).unwrap_or(palette);
        let layers: [TextBg; 4] = std::array::from_fn(|n| self.text_bg(n));
        // The layer in front is the one with the lowest priority value, and among equals the lower
        // number: sorting by (priority, number) puts the front-most first.
        let mut order = [0usize, 1, 2, 3];
        order.sort_unstable_by_key(|&n| (layers[n].priority, n));
        let backdrop = expand_color(text::palette_color(palette, 0));
        out.iter_mut().enumerate().for_each(|(x, pixel)| {
            *pixel = order
                .iter()
                .filter(|&&n| text::is_drawn(self.dispcnt_a, n))
                .find_map(|&n| text::pixel(&layers[n], vram, palette, x as u32, y as u32))
                .map_or(backdrop, expand_color);
        });
    }
}

/// Engine A is wired to the ARM9 alone (`AGENTS.md` §4, "Per-CPU devices"). Its registers are
/// bytes; a wider access is split into them.
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
