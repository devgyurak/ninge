// SPDX-License-Identifier: Apache-2.0
//! Texels: where a pixel's S and T land in a texture, what colour and alpha that texel has, and how
//! it joins the vertex colour (`docs/hw/textures.md`).
//!
//! A polygon's TEXIMAGE_PARAM and PLTT_BASE are parsed once into a [`Tex`] when the frame's
//! snapshot is built. The line draw then reads only that and the texture memory ([`Textures`]), and
//! allocates nothing.

use crate::bus::Textures;

// TEXIMAGE_PARAM fields (`docs/hw/textures.md`, "TEXIMAGE_PARAM bits").
const OFFSET_MASK: u32 = 0xFFFF;
/// The image offset is in units of 8 bytes.
const OFFSET_UNIT: u32 = 8;
const REPEAT_S: u32 = 1 << 16;
const REPEAT_T: u32 = 1 << 17;
const FLIP_S: u32 = 1 << 18;
const FLIP_T: u32 = 1 << 19;
const SIZE_S_SHIFT: u32 = 20;
const SIZE_T_SHIFT: u32 = 23;
const FORMAT_SHIFT: u32 = 26;
const COLOR0_TRANSPARENT: u32 = 1 << 29;
/// PLTT_BASE bits 0-12.
const PALETTE_MASK: u32 = 0x1FFF;
/// The palette base is in units of 16 bytes, 8 for the 4-colour format.
const PALETTE_UNIT: u32 = 16;
const PALETTE_UNIT_4COLOR: u32 = 8;
/// DISP3DCNT bit 0: texture mapping on.
pub(super) const DISP3DCNT_TEXTURE: u16 = 1;
/// A fully opaque alpha, 5 bits: a texel's, and a pixel's.
pub(super) const OPAQUE: u8 = 31;
/// Bytes of one texture image slot (128 KiB).
const SLOT_BYTES: u32 = 0x2_0000;
/// The fractional bits of S and T (1.11.4).
const ST_FRAC: u32 = 4;

/// The formats this module samples so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// A3I5: 8 bits, a 3-bit alpha above a 5-bit index into a 32-colour palette.
    Alpha3Index5,
    /// 2 bits a texel, palette base in units of 8 bytes.
    Colors4,
    /// 4 bits a texel.
    Colors16,
    /// 8 bits a texel.
    Colors256,
    /// A5I3: 8 bits, a 5-bit alpha above a 3-bit index into an 8-colour palette.
    Alpha5Index3,
    /// 16 bits a texel: a 15-bit colour and an alpha bit, no palette.
    Direct,
    /// 4x4-texel blocks of 2-bit texels, with a palette halfword per block in slot 1.
    Compressed,
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
            1 => Format::Alpha3Index5,
            6 => Format::Alpha5Index3,
            7 => Format::Direct,
            5 => Format::Compressed,
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

    /// The texels can hold a partial alpha (A3I5 and A5I3), which is what makes a polygon draw with
    /// the translucent ones (`textures.md`, "Texture alpha, translucency and the buffers").
    pub(super) fn has_alpha(&self) -> bool {
        matches!(self.format, Format::Alpha3Index5 | Format::Alpha5Index3)
    }

    /// The texel at S and T (1.11.4, as interpolated), from texture memory `mem`.
    // TODO(hw-unverified): the texel is the floor of S and T, sampled at the pixel's top left
    // corner (`textures.md`, "Unverified").
    pub(super) fn texel(&self, mem: &Textures<'_>, s: i64, t: i64) -> Texel {
        let x = wrap(s >> ST_FRAC, self.size_s, self.repeat_s, self.flip_s);
        let y = wrap(t >> ST_FRAC, self.size_t, self.repeat_t, self.flip_t);
        let i = y * self.size_s + x;
        let byte = |at: u32| u32::from(mem.image_byte(self.image + at));
        // The palette index, and the alpha a texel of this format carries (5 bits).
        let (index, alpha) = match self.format {
            Format::Colors4 => ((byte(i / 4) >> (2 * (i % 4))) & 0x3, OPAQUE),
            Format::Colors16 => ((byte(i / 2) >> (4 * (i % 2))) & 0xF, OPAQUE),
            Format::Colors256 => (byte(i), OPAQUE),
            Format::Alpha3Index5 => {
                // The 3-bit alpha widens to 5 bits as `a x 4 + a / 2` (0 stays 0, 7 becomes 31).
                let a = byte(i) >> 5;
                (byte(i) & 0x1F, (a * 4 + a / 2) as u8)
            }
            Format::Alpha5Index3 => (byte(i) & 0x7, (byte(i) >> 3) as u8),
            Format::Compressed => return self.compressed(mem, x, y),
            Format::Direct => {
                // Bit 15 is the alpha, 31 when set and 0 when clear; there is no palette.
                let half = byte(2 * i) | byte(2 * i + 1) << 8;
                let alpha = if half & 0x8000 != 0 { OPAQUE } else { 0 };
                return Texel {
                    color: (half & 0x7FFF) as u16,
                    alpha,
                };
            }
        };
        // Bit 29 applies to the 4-, 16- and 256-colour formats only.
        let palette_format = matches!(
            self.format,
            Format::Colors4 | Format::Colors16 | Format::Colors256
        );
        let hidden = palette_format && self.color0_clear && index == 0;
        Texel {
            color: mem.palette_color(self.palette + index * 2),
            alpha: if hidden { 0 } else { alpha },
        }
    }
}

impl Tex {
    /// The texel at column `x`, row `y` of a 4x4 compressed texture (`textures.md`, "4x4 compressed
    /// texels"): the block's 2-bit texel picks a colour by the mode in the block's palette halfword.
    // TODO(hw-unverified): the halfword of a block whose data is in slot 1 or 3 reads as 0 (the
    // formula covers slots 0 and 2 only), and the mixes truncate per 5-bit channel
    // (`textures.md`, "Unverified").
    fn compressed(&self, mem: &Textures<'_>, x: u32, y: u32) -> Texel {
        let block = (y / 4) * (self.size_s / 4) + x / 4;
        let at = self.image + 4 * block;
        let row = u32::from(mem.image_byte(at + y % 4));
        let texel = (row >> (2 * (x % 4))) & 0x3;
        // The halfword sits in slot 1 at half the block's offset in its slot: in the lower 64 KiB
        // for data in slot 0, in the upper 64 KiB for data in slot 2.
        let within = (at % SLOT_BYTES) / 2;
        let info = match at / SLOT_BYTES {
            0 => Some(SLOT_BYTES + within),
            2 => Some(SLOT_BYTES + SLOT_BYTES / 2 + within),
            _ => None,
        };
        let half = info.map_or(0, |a| {
            u32::from(mem.image_byte(a)) | u32::from(mem.image_byte(a + 1)) << 8
        });
        let (mode, base) = (half >> 14, self.palette + (half & 0x3FFF) * 4);
        let color = |k: u32| mem.palette_color(base + 2 * k);
        let visible = |c: u16| Texel {
            color: c,
            alpha: OPAQUE,
        };
        match (mode, texel) {
            (0 | 1, 3) => Texel { color: 0, alpha: 0 },
            (_, 0) => visible(color(0)),
            (_, 1) => visible(color(1)),
            (1, 2) => visible(mix(color(0), color(1), 1, 1)),
            (3, 2) => visible(mix(color(0), color(1), 5, 3)),
            (3, _) => visible(mix(color(0), color(1), 3, 5)),
            (_, 2) => visible(color(2)),
            _ => visible(color(3)),
        }
    }
}

/// `(a x wa + b x wb) / (wa + wb)` per 5-bit channel of two 15-bit colours, truncating.
fn mix(a: u16, b: u16, wa: u16, wb: u16) -> u16 {
    let channel = |shift: u32| {
        let (a, b) = ((a >> shift) & 0x1F, (b >> shift) & 0x1F);
        (a * wa + b * wb) / (wa + wb)
    };
    channel(0) | (channel(5) << 5) | (channel(10) << 10)
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

/// Decal (`textures.md`, "Blending the texel with the vertex colour"): the texel alpha is the ratio of
/// texture to vertex colour, `(t x a + v x (63 - a)) / 64` per 6-bit channel, with the vertex colour
/// alone at a texel alpha of 0 and the texel colour alone at 31. The pixel's alpha is the polygon's.
/// Returns the 6-bit colour and the 5-bit alpha.
// TODO(hw-unverified): the special cases test the 5-bit alpha and the ratio uses the 6-bit one, the
// division truncates, and a transparent texel (alpha 0) shows the vertex colour at the polygon's
// alpha (`textures.md`, "Unverified").
pub(super) fn decal(texel: Texel, vertex6: [u8; 3], alpha: u8) -> ([u8; 3], u8) {
    let ratio = u16::from(expand6(u16::from(texel.alpha)));
    let mix = |t: u16, v: u8| match texel.alpha {
        0 => v,
        OPAQUE => t as u8,
        _ => ((t * ratio + u16::from(v) * (63 - ratio)) / 64) as u8,
    };
    let t = |shift: u32| u16::from(expand6((texel.color >> shift) & 0x1F));
    let color = [
        mix(t(0), vertex6[0]),
        mix(t(5), vertex6[1]),
        mix(t(10), vertex6[2]),
    ];
    (color, alpha)
}
