// SPDX-License-Identifier: Apache-2.0
//! The geometry engine's matrices and matrix stacks (`docs/hw/geometry-engine.md`, "Matrices").
//!
//! Every matrix is 4x4, row by row, of signed 32-bit elements in 1.19.12 fixed point (12 fraction
//! bits, `1.0` is `1000h`). Arithmetic is integer with an `i64` accumulator; nothing is floating
//! point, as on the hardware.

use serde::{Deserialize, Serialize};

/// A 4x4 matrix, row by row: element `m[4 * y + x]` is row `y`, column `x`.
pub(super) type Matrix = [i32; 16];

/// `1.0` in the 12-bit fixed-point format.
pub(super) const ONE: i32 = 1 << FRACTION_BITS;
/// Fraction bits of a matrix element.
const FRACTION_BITS: u32 = 12;

/// The identity matrix, `1.0` on the diagonal.
pub(super) const IDENTITY: Matrix = [
    ONE, 0, 0, 0, //
    0, ONE, 0, 0, //
    0, 0, ONE, 0, //
    0, 0, 0, ONE,
];

/// Entries of the coordinate and directional stacks: the pointer is 6 bits and the upper half
/// mirrors the lower, so 32 entries exist (`geometry-engine.md`, "Matrices"; entry 31 exists and
/// raises the error flag).
pub(super) const COORDINATE_ENTRIES: usize = 32;
/// The mask that mirrors a 6-bit pointer onto those entries.
const ENTRY_MASK: u8 = (COORDINATE_ENTRIES - 1) as u8;

/// `a x b`: each element is the sum of four products with their full 24-bit fraction, shifted right
/// by 12 once at the end. The shift is arithmetic (it rounds toward negative infinity) and a result
/// that does not fit 32 bits wraps.
// TODO(hw-unverified): the wrap on overflow and the accumulator's width (`geometry-engine.md`,
// "Unverified").
pub(super) fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|index| {
        let (y, x) = (index / 4, index % 4);
        // Four products of two 32-bit values can exceed 64 bits: the accumulator wraps, as a guest
        // value may never panic the core.
        let sum = (0..4).fold(0i64, |sum, k| {
            sum.wrapping_add(i64::from(a[4 * y + k]) * i64::from(b[4 * k + x]))
        });
        (sum >> FRACTION_BITS) as i32
    })
}

/// A parameter word as a matrix element.
const fn element(word: u32) -> i32 {
    word as i32
}

/// The matrix the parameters of command `id` describe: the shapes of GBATEK's "Matrix Types".
/// A command that is not a matrix command gives the identity.
pub(super) fn shape(id: u8, params: &[u32]) -> Matrix {
    let at = |i: usize| params.get(i).copied().map_or(0, element);
    match id {
        // MTX_LOAD_4x4 and MTX_MULT_4x4: sixteen elements in order.
        0x16 | 0x18 => std::array::from_fn(&at),
        // MTX_LOAD_4x3 and MTX_MULT_4x3: three columns of four rows, the fourth column `0, 0, 0,
        // 1.0`.
        0x17 | 0x19 => std::array::from_fn(|i| match i % 4 {
            3 if i == 15 => ONE,
            3 => 0,
            x => at(3 * (i / 4) + x),
        }),
        // MTX_MULT_3x3: the upper left 3x3, the rest of the identity.
        0x1A => std::array::from_fn(|i| match (i / 4, i % 4) {
            (3, 3) => ONE,
            (3, _) | (_, 3) => 0,
            (y, x) => at(3 * y + x),
        }),
        // MTX_SCALE: the three values on the diagonal.
        0x1B => std::array::from_fn(|i| match i {
            0 | 5 | 10 => at(i / 5),
            15 => ONE,
            _ => 0,
        }),
        // MTX_TRANS: the identity with the three values in the bottom row.
        0x1C => std::array::from_fn(|i| match i {
            12..=14 => at(i - 12),
            _ => IDENTITY[i],
        }),
        _ => IDENTITY,
    }
}

/// Which matrices a matrix command reaches (`geometry-engine.md`, "Matrices").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Projection,
    /// Position and vector together (mode 2), or the position alone (mode 1) when `vector` is false.
    Coordinate {
        vector: bool,
    },
    Texture,
}

/// The four current matrices, the clip matrix and the stacks. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Matrices {
    pub(super) projection: Matrix,
    pub(super) position: Matrix,
    pub(super) vector: Matrix,
    pub(super) texture: Matrix,
    /// Position times projection, kept up to date whenever either changes.
    pub(super) clip: Matrix,
    projection_stack: Matrix,
    texture_stack: Matrix,
    position_stack: [Matrix; COORDINATE_ENTRIES],
    vector_stack: [Matrix; COORDINATE_ENTRIES],
}

impl Matrices {
    /// Everything the identity.
    // TODO(hw-unverified): the matrices and the stack entries start as the identity
    // (`geometry-engine.md`, "Unverified").
    pub(super) fn new() -> Self {
        Self {
            projection: IDENTITY,
            position: IDENTITY,
            vector: IDENTITY,
            texture: IDENTITY,
            clip: IDENTITY,
            projection_stack: IDENTITY,
            texture_stack: IDENTITY,
            position_stack: [IDENTITY; COORDINATE_ENTRIES],
            vector_stack: [IDENTITY; COORDINATE_ENTRIES],
        }
    }

    /// `C = matrix` on each matrix the target names.
    pub(super) fn load(&mut self, target: Target, matrix: &Matrix) {
        self.apply(target, |_| *matrix);
    }

    /// `C = matrix x C` on each matrix the target names.
    pub(super) fn multiply_by(&mut self, target: Target, matrix: &Matrix) {
        self.apply(target, |current| multiply(matrix, current));
    }

    fn apply(&mut self, target: Target, change: impl Fn(&Matrix) -> Matrix) {
        match target {
            Target::Projection => self.projection = change(&self.projection),
            Target::Coordinate { vector } => {
                self.position = change(&self.position);
                if vector {
                    self.vector = change(&self.vector);
                }
            }
            Target::Texture => self.texture = change(&self.texture),
        }
        self.update_clip();
    }

    /// `clip = position x projection`.
    fn update_clip(&mut self) {
        self.clip = multiply(&self.position, &self.projection);
    }

    /// Stores the current matrix of the target's stack at `entry`: the coordinate stacks keep the
    /// position and the vector matrix at the entry the 6-bit pointer mirrors onto; the projection
    /// and texture stacks have the one entry 0 and ignore any other.
    // TODO(hw-unverified): an access to entry 1 of a one-entry stack stores and loads nothing
    // (`geometry-engine.md`, "Unverified").
    pub(super) fn store(&mut self, target: Target, entry: u8) {
        match target {
            Target::Projection if entry == 0 => self.projection_stack = self.projection,
            Target::Texture if entry == 0 => self.texture_stack = self.texture,
            Target::Projection | Target::Texture => {}
            Target::Coordinate { .. } => {
                let slot = usize::from(entry & ENTRY_MASK);
                self.position_stack[slot] = self.position;
                self.vector_stack[slot] = self.vector;
            }
        }
    }

    /// Loads the current matrix of the target's stack from `entry`; see [`Matrices::store`].
    pub(super) fn restore(&mut self, target: Target, entry: u8) {
        match target {
            Target::Projection if entry == 0 => self.projection = self.projection_stack,
            Target::Texture if entry == 0 => self.texture = self.texture_stack,
            Target::Projection | Target::Texture => {}
            Target::Coordinate { .. } => {
                let slot = usize::from(entry & ENTRY_MASK);
                self.position = self.position_stack[slot];
                self.vector = self.vector_stack[slot];
            }
        }
        self.update_clip();
    }

    /// Element `index` (0-15) of the clip matrix, for CLIPMTX_RESULT.
    pub(super) fn clip_word(&self, index: usize) -> u32 {
        self.clip.get(index).copied().unwrap_or(0) as u32
    }

    /// Word `index` (0-8) of the vector matrix's upper 3x3, for VECMTX_RESULT.
    // TODO(hw-unverified): the nine words are the upper 3x3 row by row (`geometry-engine.md`,
    // "Unverified").
    pub(super) fn vector_word(&self, index: usize) -> u32 {
        let (y, x) = (index / 3, index % 3);
        self.vector.get(4 * y + x).copied().unwrap_or(0) as u32
    }
}

impl Default for Matrices {
    fn default() -> Self {
        Self::new()
    }
}
