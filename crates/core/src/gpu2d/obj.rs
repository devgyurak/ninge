// SPDX-License-Identifier: Apache-2.0
//! Sprites (OBJ): one entry of OAM decoded ([`ObjAttrs`]) and the line of object pixels the
//! engine makes before it puts the background layers behind them (`docs/hw/obj.md`).
//!
//! This step draws plain (non-affine) tile objects. The rest of the object kinds are markers
//! here and come with the next steps of #31.

use super::spec::EngineInputs;
use super::text::{self, BG_PALETTE_BYTES};
use crate::SCREEN_WIDTH;

/// Entries in an engine's OAM, and the bytes of one (`obj.md`, "The object table").
pub const OBJ_COUNT: usize = 128;
const ENTRY_BYTES: usize = 8;

/// attr0 bit 8: the object is affine.
const ATTR0_AFFINE: u16 = 1 << 8;
/// attr0 bit 9: double-size (affine) or disabled (plain).
const ATTR0_DOUBLE_OR_DISABLE: u16 = 1 << 9;
const ATTR0_MODE_SHIFT: u32 = 10;
const ATTR0_MODE_MASK: u16 = 0x3;
/// attr0 bit 12: mosaic.
const ATTR0_MOSAIC: u16 = 1 << 12;
/// attr0 bit 13: 256 colours.
const ATTR0_EIGHT_BPP: u16 = 1 << 13;
const ATTR0_SHAPE_SHIFT: u32 = 14;
/// attr1 bits 0-8: X, a 9-bit signed number.
const ATTR1_X_MASK: u16 = 0x1FF;
/// The sign bit of X, and the value that sign-extends it (`obj.md`, "Where an object is on the
/// screen").
const X_SIGN: u16 = 0x100;
const X_RANGE: i16 = 0x200;
const ATTR1_GROUP_SHIFT: u32 = 9;
const ATTR1_GROUP_MASK: u16 = 0x1F;
const ATTR1_H_FLIP: u16 = 1 << 12;
const ATTR1_V_FLIP: u16 = 1 << 13;
const ATTR1_SIZE_SHIFT: u32 = 14;
const ATTR2_TILE_MASK: u16 = 0x3FF;
const ATTR2_PRIORITY_SHIFT: u32 = 10;
const ATTR2_PRIORITY_MASK: u16 = 0x3;
const ATTR2_PALETTE_SHIFT: u32 = 12;

/// OBJ mode 1: semi-transparent (`TODO(phase4)`, #34). Mode 2 is the OBJ window and 3 a bitmap object.
const MODE_WINDOW: u8 = 2;
const MODE_BITMAP: u8 = 3;
/// Shape 3 is not allowed.
const SHAPE_PROHIBITED: u8 = 3;

/// DISPCNT bit 4: tile OBJ mapping, 1 = 1D.
const DISPCNT_TILE_1D: u32 = 1 << 4;
/// DISPCNT bits 20-21: the 1D boundary is `32 << n` bytes.
const DISPCNT_BOUNDARY_SHIFT: u32 = 20;
const DISPCNT_BOUNDARY_MASK: u32 = 0x3;
const BOUNDARY_BASE: u32 = 32;
/// The 2D matrix of tile numbers is 32 wide (`obj.md`, "A plain tile object").
const MATRIX_WIDTH: u32 = 32;
/// A tile number counts 32 bytes in 2D mapping; a tile is 32 bytes at 4 bits a pixel and 64 at 8.
const TILE_UNIT: u32 = 32;
const TILE_BYTES_4BPP: u32 = 32;
const TILE_BYTES_8BPP: u32 = 64;
const PALETTE_COLORS: u32 = 16;

/// A decoded OAM entry (`obj.md`, "The object table"). A debugger's OAM view (#58) shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjAttrs {
    /// attr0 bits 0-7.
    pub y: u8,
    /// attr1 bits 0-8 as a signed number, -256 to 255.
    pub x: i16,
    /// attr0 bit 8: rotation/scaling.
    pub affine: bool,
    /// attr0 bit 9 of an affine object.
    pub double_size: bool,
    /// attr0 bit 9 of a plain object: not drawn.
    pub disabled: bool,
    /// attr0 bits 10-11: 0 normal, 1 semi-transparent, 2 OBJ window, 3 bitmap.
    pub mode: u8,
    /// attr0 bit 12.
    pub mosaic: bool,
    /// attr0 bit 13: 256 colours.
    pub eight_bpp: bool,
    /// attr0 bits 14-15: 0 square, 1 wide, 2 tall, 3 not allowed.
    pub shape: u8,
    /// attr1 bits 14-15.
    pub size: u8,
    /// attr1 bit 12 of a plain object.
    pub h_flip: bool,
    /// attr1 bit 13 of a plain object.
    pub v_flip: bool,
    /// attr1 bits 9-13 of an affine object: its parameter group.
    pub group: u8,
    /// attr2 bits 0-9.
    pub tile: u16,
    /// attr2 bits 10-11; 0 is in front.
    pub priority: u8,
    /// attr2 bits 12-15.
    pub palette: u8,
}

/// Width and height in pixels by shape and size (`obj.md`, "attr1").
const DIMENSIONS: [[(u32, u32); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
];

impl ObjAttrs {
    /// Entry `index` (taken modulo 128) of an engine's `oam`, its 1 KiB. An entry the slice does
    /// not hold reads as zeros.
    // TODO(hw-unverified): X is a 9-bit signed number; GBATEK says only 0-511 (`obj.md`,
    // "Unverified").
    pub fn decode(oam: &[u8], index: usize) -> Self {
        let at = (index % OBJ_COUNT) * ENTRY_BYTES;
        let half = |offset: usize| {
            oam.get(at + offset..at + offset + 2)
                .and_then(|bytes| bytes.try_into().ok())
                .map_or(0, u16::from_le_bytes)
        };
        let (attr0, attr1, attr2) = (half(0), half(2), half(4));
        let affine = attr0 & ATTR0_AFFINE != 0;
        let x = (attr1 & ATTR1_X_MASK) as i16;
        Self {
            y: attr0 as u8,
            x: if attr1 & X_SIGN != 0 { x - X_RANGE } else { x },
            affine,
            double_size: affine && attr0 & ATTR0_DOUBLE_OR_DISABLE != 0,
            disabled: !affine && attr0 & ATTR0_DOUBLE_OR_DISABLE != 0,
            mode: ((attr0 >> ATTR0_MODE_SHIFT) & ATTR0_MODE_MASK) as u8,
            mosaic: attr0 & ATTR0_MOSAIC != 0,
            eight_bpp: attr0 & ATTR0_EIGHT_BPP != 0,
            shape: (attr0 >> ATTR0_SHAPE_SHIFT) as u8,
            size: (attr1 >> ATTR1_SIZE_SHIFT) as u8,
            h_flip: !affine && attr1 & ATTR1_H_FLIP != 0,
            v_flip: !affine && attr1 & ATTR1_V_FLIP != 0,
            group: if affine {
                ((attr1 >> ATTR1_GROUP_SHIFT) & ATTR1_GROUP_MASK) as u8
            } else {
                0
            },
            tile: attr2 & ATTR2_TILE_MASK,
            priority: ((attr2 >> ATTR2_PRIORITY_SHIFT) & ATTR2_PRIORITY_MASK) as u8,
            palette: (attr2 >> ATTR2_PALETTE_SHIFT) as u8,
        }
    }

    /// The object's own width and height in pixels, or `None` for shape 3, which is not allowed.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        DIMENSIONS
            .get(usize::from(self.shape))
            .and_then(|row| row.get(usize::from(self.size)))
            .copied()
    }
}

/// One pixel of the object layer: the 15-bit colour and the priority of the object that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ObjPixel {
    pub(super) color: u16,
    pub(super) priority: u8,
}

/// The object layer of one line; `None` where no object has a pixel.
pub(super) type ObjLine = [Option<ObjPixel>; SCREEN_WIDTH];

/// The line `y` of the object layer under `dispcnt` (`obj.md`, "Behavior"). Objects are walked in
/// entry order; at a column the object with the lowest priority value wins, and among equals the
/// lower entry (`obj.md`, "Priority").
// TODO(phase4): affine objects, bitmap objects, mosaic and the line limit are the next steps of #31;
// semi-transparent objects draw as normal ones (#34), the OBJ window is not drawn (#33) and
// extended palettes are not read (#32).
pub(super) fn draw_line(inputs: &EngineInputs<'_>, dispcnt: u32, y: usize) -> ObjLine {
    let mut line: ObjLine = [None; SCREEN_WIDTH];
    let palette = inputs.palette.get(BG_PALETTE_BYTES..).unwrap_or(&[]);
    (0..OBJ_COUNT)
        .map(|index| ObjAttrs::decode(inputs.oam, index))
        .filter(drawn)
        .for_each(|obj| draw_object(inputs, dispcnt, palette, &obj, y, &mut line));
    line
}

/// Whether this step draws the object at all.
// TODO(hw-unverified): shape 3 draws nothing (`obj.md`, "Edge cases").
// TODO(phase4): affine objects (next step of #31), bitmap objects (mode 3, #31) and the OBJ window
// (mode 2, #33) are not drawn.
fn drawn(obj: &ObjAttrs) -> bool {
    !obj.affine
        && !obj.disabled
        && obj.mode != MODE_WINDOW
        && obj.mode != MODE_BITMAP
        && obj.shape != SHAPE_PROHIBITED
}

/// Draws the part of a plain tile object that lies on line `y` into `line`.
fn draw_object(
    inputs: &EngineInputs<'_>,
    dispcnt: u32,
    palette: &[u8],
    obj: &ObjAttrs,
    y: usize,
    line: &mut ObjLine,
) {
    let Some((width, height)) = obj.dimensions() else {
        return;
    };
    // The box covers lines `(Y + r) mod 256`, so a box that runs past line 255 shows at the top
    // (`obj.md`, "Where an object is on the screen").
    // TODO(hw-unverified): the DS page's vertical wrap is followed (`obj.md`, "Unverified").
    let row = (y as u32).wrapping_sub(u32::from(obj.y)) & 0xFF;
    if row >= height {
        return;
    }
    let src_row = if obj.v_flip { height - 1 - row } else { row };
    (0..width).for_each(|column| {
        let screen_x = i32::from(obj.x) + column as i32;
        let Some(slot) = usize::try_from(screen_x)
            .ok()
            .and_then(|at| line.get_mut(at))
        else {
            return;
        };
        let src_column = if obj.h_flip {
            width - 1 - column
        } else {
            column
        };
        let value = tile_value(inputs, dispcnt, obj, width, src_column, src_row);
        if value == 0 {
            return;
        }
        // The first object to reach a column keeps it unless a later one has a smaller priority
        // value: the entry order breaks the ties (`obj.md`, "Priority").
        let in_front = slot.is_none_or(|have| obj.priority < have.priority);
        if in_front {
            let index = match obj.eight_bpp {
                true => u32::from(value),
                false => u32::from(obj.palette) * PALETTE_COLORS + u32::from(value),
            };
            *slot = Some(ObjPixel {
                color: text::palette_color(palette, index),
                priority: obj.priority,
            });
        }
    });
}

/// The palette value (0 is transparent) of the pixel (`column`, `row`) of the object's picture,
/// after the flips, in a `width`-pixel-wide object (`obj.md`, "A plain tile object").
// TODO(hw-unverified): a tile outside the OBJ window reads 0, so it is transparent (`obj.md`,
// "Unverified").
fn tile_value(
    inputs: &EngineInputs<'_>,
    dispcnt: u32,
    obj: &ObjAttrs,
    width: u32,
    column: u32,
    row: u32,
) -> u8 {
    let (tile_column, tile_row) = (column / 8, row / 8);
    let (x, y) = (column % 8, row % 8);
    let tile = u32::from(obj.tile);
    let tile_bytes = match obj.eight_bpp {
        true => TILE_BYTES_8BPP,
        false => TILE_BYTES_4BPP,
    };
    let tile_at = if dispcnt & DISPCNT_TILE_1D != 0 {
        let boundary =
            BOUNDARY_BASE << ((dispcnt >> DISPCNT_BOUNDARY_SHIFT) & DISPCNT_BOUNDARY_MASK);
        tile * boundary + (tile_column + tile_row * (width / 8)) * tile_bytes
    } else {
        // 2D: a 256-colour tile is two numbers wide, and bit 0 of the number is ignored.
        let (first, step) = match obj.eight_bpp {
            true => (tile & !1, 2),
            false => (tile, 1),
        };
        (first + tile_column * step + tile_row * MATRIX_WIDTH) * TILE_UNIT
    };
    if obj.eight_bpp {
        inputs.obj_vram.byte(tile_at + y * 8 + x)
    } else {
        let byte = inputs.obj_vram.byte(tile_at + y * 4 + x / 2);
        // The left pixel of a pair is the low nibble.
        match x & 1 {
            0 => byte & 0xF,
            _ => byte >> 4,
        }
    }
}
