// SPDX-License-Identifier: Apache-2.0
//! One 2D engine: its register file and the line it draws (`docs/hw/bg-modes.md`,
//! `docs/hw/bg-text.md`). Engine A and engine B are the same circuit, so there is one type; what
//! differs is data ([`EngineSpec`](super::spec::EngineSpec)) and the memory each reads.

use super::affine::{self, Affine, AffineBg};
use super::kind::{Kind, drawn_kind, kind_of};
use super::spec::{EngineInputs, EngineSpec};
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
/// BG2PA: the transform registers of BG2 and BG3, 16 bytes each (`bg-modes.md`, "Registers").
pub(super) const AFFINE_OFFSET: u32 = 0x20;
/// MOSAIC, a halfword (`bg-modes.md`, "Registers"); the block of the address map is 4 bytes.
pub(super) const MOSAIC_OFFSET: u32 = 0x4C;
pub(super) const MOSAIC_BLOCK_BYTES: u32 = 4;
/// Bytes of one layer's transform registers, and of both.
const AFFINE_LAYER_BYTES: u32 = 16;
pub(super) const AFFINE_REGS_BYTES: u32 = 2 * AFFINE_LAYER_BYTES;
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
    // BG2's and BG3's transform registers and internal reference points (`bg-modes.md`).
    affine: [Affine; 2],
    // MOSAIC: BG and OBJ horizontal and vertical sizes minus 1, a nibble each.
    mosaic: u16,
}

/// The mosaic block sizes in pixels of an engine (`bg-modes.md`, "Mosaic"): each is the register's
/// field plus 1, so 1 means no mosaic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mosaic {
    /// BG horizontal size, 1-16 pixels.
    pub bg_h: u8,
    /// BG vertical size, 1-16 lines.
    pub bg_v: u8,
    /// OBJ horizontal size, 1-16 pixels (sprites are `TODO(phase4)`, #31).
    pub obj_h: u8,
    /// OBJ vertical size, 1-16 lines.
    pub obj_v: u8,
}

/// A register of [`Engine`] and which byte of it an offset falls on.
#[derive(Clone, Copy)]
enum Field {
    Dispcnt,
    BgCnt(usize),
    BgHofs(usize),
    BgVofs(usize),
    /// A byte of one affine layer's 16 bytes of registers: the layer (0 = BG2, 1 = BG3) and the
    /// byte.
    Affine(usize, u32),
    Mosaic,
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
            affine: [Affine::new(); 2],
            mosaic: 0,
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

    /// Affine, extended or large-bitmap layer `layer` (2 or 3; others are taken modulo 2) decoded for
    /// an engine of `spec`.
    pub(super) fn affine_bg(&self, spec: &EngineSpec, layer: usize) -> AffineBg {
        let dispcnt = spec.dispcnt(self.dispcnt);
        let kind = kind_of(dispcnt, (layer & 1) + 2, spec);
        AffineBg::decode(
            dispcnt,
            self.bgcnt[(layer & 1) + 2],
            &self.affine[layer & 1],
            kind,
        )
    }

    /// The mosaic sizes MOSAIC holds.
    pub(super) fn mosaic(&self) -> Mosaic {
        let size = |shift: u32| ((self.mosaic >> shift) & 0xF) as u8 + 1;
        Mosaic {
            bg_h: size(0),
            bg_v: size(4),
            obj_h: size(8),
            obj_v: size(12),
        }
    }

    /// A visible line has been drawn: both affine layers' internal reference points move down one
    /// line (`bg-modes.md`, "When the picture is made").
    pub(super) fn end_line(&mut self) {
        self.affine.iter_mut().for_each(Affine::end_line);
    }

    /// VBlank starts: both affine layers' internal reference points are loaded from BGxX and BGxY.
    pub(super) fn vblank_started(&mut self) {
        self.affine.iter_mut().for_each(Affine::reload);
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
        } else if (AFFINE_OFFSET..AFFINE_OFFSET + AFFINE_REGS_BYTES).contains(&offset) {
            let at = offset - AFFINE_OFFSET;
            Some((
                Field::Affine((at / AFFINE_LAYER_BYTES) as usize, at % AFFINE_LAYER_BYTES),
                0,
            ))
        } else if (MOSAIC_OFFSET..MOSAIC_OFFSET + 2).contains(&offset) {
            Some((Field::Mosaic, offset - MOSAIC_OFFSET))
        } else {
            None
        }
    }

    /// The byte at `offset` in the register block; 0 where no register is.
    // TODO(hw-unverified): the scroll registers and the transform registers (PA-PD, BGxX, BGxY) are
    // write-only and read 0 (`bg-text.md` and `bg-modes.md`, "Unverified").
    pub(super) fn read_byte(&self, offset: u32) -> u8 {
        let Some((field, byte)) = Self::locate(offset) else {
            return 0;
        };
        let value = match field {
            Field::Dispcnt => self.dispcnt,
            Field::BgCnt(n) => u32::from(self.bgcnt[n & 3]),
            // The transform registers are write-only too (`bg-modes.md`, "Unverified").
            Field::BgHofs(_) | Field::BgVofs(_) | Field::Affine(..) | Field::Mosaic => 0,
        };
        (value >> (byte * 8)) as u8
    }

    /// Writes one byte of a register; an offset that is no register is ignored.
    // TODO(hw-unverified): BGxCNT bit 13 is kept as written (`bg-text.md`, "Unverified"); a BGxX or
    // BGxY write on the line where VBlank reloads the point wins over the reload (`bg-modes.md`,
    // "Unverified").
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
            Field::Affine(n, at) => self.affine[n & 1].write_byte(at, value),
            Field::Mosaic => self.mosaic = with_byte(u32::from(self.mosaic), byte, value) as u16,
        }
    }

    /// Line `y` of the layers (display mode 1): each pixel is the front-most layer that is not
    /// transparent there, or the backdrop, palette entry 0 (`bg-text.md`, "Putting the layers
    /// together").
    // TODO(phase4): OBJ, windows, blending and extended palettes complete the line.
    pub(super) fn render_line(&self, y: usize, inputs: &EngineInputs<'_>, out: &mut [u32]) {
        let dispcnt = inputs.spec.dispcnt(self.dispcnt);
        let palette = inputs
            .palette
            .get(..text::BG_PALETTE_BYTES)
            .unwrap_or(inputs.palette);
        let texts: [TextBg; 4] = std::array::from_fn(|n| self.text_bg(dispcnt, n));
        // TODO(hw-unverified): the block's first pixel is the layer's own pixel at the block's first
        // screen position, a transparent one makes the block transparent, and an affine layer's
        // point does not step within a block, and a BGxX/BGxY write inside a block is crossed by the
        // rewind (`bg-modes.md`, "Unverified").
        // Mosaic: a layer with BGxCNT bit 6 shows, at each pixel, what the first pixel of its block
        // would show; blocks are counted from the top left of the screen (`bg-modes.md`, "Mosaic").
        let mosaic = self.mosaic();
        let (block_w, block_h) = (usize::from(mosaic.bg_h), usize::from(mosaic.bg_v));
        let block_y = y - y % block_h;
        let back = (y - block_y) as i32;
        let affines: [AffineBg; 2] = std::array::from_fn(|n| {
            let mut layer = self.affine_bg(inputs.spec, n + 2);
            if layer.mosaic {
                // The line of the block's first row starts `back` steps of PB and PD earlier.
                let [pb, pd] = [layer.params[1], layer.params[3]].map(i32::from);
                layer.internal = [
                    layer.internal[0].wrapping_sub(back.wrapping_mul(pb)),
                    layer.internal[1].wrapping_sub(back.wrapping_mul(pd)),
                ];
            }
            layer
        });
        // The layer in front is the one with the lowest priority value, and among equals the lower
        // number: sorting by (priority, number) puts the front-most first.
        let mut order = [0usize, 1, 2, 3];
        order.sort_unstable_by_key(|&n| (texts[n].priority, n));
        let backdrop = expand_color(text::palette_color(palette, 0));
        out.iter_mut().enumerate().for_each(|(x, pixel)| {
            // The first column of the pixel's block, for a mosaic layer.
            let block_x = (x - x % block_w) as u32;
            *pixel = order
                .iter()
                .find_map(|&n| match drawn_kind(dispcnt, n, inputs.spec)? {
                    Kind::Text => {
                        let (px, py) = match texts[n].mosaic {
                            true => (block_x, block_y),
                            false => (x as u32, y),
                        };
                        text::pixel(&texts[n], &inputs.vram, palette, px, py as u32)
                    }
                    Kind::Affine | Kind::Extended | Kind::Large => {
                        let layer = &affines[n & 1];
                        let px = if layer.mosaic { block_x } else { x as u32 };
                        affine::pixel(layer, &inputs.vram, palette, px)
                    }
                    Kind::Other => None,
                })
                .map_or(backdrop, expand_color);
        });
    }
}
