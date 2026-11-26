// SPDX-License-Identifier: Apache-2.0
//! The per-frame snapshot: clip coordinates to the screen through each polygon's VIEWPORT, the
//! polygon's drawing attributes and the rear plane (`docs/hw/rasterizer.md`, "From clip
//! coordinates to the screen", "Which pixels a polygon covers", "The rear plane").

use super::Frame;
use super::texture::{self, Tex};
use crate::gpu3d::polygon::{MAX_POLYGON_VERTICES, Vertex};
use crate::gpu3d::render::{RenderRegs, Scene};

/// Bits after the point in a screen coordinate.
pub(super) const FRAC: u32 = 8;
/// One pixel in a screen coordinate.
pub(super) const ONE: i64 = 1 << FRAC;
/// The attributes interpolated across a polygon: 1/W, the three colour channels times 1/W, depth,
/// and S and T times 1/W.
pub(super) const ATTRS: usize = 7;
pub(super) type Attrs = [i64; ATTRS];
/// Index of 1/W in [`Attrs`]; the channels follow, the depth is last.
pub(super) const IW: usize = 0;
pub(super) const DEPTH: usize = 4;
/// Index of S (1.11.4) times 1/W; T follows.
pub(super) const S: usize = 5;
pub(super) const T: usize = 6;
/// 1/W is held with this many fractional bits.
pub(super) const IW_ONE: i64 = 1 << 40;
/// The most a screen coordinate may be from 0, in 1/256 pixel (65536 pixels).
const COORD_LIMIT: i64 = 1 << 24;
/// The largest 24-bit depth.
pub(super) const MAX_DEPTH: i64 = 0x00FF_FFFF;

// POLYGON_ATTR fields (`docs/hw/geometry-engine.md`).
const ATTR_MODE_SHIFT: u32 = 4;
const ATTR_BACK: u32 = 1 << 6;
const ATTR_FRONT: u32 = 1 << 7;
const ATTR_ALPHA_SHIFT: u32 = 16;
const ATTR_ID_SHIFT: u32 = 24;
const MODE_MODULATION: u32 = 0;
const MODE_DECAL: u32 = 1;
const MODE_SHADOW: u32 = 3;
/// POLYGON_ATTR bit 14: the depth test passes on equal depth, not less.
const ATTR_DEPTH_EQUAL: u32 = 1 << 14;
/// POLYGON_ATTR bit 13: draw a 1-dot polygon behind DISP_1DOT_DEPTH too.
const ATTR_ONE_DOT: u32 = 1 << 13;
/// A clip W has 12 fractional bits and DISP_1DOT_DEPTH 3: shift right by the difference.
const W_TO_ONE_DOT: u32 = 9;
/// SWAP_BUFFERS bit 1: the depth buffer holds W, not Z.
const SWAP_W_BUFFER: u8 = 1 << 1;
/// SWAP_BUFFERS bit 0: translucent polygons keep the order they were completed in.
const SWAP_MANUAL_SORT: u8 = 1;
/// POLYGON_ATTR bit 11: a translucent pixel writes its depth.
const ATTR_UPDATE_DEPTH: u32 = 1 << 11;
// DISP3DCNT bits 2 and 3.
const DISP3DCNT_ALPHA_TEST: u16 = 1 << 2;
const DISP3DCNT_BLEND: u16 = 1 << 3;
/// The alpha of an opaque pixel (`texture::OPAQUE`) as a POLYGON_ATTR field.
pub(super) const OPAQUE: u32 = texture::OPAQUE as u32;
/// DISP3DCNT bit 0: texture mapping on, and bit 14: the rear plane is the bitmaps in texture slots 2
/// and 3.
const DISP3DCNT_TEXTURE: u16 = 1;
const DISP3DCNT_REAR_BITMAP: u16 = 1 << 14;
// DISP3DCNT bits that change which edges draw.
const DISP3DCNT_ANTIALIAS: u16 = 1 << 4;
const DISP3DCNT_EDGE_MARKING: u16 = 1 << 5;

/// One vertex on the screen: a position in 1/256 pixel (y down, line 0 first) and its attributes.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ScreenVertex {
    pub(super) x: i64,
    pub(super) y: i64,
    pub(super) attrs: Attrs,
}

/// One polygon that draws.
#[derive(Clone, Copy, Debug)]
pub(super) struct PolyDraw {
    /// Index of its first vertex in the frame's vertex list, and how many it has.
    pub(super) first: usize,
    pub(super) count: usize,
    /// The texture the pixels are blended with, if the polygon has one that is drawn.
    pub(super) tex: Option<Tex>,
    /// Polygon mode 1: the texture blends by its alpha instead of multiplying the vertex colour.
    pub(super) decal: bool,
    /// The polygon ID, 6 bits.
    pub(super) id: u8,
    /// The alpha of the polygon's pixels, 5 bits: 31 for an opaque polygon, a wire frame and a line
    /// segment, the POLYGON_ATTR alpha for a translucent one.
    pub(super) alpha: u8,
    /// Alpha 1 to 30, or an A3I5 or A5I3 texture in modulation mode: drawn after the opaque polygons
    /// and blended into the buffer where a pixel's own alpha is below 31.
    pub(super) translucent: bool,
    /// A translucent pixel writes its depth (POLYGON_ATTR bit 11).
    pub(super) update_depth: bool,
    /// DISP3DCNT bit 3: translucent pixels blend with what is there.
    pub(super) blend: bool,
    /// A pixel draws only if its alpha is greater than this: ALPHA_TEST_REF with the test on, else
    /// 0.
    pub(super) alpha_ref: u8,
    /// A shadow polygon (polygon mode 3): two passes over a stencil buffer.
    pub(super) shadow: bool,
    /// A shadow polygon of ID 0, the mask pass: it draws no colour and sets the stencil.
    pub(super) mask: bool,
    /// A mask pass that follows render passes: it starts a new volume and clears the stencil first.
    pub(super) new_volume: bool,
    /// Alpha 0: only the edges draw.
    pub(super) wire: bool,
    /// The lower and right edges do not draw (`rasterizer.md`, "Which pixels a polygon covers").
    pub(super) exclude_edges: bool,
    /// The depth test passes on equal depth (within a tolerance) instead of less.
    pub(super) equal: bool,
    /// The depth is the W value, not Z (SWAP_BUFFERS bit 1).
    pub(super) w_buffer: bool,
    /// Every vertex is on one row: it draws that row, edges and all.
    pub(super) flat: bool,
    /// The polygon rounds to one dot: where, in whole pixels.
    pub(super) dot: Option<(i64, i64)>,
    /// The vertical extent in 1/256 pixel.
    pub(super) y_min: i64,
    pub(super) y_max: i64,
}

/// The scroll of the rear bitmaps: CLRIMAGE_OFFSET's X in bits 0-7 and Y in bits 8-14.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RearBitmap {
    pub(super) x: u8,
    pub(super) y: u8,
}

/// The rear plane's value for a pixel nothing covers.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Clear {
    /// The rear bitmaps in texture slots 2 and 3 (DISP3DCNT bit 14), with the scroll offsets; `None`
    /// for the uniform plane of CLEAR_COLOR and CLEAR_DEPTH.
    pub(super) bitmap: Option<RearBitmap>,
    pub(super) color: u16,
    /// The colour as 6-bit channels (`X x 2 + 1`, 0 stays 0).
    pub(super) color6: [u8; 3],
    pub(super) alpha: u8,
    pub(super) depth: u32,
    pub(super) id: u8,
}

/// A 5-bit channel expanded to 6 bits (`texture::expand6`).
fn expand6(c5: u16) -> i64 {
    i64::from(texture::expand6(c5))
}

/// The 15-bit CLEAR_DEPTH as 24 bits: `X x 200h + ((X + 1) / 8000h) x 1FFh`
/// (`rasterizer.md`, "Depth").
pub(super) fn expand_clear_depth(x: u16) -> u32 {
    let x = u32::from(x);
    x * 0x200 + (x + 1) / 0x8000 * 0x1FF
}

/// The vertex `v` seen through `viewport` (X1, Y1, X2, Y2 in bytes, origin lower left).
fn to_screen(v: &Vertex, viewport: u32) -> ScreenVertex {
    let byte = |shift: u32| i64::from((viewport >> shift) & 0xFF);
    let (x1, y1, x2, y2) = (byte(0), byte(8), byte(16), byte(24));
    // TODO(hw-unverified): the width and height are one more than the corner difference, and the
    // division truncates at 1/256 pixel (`rasterizer.md`, "Unverified").
    let width = (x2 - x1 + 1).max(1);
    let height = (y2 - y1 + 1).max(1);
    let w = i64::from(v.w).max(1);
    let sx = (i64::from(v.x) + w) * width * ONE / (2 * w) + x1 * ONE;
    let sy_up = (i64::from(v.y) + w) * height * ONE / (2 * w) + y1 * ONE;
    // The screen is 192 lines tall: a viewport y of 192 is the top edge of line 0. Positions are
    // pixel edges, so a polygon that spans the screen spans lines 0 to 191.
    let sy = 192 * ONE - sy_up;
    // A vertex far outside the screen (a crafted savestate can hold one) is held to a bound that
    // keeps every product below in range. It only guards that case: clamping changes the slope of
    // an edge that starts far out, so a polygon with such a vertex draws different pixels than it
    // would unclamped.
    let (sx, sy) = (
        sx.clamp(-COORD_LIMIT, COORD_LIMIT),
        sy.clamp(-COORD_LIMIT, COORD_LIMIT),
    );
    // TODO(hw-unverified): the Z-to-24-bit depth mapping and the 1/W weights are the model's
    // (`rasterizer.md`, "Unverified").
    let depth = ((i64::from(v.z) + w) * MAX_DEPTH / (2 * w)).clamp(0, MAX_DEPTH);
    let iw = IW_ONE / w;
    let channel = |shift: u32| expand6((v.color >> shift) & 0x1F) * iw;
    ScreenVertex {
        x: sx,
        y: sy,
        attrs: [
            iw,
            channel(0),
            channel(5),
            channel(10),
            depth,
            i64::from(v.s) * iw,
            i64::from(v.t) * iw,
        ],
    }
}

/// Twice the signed area of the screen polygon, y down: negative for an anti-clockwise polygon as
/// the viewer sees it (a front face).
fn area2(vs: &[ScreenVertex]) -> i64 {
    (0..vs.len())
        .map(|i| {
            let (a, b) = (&vs[i], &vs[(i + 1) % vs.len()]);
            (a.x * b.y - b.x * a.y) / ONE
        })
        .sum()
}

/// The snapshot of a scene (`Frame::new`).
pub(super) fn build(scene: Scene<'_>, regs: &RenderRegs, disp3dcnt: u16) -> Frame {
    let ram = scene.ram;
    let mut frame = Frame {
        vertices: Vec::with_capacity(ram.vertices.len()),
        polygons: Vec::with_capacity(ram.polygons.len()),
        clear: clear_from(regs, disp3dcnt),
    };
    let marked = disp3dcnt & (DISP3DCNT_EDGE_MARKING | DISP3DCNT_ANTIALIAS) != 0;
    ram.polygons.iter().for_each(|p| {
        let alpha = (p.attr >> ATTR_ALPHA_SHIFT) & 0x1F;
        let mode = (p.attr >> ATTR_MODE_SHIFT) & 3;
        let shadow = mode == MODE_SHADOW;
        let n = usize::from(p.count).min(MAX_POLYGON_VERTICES);
        let mut source = [Vertex::default(); MAX_POLYGON_VERTICES];
        let mut screen = [ScreenVertex::default(); MAX_POLYGON_VERTICES];
        // `n` is at most MAX_POLYGON_VERTICES, the length of both arrays.
        for k in 0..n {
            let Some(v) = p
                .vertices
                .get(k)
                .and_then(|&i| ram.vertices.get(usize::from(i)))
            else {
                return;
            };
            source[k] = *v;
            screen[k] = to_screen(v, p.viewport);
        }
        let (source, screen) = (&source[..n], &screen[..n]);
        // A line segment is a triangle with a repeated vertex (`rasterizer.md`); a quad with one
        // repeated vertex is an ordinary polygon.
        let segment = n == 3
            && source.iter().enumerate().any(|(i, a)| {
                source[i + 1..]
                    .iter()
                    .any(|b| (a.x, a.y, a.z, a.w) == (b.x, b.y, b.z, b.w))
            });
        // TODO(hw-unverified): a polygon of zero area counts as front facing.
        let front = area2(screen) <= 0;
        let shown = segment
            || if front {
                p.attr & ATTR_FRONT
            } else {
                p.attr & ATTR_BACK
            } != 0;
        if !shown || screen.is_empty() {
            return;
        }
        let round = |v: i64| (v + ONE / 2) >> FRAC;
        let xs = screen.iter().map(|v| round(v.x));
        let ys = screen.iter().map(|v| round(v.y));
        let (x_lo, x_hi) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
        let (y_lo, y_hi) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
        // Polygons of 0x0, 1x0, 0x1 and 1x1 dots all draw as one dot (`rasterizer.md`). Only the
        // 0x0 one is subject to the 1-dot depth check: POLYGON_ATTR bit 13, or a vertex whose W
        // (12.3 fixed point) is at most DISP_1DOT_DEPTH; a polygon that fails it does not draw.
        let dot = (x_hi - x_lo <= 1 && y_hi - y_lo <= 1).then_some((x_lo, y_lo));
        // TODO(hw-unverified): the vertex W is truncated, not rounded, to the register's 3
        // fractional bits for the comparison (`rasterizer.md`, "Unverified").
        let nearest_w = source
            .iter()
            .map(|v| i64::from(v.w) >> W_TO_ONE_DOT)
            .min()
            .unwrap_or(0);
        let hidden_dot = x_hi == x_lo
            && y_hi == y_lo
            && p.attr & ATTR_ONE_DOT == 0
            && nearest_w > i64::from(regs.one_dot_depth);
        if hidden_dot {
            return;
        }
        // A line segment draws as a wire whatever its alpha.
        let y_min_fix = screen.iter().map(|v| v.y).min().unwrap_or(0);
        let y_max_fix = screen.iter().map(|v| v.y).max().unwrap_or(0);
        let wire = alpha == 0 || segment;
        // A wire frame (alpha 0) draws at alpha 31; a line segment of alpha 1 to 30 is translucent
        // like any polygon, and a shadow polygon is translucent whatever its alpha: it draws after
        // the opaque ones (`rasterizer.md`, "Which pixels a polygon covers", "Shadow polygons").
        let by_alpha = alpha != 0 && (alpha != OPAQUE || shadow);
        // TODO(phase7, #51): toon and highlight (mode 2) draw no texture yet. TODO(hw-unverified): a
        // shadow polygon draws no texture (`textures.md`, "Unverified").
        let decal = mode == MODE_DECAL;
        let tex = if (mode == MODE_MODULATION || decal) && disp3dcnt & DISP3DCNT_TEXTURE != 0 {
            Tex::parse(p.texture, p.palette)
        } else {
            None
        };
        // A polygon whose texels can hold a partial alpha draws with the translucent ones, but only
        // where that alpha reaches the pixel: decal ignores it (`textures.md`, "Texture alpha,
        // translucency and the buffers"). TODO(hw-unverified): the draw order of such a polygon.
        let translucent = by_alpha || (!decal && tex.is_some_and(|t| t.has_alpha()));
        let blend = disp3dcnt & DISP3DCNT_BLEND != 0;
        let flat = y_min_fix == y_max_fix;
        frame.polygons.push(PolyDraw {
            first: frame.vertices.len(),
            count: screen.len(),
            tex,
            decal,
            id: ((p.attr >> ATTR_ID_SHIFT) & 0x3F) as u8,
            // TODO(hw-unverified): a wire frame (alpha 31 here) goes through the alpha test like
            // any pixel (`rasterizer.md`, "Unverified").
            alpha: if by_alpha { alpha as u8 } else { OPAQUE as u8 },
            shadow,
            mask: shadow && (p.attr >> ATTR_ID_SHIFT) & 0x3F == 0,
            new_volume: false,
            translucent,
            update_depth: p.attr & ATTR_UPDATE_DEPTH != 0,
            blend,
            alpha_ref: if disp3dcnt & DISP3DCNT_ALPHA_TEST != 0 {
                regs.alpha_test_ref
            } else {
                0
            },
            wire,
            // Opaque polygons without edge marking and anti-aliasing, and translucent ones with
            // blending off, drop their lower and right edges (`rasterizer.md`).
            // TODO(hw-unverified): a polygon with an alpha-format texture follows the translucent
            // polygons' edge rule (`textures.md`, "Unverified").
            exclude_edges: !wire && if translucent { !blend } else { !marked },
            equal: p.attr & ATTR_DEPTH_EQUAL != 0,
            w_buffer: scene.swap_bits & SWAP_W_BUFFER != 0,
            flat,
            dot,
            y_min: y_min_fix,
            y_max: y_max_fix,
        });
        let first = frame.vertices.len();
        frame.vertices.extend_from_slice(screen);
        if flat {
            // The hardware rounds a polygon's height up to at least one line: a level polygon draws
            // on the line nearest it (`rasterizer.md`, "Which pixels a polygon covers").
            // TODO(hw-unverified): which line a level polygon between two lines draws on.
            let line = round(y_min_fix) << FRAC;
            frame.vertices[first..].iter_mut().for_each(|v| v.y = line);
            if let Some(poly) = frame.polygons.last_mut() {
                (poly.y_min, poly.y_max) = (line, line);
            }
        }
    });
    // Opaque polygons first, then the translucent ones, sorted by their top line unless the swap
    // asked for manual order. The sort is stable, so equal keys keep the order of completion.
    // TODO(hw-unverified): the sort key (`rasterizer.md`, "Unverified").
    let manual = scene.swap_bits & SWAP_MANUAL_SORT != 0;
    frame.polygons.sort_by_key(|p| {
        (
            p.translucent,
            if p.translucent && !manual { p.y_min } else { 0 },
        )
    });
    // A mask pass that follows render passes starts a new shadow volume: the flags of the last one
    // are cleared before it sets its own. Decided on the final drawing order, per polygon and not
    // per pixel, so every line agrees (`rasterizer.md`, "Shadow polygons"). TODO(hw-unverified):
    // when the stencil is cleared (`rasterizer.md`, "Unverified").
    let mut render_seen = false;
    frame
        .polygons
        .iter_mut()
        .filter(|p| p.shadow)
        .for_each(|p| {
            if p.mask {
                p.new_volume = render_seen;
                render_seen = false;
            } else {
                render_seen = true;
            }
        });
    frame
}

/// The rear plane from CLEAR_COLOR and CLEAR_DEPTH.
fn clear_from(regs: &RenderRegs, disp3dcnt: u16) -> Clear {
    // TODO(hw-unverified): the clear colour is the 15-bit register value as it is; whether it is
    // expanded to 6 bits like a vertex colour is not stated (`rasterizer.md`, "Unverified").
    Clear {
        // TODO(hw-unverified): the Y offset is bits 8-14 as the register stores them, so 0 to 127
        // (`textures.md`, "Unverified").
        bitmap: (disp3dcnt & DISP3DCNT_REAR_BITMAP != 0).then_some(RearBitmap {
            x: (regs.clrimage_offset & 0xFF) as u8,
            y: ((regs.clrimage_offset >> 8) & 0x7F) as u8,
        }),
        color: (regs.clear_color & 0x7FFF) as u16,
        color6: [0, 5, 10].map(|shift| expand6(((regs.clear_color >> shift) & 0x1F) as u16) as u8),
        alpha: ((regs.clear_color >> 16) & 0x1F) as u8,
        depth: expand_clear_depth(regs.clear_depth),
        id: ((regs.clear_color >> 24) & 0x3F) as u8,
    }
}
