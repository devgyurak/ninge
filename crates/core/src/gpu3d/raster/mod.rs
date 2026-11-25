// SPDX-License-Identifier: Apache-2.0
//! The software rasterizer's fill: one frame's polygons into a 256x192 line buffer of
//! [`Pixel3d`] (`docs/hw/rasterizer.md`).
//!
//! A frame is drawn in two steps. [`Frame::new`] makes the per-frame snapshot once: every polygon's
//! vertices mapped to the screen, its attributes and its drawing order. Then each scanline walks
//! that list and writes only its own line ([`Frame::draw_line`]), so the lines are independent of
//! each other and of how a [`ScanlineExecutor`] schedules them (`docs/hw/rasterizer.md`, "The
//! model"). The per-line working state (depth, polygon ID) is a pair of stack arrays: nothing
//! allocates on the pixel path. The snapshot is a pure function of the scene and the registers, so
//! it is never saved; only the finished output is (`Machine::frame3d`).
// TODO(phase7): the resolve pass (edge marking, fog, anti-aliasing) is #51; textures, toon and
// decal shading are #50 and #51; the rear bitmap is #50.

mod setup;
mod span;
#[cfg(test)]
mod tests;

use super::executor::ScanlineExecutor;
use super::render::{RenderRegs, Scene};
use crate::SCREEN_WIDTH;
use crate::gpu2d::{Line3d, Pixel3d};
use setup::{Clear, PolyDraw, ScreenVertex};

/// One frame's drawing snapshot (`docs/hw/rasterizer.md`, "The model").
#[derive(Clone, Debug, Default)]
pub(crate) struct Frame {
    /// The screen vertices of every polygon that draws, polygon after polygon.
    vertices: Vec<ScreenVertex>,
    /// The polygons that draw, in drawing order.
    polygons: Vec<PolyDraw>,
    /// What a pixel nothing covers holds (the rear plane).
    clear: Clear,
}

/// The working state of one line: what the depth test and the ID rules need, per pixel.
struct LineState {
    depth: [u32; SCREEN_WIDTH],
    id: [u8; SCREEN_WIDTH],
    /// The 6-bit colour of each pixel, which a blend reads (the output holds it narrowed).
    color6: [[u8; 3]; SCREEN_WIDTH],
    /// The pixel was last written by a translucent polygon.
    translucent: [bool; SCREEN_WIDTH],
    /// The stencil flag shadow polygons set and clear (`rasterizer.md`, "Shadow polygons").
    stencil: [bool; SCREEN_WIDTH],
}

impl Frame {
    /// The snapshot of `scene` under the rendering registers `regs` and DISP3DCNT `disp3dcnt`.
    pub(crate) fn new(scene: Scene<'_>, regs: &RenderRegs, disp3dcnt: u16) -> Self {
        setup::build(scene, regs, disp3dcnt)
    }

    /// Draws line `y` into `out`: the rear plane, then every polygon in order. Reads only the
    /// snapshot and writes only `out`.
    pub(crate) fn draw_line(&self, y: usize, out: &mut Line3d) {
        let clear = Pixel3d {
            color: self.clear.color,
            alpha: self.clear.alpha,
        };
        out.fill(clear);
        let mut state = LineState {
            depth: [self.clear.depth; SCREEN_WIDTH],
            id: [self.clear.id; SCREEN_WIDTH],
            color6: [self.clear.color6; SCREEN_WIDTH],
            translucent: [false; SCREEN_WIDTH],
            stencil: [false; SCREEN_WIDTH],
        };
        self.polygons.iter().for_each(|poly| {
            span::draw_polygon(poly, &self.vertices, y, &mut state, out);
        });
    }

    /// Draws every line of `lines` (one per screen line) through `executor`.
    pub(crate) fn draw<E: ScanlineExecutor>(&self, executor: &E, lines: &mut [Line3d]) {
        executor.for_each_line(lines, |y, line| self.draw_line(y, line));
    }
}
