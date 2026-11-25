// SPDX-License-Identifier: Apache-2.0
//! The rendering engine's registers, `0400_0330h`-`0400_03BFh`, and the value SWAP_BUFFERS hands on
//! (`docs/hw/geometry-engine.md`, "What the rasterizer reads", "The rendering registers").
//!
//! The registers are stored and masked here; what each one does is the rasterizer's (#49-#51).
//! SWAP_BUFFERS does not copy them, so the rasterizer reads them live.

use super::polygon::Ram;
use serde::{Deserialize, Serialize};

/// What SWAP_BUFFERS hands to the rendering engine: the drawn set of Polygon RAM and Vertex RAM and
/// the swap's two parameter bits. Each polygon carries the VIEWPORT it was made under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene<'a> {
    /// The set the last swap handed on.
    pub ram: &'a Ram,
    /// SWAP_BUFFERS bit 0 (manual translucent sort) and bit 1 (W-buffering).
    pub swap_bits: u8,
}

/// Offsets inside the block, from `0400_0320h`.
const EDGE_COLOR: u32 = 0x10;
const ALPHA_TEST_REF: u32 = 0x20;
const CLEAR_COLOR: u32 = 0x30;
const CLEAR_DEPTH: u32 = 0x34;
const CLRIMAGE_OFFSET: u32 = 0x36;
const FOG_COLOR: u32 = 0x38;
const FOG_OFFSET: u32 = 0x3C;
const FOG_TABLE: u32 = 0x40;
const TOON_TABLE: u32 = 0x60;
/// One past the last byte of the block.
const END: u32 = 0xA0;

/// The bits each register stores (`geometry-engine.md`, "The rendering registers").
const COLOR15_MASK: u32 = 0x7FFF;
const ALPHA_TEST_MASK: u32 = 0x1F;
const CLEAR_COLOR_MASK: u32 = 0x3F1F_FFFF;
const FIFTEEN_BIT_MASK: u32 = 0x7FFF;
const FOG_COLOR_MASK: u32 = 0x001F_7FFF;
const FOG_DENSITY_MASK: u32 = 0x7F;

/// The rendering registers as written, each with its unused bits cleared.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderRegs {
    /// EDGE_COLOR 0-7, RGB555.
    pub edge_color: [u16; 8],
    /// ALPHA_TEST_REF, 0-31.
    pub alpha_test_ref: u8,
    /// CLEAR_COLOR: RGB555, fog flag (bit 15), alpha (16-20), polygon ID (24-29).
    pub clear_color: u32,
    /// CLEAR_DEPTH, 15 bits.
    pub clear_depth: u16,
    /// CLRIMAGE_OFFSET: X in bits 0-7, Y in bits 8-14.
    pub clrimage_offset: u16,
    /// FOG_COLOR: RGB555 (0-14) and alpha (16-20).
    pub fog_color: u32,
    /// FOG_OFFSET, 15 bits.
    pub fog_offset: u16,
    /// FOG_TABLE: 32 densities of 7 bits.
    pub fog_table: [u8; 32],
    /// TOON_TABLE: 32 colours, RGB555.
    pub toon_table: [u16; 32],
}

/// `old` with the byte at `index` (0 = lowest) replaced by `byte`, then masked.
fn merge(old: u32, index: u32, byte: u8, mask: u32) -> u32 {
    let shift = 8 * index;
    (old & !(0xFF << shift) | u32::from(byte) << shift) & mask
}

impl RenderRegs {
    /// Stores `byte` at `rel` bytes into the block (from `0400_0320h`); RDLINES_COUNT, the gaps
    /// and anything past the block drop the write.
    // TODO(hw-unverified): a narrow write stores just its bytes (`geometry-engine.md`, "Unverified").
    pub(super) fn write_byte(&mut self, rel: u32, byte: u8) {
        match rel {
            EDGE_COLOR..ALPHA_TEST_REF => {
                let at = rel - EDGE_COLOR;
                let slot = &mut self.edge_color[(at / 2) as usize];
                *slot = merge(u32::from(*slot), at % 2, byte, COLOR15_MASK) as u16;
            }
            ALPHA_TEST_REF => self.alpha_test_ref = byte & ALPHA_TEST_MASK as u8,
            CLEAR_COLOR..CLEAR_DEPTH => {
                self.clear_color =
                    merge(self.clear_color, rel - CLEAR_COLOR, byte, CLEAR_COLOR_MASK);
            }
            CLEAR_DEPTH..CLRIMAGE_OFFSET => {
                let value = merge(
                    u32::from(self.clear_depth),
                    rel - CLEAR_DEPTH,
                    byte,
                    FIFTEEN_BIT_MASK,
                );
                self.clear_depth = value as u16;
            }
            // TODO(hw-unverified): the Y offset's width (`geometry-engine.md`, "Unverified").
            CLRIMAGE_OFFSET..FOG_COLOR => {
                let value = merge(
                    u32::from(self.clrimage_offset),
                    rel - CLRIMAGE_OFFSET,
                    byte,
                    FIFTEEN_BIT_MASK,
                );
                self.clrimage_offset = value as u16;
            }
            FOG_COLOR..FOG_OFFSET => {
                self.fog_color = merge(self.fog_color, rel - FOG_COLOR, byte, FOG_COLOR_MASK);
            }
            FOG_OFFSET..FOG_TABLE => {
                let value = merge(
                    u32::from(self.fog_offset),
                    rel - FOG_OFFSET,
                    byte,
                    FIFTEEN_BIT_MASK,
                );
                self.fog_offset = value as u16;
            }
            FOG_TABLE..TOON_TABLE => {
                self.fog_table[(rel - FOG_TABLE) as usize] = byte & FOG_DENSITY_MASK as u8;
            }
            TOON_TABLE..END => {
                let at = rel - TOON_TABLE;
                let slot = &mut self.toon_table[(at / 2) as usize];
                *slot = merge(u32::from(*slot), at % 2, byte, COLOR15_MASK) as u16;
            }
            // RDLINES_COUNT and the gaps: the write is dropped.
            // TODO(hw-unverified): that a write to the read-only RDLINES_COUNT is dropped
            // (`geometry-engine.md`, "Unverified").
            _ => {}
        }
    }

    /// Whether every register holds only bits it stores, for a savestate load.
    pub(super) fn well_formed(&self) -> bool {
        let colors = |all: &[u16]| all.iter().all(|&c| u32::from(c) & !COLOR15_MASK == 0);
        colors(&self.edge_color)
            && colors(&self.toon_table)
            && u32::from(self.alpha_test_ref) & !ALPHA_TEST_MASK == 0
            && self.clear_color & !CLEAR_COLOR_MASK == 0
            && u32::from(self.clear_depth) & !FIFTEEN_BIT_MASK == 0
            && u32::from(self.clrimage_offset) & !FIFTEEN_BIT_MASK == 0
            && self.fog_color & !FOG_COLOR_MASK == 0
            && u32::from(self.fog_offset) & !FIFTEEN_BIT_MASK == 0
            && self
                .fog_table
                .iter()
                .all(|&d| u32::from(d) & !FOG_DENSITY_MASK == 0)
    }
}
