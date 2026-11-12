// SPDX-License-Identifier: Apache-2.0
//! One 2D engine: its register file and the line it draws (`docs/hw/bg-modes.md`,
//! `docs/hw/bg-text.md`). Engine A and engine B are the same circuit, so there is one type; what
//! differs is data ([`EngineSpec`](super::spec::EngineSpec)) and the memory each reads.

use super::spec::EngineInputs;
use super::text::{self, TextBg};
use super::{expand_color, with_byte};
use serde::{Deserialize, Serialize};

/// The engine's register block starts at DISPCNT; these are offsets from there, the same in
/// both engines. BG0CNT-BG3CNT, then BG0HOFS, BG0VOFS, BG1HOFS, ... BG3VOFS (`bg-text.md`,
/// "Registers").
pub(super) const BGCNT_OFFSET: u32 = 0x08;
const SCROLL_OFFSET: u32 = BGCNT_OFFSET + 8;
/// Bytes of the BG control and scroll block: four BGxCNT (2 bytes) and four scroll pairs (4 bytes).
pub(super) const BG_REGS_BYTES: u32 = 24;
const SCROLL_END: u32 = BGCNT_OFFSET + BG_REGS_BYTES;
/// The scroll registers hold 9 bits (`bg-text.md`, "BGxHOFS / BGxVOFS").
const SCROLL_MASK: u32 = 0x1FF;

/// An engine's registers (`bg-text.md`, "Registers").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Engine {
    dispcnt: u32,
    // BG0CNT-BG3CNT and the nine-bit scrolls of BG0-BG3.
    bgcnt: [u16; 4],
    bghofs: [u16; 4],
    bgvofs: [u16; 4],
}

/// A register of [`Engine`] and which byte of it an offset falls on.
#[derive(Clone, Copy)]
enum Field {
    Dispcnt,
    BgCnt(usize),
    BgHofs(usize),
    BgVofs(usize),
}

impl Engine {
    /// Registers in their power-on state.
    // TODO(hw-unverified): every register starts at 0, which GBATEK does not state
    // (`display-mode-vram.md` and `bg-text.md`, "Unverified").
    pub(super) fn new() -> Self {
        Self {
            dispcnt: 0,
            bgcnt: [0; 4],
            bghofs: [0; 4],
            bgvofs: [0; 4],
        }
    }

    /// DISPCNT as written.
    pub(super) fn dispcnt(&self) -> u32 {
        self.dispcnt
    }

    /// Layer `layer`'s registers decoded (`layer` is taken modulo 4) under `dispcnt`, the DISPCNT
    /// as the engine sees it.
    pub(super) fn text_bg(&self, dispcnt: u32, layer: usize) -> TextBg {
        let n = layer & 3;
        TextBg::decode(dispcnt, self.bgcnt[n], self.bghofs[n], self.bgvofs[n])
    }

    /// The register that holds `offset` and the byte of it that `offset` is.
    fn locate(offset: u32) -> Option<(Field, u32)> {
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
        } else {
            None
        }
    }

    /// The byte at `offset` in the register block; 0 where no register is.
    // TODO(hw-unverified): the scroll registers are write-only and read 0 (`bg-text.md`,
    // "Unverified").
    pub(super) fn read_byte(&self, offset: u32) -> u8 {
        let Some((field, byte)) = Self::locate(offset) else {
            return 0;
        };
        let value = match field {
            Field::Dispcnt => self.dispcnt,
            Field::BgCnt(n) => u32::from(self.bgcnt[n & 3]),
            Field::BgHofs(_) | Field::BgVofs(_) => 0,
        };
        (value >> (byte * 8)) as u8
    }

    /// Writes one byte of a register; an offset that is no register is ignored.
    // TODO(hw-unverified): BGxCNT bit 13 is kept as written (`bg-text.md`, "Unverified").
    pub(super) fn write_byte(&mut self, offset: u32, value: u8) {
        let Some((field, byte)) = Self::locate(offset) else {
            return;
        };
        match field {
            Field::Dispcnt => self.dispcnt = with_byte(self.dispcnt, byte, value),
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

    /// Line `y` of the layers (display mode 1): each pixel is the front-most text layer that is not
    /// transparent there, or the backdrop, palette entry 0 (`bg-text.md`, "Putting the layers
    /// together").
    // TODO(phase4): OBJ, affine and extended layers, windows, blending, extended palettes and
    // mosaic complete the line.
    pub(super) fn render_line(&self, y: usize, inputs: &EngineInputs<'_>, out: &mut [u32]) {
        let dispcnt = inputs.spec.dispcnt(self.dispcnt);
        let palette = inputs
            .palette
            .get(..text::BG_PALETTE_BYTES)
            .unwrap_or(inputs.palette);
        let layers: [TextBg; 4] = std::array::from_fn(|n| self.text_bg(dispcnt, n));
        // The layer in front is the one with the lowest priority value, and among equals the lower
        // number: sorting by (priority, number) puts the front-most first.
        let mut order = [0usize, 1, 2, 3];
        order.sort_unstable_by_key(|&n| (layers[n].priority, n));
        let backdrop = expand_color(text::palette_color(palette, 0));
        out.iter_mut().enumerate().for_each(|(x, pixel)| {
            *pixel = order
                .iter()
                .filter(|&&n| text::is_drawn(dispcnt, n))
                .find_map(|&n| text::pixel(&layers[n], &inputs.vram, palette, x as u32, y as u32))
                .map_or(backdrop, expand_color);
        });
    }
}
