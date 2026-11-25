// SPDX-License-Identifier: Apache-2.0
//! The per-frame snapshot: clip coordinates to the screen through each polygon's VIEWPORT, the
//! polygon's drawing attributes and the rear plane (`docs/hw/rasterizer.md`, "From clip
//! coordinates to the screen", "Which pixels a polygon covers", "The rear plane").

use super::Frame;
use crate::gpu3d::polygon::{MAX_POLYGON_VERTICES, Vertex};
use crate::gpu3d::render::{RenderRegs, Scene};

/// Bits after the point in a screen coordinate.
pub(super) const FRAC: u32 = 8;
/// One pixel in a screen coordinate.
pub(super) const ONE: i64 = 1 << FRAC;
/// The attributes interpolated across a polygon: 1/W, the three colour channels times 1/W, depth.
pub(super) const ATTRS: usize = 5;
pub(super) type Attrs = [i64; ATTRS];
/// Index of 1/W in [`Attrs`]; the channels follow, the depth is last.
pub(super) const IW: usize = 0;
pub(super) const DEPTH: usize = 4;
/// 1/W is held with this many fractional bits.
pub(super) const IW_ONE: i64 = 1 << 40;
/// The largest 24-bit depth.
pub(super) const MAX_DEPTH: i64 = 0x00FF_FFFF;

// POLYGON_ATTR fields (`docs/hw/geometry-engine.md`).
const ATTR_MODE_SHIFT: u32 = 4;
const ATTR_BACK: u32 = 1 << 6;
const ATTR_FRONT: u32 = 1 << 7;
const ATTR_ALPHA_SHIFT: u32 = 16;
const ATTR_ID_SHIFT: u32 = 24;
const MODE_SHADOW: u32 = 3;
/// POLYGON_ATTR bit 14: the depth test passes on equal depth, not less.
const ATTR_DEPTH_EQUAL: u32 = 1 << 14;
/// SWAP_BUFFERS bit 1: the depth buffer holds W, not Z.
const SWAP_W_BUFFER: u8 = 1 << 1;
const OPAQUE: u32 = 31;
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
    /// The polygon ID, 6 bits.
    pub(super) id: u8,
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

/// The rear plane's value for a pixel nothing covers.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Clear {
    pub(super) color: u16,
    pub(super) alpha: u8,
    pub(super) depth: u32,
    pub(super) id: u8,
}

/// A 5-bit channel expanded to 6 bits: `X x 2 + 1`, but 0 stays 0 (`rasterizer.md`, "Colour along
/// a polygon").
fn expand6(c5: u16) -> i64 {
    if c5 == 0 { 0 } else { i64::from(c5) * 2 + 1 }
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
    // TODO(hw-unverified): the Z-to-24-bit depth mapping and the 1/W weights are the model's
    // (`rasterizer.md`, "Unverified").
    let depth = ((i64::from(v.z) + w) * MAX_DEPTH / (2 * w)).clamp(0, MAX_DEPTH);
    let iw = IW_ONE / w;
    let channel = |shift: u32| expand6((v.color >> shift) & 0x1F) * iw;
    ScreenVertex {
        x: sx,
        y: sy,
        attrs: [iw, channel(0), channel(5), channel(10), depth],
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
        clear: clear_from(regs),
    };
    let marked = disp3dcnt & (DISP3DCNT_EDGE_MARKING | DISP3DCNT_ANTIALIAS) != 0;
    ram.polygons.iter().for_each(|p| {
        let alpha = (p.attr >> ATTR_ALPHA_SHIFT) & 0x1F;
        let mode = (p.attr >> ATTR_MODE_SHIFT) & 3;
        // TODO(phase7): translucent polygons are #49's next part and shadow polygons the one after;
        // until then they draw nothing.
        if (alpha != 0 && alpha != OPAQUE) || mode == MODE_SHADOW {
            return;
        }
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
        let segment = source.iter().enumerate().any(|(i, a)| {
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
        // Polygons of 0x0, 1x0, 0x1 and 1x1 dots all draw as one dot (`rasterizer.md`).
        // TODO(hw-unverified): DISP_1DOT_DEPTH is not stored yet, so the dot passes the depth check.
        let dot = (x_hi - x_lo <= 1 && y_hi - y_lo <= 1).then_some((x_lo, y_lo));
        // A line segment draws as a wire whatever its alpha.
        let y_min_fix = screen.iter().map(|v| v.y).min().unwrap_or(0);
        let y_max_fix = screen.iter().map(|v| v.y).max().unwrap_or(0);
        let wire = alpha == 0 || segment;
        let flat = y_min_fix == y_max_fix;
        frame.polygons.push(PolyDraw {
            first: frame.vertices.len(),
            count: screen.len(),
            id: ((p.attr >> ATTR_ID_SHIFT) & 0x3F) as u8,
            wire,
            exclude_edges: !wire && !marked,
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
    frame
}

/// The rear plane from CLEAR_COLOR and CLEAR_DEPTH.
fn clear_from(regs: &RenderRegs) -> Clear {
    Clear {
        color: (regs.clear_color & 0x7FFF) as u16,
        alpha: ((regs.clear_color >> 16) & 0x1F) as u8,
        depth: expand_clear_depth(regs.clear_depth),
        id: ((regs.clear_color >> 24) & 0x3F) as u8,
    }
}
