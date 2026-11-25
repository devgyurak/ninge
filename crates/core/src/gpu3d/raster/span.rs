// SPDX-License-Identifier: Apache-2.0
//! One polygon on one scanline: where it covers, what it interpolates, and the depth test
//! (`docs/hw/rasterizer.md`, "Which pixels a polygon covers", "Colour along a polygon", "Depth").
//!
//! A scanline samples the polygon at its integer y. A pixel is covered from the first whole x at or
//! right of the left edge's crossing to the last whole x left of the right edge's crossing; a
//! polygon that keeps its lower and right edges also covers the last row and the crossing's own x.
//! All of it is arithmetic on 1/256 pixels and 16-bit-fraction weights, truncating toward minus
//! infinity (`>>`), in `i64`.
// TODO(hw-unverified): the coverage rule, the precision and the rounding are the model's, not
// stated by the source (`rasterizer.md`, "Unverified").

use super::LineState;
use super::setup::{ATTRS, Attrs, DEPTH, FRAC, IW, MAX_DEPTH, ONE, PolyDraw, ScreenVertex};
use crate::SCREEN_WIDTH;
use crate::gpu2d::{Line3d, Pixel3d};

/// Opaque, in 5 bits.
const OPAQUE: u8 = 31;
/// A weight of 1.0 in `lerp`.
const WEIGHT_ONE: i64 = 1 << 16;

/// `a` toward `b` by `t` (0 to [`WEIGHT_ONE`]), per attribute. The product stays under 2^63: a
/// channel difference is at most 63 x 2^40 (1/W is at most 2^40) and `t` at most 2^16.
pub(super) fn lerp(a: &Attrs, b: &Attrs, t: i64) -> Attrs {
    let mut out = [0; ATTRS];
    (0..ATTRS).for_each(|k| out[k] = a[k] + (((b[k] - a[k]) * t) >> 16));
    out
}

/// A crossing of the scanline with an edge.
#[derive(Clone, Copy)]
struct Cross {
    x: i64,
    attrs: Attrs,
    /// The edge is vertical.
    vertical: bool,
}

/// The leftmost and rightmost crossings of row `y` (in 1/256 pixel) with the polygon's edges;
/// `closed` takes an edge's last row too.
fn crossings(vs: &[ScreenVertex], y: i64, closed: bool) -> Option<(Cross, Cross)> {
    let mut left: Option<Cross> = None;
    let mut right: Option<Cross> = None;
    let mut push = |c: Cross| {
        if left.is_none_or(|l| c.x < l.x) {
            left = Some(c);
        }
        match right.as_mut() {
            // Crossings at one x: the right edge is vertical only if every edge there is.
            Some(r) if c.x == r.x => r.vertical &= c.vertical,
            Some(r) if c.x < r.x => {}
            _ => right = Some(c),
        }
    };
    (0..vs.len()).for_each(|i| {
        let (a, b) = (&vs[i], &vs[(i + 1) % vs.len()]);
        let dy = b.y - a.y;
        if dy == 0 {
            // A level edge on this row: both ends cover it, but only when the last row counts.
            if closed && a.y == y {
                push(Cross {
                    x: a.x,
                    attrs: a.attrs,
                    vertical: true,
                });
                push(Cross {
                    x: b.x,
                    attrs: b.attrs,
                    vertical: true,
                });
            }
            return;
        }
        let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
        let inside = lo <= y && (y < hi || closed && y == hi);
        if inside {
            let t = ((y - a.y) << 16) / dy;
            push(Cross {
                x: a.x + (((b.x - a.x) * t) >> 16),
                attrs: lerp(&a.attrs, &b.attrs, t),
                vertical: a.x == b.x,
            });
        }
    });
    left.zip(right)
}

/// Writes one pixel of attributes `attrs` at `x` if it passes the depth test: closer than what the
/// line holds (`rasterizer.md`, "Depth").
// TODO(phase7): the equal test and W-buffering are the next part of #49.
fn plot(x: i64, attrs: &Attrs, id: u8, state: &mut LineState, out: &mut Line3d) {
    let Ok(x) = usize::try_from(x) else { return };
    if x >= SCREEN_WIDTH {
        return;
    }
    let depth = attrs[DEPTH].clamp(0, MAX_DEPTH) as u32;
    if depth >= state.depth[x] {
        return;
    }
    let iw = attrs[IW].max(1);
    // TODO(hw-unverified): the colour is perspective correct and narrows by dropping the low bit
    // (`rasterizer.md`, "Unverified").
    // The 6-bit channel, narrowed to the 15-bit output (`rasterizer.md`, "The output").
    let channel = |k: usize| ((attrs[k] / iw).clamp(0, 63) >> 1) as u16;
    out[x] = Pixel3d {
        color: channel(1) | channel(2) << 5 | channel(3) << 10,
        alpha: OPAQUE,
    };
    state.depth[x] = depth;
    state.id[x] = id;
}

/// Polygon `poly` on line `y`: its interior, its edges (a wire frame) or its one dot.
pub(super) fn draw_polygon(
    poly: &PolyDraw,
    vertices: &[ScreenVertex],
    y: usize,
    state: &mut LineState,
    out: &mut Line3d,
) {
    let Some(vs) = vertices.get(poly.first..poly.first + poly.count) else {
        return;
    };
    let row = (y as i64) << FRAC;
    if row + ONE / 2 <= poly.y_min || row - ONE / 2 > poly.y_max {
        return;
    }
    // A polygon whose vertices all round to one dot draws that dot at its first vertex's depth and
    // colour (`rasterizer.md`), whatever its type.
    if let Some((dot_x, dot_y)) = poly.dot {
        if dot_y == y as i64 {
            plot(dot_x, &vs[0].attrs, poly.id, state, out);
        }
        return;
    }
    if poly.wire {
        wire(poly, vs, row, state, out);
    } else {
        fill(poly, vs, row, state, out);
    }
}

/// The interior of an opaque polygon on row `row`.
fn fill(poly: &PolyDraw, vs: &[ScreenVertex], row: i64, state: &mut LineState, out: &mut Line3d) {
    let Some((left, right)) = crossings(vs, row, !poly.exclude_edges || poly.flat) else {
        return;
    };
    // First whole x at or right of the left crossing; the last one left of the right crossing, or on
    // it when the right edge draws. A vertical right edge never does.
    let keeps_right = !poly.exclude_edges && !right.vertical;
    let first = (left.x + ONE - 1) >> FRAC;
    let last = if keeps_right {
        right.x >> FRAC
    } else {
        ((right.x + ONE - 1) >> FRAC) - 1
    };
    // The hardware rounds a polygon's width up to at least one pixel: a span that no pixel starts in
    // still draws its first one (`rasterizer.md`, "Which pixels a polygon covers").
    // TODO(hw-unverified): that a sliver of a larger polygon is widened the same way.
    let last = last.max(first);
    let span = right.x - left.x;
    (first.max(0)..=last.min(SCREEN_WIDTH as i64 - 1)).for_each(|x| {
        let t = if span == 0 {
            0
        } else {
            (((x << FRAC) - left.x) << 16) / span
        };
        let attrs = lerp(&left.attrs, &right.attrs, t.clamp(0, WEIGHT_ONE));
        plot(x, &attrs, poly.id, state, out);
    });
}

/// The edges of a wire frame on row `row`: each edge covers the whole x range it crosses within the
/// row's band, one pixel for a steep edge, at the fixed alpha of 31.
// TODO(hw-unverified): the line algorithm is the model's (`rasterizer.md`, "Unverified").
fn wire(poly: &PolyDraw, vs: &[ScreenVertex], row: i64, state: &mut LineState, out: &mut Line3d) {
    let (band_lo, band_hi) = (row - ONE / 2, row + ONE / 2 - 1);
    (0..vs.len()).for_each(|i| {
        let (a, b) = (&vs[i], &vs[(i + 1) % vs.len()]);
        let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
        let (y0, y1) = (band_lo.max(lo), band_hi.min(hi));
        if y0 > y1 {
            return;
        }
        let at = |y: i64| -> (i64, i64) {
            let dy = b.y - a.y;
            let t = if dy == 0 {
                0
            } else {
                (((y - a.y) << 16) / dy).clamp(0, WEIGHT_ONE)
            };
            (a.x + (((b.x - a.x) * t) >> 16), t)
        };
        // A level edge spans its whole length in its row.
        let (xa, ta, xb, tb) = if a.y == b.y {
            (a.x, 0, b.x, WEIGHT_ONE)
        } else {
            let ((xa, ta), (xb, tb)) = (at(y0), at(y1));
            (xa, ta, xb, tb)
        };
        let round = |v: i64| (v + ONE / 2) >> FRAC;
        let (x_lo, x_hi) = (round(xa.min(xb)), round(xa.max(xb)));
        (x_lo..=x_hi).for_each(|x| {
            let t = (ta + tb) / 2;
            plot(x, &lerp(&a.attrs, &b.attrs, t), poly.id, state, out);
        });
    });
}
