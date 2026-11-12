// SPDX-License-Identifier: Apache-2.0
//! What each background layer of a BG mode is (`docs/hw/bg-modes.md`, "The mode table").

use super::spec::EngineSpec;

/// How a layer gets its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Tiles and a map of 32x32-cell blocks (`bg-text.md`).
    Text,
    /// A rotated and scaled tile layer with one-byte map entries (`bg-modes.md`, "Affine layers").
    Affine,
    /// An extended layer: 16-bit map entries, a 256-colour bitmap or a direct-colour bitmap, by
    /// BGxCNT bits 7 and 2 (`bg-modes.md`, "Extended layers").
    Extended,
    /// The large 256-colour bitmap of BG mode 6 (engine A only).
    Large,
    /// The 3D layer or absent: nothing is drawn (`TODO(phase7)` for the 3D layer).
    Other,
}

use Kind::{Affine, Extended, Large, Other, Text};

/// DISPCNT bits 0-2: the BG mode.
const DISPCNT_BG_MODE_MASK: u32 = 0x7;
/// DISPCNT bit 3: BG0 shows the 3D engine instead of its own map.
const DISPCNT_BG0_3D: u32 = 1 << 3;
/// DISPCNT bits 8-11: BG0-BG3 on.
const DISPCNT_LAYER_SHIFT: u32 = 8;

/// The kind of each layer for BG mode 0-7.
// TODO(hw-unverified): mode 7 is not defined and draws nothing (`bg-text.md`, "Unverified").
const KINDS: [[Kind; 4]; 8] = [
    [Text, Text, Text, Text],
    [Text, Text, Text, Affine],
    [Text, Text, Affine, Affine],
    [Text, Text, Text, Extended],
    [Text, Text, Affine, Extended],
    [Text, Text, Extended, Extended],
    // Mode 6: BG0 is the 3D layer only, BG2 the large bitmap (GBATEK "DS Video BG Modes / Control").
    // TODO(phase7): the 3D layer.
    [Other, Other, Large, Other],
    [Other, Other, Other, Other],
];

/// The kind the BG mode of `dispcnt` (as the engine sees it) gives layer `layer`, whether or not
/// the layer is switched on. An engine without BG mode 6 has no large bitmap: the layer is
/// absent.
pub(super) fn kind_of(dispcnt: u32, layer: usize, spec: &EngineSpec) -> Kind {
    match KINDS[(dispcnt & DISPCNT_BG_MODE_MASK) as usize][layer & 3] {
        Large if !spec.large_bitmap => Other,
        kind => kind,
    }
}

/// The kind of layer `layer` if this call draws it: switched on in `dispcnt` (the DISPCNT as the
/// engine sees it), a kind this phase draws in the BG mode, and not BG0 handed to the 3D engine
/// (`TODO(phase7)`).
pub(super) fn drawn_kind(dispcnt: u32, layer: usize, spec: &EngineSpec) -> Option<Kind> {
    let on = dispcnt >> (DISPCNT_LAYER_SHIFT + layer as u32) & 1 != 0;
    let three_d = layer == 0 && dispcnt & DISPCNT_BG0_3D != 0;
    let kind = kind_of(dispcnt, layer, spec);
    (on && !three_d && kind != Other).then_some(kind)
}
