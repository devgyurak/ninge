// SPDX-License-Identifier: Apache-2.0
//! The rear plane as two bitmaps in texture slots 2 and 3 (DISP3DCNT bit 14): colour and alpha per
//! pixel from slot 2, depth from slot 3, scrolled by CLRIMAGE_OFFSET (`docs/hw/textures.md`, "The
//! rear bitmap").
//!
//! Each bitmap is 256x256 entries of 16 bits, row-major, a whole 128 KiB slot. The picture is
//! the visible 256x192 of it, moved by the scroll and wrapping at the right and bottom edges. The
//! polygon ID of every pixel still comes from CLEAR_COLOR.
// TODO(phase7, #51): the fog flag (bit 15 of a depth entry) is the pixel's starting fog enable;
// there is no fog state until fog lands.

use super::LineState;
use super::setup::{RearBitmap, expand_clear_depth};
use super::texture::expand6;
use crate::SCREEN_WIDTH;
use crate::bus::{TEX_IMAGE_SLOT_BYTES, Textures};
use crate::gpu2d::{Line3d, Pixel3d};

/// Image offsets of the colour bitmap (slot 2) and the depth bitmap (slot 3).
const COLOR_AT: u32 = 2 * TEX_IMAGE_SLOT_BYTES;
const DEPTH_AT: u32 = 3 * TEX_IMAGE_SLOT_BYTES;
/// Entries on a side of a bitmap.
const SIDE: u32 = 256;

/// The halfword at image offset `at`.
fn half(textures: &Textures<'_>, at: u32) -> u16 {
    u16::from(textures.image_byte(at)) | u16::from(textures.image_byte(at + 1)) << 8
}

/// Fills line `y`'s output and working state with the rear bitmaps: the colour and alpha into `out`
/// and `state.color6`, the depth into `state.depth`. What a slot no bank backs reads is 0, so the
/// pixel is transparent colour 0 at depth 0 (the nearest: nothing draws over it).
// TODO(hw-unverified): the unmapped-slot case and the wrap of the scroll are the model's
// (`textures.md`, "Unverified").
pub(super) fn fill(
    y: usize,
    bitmap: &RearBitmap,
    textures: &Textures<'_>,
    state: &mut LineState,
    out: &mut Line3d,
) {
    // `y` is a screen line, at most 191.
    let row = (y as u32 + u32::from(bitmap.y)) % SIDE;
    (0..SCREEN_WIDTH).for_each(|x| {
        let column = (x as u32 + u32::from(bitmap.x)) % SIDE;
        let at = (row * SIDE + column) * 2;
        let color = half(textures, COLOR_AT + at);
        let depth = half(textures, DEPTH_AT + at);
        out[x] = Pixel3d {
            color: color & 0x7FFF,
            alpha: if color & 0x8000 != 0 { 31 } else { 0 },
        };
        state.color6[x] = [0, 5, 10].map(|shift| expand6((color >> shift) & 0x1F));
        state.depth[x] = expand_clear_depth(depth & 0x7FFF);
    });
}
