// SPDX-License-Identifier: Apache-2.0
//! Affine and extended layers (BG2 and BG3): the transform registers, the internal reference
//! point, and what one layer shows at one pixel (`docs/hw/bg-modes.md`, "Affine layers",
//! "Extended layers"). All of them walk the layer along the same transform and differ in what a
//! texel is. Pure functions of the registers and the bytes they point at; nothing is allocated,
//! formatted or collected.

use super::text::{
    CHAR_BLOCK_SIZE, CNT_CHAR_MASK, CNT_CHAR_SHIFT, CNT_PRIORITY_MASK, CNT_SCREEN_MASK,
    CNT_SCREEN_SHIFT, CNT_SIZE_SHIFT, DISPCNT_BASE_MASK, DISPCNT_BASE_SIZE, DISPCNT_CHAR_SHIFT,
    DISPCNT_SCREEN_SHIFT, SCREEN_BLOCK_SIZE, palette_color,
};
use crate::bus::VramWindow;
use serde::{Deserialize, Serialize};

/// BGxCNT bit 2 and bit 7 select the extended format (`bg-modes.md`, "Extended layers"): bit 7
/// clear is a tile layer with 16-bit entries; with it set, bit 2 picks the direct-colour bitmap.
const CNT_BITMAP: u16 = 1 << 7;
const CNT_DIRECT: u16 = 1 << 2;
/// BGxCNT bits 8-12 of a bitmap layer count in 16 KiB units (`bg-modes.md`, "Sizes and where the
/// data is").
const BITMAP_BLOCK_SIZE: u32 = 16 << 10;
/// A map entry's tile number, flips, and the colour channel of a direct-colour pixel.
const ENTRY_TILE_MASK: u16 = 0x3FF;
const ENTRY_H_FLIP: u16 = 1 << 10;
const ENTRY_V_FLIP: u16 = 1 << 11;
/// Direct-colour bit 15: the pixel is shown (`bg-modes.md`, "Extended layers").
const DIRECT_OPAQUE: u16 = 1 << 15;
/// BGxCNT bit 13: area overflow, 0 = transparent outside the layer, 1 = wrap around.
const CNT_WRAP: u16 = 1 << 13;
/// The smallest affine layer is 128 pixels square; each size step doubles it.
const BASE_SIZE: u32 = 128;
/// Pixels in a tile row, and the mask for a position inside a tile.
const TILE_SIZE: i32 = 8;
const TILE_MASK: i32 = 7;
/// Bytes a tile takes: 8 bits per pixel.
const TILE_BYTES: u32 = 64;
/// The reference point registers hold 28 bits (`bg-modes.md`, "Reference point and parameters").
const REF_MASK: u32 = 0x0FFF_FFFF;
/// Fraction bits of the 20.8 reference point and of PA-PD (8.8).
const FRACTION_BITS: u32 = 8;

/// Offsets inside one layer's 16 bytes of registers: PA, PB, PC, PD, then X and Y.
const PARAMS_END: u32 = 8;
const X_END: u32 = 12;

/// One affine layer's registers and its internal reference point (`bg-modes.md`, "Registers").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Affine {
    /// PA, PB, PC, PD in that order, signed 8.8.
    params: [i16; 4],
    /// BGxX and BGxY as written, 28 bits.
    x: u32,
    y: u32,
    /// The internal reference point: where the line being drawn starts in the layer, 20.8.
    ref_x: i32,
    ref_y: i32,
}

/// A 28-bit two's-complement number as an `i32`.
fn sign_extend_28(value: u32) -> i32 {
    ((value << 4) as i32) >> 4
}

impl Affine {
    /// Registers in their power-on state.
    // TODO(hw-unverified): every register starts at 0 (`bg-modes.md`, "Unverified").
    pub(super) fn new() -> Self {
        Self {
            params: [0; 4],
            x: 0,
            y: 0,
            ref_x: 0,
            ref_y: 0,
        }
    }

    /// Writes byte `at` (0-15) of the layer's registers. A write to X or Y loads that internal
    /// coordinate at once (`bg-modes.md`, "Affine layers"): the line drawn next starts from it.
    pub(super) fn write_byte(&mut self, at: u32, value: u8) {
        let byte = at % 2;
        if at < PARAMS_END {
            let param = &mut self.params[(at / 2) as usize];
            *param = super::with_byte(u32::from(*param as u16), byte, value) as u16 as i16;
        } else if at < X_END {
            self.x = super::with_byte(self.x, at - PARAMS_END, value) & REF_MASK;
            self.ref_x = sign_extend_28(self.x);
        } else {
            self.y = super::with_byte(self.y, at - X_END, value) & REF_MASK;
            self.ref_y = sign_extend_28(self.y);
        }
    }

    /// The internal reference point moves down one line: `+= PB`, `+= PD`.
    // TODO(hw-unverified): the point adds at 32 bits, wrapping (`bg-modes.md`, "Unverified").
    pub(super) fn end_line(&mut self) {
        self.ref_x = self.ref_x.wrapping_add(i32::from(self.params[1]));
        self.ref_y = self.ref_y.wrapping_add(i32::from(self.params[3]));
    }

    /// The internal reference point is loaded from BGxX and BGxY: at the start of VBlank.
    pub(super) fn reload(&mut self) {
        self.ref_x = sign_extend_28(self.x);
        self.ref_y = sign_extend_28(self.y);
    }
}

/// How a layer's texels are stored (`bg-modes.md`, "Affine layers", "Extended layers").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AffineFormat {
    /// Tiles of 256 colours with one-byte map entries (an affine layer).
    Tiles8,
    /// Tiles of 256 colours with 16-bit map entries: tile, flips, palette number.
    Tiles16,
    /// A bitmap of one byte a pixel, through the palette; value 0 is transparent.
    Bitmap256,
    /// A bitmap of 15-bit colours, bit 15 set for a shown pixel.
    BitmapDirect,
}

/// An affine or extended layer's registers, decoded (`bg-modes.md`, "Affine layers", "Extended
/// layers"). A debugger's BG panel (#58) shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AffineBg {
    /// Bits 0-1 of BGxCNT; 0 is in front.
    pub priority: u8,
    /// How the texels are stored.
    pub format: AffineFormat,
    /// Where the tile data starts, counted from the start of the engine's BG VRAM (the tile
    /// formats only).
    pub char_base: u32,
    /// Where the map (tile formats) or the bitmap (bitmap formats) starts, counted from the start
    /// of the engine's BG VRAM.
    pub screen_base: u32,
    /// The layer's width in pixels: 128 to 1024.
    pub width: u16,
    /// The layer's height in pixels: 128 to 1024.
    pub height: u16,
    /// Whether the layer wraps around (BGxCNT bit 13) rather than being transparent outside.
    pub wrap: bool,
    /// PA, PB, PC, PD as signed 8.8 numbers.
    pub params: [i16; 4],
    /// BGxX and BGxY, sign-extended from 28 bits, 20.8.
    pub reference: [i32; 2],
    /// The internal reference point the next line starts from, 20.8.
    pub internal: [i32; 2],
}

impl AffineBg {
    /// Decodes BGxCNT and the transform registers of one layer under `dispcnt`, the DISPCNT as the
    /// engine sees it; `extended` is whether the BG mode makes this layer an extended one.
    pub(super) fn decode(dispcnt: u32, cnt: u16, regs: &Affine, extended: bool) -> Self {
        let format = match (extended, cnt & CNT_BITMAP != 0, cnt & CNT_DIRECT != 0) {
            (false, _, _) => AffineFormat::Tiles8,
            (true, false, _) => AffineFormat::Tiles16,
            (true, true, false) => AffineFormat::Bitmap256,
            (true, true, true) => AffineFormat::BitmapDirect,
        };
        let bitmap = matches!(format, AffineFormat::Bitmap256 | AffineFormat::BitmapDirect);
        let char_base = ((dispcnt >> DISPCNT_CHAR_SHIFT) & DISPCNT_BASE_MASK) * DISPCNT_BASE_SIZE
            + u32::from((cnt >> CNT_CHAR_SHIFT) & CNT_CHAR_MASK) * CHAR_BLOCK_SIZE;
        let block = u32::from((cnt >> CNT_SCREEN_SHIFT) & CNT_SCREEN_MASK);
        // A bitmap does not add the DISPCNT screen base, and counts its block in 16 KiB.
        let screen_base = match bitmap {
            true => block * BITMAP_BLOCK_SIZE,
            false => {
                ((dispcnt >> DISPCNT_SCREEN_SHIFT) & DISPCNT_BASE_MASK) * DISPCNT_BASE_SIZE
                    + block * SCREEN_BLOCK_SIZE
            }
        };
        let size = cnt >> CNT_SIZE_SHIFT;
        // The tile layers are square; a bitmap's size 2 is 512x256 and 3 is 512x512.
        let (width, height) = match (bitmap, size) {
            (true, 2) => (512, 256),
            (true, 3) => (512, 512),
            _ => {
                let side = (BASE_SIZE << size) as u16;
                (side, side)
            }
        };
        Self {
            priority: (cnt & CNT_PRIORITY_MASK) as u8,
            format,
            char_base,
            screen_base,
            width,
            height,
            wrap: cnt & CNT_WRAP != 0,
            params: regs.params,
            reference: [sign_extend_28(regs.x), sign_extend_28(regs.y)],
            internal: [regs.ref_x, regs.ref_y],
        }
    }
}

/// The 15-bit colour of layer `layer` at screen pixel column `x` of the line being drawn, or
/// `None` where it is transparent (`bg-modes.md`, "Affine layers", "Extended layers").
// TODO(phase4): extended palettes (DISPCNT bit 30) for the 16-bit tile format (#32).
// TODO(hw-unverified): the 16-bit tile format reads the standard palette whatever the entry's
// palette number says (`bg-modes.md`, "Unverified").
pub(super) fn pixel(
    layer: &AffineBg,
    vram: &VramWindow<'_>,
    palette: &[u8],
    x: u32,
) -> Option<u16> {
    let step = x as i32;
    // The layer point of this pixel, 20.8; the products wrap rather than panic on a huge PA.
    let u = layer.internal[0].wrapping_add(i32::from(layer.params[0]).wrapping_mul(step));
    let v = layer.internal[1].wrapping_add(i32::from(layer.params[2]).wrapping_mul(step));
    // An arithmetic shift rounds toward minus infinity, as the note says.
    let (tx, ty) = (u >> FRACTION_BITS, v >> FRACTION_BITS);
    let (width, height) = (i32::from(layer.width), i32::from(layer.height));
    let inside = (0..width).contains(&tx) && (0..height).contains(&ty);
    if !inside && !layer.wrap {
        return None;
    }
    // The sizes are powers of two, so the mask is the wrap (also for a negative coordinate).
    let (tx, ty) = (tx & (width - 1), ty & (height - 1));
    match layer.format {
        AffineFormat::Tiles8 => {
            let cell = (ty / TILE_SIZE) * (width / TILE_SIZE) + tx / TILE_SIZE;
            let tile = u32::from(vram.byte(layer.screen_base + cell as u32));
            tile_pixel(layer, vram, palette, tile, tx & TILE_MASK, ty & TILE_MASK)
        }
        AffineFormat::Tiles16 => {
            let cell = (ty / TILE_SIZE) * (width / TILE_SIZE) + tx / TILE_SIZE;
            let entry = vram.half(layer.screen_base + cell as u32 * 2);
            // The flips mirror the pixel inside the tile.
            let column = match entry & ENTRY_H_FLIP != 0 {
                true => TILE_MASK - (tx & TILE_MASK),
                false => tx & TILE_MASK,
            };
            let row = match entry & ENTRY_V_FLIP != 0 {
                true => TILE_MASK - (ty & TILE_MASK),
                false => ty & TILE_MASK,
            };
            let tile = u32::from(entry & ENTRY_TILE_MASK);
            tile_pixel(layer, vram, palette, tile, column, row)
        }
        AffineFormat::Bitmap256 => {
            let at = layer.screen_base + (ty * width + tx) as u32;
            match vram.byte(at) {
                0 => None,
                value => Some(palette_color(palette, u32::from(value))),
            }
        }
        AffineFormat::BitmapDirect => {
            let at = layer.screen_base + (ty * width + tx) as u32 * 2;
            let color = vram.half(at);
            (color & DIRECT_OPAQUE != 0).then_some(color)
        }
    }
}

/// The colour of pixel (`column`, `row`) of 256-colour tile `tile`, or `None` where it is 0.
fn tile_pixel(
    layer: &AffineBg,
    vram: &VramWindow<'_>,
    palette: &[u8],
    tile: u32,
    column: i32,
    row: i32,
) -> Option<u16> {
    let at = layer.char_base + tile * TILE_BYTES + row as u32 * TILE_SIZE as u32 + column as u32;
    match vram.byte(at) {
        0 => None,
        value => Some(palette_color(palette, u32::from(value))),
    }
}
