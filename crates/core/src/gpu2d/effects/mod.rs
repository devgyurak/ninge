// SPDX-License-Identifier: Apache-2.0
//! The colour special effects: BLDCNT, BLDALPHA and BLDY decoded, and what they do to the top two
//! pixels of a screen position (`docs/hw/blending.md`). The channels are the 5-bit ones the layers
//! hold, and no allocation happens while a pixel is made.

use super::{BLUE_SHIFT, CHANNEL_MASK, GREEN_SHIFT};

/// BLDCNT, at offset `50h` of an engine's register block; the block runs BLDCNT, BLDALPHA, BLDY,
/// two bytes each (`blending.md`, "Registers"), padded to the 4-byte grid of the address map.
pub(super) const BLD_OFFSET: u32 = 0x50;
/// Bytes of that block in the address map.
pub(super) const BLD_BLOCK_BYTES: u32 = 8;

/// BLDCNT keeps bits 0-13: bits 14-15 are not used and read as 0.
// TODO(hw-unverified): the unused bits are cleared on write and read back 0 (`blending.md`,
// "Unverified").
pub(super) const BLDCNT_MASK: u16 = 0x3FFF;
/// BLDALPHA keeps EVA (bits 0-4) and EVB (bits 8-12).
pub(super) const BLDALPHA_MASK: u16 = 0x1F1F;
/// BLDY keeps EVY, bits 0-4.
pub(super) const BLDY_MASK: u16 = 0x1F;

/// The layer indices of a pixel: BG0-BG3 are 0-3, then the objects and the backdrop. They are the
/// bit positions of a target set (`blending.md`, "BLDCNT bits").
pub(super) const LAYER_OBJ: u8 = 4;
pub(super) const LAYER_BACKDROP: u8 = 5;
/// The six layer bits of a target set.
const TARGET_BITS: u16 = 0x3F;
/// BLDCNT bits 8-13: the second targets sit above the first ones.
const SECOND_SHIFT: u32 = 8;
/// BLDCNT bits 6-7: the effect.
const EFFECT_SHIFT: u32 = 6;
const EFFECT_MASK: u16 = 0x3;
/// BLDALPHA bits 8-12: EVB.
const EVB_SHIFT: u32 = 8;
/// The coefficients are sixteenths, and a value above 16 counts as 16 (`blending.md`,
/// "BLDALPHA bits").
const COEFFICIENT_ONE: u16 = 16;
/// A channel holds five bits.
const CHANNEL_MAX: u16 = CHANNEL_MASK;

/// BLDCNT bits 6-7 (`blending.md`, "BLDCNT bits").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectMode {
    /// The special effects are off.
    None,
    /// The first target and the pixel below it are mixed.
    Alpha,
    /// The first target becomes whiter.
    Brighten,
    /// The first target becomes blacker.
    Darken,
}

/// A set of layers that BLDCNT names as targets: BG0-BG3, the object layer and the backdrop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Targets(u8);

impl Targets {
    /// Whether background layer `n` (taken modulo 4) is in the set.
    pub const fn bg(self, n: usize) -> bool {
        self.0 >> (n & 3) & 1 != 0
    }

    /// Whether the object layer is in the set.
    pub const fn obj(self) -> bool {
        self.0 >> LAYER_OBJ & 1 != 0
    }

    /// Whether the backdrop is in the set.
    pub const fn backdrop(self) -> bool {
        self.0 >> LAYER_BACKDROP & 1 != 0
    }

    /// Whether the layer with index `layer` (`LAYER_OBJ` and the like) is in the set.
    const fn has(self, layer: u8) -> bool {
        self.0 >> (layer & 7) & 1 != 0
    }
}

/// The special effects an engine's registers select (`blending.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effects {
    /// BLDCNT bits 6-7.
    pub mode: EffectMode,
    /// The layers the effect acts on.
    pub first: Targets,
    /// The layers alpha blending mixes into the first target.
    pub second: Targets,
    /// EVA in sixteenths, 0-16 (a register value above 16 reads as 16).
    pub eva: u8,
    /// EVB in sixteenths, 0-16.
    pub evb: u8,
    /// EVY in sixteenths, 0-16.
    pub evy: u8,
}

/// A screen pixel as the effects see it: its colour, the layer it came from, and whether it is a
/// semi-transparent object pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pixel {
    pub(super) color: u16,
    pub(super) layer: u8,
    pub(super) semi: bool,
    /// The 3D layer's alpha (0-31) where the pixel is from it, else `None`.
    // TODO(phase7): the 3D layer's alpha as a first target (`blending.md`, "The 3D layer", #49).
    pub(super) alpha: Option<u8>,
}

/// `a` and `b` combined per 5-bit channel by `f`; the top bit of the colour stays clear.
fn per_channel(a: u16, b: u16, f: impl Fn(u16, u16) -> u16) -> u16 {
    [0, GREEN_SHIFT, BLUE_SHIFT]
        .iter()
        .fold(0, |color, &shift| {
            color | (f((a >> shift) & CHANNEL_MAX, (b >> shift) & CHANNEL_MAX) << shift)
        })
}

impl Effects {
    /// The effects BLDCNT, BLDALPHA and BLDY hold.
    pub(super) fn decode(bldcnt: u16, bldalpha: u16, bldy: u16) -> Self {
        let coefficient = |value: u16| value.min(COEFFICIENT_ONE) as u8;
        Self {
            mode: match (bldcnt >> EFFECT_SHIFT) & EFFECT_MASK {
                0 => EffectMode::None,
                1 => EffectMode::Alpha,
                2 => EffectMode::Brighten,
                _ => EffectMode::Darken,
            },
            first: Targets((bldcnt & TARGET_BITS) as u8),
            second: Targets(((bldcnt >> SECOND_SHIFT) & TARGET_BITS) as u8),
            eva: coefficient(bldalpha & CHANNEL_MASK),
            evb: coefficient((bldalpha >> EVB_SHIFT) & CHANNEL_MASK),
            evy: coefficient(bldy & CHANNEL_MASK),
        }
    }

    /// The colour of a screen position whose top pixel is `top` and whose next pixel, asked for only
    /// when the effect needs it, is `below` (`None` when nothing is below). `allowed` is the
    /// window region's effect bit (`blending.md`, "The effect bit of a window region").
    // TODO(hw-unverified): a semi-transparent object in a region whose effect bit is clear shows
    // unchanged, and one over a non-second target takes the BLDCNT brightness effect
    // (`blending.md`, "Unverified").
    pub(super) fn apply(
        &self,
        top: Pixel,
        below: impl FnOnce() -> Option<Pixel>,
        allowed: bool,
    ) -> u16 {
        if !allowed || (self.mode == EffectMode::None && !top.semi) {
            return top.color;
        }
        // A semi-transparent object is a first target whatever BLDCNT says, and blends whatever
        // the effect is; the others are first targets when BLDCNT says so.
        if !top.semi && !self.first.has(top.layer) {
            return top.color;
        }
        let blend = top.semi || self.mode == EffectMode::Alpha;
        if blend {
            return match below().filter(|pixel| self.second.has(pixel.layer)) {
                Some(below) => self.mix(top.color, below.color),
                // Nothing to mix with: only a semi-transparent object still takes the brightness
                // effect of BLDCNT; an alpha effect leaves the pixel alone.
                None => self.brightness(top.color),
            };
        }
        self.brightness(top.color)
    }

    /// `top` and `below` mixed with EVA and EVB: `min(31, top * EVA + below * EVB)` in sixteenths.
    // TODO(hw-unverified): the sum of sixteenths is truncated (`blending.md`, "Unverified").
    fn mix(&self, top: u16, below: u16) -> u16 {
        let (eva, evb) = (u16::from(self.eva), u16::from(self.evb));
        per_channel(top, below, |a, b| {
            ((a * eva + b * evb) / COEFFICIENT_ONE).min(CHANNEL_MAX)
        })
    }

    /// `color` lightened or darkened by EVY when the effect is a brightness one, else unchanged.
    // TODO(hw-unverified): the sixteenths of EVY are truncated (`blending.md`, "Unverified").
    fn brightness(&self, color: u16) -> u16 {
        let evy = u16::from(self.evy);
        match self.mode {
            EffectMode::Brighten => per_channel(color, 0, |c, _| {
                c + (CHANNEL_MAX - c) * evy / COEFFICIENT_ONE
            }),
            EffectMode::Darken => per_channel(color, 0, |c, _| c - c * evy / COEFFICIENT_ONE),
            EffectMode::None | EffectMode::Alpha => color,
        }
    }
}

#[cfg(test)]
mod tests;
