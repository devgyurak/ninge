// SPDX-License-Identifier: Apache-2.0
//! Vertices, polygon assembly, clipping and Polygon RAM / Vertex RAM with their double buffer
//! (`docs/hw/geometry-engine.md`, "Vertices", "Primitives and strips", "Clipping", "Polygon RAM,
//! Vertex RAM and the double buffer").
//!
//! The geometry engine fills one set of RAM while the rendering engine (#49) reads the other;
//! SWAP_BUFFERS exchanges them. Everything is fixed-size plain data: the vectors are reserved once
//! and a polygon or a vertex that does not fit is dropped, never grown.

use super::lighting::{Lighting, direction, unpack};
use super::matrix::{Matrix, ONE};
use serde::{Deserialize, Serialize};

/// Polygons one set of Polygon RAM holds (`geometry-engine.md`).
pub const POLYGON_RAM: usize = 2048;
/// Vertices one set of Vertex RAM holds.
pub const VERTEX_RAM: usize = 6144;
/// The most vertices one polygon has after clipping: a quad against six planes.
pub const MAX_POLYGON_VERTICES: usize = 10;

/// One vertex as Vertex RAM holds it: clip coordinates (1.19.12), the vertex colour (RGB555) and
/// the texture coordinates (1.11.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vertex {
    /// Clip-space X, 1.19.12.
    pub x: i32,
    /// Clip-space Y.
    pub y: i32,
    /// Clip-space Z.
    pub z: i32,
    /// Clip-space W: a vertex is inside while `-w <= x, y, z <= w`.
    pub w: i32,
    /// The vertex colour, RGB555.
    pub color: u16,
    /// Texture coordinate S, 1.11.4.
    pub s: i16,
    /// Texture coordinate T, 1.11.4.
    pub t: i16,
}

/// One polygon as Polygon RAM holds it: its attributes and the indices of its vertices in the same
/// set of Vertex RAM, in drawing order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Polygon {
    /// POLYGON_ATTR as it stood at BEGIN_VTXS.
    pub attr: u32,
    /// TEXIMAGE_PARAM when the polygon was completed.
    pub texture: u32,
    /// PLTT_BASE when the polygon was completed.
    pub palette: u32,
    /// How many of `vertices` are used, 3 to 10.
    pub count: u8,
    /// Indices into the same set's `vertices`, in drawing order.
    pub vertices: [u16; MAX_POLYGON_VERTICES],
}

/// One set of Polygon RAM and Vertex RAM.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ram {
    /// The polygons stored, in the order they were completed.
    pub polygons: Vec<Polygon>,
    /// The vertices the polygons point at.
    pub vertices: Vec<Vertex>,
}

/// A clone keeps the room a full set needs, so that vertices stored after it never allocate.
impl Clone for Ram {
    fn clone(&self) -> Self {
        let mut ram = Self::new();
        ram.polygons.extend_from_slice(&self.polygons);
        ram.vertices.extend_from_slice(&self.vertices);
        ram
    }
}

impl Ram {
    fn new() -> Self {
        Self {
            polygons: Vec::with_capacity(POLYGON_RAM),
            vertices: Vec::with_capacity(VERTEX_RAM),
        }
    }

    /// Empties the set; the room stays.
    fn clear(&mut self) {
        self.polygons.clear();
        self.vertices.clear();
    }

    /// Makes room for a full set. A set made by `new` or a clone has it already; a set a savestate
    /// load decoded has exactly its length, so the machine calls this once after a load
    /// (`Gpu3d::reserve_ram`) and nothing allocates while commands run.
    pub(super) fn reserve(&mut self) {
        self.polygons
            .reserve_exact(POLYGON_RAM.saturating_sub(self.polygons.len()));
        self.vertices
            .reserve_exact(VERTEX_RAM.saturating_sub(self.vertices.len()));
    }
}

/// A vertex held while its polygon is assembled, and the Vertex RAM index it was stored at, if it
/// was (a strip shares its stored vertices between polygons).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Held {
    vertex: Vertex,
    stored: Option<u16>,
}

/// POLYGON_ATTR bit 12: render a polygon that crosses the far plane.
const ATTR_FAR_PLANE: u32 = 1 << 12;
/// Texture coordinates and colour as a channel triple, for interpolation.
const CHANNELS: usize = 3;

/// The engine's state for building polygons: the current vertex attributes, the list being
/// assembled and the two sets of RAM.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Geometry {
    /// The current vertex position, 1.3.12.
    position: [i16; 3],
    /// The current vertex colour, RGB555.
    pub(super) color: u16,
    /// The texture coordinates as TEXCOORD gave them, 1.11.4.
    texcoord: [i16; 2],
    /// The texture coordinates vertices take: `texcoord` after the transformation mode has run.
    st: [i16; 2],
    /// The lights, the material and the shininess table (`geometry-engine.md`, "Lighting").
    lighting: Lighting,
    /// POLYGON_ATTR as written, applied at the next BEGIN_VTXS.
    attr_written: u32,
    /// POLYGON_ATTR in force.
    attr: u32,
    pub(super) texture: u32,
    pub(super) palette: u32,
    /// VIEWPORT as written; the rasterizer (#49) scales to it.
    pub(super) viewport: u32,
    /// The primitive of the open list, if a BEGIN_VTXS has run.
    primitive: Option<u8>,
    /// Vertices since BEGIN_VTXS.
    count: u32,
    held: [Held; 4],
    /// The two sets of RAM; `fill` indexes the one the geometry engine writes.
    ram: [Ram; 2],
    fill: u8,
    /// DISP3DCNT bit 13: a polygon or vertex was dropped because a RAM was full.
    pub(super) overflow: bool,
    /// SWAP_BUFFERS' two parameter bits, applied to what the last swap handed on.
    pub(super) swap_bits: u8,
}

impl Geometry {
    pub(super) fn new() -> Self {
        Self {
            position: [0; 3],
            color: 0x7FFF,
            texcoord: [0; 2],
            st: [0; 2],
            lighting: Lighting::new(),
            attr_written: 0,
            attr: 0,
            texture: 0,
            palette: 0,
            viewport: 0,
            primitive: None,
            count: 0,
            held: [Held::default(); 4],
            ram: [Ram::new(), Ram::new()],
            fill: 0,
            overflow: false,
            swap_bits: 0,
        }
    }

    /// The RAM the geometry engine is filling.
    pub(super) fn filling(&self) -> &Ram {
        &self.ram[usize::from(self.fill & 1)]
    }

    /// The RAM the last SWAP_BUFFERS handed to the rendering engine.
    pub(super) fn rendering(&self) -> &Ram {
        &self.ram[usize::from(!self.fill & 1)]
    }

    /// RAM_COUNT: polygons in bits 0-11, vertices in bits 16-28.
    pub(super) fn ram_count(&self) -> u32 {
        let ram = self.filling();
        (ram.polygons.len() as u32) | (ram.vertices.len() as u32) << 16
    }

    /// POLYGON_ATTR: held until the next BEGIN_VTXS.
    pub(super) fn write_attr(&mut self, attr: u32) {
        self.attr_written = attr;
    }

    /// BEGIN_VTXS: a list of `primitive` (bits 0-1) starts and POLYGON_ATTR takes effect.
    pub(super) fn begin(&mut self, param: u32) {
        self.attr = self.attr_written;
        self.primitive = Some((param & 3) as u8);
        self.count = 0;
        self.held = [Held::default(); 4];
    }

    /// Reserves the full room in both sets of RAM; see [`Ram::reserve`].
    pub(super) fn reserve_ram(&mut self) {
        self.ram.iter_mut().for_each(Ram::reserve);
    }

    /// The lights POLYGON_ATTR enables, as a count.
    pub(super) fn enabled_lights(&self) -> u32 {
        (self.attr & LIGHT_BITS).count_ones()
    }

    /// TEXIMAGE_PARAM bits 30-31: how texture coordinates are transformed.
    fn texture_mode(&self) -> u32 {
        self.texture >> 30
    }

    /// TEXCOORD: S in bits 0-15 and T in bits 16-31. Modes 0 and 1 give the coordinates a vertex
    /// takes now; modes 2 and 3 compute them later (at NORMAL and at each vertex).
    pub(super) fn texcoord(&mut self, param: u32, texture: &Matrix) {
        self.texcoord = [param as u16 as i16, (param >> 16) as u16 as i16];
        match self.texture_mode() {
            0 => self.st = self.texcoord,
            1 => self.st = map_texcoord(&self.texcoord, texture),
            _ => {}
        }
    }

    /// NORMAL: the vertex colour from the lights, and in texture mode 2 the coordinates from the
    /// normal.
    pub(super) fn normal(&mut self, param: u32, vector: &Matrix, texture: &Matrix) {
        // GBATEK's order: the texture coordinates come from the normal as it arrives, then the
        // vector matrix turns it for the lights (a sphere map loads the texture matrix from the
        // vector matrix, so the normal must not be turned twice).
        let arriving = unpack(param);
        if self.texture_mode() == 2 {
            self.st = texture_from(arriving.map(i64::from), VECTOR_TEXTURE_SHIFT, self, texture);
        }
        let normal = direction(arriving, vector);
        self.color = self.lighting.vertex_color(normal, self.attr & LIGHT_BITS);
    }

    pub(super) fn diffuse_ambient(&mut self, param: u32) {
        if let Some(color) = self.lighting.set_diffuse_ambient(param) {
            self.color = color;
        }
    }

    pub(super) fn specular_emission(&mut self, param: u32) {
        self.lighting.set_specular_emission(param);
    }

    pub(super) fn light_vector(&mut self, param: u32, vector: &Matrix) {
        self.lighting.set_direction(param, vector);
    }

    pub(super) fn light_color(&mut self, param: u32) {
        self.lighting.set_color(param);
    }

    pub(super) fn shininess(&mut self, words: &[u32]) {
        self.lighting.set_table(words);
    }

    /// Sets the current position from a vertex command and processes the vertex.
    pub(super) fn vertex(&mut self, id: u8, params: &[u32], clip: &Matrix, texture: &Matrix) {
        let p0 = params[0];
        let half = |word: u32, high: bool| (word >> if high { 16 } else { 0 }) as u16 as i16;
        match id {
            // VTX_16: X, Y in the first word, Z in the second.
            0x23 => self.position = [half(p0, false), half(p0, true), half(params[1], false)],
            // VTX_10: three 10-bit fields with a 6-bit fraction: shifted to the 12-bit one.
            0x24 => {
                let field = |n: u32| (sign_extend_10(p0 >> (10 * n)) << 6) as i16;
                self.position = [field(0), field(1), field(2)];
            }
            0x25 => (self.position[0], self.position[1]) = (half(p0, false), half(p0, true)),
            0x26 => (self.position[0], self.position[2]) = (half(p0, false), half(p0, true)),
            0x27 => (self.position[1], self.position[2]) = (half(p0, false), half(p0, true)),
            // VTX_DIFF: each field is added to the previous coordinate as a 12-bit-fraction count.
            // TODO(hw-unverified): the field format and the wrap of the sum (`geometry-engine.md`,
            // "Unverified").
            _ => {
                (0..3).for_each(|n| {
                    let delta = sign_extend_10(p0 >> (10 * n as u32)) as i16;
                    self.position[n] = self.position[n].wrapping_add(delta);
                });
            }
        }
        if self.texture_mode() == 3 {
            let position = self.position.map(i64::from);
            self.st = texture_from(position, POSITION_TEXTURE_SHIFT, self, texture);
        }
        let vertex = self.transform(clip);
        self.add_vertex(vertex);
    }

    /// The current position through the clip matrix: `(x, y, z, 1.0) x clip`.
    fn transform(&self, clip: &Matrix) -> Vertex {
        let [x, y, z] = self.position.map(i64::from);
        let column = |c: usize| {
            let sum = x
                .wrapping_mul(i64::from(clip[c]))
                .wrapping_add(y.wrapping_mul(i64::from(clip[4 + c])))
                .wrapping_add(z.wrapping_mul(i64::from(clip[8 + c])))
                .wrapping_add(i64::from(ONE).wrapping_mul(i64::from(clip[12 + c])));
            (sum >> 12) as i32
        };
        Vertex {
            x: column(0),
            y: column(1),
            z: column(2),
            w: column(3),
            color: self.color,
            s: self.st[0],
            t: self.st[1],
        }
    }

    /// A vertex joins the open list; a vertex that completes a polygon stores it.
    // TODO(hw-unverified): a vertex outside BEGIN_VTXS moves the position and builds nothing
    // (`geometry-engine.md`, "Unverified").
    fn add_vertex(&mut self, vertex: Vertex) {
        let Some(primitive) = self.primitive else {
            return;
        };
        let held = Held {
            vertex,
            stored: None,
        };
        let n = self.count as usize;
        self.count = self.count.saturating_add(1);
        match primitive {
            0 => {
                self.held[n % 3] = held;
                if n % 3 == 2 {
                    self.store(&[0, 1, 2]);
                }
            }
            1 => {
                self.held[n % 4] = held;
                if n % 4 == 3 {
                    self.store(&[0, 1, 2, 3]);
                }
            }
            2 => {
                if n < 3 {
                    self.held[n] = held;
                } else {
                    self.held.copy_within(1..3, 0);
                    self.held[2] = held;
                }
                if n >= 2 {
                    // Every second triangle of a strip is clockwise: swap its first two vertices
                    // to keep the facing (`geometry-engine.md`, "Primitives and strips").
                    match (n - 2) % 2 {
                        0 => self.store(&[0, 1, 2]),
                        _ => self.store(&[1, 0, 2]),
                    }
                }
            }
            _ => match n {
                0..=3 => {
                    self.held[n] = held;
                    if n == 3 {
                        self.store(&[0, 1, 3, 2]);
                    }
                }
                _ if n.is_multiple_of(2) => {
                    self.held.copy_within(2..4, 0);
                    self.held[2] = held;
                }
                _ => {
                    self.held[3] = held;
                    self.store(&[0, 1, 3, 2]);
                }
            },
        }
    }

    /// Clips the polygon made of the held vertices at `order` and stores what is left.
    fn store(&mut self, order: &[usize]) {
        let mut corners = [Vertex::default(); 4];
        order
            .iter()
            .zip(corners.iter_mut())
            .for_each(|(&at, corner)| *corner = self.held[at].vertex);
        let corners = &corners[..order.len()];
        let mut cut = [Vertex::default(); MAX_POLYGON_VERTICES];
        match clip(corners, self.attr & ATTR_FAR_PLANE != 0, &mut cut) {
            Clipped::Outside => {}
            Clipped::Inside => {
                let needed = order
                    .iter()
                    .filter(|&&at| self.held[at].stored.is_none())
                    .count();
                if !self.room(needed) {
                    return;
                }
                // Vertex RAM takes the vertices in the order they arrived, which the held slots
                // keep; the polygon lists them in drawing order.
                (0..self.held.len())
                    .filter(|at| order.contains(at))
                    .for_each(|at| {
                        self.store_held(at);
                    });
                let indices: [u16; MAX_POLYGON_VERTICES] = std::array::from_fn(|i| {
                    order
                        .get(i)
                        .and_then(|&at| self.held[at].stored)
                        .unwrap_or(0)
                });
                self.push_polygon(order.len(), indices);
            }
            // A polygon that was cut stands alone: its corners are stored afresh.
            // TODO(hw-unverified): strip clipping stores the cut polygon afresh and the strip's own
            // vertices again later (`geometry-engine.md`, "Unverified").
            Clipped::Cut(count) => {
                if !self.room(count) {
                    return;
                }
                // A polygon that was cut stands alone: its vertices are stored afresh and the strip
                // that fed it will store its own again (`geometry-engine.md`, "Clipping").
                let first = self.filling().vertices.len();
                let fill = usize::from(self.fill & 1);
                self.ram[fill].vertices.extend_from_slice(&cut[..count]);
                order.iter().for_each(|&at| self.held[at].stored = None);
                let indices: [u16; MAX_POLYGON_VERTICES] =
                    std::array::from_fn(|i| (first + i) as u16);
                self.push_polygon(count, indices);
            }
        }
    }

    /// Whether a polygon with `vertices` new vertices fits; raises the overflow flag when it does
    /// not.
    // TODO(hw-unverified): what is dropped on overflow (`geometry-engine.md`, "Unverified").
    fn room(&mut self, vertices: usize) -> bool {
        let ram = self.filling();
        let fits = ram.polygons.len() < POLYGON_RAM && ram.vertices.len() + vertices <= VERTEX_RAM;
        self.overflow |= !fits;
        fits
    }

    /// The Vertex RAM index of held vertex `at`, storing it first if it is not stored yet.
    fn store_held(&mut self, at: usize) -> u16 {
        if let Some(index) = self.held[at].stored {
            return index;
        }
        let fill = usize::from(self.fill & 1);
        let index = self.ram[fill].vertices.len() as u16;
        self.ram[fill].vertices.push(self.held[at].vertex);
        self.held[at].stored = Some(index);
        index
    }

    fn push_polygon(&mut self, count: usize, vertices: [u16; MAX_POLYGON_VERTICES]) {
        let polygon = Polygon {
            attr: self.attr,
            texture: self.texture,
            palette: self.palette,
            count: count as u8,
            vertices,
        };
        let fill = usize::from(self.fill & 1);
        self.ram[fill].polygons.push(polygon);
    }

    /// Whether the open list ends in a polygon that was never completed: SWAP_BUFFERS then locks
    /// the 3D hardware up (`geometry-engine.md`, "SWAP_BUFFERS").
    // TODO(hw-unverified): which lists count as unfinished (`geometry-engine.md`, "Unverified").
    pub(super) fn unfinished(&self) -> bool {
        let n = self.count;
        match self.primitive {
            Some(0) => !n.is_multiple_of(3),
            Some(1) => !n.is_multiple_of(4),
            Some(2) => (1..=2).contains(&n),
            Some(_) => n > 0 && (n < 4 || n % 2 == 1),
            None => false,
        }
    }

    /// SWAP_BUFFERS: the two sets exchange roles and the one the geometry engine gets back is
    /// emptied; the swap's two parameter bits are kept for the renderer.
    // TODO(hw-unverified): the sets swap and RAM_COUNT clears when SWAP_BUFFERS ends, 784 master cycles
    // after VBlank begins, where GBATEK says 10 cycles after VBlank (`geometry-engine.md`,
    // "Unverified").
    pub(super) fn swap(&mut self, param: u32) {
        self.fill ^= 1;
        let fill = usize::from(self.fill & 1);
        self.ram[fill].clear();
        self.held.iter_mut().for_each(|held| held.stored = None);
        self.swap_bits = (param & 3) as u8;
    }

    /// Whether the buffers and the list hold only values that exist, for a savestate load.
    pub(super) fn well_formed(&self) -> bool {
        let ram_ok = |ram: &Ram| {
            ram.polygons.len() <= POLYGON_RAM
                && ram.vertices.len() <= VERTEX_RAM
                && ram.polygons.iter().all(|polygon| {
                    (3..=MAX_POLYGON_VERTICES).contains(&usize::from(polygon.count))
                        && polygon.vertices[..usize::from(polygon.count)]
                            .iter()
                            .all(|&index| usize::from(index) < ram.vertices.len())
                })
        };
        self.fill <= 1
            && self.primitive.is_none_or(|primitive| primitive <= 3)
            && self.ram.iter().all(ram_ok)
            && self.held.iter().all(|held| {
                held.stored
                    .is_none_or(|index| usize::from(index) < self.filling().vertices.len())
            })
    }
}

impl Default for Geometry {
    fn default() -> Self {
        Self::new()
    }
}

/// POLYGON_ATTR bits 0-3: the lights NORMAL takes into account.
const LIGHT_BITS: u32 = 0xF;
/// A normal (9-bit fraction) times a matrix element (12 bits) has 21 fraction bits; the result keeps
/// 4. A position (12-bit fraction) has 24.
const VECTOR_TEXTURE_SHIFT: u32 = 17;
const POSITION_TEXTURE_SHIFT: u32 = 20;

/// Texture mode 1: `(S, T, 1/16, 1/16)` times the first two columns of the texture matrix. S and T
/// have 4 fraction bits, the matrix 12, so the sum shifts right by 12; the two constants are `1/16`
/// (a raw 1). The result is masked to 16 bits.
// TODO(hw-unverified): the constants' use and the shift (`geometry-engine.md`, "Unverified").
fn map_texcoord(st: &[i16; 2], m: &Matrix) -> [i16; 2] {
    let [s, t] = st.map(i64::from);
    std::array::from_fn(|c| {
        let sum = s
            .wrapping_mul(i64::from(m[c]))
            .wrapping_add(t.wrapping_mul(i64::from(m[4 + c])))
            .wrapping_add(i64::from(m[8 + c]))
            .wrapping_add(i64::from(m[12 + c]));
        (sum >> 12) as i16
    })
}

/// Texture modes 2 and 3: `(x, y, z, 1.0)` times the first two columns with the bottom row replaced
/// by S and T. The vector terms shift right by `shift` to leave 4 fraction bits and S, T are added as
/// they are.
// TODO(hw-unverified): the units of the bottom row and the shifts (`geometry-engine.md`,
// "Unverified").
fn texture_from(v: [i64; 3], shift: u32, geometry: &Geometry, m: &Matrix) -> [i16; 2] {
    std::array::from_fn(|c| {
        let sum = (0..3).fold(0i64, |sum, k| {
            sum.wrapping_add(v[k].wrapping_mul(i64::from(m[4 * k + c])))
        });
        ((sum >> shift) as i16).wrapping_add(geometry.texcoord[c])
    })
}

/// Bits 0-9 of `value` as a signed number.
const fn sign_extend_10(value: u32) -> i32 {
    ((value << 22) as i32) >> 22
}

/// What clipping made of a polygon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Clipped {
    /// Wholly outside the view volume, or hidden by the far-plane rule: not stored.
    Outside,
    /// Wholly inside: stored as it is.
    Inside,
    /// Cut by at least one plane: this many corners, in the array `clip` filled.
    Cut(usize),
}

/// The six clip planes as the distance from each, positive inside: left, right, bottom, top, near,
/// far (`-w <= x <= w` and so on).
fn distance(plane: usize, v: &Vertex) -> i64 {
    let (w, x, y, z) = (
        i64::from(v.w),
        i64::from(v.x),
        i64::from(v.y),
        i64::from(v.z),
    );
    match plane {
        0 => w + x,
        1 => w - x,
        2 => w + y,
        3 => w - y,
        4 => w + z,
        _ => w - z,
    }
}

/// The far plane's index in [`distance`].
const FAR: usize = 5;

/// The point on the edge `a` to `b` where its distance to the plane is 0, every attribute
/// interpolated linearly in clip space. The parameter is `da / (da - db)` and each field is
/// `a + (b - a) x da / (da - db)`, truncated toward zero.
// TODO(hw-unverified): the interpolation and its rounding (`geometry-engine.md`, "Unverified").
fn intersect(a: &Vertex, b: &Vertex, da: i64, db: i64) -> Vertex {
    let span = i128::from(da - db);
    let lerp = |from: i64, to: i64| -> i64 {
        (i128::from(from) + i128::from(to - from) * i128::from(da) / span) as i64
    };
    let channel = |c: u16, shift: u32| i64::from((c >> shift) & 0x1F);
    let color = (0..CHANNELS as u32).fold(0u16, |color, n| {
        let value = lerp(channel(a.color, 5 * n), channel(b.color, 5 * n)).clamp(0, 31) as u16;
        color | value << (5 * n)
    });
    Vertex {
        x: lerp(i64::from(a.x), i64::from(b.x)) as i32,
        y: lerp(i64::from(a.y), i64::from(b.y)) as i32,
        z: lerp(i64::from(a.z), i64::from(b.z)) as i32,
        w: lerp(i64::from(a.w), i64::from(b.w)) as i32,
        color,
        s: lerp(i64::from(a.s), i64::from(b.s)) as i16,
        t: lerp(i64::from(a.t), i64::from(b.t)) as i16,
    }
}

/// Clips a polygon of 3 or 4 corners against the six sides of the view volume, one plane after the
/// other. `render_far` is POLYGON_ATTR bit 12: with it clear a polygon that reaches past the far
/// plane is hidden whole.
// TODO(hw-unverified): the far-plane rule, the inside test and the order of the corners that remain
// (`geometry-engine.md`, "Unverified").
pub(super) fn clip(
    corners: &[Vertex],
    render_far: bool,
    out: &mut [Vertex; MAX_POLYGON_VERTICES],
) -> Clipped {
    if !render_far && corners.iter().any(|v| distance(FAR, v) < 0) {
        return Clipped::Outside;
    }
    let mut points = [Vertex::default(); WORK];
    points[..corners.len()].copy_from_slice(corners);
    let mut count = corners.len();
    let mut cut = false;
    for plane in 0..PLANES {
        let (next, kept, touched) = clip_plane(&points, count, plane);
        points = next;
        count = kept;
        cut |= touched;
        if count == 0 {
            return Clipped::Outside;
        }
    }
    match (count < 3, cut) {
        (true, _) => Clipped::Outside,
        (false, false) => Clipped::Inside,
        (false, true) => {
            out.iter_mut()
                .zip(points.iter())
                .for_each(|(slot, point)| *slot = *point);
            Clipped::Cut(count.min(MAX_POLYGON_VERTICES))
        }
    }
}

/// Planes of the view volume.
const PLANES: usize = 6;
/// Room for a polygon while it is being clipped: a convex quad gains at most one corner for each
/// plane. A crossed quad or one with an angle over 180 degrees (GBATEK's "prohibited quads") can gain
/// more; the corners past the room are dropped, never a panic.
// TODO(hw-unverified): what the hardware does with a prohibited quad (`geometry-engine.md`,
// "Unverified").
const WORK: usize = MAX_POLYGON_VERTICES + 2;

/// One pass of the clipper: the polygon in `points[..count]` against one `plane`. Returns the
/// corners that remain, how many, and whether an edge crossed the plane.
// TODO(hw-unverified): the inside test `-w <= x, y, z <= w`, the order of the corners that remain
// and the handling of `w <= 0` (`geometry-engine.md`, "Unverified").
fn clip_plane(
    points: &[Vertex; WORK],
    count: usize,
    plane: usize,
) -> ([Vertex; WORK], usize, bool) {
    let mut next = [Vertex::default(); WORK];
    let mut kept = 0;
    let mut crossed = false;
    for (i, a) in points[..count].iter().enumerate() {
        let b = points[(i + 1) % count];
        let (da, db) = (distance(plane, a), distance(plane, &b));
        crossed |= da < 0 || db < 0;
        let produced = match (da >= 0, db >= 0) {
            (true, true) => [Some(b), None],
            (true, false) => [Some(intersect(a, &b, da, db)), None],
            (false, true) => [Some(intersect(a, &b, da, db)), Some(b)],
            (false, false) => [None, None],
        };
        for vertex in produced.into_iter().flatten() {
            if let Some(slot) = next.get_mut(kept) {
                *slot = vertex;
                kept += 1;
            }
        }
    }
    (next, kept, crossed)
}
