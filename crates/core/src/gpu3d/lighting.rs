// SPDX-License-Identifier: Apache-2.0
//! The four lights, the material and the vertex colour NORMAL calculates
//! (`docs/hw/geometry-engine.md`, "Lighting").
//!
//! GBATEK names the registers and what each colour stands for but gives no equation, so the sum
//! here is the model of the note ("Unverified"): emission plus, for each enabled light, the light's
//! colour times diffuse, specular and ambient terms, in integer arithmetic with the 5-bit colour
//! channels of the hardware.

use super::matrix::Matrix;
use serde::{Deserialize, Serialize};

/// Lights the geometry engine has.
pub(super) const LIGHTS: usize = 4;
/// Entries of the shininess table: 128 bytes in 32 words.
const TABLE_WORDS: usize = 32;
const TABLE_ENTRIES: usize = 4 * TABLE_WORDS;
/// `1.0` in a 1.0.9 vector component.
const UNIT: i64 = 1 << 9;
/// A colour channel is 5 bits.
const CHANNEL_MAX: i64 = 31;
/// Fraction bits of a vector component: 1.0.9.
const VECTOR_FRACTION: u32 = 9;
/// Fraction bits of a matrix element.
const MATRIX_FRACTION: u32 = 12;

/// The lights, the material and the shininess table. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Lighting {
    /// Each light's direction after the vector matrix, 1.0.9 components.
    direction: [[i32; 3]; LIGHTS],
    /// Each light's colour, RGB555.
    color: [u16; LIGHTS],
    diffuse: u16,
    ambient: u16,
    specular: u16,
    emission: u16,
    /// SPE_EMI bit 15: use the shininess table; clear, a linear ramp.
    use_table: bool,
    /// The table as 4 rows of 32 entries (serde has no impl for 128).
    table: [[u8; TABLE_ENTRIES / 4]; 4],
}

impl Lighting {
    pub(super) fn new() -> Self {
        Self {
            direction: [[0; 3]; LIGHTS],
            color: [0; LIGHTS],
            diffuse: 0,
            ambient: 0,
            specular: 0,
            emission: 0,
            use_table: false,
            table: [[0; TABLE_ENTRIES / 4]; 4],
        }
    }

    /// LIGHT_VECTOR: components in bits 0-9, 10-19 and 20-29, the light in bits 30-31; the vector is
    /// multiplied by the vector matrix as it arrives.
    pub(super) fn set_direction(&mut self, param: u32, vector: &Matrix) {
        self.direction[(param >> 30) as usize] = direction(unpack(param), vector);
    }

    /// LIGHT_COLOR: RGB555 in bits 0-14, the light in bits 30-31.
    pub(super) fn set_color(&mut self, param: u32) {
        self.color[(param >> 30) as usize] = (param & 0x7FFF) as u16;
    }

    /// DIF_AMB: diffuse in bits 0-14, ambient in bits 16-30. Returns the colour to set as the vertex
    /// colour when bit 15 asks for it.
    pub(super) fn set_diffuse_ambient(&mut self, param: u32) -> Option<u16> {
        self.diffuse = (param & 0x7FFF) as u16;
        self.ambient = ((param >> 16) & 0x7FFF) as u16;
        (param & (1 << 15) != 0).then_some(self.diffuse)
    }

    /// SPE_EMI: specular in bits 0-14, the table switch in bit 15, emission in bits 16-30.
    pub(super) fn set_specular_emission(&mut self, param: u32) {
        self.specular = (param & 0x7FFF) as u16;
        self.use_table = param & (1 << 15) != 0;
        self.emission = ((param >> 16) & 0x7FFF) as u16;
    }

    /// SHININESS: 32 words of four 8-bit entries, entry 0 in the low byte of the first word.
    pub(super) fn set_table(&mut self, words: &[u32]) {
        self.table = std::array::from_fn(|row| {
            std::array::from_fn(|i| {
                let entry = 32 * row + i;
                let word = words.get(entry / 4).copied().unwrap_or(0);
                (word >> (8 * (entry % 4))) as u8
            })
        });
    }

    /// The vertex colour NORMAL calculates for `normal` (1.0.9, already through the vector matrix)
    /// with the lights in `enabled` (POLYGON_ATTR bits 0-3).
    ///
    /// Per channel: emission, plus for each enabled light its colour times diffuse x
    /// `max(0, -(N . L))`, specular x the shininess of `max(0, -(N . H))` and ambient, clamped to
    /// 5 bits; `H` is the halfway vector of the light and the view direction `(0, 0, -1)`.
    // TODO(hw-unverified): the whole equation and its rounding (`geometry-engine.md`, "Unverified").
    pub(super) fn vertex_color(&self, normal: [i32; 3], enabled: u32) -> u16 {
        let dot = |a: [i64; 3], b: [i64; 3]| (0..3).map(|i| a[i] * b[i]).sum::<i64>();
        let n = normal.map(i64::from);
        let channel = |shift: u32, of: u16| i64::from((of >> shift) & 0x1F);
        let total = |shift: u32| {
            let lit: i64 = (0..LIGHTS)
                .filter(|&i| enabled & (1 << i) != 0)
                .map(|i| {
                    let l = self.direction[i].map(i64::from);
                    // The halfway vector of L and the view direction (0, 0, -1.0), halved.
                    let h = [l[0] >> 1, l[1] >> 1, (l[2] - UNIT) >> 1];
                    let diffuse = (-dot(n, l) >> VECTOR_FRACTION).clamp(0, UNIT);
                    let sdot = (-dot(n, h) >> VECTOR_FRACTION).clamp(0, UNIT);
                    let shine = self.shininess(sdot);
                    let light = channel(shift, self.color[i]);
                    channel(shift, self.diffuse) * light * diffuse / (CHANNEL_MAX * UNIT)
                        + channel(shift, self.specular) * light * shine / (CHANNEL_MAX * 256)
                        + channel(shift, self.ambient) * light / CHANNEL_MAX
                })
                .sum();
            (channel(shift, self.emission) + lit).clamp(0, CHANNEL_MAX) as u16
        };
        total(0) | total(5) << 5 | total(10) << 10
    }

    /// The shininess for a specular dot product of `sdot` (0 to 512): the table entry at `sdot / 4`,
    /// or a linear ramp when the table is off.
    // TODO(hw-unverified): the index and the ramp (`geometry-engine.md`, "Unverified").
    fn shininess(&self, sdot: i64) -> i64 {
        let index = ((sdot >> 2) as usize).min(TABLE_ENTRIES - 1);
        match self.use_table {
            true => i64::from(self.table[index / 32][index % 32]),
            false => 2 * index as i64,
        }
    }
}

impl Default for Lighting {
    fn default() -> Self {
        Self::new()
    }
}

/// The three 10-bit signed components of a LIGHT_VECTOR, NORMAL or VEC_TEST parameter, 1.0.9.
pub(super) const fn unpack(param: u32) -> [i32; 3] {
    [
        component(param),
        component(param >> 10),
        component(param >> 20),
    ]
}

const fn component(bits: u32) -> i32 {
    ((bits << 22) as i32) >> 22
}

/// `(x, y, z, 0) x matrix`: a direction through the upper 3x3 of a matrix. The components are 9-bit
/// fractions and the elements 12-bit, so the sum shifts right by 12 to keep 9 bits.
pub(super) fn direction(v: [i32; 3], m: &Matrix) -> [i32; 3] {
    std::array::from_fn(|c| {
        let sum = (0..3).fold(0i64, |sum, k| {
            sum.wrapping_add(i64::from(v[k]).wrapping_mul(i64::from(m[4 * k + c])))
        });
        (sum >> MATRIX_FRACTION) as i32
    })
}
