// SPDX-License-Identifier: Apache-2.0
//! What differs between the two engines, as data (`docs/hw/bg-modes.md`, "Engine B"). One
//! [`Engine`](super::Engine) draws both; it does not know which one it is. [`EngineSpec`] says
//! which DISPCNT fields exist, and [`EngineInputs`] is what the machine hands a line: the memory
//! that engine reads. The spec is a constant, not state, so it is not saved.

use crate::bus::VramWindow;

/// DISPCNT bit 3: BG0 is the 3D engine's layer (engine A only).
pub(super) const DISPCNT_BG0_3D: u32 = 1 << 3;
/// DISPCNT bit 17: with bit 16 the display mode; engine B has modes 0-1 only.
const DISPCNT_MODE_HIGH: u32 = 1 << 17;
/// DISPCNT bits 18-19: the VRAM block of display mode 2 (engine A only).
const DISPCNT_VRAM_BLOCK: u32 = 0x3 << 18;
/// DISPCNT bits 24-29: the character and screen base in 64 KiB steps (engine A only).
const DISPCNT_BASES: u32 = 0x3F << 24;

/// Which fields of an engine's registers exist (`docs/hw/bg-modes.md`, "Engine B").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EngineSpec {
    /// The DISPCNT bits the engine has; a bit it lacks reads as 0 to the renderer, so the
    /// features behind it (3D BG0, display modes 2-3, the character and screen bases) are off.
    // TODO(hw-unverified): the bits engine B lacks are stored as written and ignored
    // (`bg-modes.md`, "Unverified").
    pub(crate) dispcnt_mask: u32,
    /// Whether BG mode 6 exists: BG2 as the large bitmap (`bg-modes.md`, "Large bitmap").
    pub(crate) large_bitmap: bool,
}

/// Engine A: every field.
pub(crate) const MAIN_SPEC: EngineSpec = EngineSpec {
    dispcnt_mask: u32::MAX,
    large_bitmap: true,
};

/// Engine B: no 3D BG0, display modes 0-1 only, no VRAM block, no character or screen base, no
/// BG mode 6.
pub(crate) const SUB_SPEC: EngineSpec = EngineSpec {
    dispcnt_mask: !(DISPCNT_BG0_3D | DISPCNT_MODE_HIGH | DISPCNT_VRAM_BLOCK | DISPCNT_BASES),
    large_bitmap: false,
};

impl EngineSpec {
    /// `raw` DISPCNT as this engine sees it.
    pub(crate) const fn dispcnt(&self, raw: u32) -> u32 {
        raw & self.dispcnt_mask
    }
}

/// What the machine hands an engine to draw one line: its windows of BG and OBJ VRAM, its half
/// of the palette RAM (`palette-ram.md`, "Layout"), its half of OAM (`obj.md`) and its spec.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EngineInputs<'a> {
    pub(crate) vram: VramWindow<'a>,
    /// The OBJ window, where sprites read their tiles.
    #[expect(
        dead_code,
        reason = "the sprite renderer reads it in the next PR (#31)"
    )]
    pub(crate) obj_vram: VramWindow<'a>,
    pub(crate) palette: &'a [u8],
    /// The engine's 1 KiB of OAM: 128 entries of 8 bytes.
    #[expect(
        dead_code,
        reason = "the sprite renderer reads it in the next PR (#31)"
    )]
    pub(crate) oam: &'a [u8],
    pub(crate) spec: &'static EngineSpec,
}

impl<'a> EngineInputs<'a> {
    /// Engine A's inputs.
    pub(crate) fn main(
        vram: VramWindow<'a>,
        obj_vram: VramWindow<'a>,
        palette: &'a [u8],
        oam: &'a [u8],
    ) -> Self {
        Self {
            vram,
            obj_vram,
            palette,
            oam,
            spec: &MAIN_SPEC,
        }
    }

    /// Engine B's inputs.
    pub(crate) fn sub(
        vram: VramWindow<'a>,
        obj_vram: VramWindow<'a>,
        palette: &'a [u8],
        oam: &'a [u8],
    ) -> Self {
        Self {
            vram,
            obj_vram,
            palette,
            oam,
            spec: &SUB_SPEC,
        }
    }
}
