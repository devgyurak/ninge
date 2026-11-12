// SPDX-License-Identifier: Apache-2.0
//! What each background layer of a BG mode is (`docs/hw/bg-modes.md`, "The mode table").

use super::spec::DISPCNT_BG0_3D;

/// How a layer gets its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Tiles and a map of 32x32-cell blocks (`bg-text.md`).
    Text,
    /// A rotated and scaled tile layer with one-byte map entries (`bg-modes.md`, "Affine layers").
    Affine,
    /// Extended, large bitmap or absent: nothing is drawn (`TODO(phase4)`).
    Other,
}

use Kind::{Affine, Other, Text};

/// DISPCNT bits 0-2: the BG mode.
const DISPCNT_BG_MODE_MASK: u32 = 0x7;
/// DISPCNT bits 8-11: BG0-BG3 on.
const DISPCNT_LAYER_SHIFT: u32 = 8;

/// The kind of each layer for BG mode 0-7.
// TODO(hw-unverified): mode 7 is not defined and draws nothing (`bg-text.md`, "Unverified").
const KINDS: [[Kind; 4]; 8] = [
    [Text, Text, Text, Text],
    [Text, Text, Text, Affine],
    [Text, Text, Affine, Affine],
    [Text, Text, Text, Other],
    [Text, Text, Affine, Other],
    [Text, Text, Other, Other],
    // Mode 6: BG0 is the 3D layer only (GBATEK "DS Video BG Modes / Control").
    // TODO(phase7): the 3D layer.
    [Other, Other, Other, Other],
    [Other, Other, Other, Other],
];

/// The kind of layer `layer` if this call draws it: switched on in `dispcnt` (the DISPCNT as the
/// engine sees it), a kind this phase draws in the BG mode, and not BG0 handed to the 3D engine
/// (`TODO(phase7)`).
pub(super) fn drawn_kind(dispcnt: u32, layer: usize) -> Option<Kind> {
    let mode = (dispcnt & DISPCNT_BG_MODE_MASK) as usize;
    let on = dispcnt >> (DISPCNT_LAYER_SHIFT + layer as u32) & 1 != 0;
    let three_d = layer == 0 && dispcnt & DISPCNT_BG0_3D != 0;
    let kind = KINDS[mode][layer & 3];
    (on && !three_d && kind != Other).then_some(kind)
}
