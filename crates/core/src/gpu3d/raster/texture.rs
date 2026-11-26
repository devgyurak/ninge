// SPDX-License-Identifier: Apache-2.0
//! Texels: where a pixel's S and T land in a texture, what colour and alpha that texel has, and how
//! it joins the vertex colour (`docs/hw/textures.md`).
//!
//! A polygon's TEXIMAGE_PARAM and PLTT_BASE are parsed once into a [`Tex`] when the frame's
//! snapshot is built. The line draw then reads only that and the texture memory ([`Textures`]), and
//! allocates nothing.
// TODO(phase7, #50): A3I5, A5I3, direct (the alpha-format PR) and 4x4 compressed (its own PR) are
// not sampled yet; a polygon that uses one draws its vertex colour.

use crate::bus::Textures;

// TEXIMAGE_PARAM fields (`docs/hw/textures.md`, "TEXIMAGE_PARAM bits").
const OFFSET_MASK: u32 = 0xFFFF;
/// The image offset is in units of 8 bytes.
const OFFSET_UNIT: u32 = 8;
pub(super) const REPEAT_S: u32 = 1 << 16;
pub(super) const REPEAT_T: u32 = 1 << 17;
pub(super) const FLIP_S: u32 = 1 << 18;
pub(super) const FLIP_T: u32 = 1 << 19;
const SIZE_S_SHIFT: u32 = 20;
const SIZE_T_SHIFT: u32 = 23;
const FORMAT_SHIFT: u32 = 26;
pub(super) const COLOR0_TRANSPARENT: u32 = 1 << 29;
/// PLTT_BASE bits 0-12.
const PALETTE_MASK: u32 = 0x1FFF;
/// The palette base is in units of 16 bytes, 8 for the 4-colour format.
const PALETTE_UNIT: u32 = 16;
const PALETTE_UNIT_4COLOR: u32 = 8;
/// The fractional bits of S and T (1.11.4).
const ST_FRAC: u32 = 4;

/// The palette formats this module samples so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// 2 bits a texel, palette base in units of 8 bytes.
    Colors4,
    /// 4 bits a texel.
    Colors16,
    /// 8 bits a texel.
    Colors256,
}

/// A polygon's texture, parsed from TEXIMAGE_PARAM and PLTT_BASE.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tex {
    format: Format,
    /// Byte offset of the first texel in the image space.
    image: u32,
    /// Byte offset of palette entry 0 in the palette space.
    palette: u32,
    size_s: u32,
    size_t: u32,
    repeat_s: bool,
    repeat_t: bool,
    flip_s: bool,
    flip_t: bool,
    /// Palette index 0 is transparent (TEXIMAGE_PARAM bit 29).
    color0_clear: bool,
}

/// One texel: a 15-bit colour and a 5-bit alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Texel {
    pub(super) color: u16,
    pub(super) alpha: u8,
}

impl Tex {
    /// The texture of a polygon with TEXIMAGE_PARAM `param` and PLTT_BASE `palette`, or `None` for
    /// "no texture" and for a format that is not sampled yet.
    pub(super) fn parse(param: u32, palette: u32) -> Option<Self> {
        let format = match (param >> FORMAT_SHIFT) & 7 {
            2 => Format::Colors4,
            3 => Format::Colors16,
            4 => Format::Colors256,
            _ => return None,
        };
        let unit = if format == Format::Colors4 {
            PALETTE_UNIT_4COLOR
        } else {
            PALETTE_UNIT
        };
        Some(Self {
            format,
            image: (param & OFFSET_MASK) * OFFSET_UNIT,
            palette: (palette & PALETTE_MASK) * unit,
            size_s: 8 << ((param >> SIZE_S_SHIFT) & 7),
            size_t: 8 << ((param >> SIZE_T_SHIFT) & 7),
            repeat_s: param & REPEAT_S != 0,
            repeat_t: param & REPEAT_T != 0,
            flip_s: param & FLIP_S != 0,
            flip_t: param & FLIP_T != 0,
            color0_clear: param & COLOR0_TRANSPARENT != 0,
        })
    }

    /// The texel at S and T (1.11.4, as interpolated), from texture memory `mem`.
    // TODO(hw-unverified): the texel is the floor of S and T, sampled at the pixel's top left
    // corner (`textures.md`, "Unverified").
    pub(super) fn texel(&self, mem: &Textures<'_>, s: i64, t: i64) -> Texel {
        let x = wrap(s >> ST_FRAC, self.size_s, self.repeat_s, self.flip_s);
        let y = wrap(t >> ST_FRAC, self.size_t, self.repeat_t, self.flip_t);
        let i = y * self.size_s + x;
        let index = match self.format {
            Format::Colors4 => u32::from(mem.image_byte(self.image + i / 4) >> (2 * (i % 4))) & 0x3,
            Format::Colors16 => {
                u32::from(mem.image_byte(self.image + i / 2) >> (4 * (i % 2))) & 0xF
            }
            Format::Colors256 => u32::from(mem.image_byte(self.image + i)),
        };
        let hidden = self.color0_clear && index == 0;
        Texel {
            color: mem.palette_color(self.palette + index * 2) & 0x7FFF,
            alpha: if hidden { 0 } else { 31 },
        }
    }
}

/// A texel coordinate `n` (any sign) on an axis of `size` texels: clamped, repeated, or repeated
/// with every second repetition mirrored. Flip needs repeat, so without it the axis clamps
/// (`textures.md`, "From S and T to a texel").
pub(super) fn wrap(n: i64, size: u32, repeat: bool, flip: bool) -> u32 {
    let size = i64::from(size);
    let wrapped = match (repeat, flip) {
        (false, _) => n.clamp(0, size - 1),
        (true, false) => n.rem_euclid(size),
        (true, true) => {
            let m = n.rem_euclid(2 * size);
            if m >= size { 2 * size - 1 - m } else { m }
        }
    };
    // `wrapped` is in 0..size by construction, and a size is at most 1024.
    wrapped as u32
}

/// A 5-bit channel as 6 bits: `X x 2 + 1`, 0 stays 0 (`rasterizer.md`, "Colour along a polygon").
pub(super) fn expand6(c5: u16) -> u8 {
    if c5 == 0 { 0 } else { (c5 * 2 + 1) as u8 }
}

/// Modulation (`textures.md`, "Blending the texel with the vertex colour"): the texel times the
/// vertex colour per 6-bit channel, `((t + 1) x (v + 1) - 1) / 64`, and the same for the alpha of
/// the texel and the polygon. Returns the 6-bit colour and the 5-bit alpha.
// TODO(hw-unverified): the division truncates and the 6-bit alpha narrows by dropping its low bit
// (`textures.md`, "Unverified").
pub(super) fn modulate(texel: Texel, vertex6: [u8; 3], alpha: u8) -> ([u8; 3], u8) {
    let mix = |t: u16, v: u8| (((t + 1) * (u16::from(v) + 1) - 1) / 64) as u8;
    let t = |shift: u32| u16::from(expand6((texel.color >> shift) & 0x1F));
    let color = [
        mix(t(0), vertex6[0]),
        mix(t(5), vertex6[1]),
        mix(t(10), vertex6[2]),
    ];
    let a = mix(
        u16::from(expand6(u16::from(texel.alpha))),
        expand6(u16::from(alpha)),
    );
    (color, a >> 1)
}
