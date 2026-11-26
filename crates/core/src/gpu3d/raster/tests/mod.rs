// SPDX-License-Identifier: Apache-2.0
//! The rasterizer's fill, worked out by hand from `docs/hw/rasterizer.md`. The helpers build a
//! scene in clip space with the viewport that covers the screen and a W of 3.0, where one screen
//! pixel is 96 clip units across and 128 down, so every expected pixel is exact.

mod alpha;
mod depth;
mod fill;
mod interp;
mod shadow;
mod texture;

use super::Frame;
use crate::SCREEN_WIDTH;
use crate::bus::Textures;
use crate::gpu2d::{Line3d, Pixel3d};
use crate::gpu3d::executor::SequentialExecutor;
use crate::gpu3d::polygon::{Polygon, Ram, Vertex};
use crate::gpu3d::render::{RenderRegs, Scene};

/// The full-screen viewport: X1 0, Y1 0, X2 255, Y2 191.
pub(super) const VIEWPORT: u32 = (191 << 24) | (255 << 16);
/// A W of 3.0 in 1.19.12.
const W: i32 = 3 << 12;
const FRONT_BACK: u32 = 0xC0;
pub(super) const OPAQUE: u32 = 31 << 16;
pub(super) const RED: u16 = 0x001F;
pub(super) const GREEN: u16 = 0x03E0;

/// Like [`vertex`] with a W of `k` times 3.0: the position, the colour and the depth ratio `z / w`
/// are those of `vertex(px, row, z / k, ..)`, so a polygon of another W lands on the same pixels.
pub(super) fn vertex_k(px: i32, row: i32, z: i32, color: u16, k: i32) -> Vertex {
    let v = vertex(px, row, 0, color);
    Vertex {
        x: v.x * k,
        y: v.y * k,
        z,
        w: v.w * k,
        ..v
    }
}

/// A clip-space vertex at screen position `(px, row)` in whole pixel edges and depth `z`
/// (-W near, W far), coloured `color`.
pub(super) fn vertex(px: i32, row: i32, z: i32, color: u16) -> Vertex {
    Vertex {
        x: 96 * px - W,
        y: 128 * (192 - row) - W,
        z,
        w: W,
        color,
        s: 0,
        t: 0,
    }
}

/// A scene of polygons, each `(attr, vertices)` with the vertices in drawing order.
pub(super) fn scene(polygons: &[(u32, Vec<Vertex>)]) -> Ram {
    let mut ram = Ram::default();
    polygons.iter().for_each(|(attr, vs)| {
        let mut polygon = Polygon {
            attr: *attr,
            viewport: VIEWPORT,
            count: vs.len() as u8,
            ..Polygon::default()
        };
        vs.iter().enumerate().for_each(|(k, v)| {
            polygon.vertices[k] = ram.vertices.len() as u16;
            ram.vertices.push(*v);
        });
        ram.polygons.push(polygon);
    });
    ram
}

/// An anti-clockwise quad (top left, bottom left, bottom right, top right) from `(x0, y0)` to
/// `(x1, y1)` at depth `z`, one colour.
pub(super) fn quad(x0: i32, y0: i32, x1: i32, y1: i32, z: i32, color: u16) -> Vec<Vertex> {
    vec![
        vertex(x0, y0, z, color),
        vertex(x0, y1, z, color),
        vertex(x1, y1, z, color),
        vertex(x1, y0, z, color),
    ]
}

/// Registers with the rear plane at the far depth, as a program sets them (`CLEAR_DEPTH` 7FFFh).
pub(super) fn regs() -> RenderRegs {
    RenderRegs {
        clear_depth: 0x7FFF,
        ..RenderRegs::default()
    }
}

/// The attributes of an opaque polygon with ID `id` that shows both faces.
pub(super) fn opaque(id: u32) -> u32 {
    OPAQUE | FRONT_BACK | id << 24
}

/// Texture memory with no bank behind any slot: every read is 0.
pub(super) fn no_textures() -> Textures<'static> {
    Textures::from_slots(&[], [None; 4], [None; 6])
}

/// All 192 lines of `ram` under DISP3DCNT `disp3dcnt` and registers `regs`.
pub(super) fn draw(ram: &Ram, disp3dcnt: u16, regs: &RenderRegs) -> Vec<Line3d> {
    let frame = Frame::new(Scene { ram, swap_bits: 0 }, regs, disp3dcnt);
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    frame.draw(&SequentialExecutor, &no_textures(), &mut lines);
    lines
}

/// Like [`draw`] under DISP3DCNT `disp3dcnt`, registers `regs` and SWAP_BUFFERS bits `swap_bits`.
pub(super) fn draw_all(ram: &Ram, disp3dcnt: u16, regs: &RenderRegs, swap_bits: u8) -> Vec<Line3d> {
    let frame = Frame::new(Scene { ram, swap_bits }, regs, disp3dcnt);
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    frame.draw(&SequentialExecutor, &no_textures(), &mut lines);
    lines
}

/// Like [`draw`] under SWAP_BUFFERS parameter bits `swap_bits`.
pub(super) fn draw_swap(ram: &Ram, swap_bits: u8) -> Vec<Line3d> {
    let frame = Frame::new(Scene { ram, swap_bits }, &regs(), 0);
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    frame.draw(&SequentialExecutor, &no_textures(), &mut lines);
    lines
}

/// The same lines drawn by a plain loop over `draw_line`, with no executor.
pub(super) fn draw_plain(ram: &Ram, disp3dcnt: u16, regs: &RenderRegs) -> Vec<Line3d> {
    let frame = Frame::new(Scene { ram, swap_bits: 0 }, regs, disp3dcnt);
    let mut lines = vec![[Pixel3d::default(); SCREEN_WIDTH]; crate::SCREEN_HEIGHT];
    lines
        .iter_mut()
        .enumerate()
        .for_each(|(y, line)| frame.draw_line(y, &no_textures(), line));
    lines
}

/// The `(x, y)` of every pixel whose alpha is not 0, in line order.
pub(super) fn covered(lines: &[Line3d]) -> Vec<(usize, usize)> {
    lines
        .iter()
        .enumerate()
        .flat_map(|(y, line)| {
            line.iter()
                .enumerate()
                .filter(|(_, p)| p.alpha != 0)
                .map(move |(x, _)| (x, y))
        })
        .collect()
}

/// Every `(x, y)` with `x0 <= x <= x1` and `y0 <= y <= y1`, in line order.
pub(super) fn rect(x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<(usize, usize)> {
    (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
        .collect()
}

/// The pixel at `(x, y)`.
pub(super) fn at(lines: &[Line3d], x: usize, y: usize) -> Pixel3d {
    lines[y][x]
}
