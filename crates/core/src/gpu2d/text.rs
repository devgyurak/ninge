// SPDX-License-Identifier: Apache-2.0
//! The text backgrounds: what one layer shows at one pixel (`docs/hw/bg-text.md`,
//! "The picture of one layer at one pixel"). Pure functions of the registers and the bytes they
//! point at; nothing is allocated, formatted or collected, so a line can be drawn from the bus
//! hot path of a frame.

use crate::bus::VramWindow;

/// BGxCNT bits 0-1: priority.
const CNT_PRIORITY_MASK: u16 = 0x3;
/// BGxCNT bits 2-5: the character base block, in 16 KiB units.
const CNT_CHAR_SHIFT: u32 = 2;
const CNT_CHAR_MASK: u16 = 0xF;
const CHAR_BLOCK_SIZE: u32 = 16 << 10;
/// BGxCNT bit 6: mosaic on for this layer (`bg-modes.md`, "Mosaic").
const CNT_MOSAIC: u16 = 1 << 6;
/// BGxCNT bit 7: 256 colors (8 bits per pixel) instead of 16 palettes of 16.
const CNT_EIGHT_BPP: u16 = 1 << 7;
/// BGxCNT bits 8-12: the screen base block, in 2 KiB units.
const CNT_SCREEN_SHIFT: u32 = 8;
const CNT_SCREEN_MASK: u16 = 0x1F;
const SCREEN_BLOCK_SIZE: u32 = 2 << 10;
/// BGxCNT bits 14-15: the screen size.
const CNT_SIZE_SHIFT: u32 = 14;
/// DISPCNT bits 24-26 and 27-29: the character and screen base, in 64 KiB units.
const DISPCNT_CHAR_SHIFT: u32 = 24;
const DISPCNT_SCREEN_SHIFT: u32 = 27;
const DISPCNT_BASE_MASK: u32 = 0x7;
const DISPCNT_BASE_SIZE: u32 = 64 << 10;

/// A screen entry (`bg-text.md`, "The picture of one layer at one pixel", step 3).
const ENTRY_TILE_MASK: u16 = 0x3FF;
const ENTRY_H_FLIP: u16 = 1 << 10;
const ENTRY_V_FLIP: u16 = 1 << 11;
const ENTRY_PALETTE_SHIFT: u32 = 12;
/// Pixels in a tile row, and the mask for a position inside a tile.
const TILE_SIZE: u32 = 8;
const TILE_MASK: u32 = 7;
/// Cells in one 32x32 map block, and the width of its rows.
const BLOCK_CELLS: u32 = 32;
/// Bytes a tile takes: 4 bits per pixel, and 8 bits per pixel.
const TILE_BYTES_4BPP: u32 = 32;
const TILE_BYTES_8BPP: u32 = 64;
/// Colors in one 4-bits-per-pixel palette.
const PALETTE_COLORS: u32 = 16;
/// The bytes of an engine's BG palette (256 colors of 16 bits); the OBJ palette follows it.
pub(super) const BG_PALETTE_BYTES: usize = 512;

/// A text layer's registers, decoded (`bg-text.md`, "BGxCNT bits"). A debugger's BG panel (#58)
/// shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextBg {
    /// Bits 0-1 of BGxCNT; 0 is in front.
    pub priority: u8,
    /// Where the tile data starts, counted from the start of the engine's BG VRAM.
    pub char_base: u32,
    /// Where the map starts, counted from the start of the engine's BG VRAM.
    pub screen_base: u32,
    /// Whether a pixel is 8 bits (256 colors) rather than 4.
    pub eight_bpp: bool,
    /// Whether mosaic is on for the layer (BGxCNT bit 6).
    pub mosaic: bool,
    /// The layer's width in pixels, 256 or 512.
    pub width: u16,
    /// The layer's height in pixels, 256 or 512.
    pub height: u16,
    /// The horizontal scroll, 0-511.
    pub hofs: u16,
    /// The vertical scroll, 0-511.
    pub vofs: u16,
}

impl TextBg {
    /// Decodes BGxCNT and the scroll registers of one layer under `dispcnt`.
    pub(super) fn decode(dispcnt: u32, cnt: u16, hofs: u16, vofs: u16) -> Self {
        let size = cnt >> CNT_SIZE_SHIFT;
        let char_base = ((dispcnt >> DISPCNT_CHAR_SHIFT) & DISPCNT_BASE_MASK) * DISPCNT_BASE_SIZE
            + u32::from((cnt >> CNT_CHAR_SHIFT) & CNT_CHAR_MASK) * CHAR_BLOCK_SIZE;
        let screen_base = ((dispcnt >> DISPCNT_SCREEN_SHIFT) & DISPCNT_BASE_MASK)
            * DISPCNT_BASE_SIZE
            + u32::from((cnt >> CNT_SCREEN_SHIFT) & CNT_SCREEN_MASK) * SCREEN_BLOCK_SIZE;
        Self {
            priority: (cnt & CNT_PRIORITY_MASK) as u8,
            char_base,
            screen_base,
            eight_bpp: cnt & CNT_EIGHT_BPP != 0,
            mosaic: cnt & CNT_MOSAIC != 0,
            // Size bit 0 doubles the width, bit 1 the height.
            width: 256 << (size & 1),
            height: 256 << (size >> 1),
            hofs,
            vofs,
        }
    }
}

/// The map cell entry for the layer point (`px`, `py`).
fn entry(layer: &TextBg, vram: &VramWindow<'_>, px: u32, py: u32) -> u16 {
    let (cx, cy) = (px / TILE_SIZE, py / TILE_SIZE);
    // A 512-wide layer has a second block to the right; a 512-high one has the next below, and
    // a 512x512 layer holds them top-left, top-right, bottom-left, bottom-right.
    let blocks_across = u32::from(layer.width) / 256;
    let block = (cy / BLOCK_CELLS) * blocks_across + cx / BLOCK_CELLS;
    let within = (cy % BLOCK_CELLS) * BLOCK_CELLS + cx % BLOCK_CELLS;
    vram.half(layer.screen_base + block * SCREEN_BLOCK_SIZE + within * 2)
}

/// The palette value of layer point (`px`, `py`) and the palette number to colour it with; value
/// 0 is transparent.
fn tile_pixel(layer: &TextBg, vram: &VramWindow<'_>, px: u32, py: u32) -> (u8, u32) {
    let entry = entry(layer, vram, px, py);
    let tile = u32::from(entry & ENTRY_TILE_MASK);
    let column = match entry & ENTRY_H_FLIP != 0 {
        true => TILE_MASK - (px & TILE_MASK),
        false => px & TILE_MASK,
    };
    let row = match entry & ENTRY_V_FLIP != 0 {
        true => TILE_MASK - (py & TILE_MASK),
        false => py & TILE_MASK,
    };
    if layer.eight_bpp {
        let at = layer.char_base + tile * TILE_BYTES_8BPP + row * TILE_SIZE + column;
        (vram.byte(at), 0)
    } else {
        let at = layer.char_base + tile * TILE_BYTES_4BPP + row * 4 + column / 2;
        let byte = vram.byte(at);
        // The left pixel of a pair is the low nibble.
        let value = match column & 1 {
            0 => byte & 0xF,
            _ => byte >> 4,
        };
        (value, u32::from(entry >> ENTRY_PALETTE_SHIFT))
    }
}

/// The 15-bit colour of layer `layer` at screen pixel (`x`, `y`), or `None` where it is
/// transparent.
// TODO(phase4): extended palettes (DISPCNT bit 30).
// TODO(hw-unverified): the standard palette is used for 8 bits per pixel whatever DISPCNT bit 30
// says (`bg-text.md`, "Unverified").
pub(super) fn pixel(
    layer: &TextBg,
    vram: &VramWindow<'_>,
    palette: &[u8],
    x: u32,
    y: u32,
) -> Option<u16> {
    // The layer wraps at its own size; the scroll registers hold 9 bits.
    let px = (x + u32::from(layer.hofs)) & (u32::from(layer.width) - 1);
    let py = (y + u32::from(layer.vofs)) & (u32::from(layer.height) - 1);
    let (value, palette_number) = tile_pixel(layer, vram, px, py);
    if value == 0 {
        return None;
    }
    let index = match layer.eight_bpp {
        true => u32::from(value),
        false => palette_number * PALETTE_COLORS + u32::from(value),
    };
    Some(palette_color(palette, index))
}

/// Entry `index` of the BG palette; a palette shorter than the table reads as 0.
pub(super) fn palette_color(palette: &[u8], index: u32) -> u16 {
    let at = index as usize * 2;
    palette
        .get(at..at + 2)
        .and_then(|bytes| bytes.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}
