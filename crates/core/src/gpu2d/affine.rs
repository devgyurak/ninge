// SPDX-License-Identifier: Apache-2.0
//! Affine layers (BG2 and BG3): the transform registers, the internal reference point, and what
//! one layer shows at one pixel (`docs/hw/bg-modes.md`, "Affine layers"). Pure functions of the
//! registers and the bytes they point at; nothing is allocated, formatted or collected.

use super::text::palette_color;
use crate::bus::VramWindow;
use serde::{Deserialize, Serialize};

/// BGxCNT bits 0-1: priority.
const CNT_PRIORITY_MASK: u16 = 0x3;
/// BGxCNT bits 2-5: the character base block, in 16 KiB units.
const CNT_CHAR_SHIFT: u32 = 2;
const CNT_CHAR_MASK: u16 = 0xF;
const CHAR_BLOCK_SIZE: u32 = 16 << 10;
/// BGxCNT bits 8-12: the screen base block, in 2 KiB units.
const CNT_SCREEN_SHIFT: u32 = 8;
const CNT_SCREEN_MASK: u16 = 0x1F;
const SCREEN_BLOCK_SIZE: u32 = 2 << 10;
/// BGxCNT bit 13: area overflow, 0 = transparent outside the layer, 1 = wrap around.
const CNT_WRAP: u16 = 1 << 13;
/// BGxCNT bits 14-15: the size.
const CNT_SIZE_SHIFT: u32 = 14;
/// The smallest affine layer is 128 pixels square; each size step doubles it.
const BASE_SIZE: u32 = 128;
/// DISPCNT bits 24-26 and 27-29: the character and screen base, in 64 KiB units.
const DISPCNT_CHAR_SHIFT: u32 = 24;
const DISPCNT_SCREEN_SHIFT: u32 = 27;
const DISPCNT_BASE_MASK: u32 = 0x7;
const DISPCNT_BASE_SIZE: u32 = 64 << 10;
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

/// An affine layer's registers, decoded (`bg-modes.md`, "Affine layers"). A debugger's BG panel
/// (#58) shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AffineBg {
    /// Bits 0-1 of BGxCNT; 0 is in front.
    pub priority: u8,
    /// Where the tile data starts, counted from the start of the engine's BG VRAM.
    pub char_base: u32,
    /// Where the map starts, counted from the start of the engine's BG VRAM.
    pub screen_base: u32,
    /// The layer's width and height in pixels: 128, 256, 512 or 1024.
    pub size: u16,
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
    /// engine sees it.
    pub(super) fn decode(dispcnt: u32, cnt: u16, regs: &Affine) -> Self {
        let char_base = ((dispcnt >> DISPCNT_CHAR_SHIFT) & DISPCNT_BASE_MASK) * DISPCNT_BASE_SIZE
            + u32::from((cnt >> CNT_CHAR_SHIFT) & CNT_CHAR_MASK) * CHAR_BLOCK_SIZE;
        let screen_base = ((dispcnt >> DISPCNT_SCREEN_SHIFT) & DISPCNT_BASE_MASK)
            * DISPCNT_BASE_SIZE
            + u32::from((cnt >> CNT_SCREEN_SHIFT) & CNT_SCREEN_MASK) * SCREEN_BLOCK_SIZE;
        Self {
            priority: (cnt & CNT_PRIORITY_MASK) as u8,
            char_base,
            screen_base,
            size: (BASE_SIZE << (cnt >> CNT_SIZE_SHIFT)) as u16,
            wrap: cnt & CNT_WRAP != 0,
            params: regs.params,
            reference: [sign_extend_28(regs.x), sign_extend_28(regs.y)],
            internal: [regs.ref_x, regs.ref_y],
        }
    }
}

/// The 15-bit colour of layer `layer` at screen pixel column `x` of the line being drawn, or
/// `None` where it is transparent (`bg-modes.md`, "Affine layers").
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
    let size = i32::from(layer.size);
    let inside = (0..size).contains(&tx) && (0..size).contains(&ty);
    if !inside && !layer.wrap {
        return None;
    }
    // The size is a power of two, so the mask is the wrap (also for a negative coordinate).
    let (tx, ty) = (tx & (size - 1), ty & (size - 1));
    let cell = (ty / TILE_SIZE) * (size / TILE_SIZE) + tx / TILE_SIZE;
    let tile = u32::from(vram.byte(layer.screen_base + cell as u32));
    let at = layer.char_base
        + tile * TILE_BYTES
        + (ty & TILE_MASK) as u32 * TILE_SIZE as u32
        + (tx & TILE_MASK) as u32;
    match vram.byte(at) {
        0 => None,
        value => Some(palette_color(palette, u32::from(value))),
    }
}
