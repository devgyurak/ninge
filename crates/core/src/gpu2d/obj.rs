// SPDX-License-Identifier: Apache-2.0
//! Sprites (OBJ): one entry of OAM decoded ([`ObjAttrs`]) and the line of object pixels the
//! engine makes before it puts the background layers behind them (`docs/hw/obj.md`).
//!
//! Plain and affine tile and bitmap objects, with mosaic and the per-line cycle budget.

use super::Mosaic;
use super::spec::{DISPCNT_OBJ_BITMAP_BOUNDARY_SHIFT, EngineInputs};
use super::text::{self, BG_PALETTE_BYTES};
use crate::SCREEN_WIDTH;

/// Entries in an engine's OAM, and the bytes of one (`obj.md`, "The object table").
pub const OBJ_COUNT: usize = 128;
const ENTRY_BYTES: usize = 8;

/// attr0 bit 8: the object is affine.
const ATTR0_AFFINE: u16 = 1 << 8;
/// Where the fourth halfword of an entry is: the parameter of an affine group (`obj.md`, "The
/// object table").
const PARAMETER_OFFSET: usize = 6;
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

/// OBJ mode 2 is the OBJ window; mode 1 (semi-transparent, `TODO(phase4)`, #34) draws as mode 0.
const MODE_WINDOW: u8 = 2;
/// OBJ mode 3 is a bitmap object.
const MODE_BITMAP: u8 = 3;

/// DISPCNT bit 5: bitmap OBJ 2D width, 0 = 128 pixels, 1 = 256.
const DISPCNT_BITMAP_WIDE: u32 = 1 << 5;
/// DISPCNT bit 6: bitmap OBJ mapping, 1 = 1D.
const DISPCNT_BITMAP_1D: u32 = 1 << 6;
/// The bitmap OBJ 1D boundary is 128 bytes, or 256 with DISPCNT bit 22 (engine A only, `spec.rs`).
const BITMAP_BOUNDARY: u32 = 128;
/// A bitmap pixel is a halfword; bit 15 set means opaque.
const BITMAP_PIXEL: u32 = 2;
const BITMAP_OPAQUE: u16 = 1 << 15;
/// In 2D mapping the tile number's column steps are 8 pixels (`0x10` bytes) and its row steps
/// are 8 lines of the source bitmap, written as `0x80` bytes per number.
const BITMAP_TILE_X: u32 = 0x10;
const BITMAP_TILE_Y: u32 = 0x80;
/// DISPCNT bit 23: OBJ processing during H-Blank, which shortens the line budget (`obj.md`, "The
/// line limit").
const DISPCNT_OBJ_HBLANK: u32 = 1 << 23;
/// The OBJ rendering cycles a line has: 1210 normally, 954 with DISPCNT bit 23.
// TODO(hw-unverified): 1210 and 954 are GBATEK's GBA values, and bit 23 is read with the GBA's
// "H-Blank Interval Free" polarity (set = 954); the DS may differ on both (`obj.md`, "Unverified").
const BUDGET: u32 = 1210;
const BUDGET_HBLANK: u32 = 954;
/// An affine object costs this many cycles more than a plain one, and twice its box width.
const AFFINE_COST_BASE: u32 = 10;
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
    pub(crate) fn decode(oam: &[u8], index: usize) -> Self {
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
// TODO(phase4): semi-transparent objects draw as normal ones (#34), the OBJ window is not drawn
// (#33) and extended palettes are not read (#32).
pub(super) fn draw_line(
    inputs: &EngineInputs<'_>,
    dispcnt: u32,
    mosaic: Mosaic,
    y: usize,
) -> ObjLine {
    let mut line: ObjLine = [None; SCREEN_WIDTH];
    let palette = inputs.palette.get(BG_PALETTE_BYTES..).unwrap_or(&[]);
    // TODO(hw-unverified): bit 23 only changes the budget here; whether OAM access timing changes
    // the picture is not modelled (`obj.md`, "Unverified").
    let mut budget = match dispcnt & DISPCNT_OBJ_HBLANK != 0 {
        true => BUDGET_HBLANK,
        false => BUDGET,
    };
    for index in 0..OBJ_COUNT {
        let obj = ObjAttrs::decode(inputs.oam, index);
        // A disabled object, one of shape 3 and one that does not reach the line cost nothing.
        // TODO(hw-unverified): a disabled object costs nothing, and neither does one that is not
        // on the line; an object off the screen to the side does cost (`obj.md`, "Unverified").
        if obj.disabled {
            continue;
        }
        let Some(place) = placement(&obj, mosaic, y) else {
            continue;
        };
        // When the budget runs out the object is dropped, and so is every later one.
        // TODO(hw-unverified): the whole object is dropped, not part of it, and the later ones
        // with it (`obj.md`, "Unverified").
        let Some(left) = budget.checked_sub(place.cost(&obj)) else {
            break;
        };
        budget = left;
        // TODO(phase4): the OBJ window (mode 2, #33) is not drawn.
        // TODO(hw-unverified): a mode 2 object costs its cycles though it is not drawn
        // (`obj.md`, "Unverified").
        if obj.mode != MODE_WINDOW {
            draw_object(inputs, dispcnt, palette, &obj, &place, &mut line);
        }
    }
    line
}

/// Where an object's box meets the line being made.
struct Placement {
    width: u32,
    height: u32,
    box_width: u32,
    box_height: u32,
    /// The box row the line shows (the row of the block's first line for a mosaic object).
    row: u32,
    block_width: u32,
}

impl Placement {
    /// The object's cost in OBJ rendering cycles: its box width, or 10 plus twice it when affine
    /// (`obj.md`, "The line limit").
    // TODO(hw-unverified): the cost of a double-size affine object is 10 + 2 * (2W)
    // (`obj.md`, "Unverified").
    fn cost(&self, obj: &ObjAttrs) -> u32 {
        match obj.affine {
            true => AFFINE_COST_BASE + 2 * self.box_width,
            false => self.box_width,
        }
    }
}

/// The object's placement on line `y`, or `None` when the object has a shape that is not allowed
/// or its box does not reach the line.
// TODO(hw-unverified): shape 3 draws nothing and costs nothing (`obj.md`, "Unverified").
fn placement(obj: &ObjAttrs, mosaic: Mosaic, y: usize) -> Option<Placement> {
    let (width, height) = obj.dimensions()?;
    // An affine object with double-size is drawn in a box twice as wide and as high.
    let (box_width, box_height) = match obj.double_size {
        true => (2 * width, 2 * height),
        false => (width, height),
    };
    // A mosaic object shows, at each screen pixel, what it would show at the first pixel of the
    // pixel's block; blocks are counted from the top left of the screen (`obj.md`, "Mosaic"). So
    // the line is the block's first line, and the columns run on to the end of the last block.
    // TODO(hw-unverified): the block origin is the screen's, and an affine object's block pixel is
    // transformed after the pick (`obj.md`, "Unverified").
    let (block_width, block_height) = match obj.mosaic {
        true => (u32::from(mosaic.obj_h), u32::from(mosaic.obj_v)),
        false => (1, 1),
    };
    let block_y = y as u32 - y as u32 % block_height;
    // The box covers lines `(Y + r) mod 256`, so a box that runs past line 255 shows at the top
    // (`obj.md`, "Where an object is on the screen").
    // TODO(hw-unverified): the DS page's vertical wrap is followed (`obj.md`, "Unverified").
    let row = block_y.wrapping_sub(u32::from(obj.y)) & 0xFF;
    (row < box_height).then_some(Placement {
        width,
        height,
        box_width,
        box_height,
        row,
        block_width,
    })
}

/// Parameter group `group` of an affine object: PA, PB, PC, PD, each the fourth halfword of one of
/// four consecutive entries (`obj.md`, "An affine object"). A group the slice does not hold reads
/// as zeros.
fn parameters(oam: &[u8], group: u8) -> [i32; 4] {
    std::array::from_fn(|n| {
        let at = (usize::from(group) * 4 + n) * ENTRY_BYTES + PARAMETER_OFFSET;
        oam.get(at..at + 2)
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(0, |bytes| i32::from(i16::from_le_bytes(bytes)))
    })
}

/// Where the object's picture is sampled for the box pixel (`column`, `row`) of an affine object
/// whose box is `box_width` by `box_height`: the offset from the box centre, through PA-PD, plus
/// the picture's centre (`obj.md`, "An affine object"). `None` outside the `width` by `height`
/// picture.
// TODO(hw-unverified): the arithmetic is inferred from the BG's convention, not stated for
// objects (`obj.md`, "Unverified").
fn affine_source(
    [pa, pb, pc, pd]: [i32; 4],
    (width, height): (u32, u32),
    (box_width, box_height): (u32, u32),
    (column, row): (u32, u32),
) -> Option<(u32, u32)> {
    let dx = column as i32 - (box_width / 2) as i32;
    let dy = row as i32 - (box_height / 2) as i32;
    let u = (pa.wrapping_mul(dx).wrapping_add(pb.wrapping_mul(dy)) >> 8) + (width / 2) as i32;
    let v = (pc.wrapping_mul(dx).wrapping_add(pd.wrapping_mul(dy)) >> 8) + (height / 2) as i32;
    let inside = |at: i32, size: u32| u32::try_from(at).ok().filter(|&at| at < size);
    Some((inside(u, width)?, inside(v, height)?))
}

/// Draws the part of a tile object that lies on line `y` into `line`.
fn draw_object(
    inputs: &EngineInputs<'_>,
    dispcnt: u32,
    palette: &[u8],
    obj: &ObjAttrs,
    place: &Placement,
    line: &mut ObjLine,
) {
    let Placement {
        width,
        height,
        box_width,
        box_height,
        row,
        block_width,
    } = *place;
    let group = obj.affine.then(|| parameters(inputs.oam, obj.group));
    let first = i32::from(obj.x).max(0);
    let end = (i32::from(obj.x) + (box_width + block_width - 1) as i32).min(SCREEN_WIDTH as i32);
    (first..end).for_each(|screen_x| {
        let block_x = screen_x - screen_x % block_width as i32;
        let (Some(slot), Ok(column)) = (
            usize::try_from(screen_x)
                .ok()
                .and_then(|at| line.get_mut(at)),
            u32::try_from(block_x - i32::from(obj.x)),
        ) else {
            return;
        };
        if column >= box_width {
            return;
        }
        // The picture's pixel this box pixel shows: a plain object's flips, or an affine object's
        // transform (the picture is cut at the box).
        let source = match group {
            Some(parameters) => affine_source(
                parameters,
                (width, height),
                (box_width, box_height),
                (column, row),
            ),
            None => Some((
                if obj.h_flip {
                    width - 1 - column
                } else {
                    column
                },
                if obj.v_flip { height - 1 - row } else { row },
            )),
        };
        let Some((src_column, src_row)) = source else {
            return;
        };
        let color = match obj.mode == MODE_BITMAP {
            true => bitmap_color(inputs, dispcnt, obj, width, src_column, src_row),
            false => tile_color(inputs, dispcnt, palette, obj, width, src_column, src_row),
        };
        let Some(color) = color else {
            return;
        };
        // The first object to reach a column keeps it unless a later one has a smaller priority
        // value: the entry order breaks the ties (`obj.md`, "Priority").
        let in_front = slot.is_none_or(|have| obj.priority < have.priority);
        if in_front {
            *slot = Some(ObjPixel {
                color,
                priority: obj.priority,
            });
        }
    });
}

/// The 15-bit colour of the pixel (`column`, `row`) of a tile object's picture, or `None` where
/// its palette value is 0.
fn tile_color(
    inputs: &EngineInputs<'_>,
    dispcnt: u32,
    palette: &[u8],
    obj: &ObjAttrs,
    width: u32,
    column: u32,
    row: u32,
) -> Option<u16> {
    let value = tile_value(inputs, dispcnt, obj, width, column, row);
    let index = match (value, obj.eight_bpp) {
        (0, _) => return None,
        (value, true) => u32::from(value),
        (value, false) => u32::from(obj.palette) * PALETTE_COLORS + u32::from(value),
    };
    Some(text::palette_color(palette, index))
}

/// The 15-bit colour of the pixel (`column`, `row`) of a bitmap object's picture, or `None` where
/// its bit 15 is clear (`obj.md`, "A bitmap object"). A bitmap object has no palette and its
/// attr2 bits 12-15 are an alpha value, not read here (`TODO(phase4)`, #34).
// TODO(hw-unverified): the flips work as on a tile object, a bitmap object with attr0 bit 13 set
// is still a bitmap object, and DISPCNT bits 5 and 6 both set (reserved) draw nothing
// (`obj.md`, "Unverified").
fn bitmap_color(
    inputs: &EngineInputs<'_>,
    dispcnt: u32,
    obj: &ObjAttrs,
    width: u32,
    column: u32,
    row: u32,
) -> Option<u16> {
    let wide = dispcnt & DISPCNT_BITMAP_WIDE != 0;
    let tile = u32::from(obj.tile);
    let (first, stride) = match (dispcnt & DISPCNT_BITMAP_1D != 0, wide) {
        // 2D: the tile number is the column in its low bits (in 8-pixel steps) and the row above
        // them (in 8-line steps); the source bitmap is 128 or 256 pixels wide.
        (false, wide) => {
            let mask = if wide { 0x1F } else { 0xF };
            let source_width = if wide { 256 } else { 128 };
            (
                (tile & mask) * BITMAP_TILE_X + (tile & !mask) * BITMAP_TILE_Y,
                source_width * BITMAP_PIXEL,
            )
        }
        // 1D: the first pixel is at tile * boundary and a row is the object's own width.
        (true, false) => {
            let boundary = BITMAP_BOUNDARY << ((dispcnt >> DISPCNT_OBJ_BITMAP_BOUNDARY_SHIFT) & 1);
            (tile * boundary, width * BITMAP_PIXEL)
        }
        (true, true) => return None,
    };
    let pixel = inputs
        .obj_vram
        .half(first + row * stride + column * BITMAP_PIXEL);
    (pixel & BITMAP_OPAQUE != 0).then_some(pixel & !BITMAP_OPAQUE)
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
