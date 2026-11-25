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
use super::setup::{ATTRS, Attrs, DEPTH, FRAC, IW, IW_ONE, MAX_DEPTH, ONE, PolyDraw, ScreenVertex};
use crate::SCREEN_WIDTH;
use crate::gpu2d::{Line3d, Pixel3d};

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

/// The tolerance of the equal depth test, either way, in the 24-bit depth (`rasterizer.md`, "Depth").
const EQUAL_TOLERANCE: u32 = 0x200;

/// Writes one pixel of attributes `attrs` at `x` if it passes the depth test: closer than what the
/// line holds, or equal to it within [`EQUAL_TOLERANCE`] for a polygon that asks for that
/// (`rasterizer.md`, "Depth").
fn plot(x: i64, attrs: &Attrs, poly: &PolyDraw, state: &mut LineState, out: &mut Line3d) {
    let Ok(x) = usize::try_from(x) else { return };
    if x >= SCREEN_WIDTH {
        return;
    }
    if poly.alpha <= poly.alpha_ref {
        return;
    }
    let iw = attrs[IW].max(1);
    // TODO(hw-unverified): in W-buffer mode the depth is the interpolated W, the 1.19.12 value
    // itself, clamped to 24 bits (`rasterizer.md`, "Unverified").
    let depth = if poly.w_buffer {
        (IW_ONE / iw).clamp(0, MAX_DEPTH) as u32
    } else {
        attrs[DEPTH].clamp(0, MAX_DEPTH) as u32
    };
    let kept = state.depth[x];
    let passes = if poly.equal {
        depth.abs_diff(kept) <= EQUAL_TOLERANCE
    } else {
        depth < kept
    };
    // A translucent polygon does not draw over a pixel another translucent pixel of its own ID wrote.
    // TODO(hw-unverified): the rule for ordinary translucent polygons is the model's
    // (`rasterizer.md`, "Unverified").
    let again = poly.translucent && !poly.shadow && state.translucent[x] && state.id[x] == poly.id;
    if !passes || again {
        return;
    }
    if poly.shadow {
        // The mask pass only sets the stencil where it passes the depth test. The render pass
        // clears a set flag and draws nothing there; where the flag is clear it draws if its ID
        // differs from the pixel's (`rasterizer.md`, "Shadow polygons").
        // TODO(phase7, #51): the fog flag of a shadowed pixel (the old flag ANDed with the
        // polygon's) comes with fog.
        // TODO(hw-unverified): the stencil is checked only where the depth test passes, and neither
        // pass writes depth (`rasterizer.md`, "Unverified").
        if poly.mask {
            state.stencil[x] = true;
            return;
        }
        if state.stencil[x] {
            state.stencil[x] = false;
            return;
        }
        if state.id[x] == poly.id {
            return;
        }
    }
    // TODO(hw-unverified): the colour is perspective correct and narrows by dropping the low bit
    // (`rasterizer.md`, "Unverified").
    let own = [1, 2, 3].map(|k| (attrs[k] / iw).clamp(0, 63) as u8);
    // The alpha already in the output stands for the buffer's alpha; nothing else changes `out` while
    // the polygons draw.
    let below = out[x].alpha;
    let blended = poly.translucent && poly.blend && below != 0;
    // Blending: (poly x (A + 1) + buffer x (31 - A)) / 32 per channel, the alpha the larger of the
    // two; it is skipped for an opaque pixel, with blending off and over alpha 0 (`rasterizer.md`,
    // "Translucency, draw order and the rear plane").
    let color6 = if blended {
        let a = u16::from(poly.alpha);
        // TODO(hw-unverified): the 5-bit formula is applied to 6-bit channels and narrowed after
        // (`rasterizer.md`, "Unverified").
        let mix =
            |new: u8, old: u8| ((u16::from(new) * (a + 1) + u16::from(old) * (31 - a)) / 32) as u8;
        [0, 1, 2].map(|k| mix(own[k], state.color6[x][k]))
    } else {
        own
    };
    // The 6-bit channels, narrowed to the 15-bit output (`rasterizer.md`, "The output").
    let narrow = |c: u8| u16::from(c >> 1);
    out[x] = Pixel3d {
        color: narrow(color6[0]) | narrow(color6[1]) << 5 | narrow(color6[2]) << 10,
        alpha: if blended {
            poly.alpha.max(below)
        } else {
            poly.alpha
        },
    };
    state.color6[x] = color6;
    // A shadow polygon writes no depth in either pass (`rasterizer.md`, "Shadow polygons").
    if !poly.shadow && (!poly.translucent || poly.update_depth) {
        state.depth[x] = depth;
    }
    state.id[x] = poly.id;
    state.translucent[x] = poly.translucent;
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
            plot(dot_x, &vs[0].attrs, poly, state, out);
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
        plot(x, &attrs, poly, state, out);
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
            plot(x, &lerp(&a.attrs, &b.attrs, t), poly, state, out);
        });
    });
}
