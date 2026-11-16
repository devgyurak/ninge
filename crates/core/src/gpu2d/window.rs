// SPDX-License-Identifier: Apache-2.0
//! The window feature: the rectangles of WIN0 and WIN1, the layer sets WININ and WINOUT give each
//! region, and which set applies at a pixel (`docs/hw/windows.md`).

use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// WIN0H, at offset `40h` of an engine's register block; the block runs WIN0H, WIN1H, WIN0V,
/// WIN1V, WININ, WINOUT, two bytes each (`windows.md`, "Registers").
pub(super) const WINDOW_OFFSET: u32 = 0x40;
/// Bytes of that block.
pub(super) const WINDOW_BLOCK_BYTES: u32 = 12;
/// DISPCNT bits 13-15: window 0, window 1 and the OBJ window on (`windows.md`, "DISPCNT bits used
/// here").
const DISPCNT_WIN0: u32 = 1 << 13;
const DISPCNT_WIN1: u32 = 1 << 14;
const DISPCNT_OBJ_WINDOW: u32 = 1 << 15;
/// The bits WININ and WINOUT keep: two six-bit groups; bits 6-7 and 14-15 are not used.
// TODO(hw-unverified): the unused bits are cleared on write and read back 0 (`windows.md`,
// "Unverified").
pub(super) const GROUP_MASK: u16 = 0x3F3F;
/// Bits of one group: BG0-BG3 in bits 0-3, OBJ in bit 4, the colour special effect in bit 5.
const GROUP_BITS: u8 = 0x3F;
const GROUP_OBJ: u8 = 1 << 4;
const GROUP_EFFECT: u8 = 1 << 5;
/// The second group of WININ and WINOUT is the high byte.
const GROUP_SHIFT: u32 = 8;

/// The layers and the effect a region shows (`windows.md`, "WININ and WINOUT").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerSet(u8);

impl LayerSet {
    /// Every layer and the effect: what a pixel shows when no window is on.
    const ALL: Self = Self(GROUP_BITS);

    /// A group of the six bits at the bottom of `bits`.
    const fn new(bits: u8) -> Self {
        Self(bits & GROUP_BITS)
    }

    /// Whether background layer `n` (taken modulo 4) is shown.
    pub const fn bg(self, n: usize) -> bool {
        self.0 >> (n & 3) & 1 != 0
    }

    /// Whether the OBJ layer is shown.
    pub const fn obj(self) -> bool {
        self.0 & GROUP_OBJ != 0
    }

    /// Whether the colour special effect applies.
    // TODO(phase4): the effect itself is #34; nothing reads this bit yet.
    pub const fn effect(self) -> bool {
        self.0 & GROUP_EFFECT != 0
    }
}

/// One window's rectangle after the rules for odd values (`windows.md`, "The rectangles"): the
/// columns `left..right` and the rows `top..bottom`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowRect {
    /// The first column (X1).
    pub left: u16,
    /// One past the last column (X2, or the screen width when X2 is past it or before X1).
    pub right: u16,
    /// The first row (Y1).
    pub top: u16,
    /// One past the last row (Y2, or the screen height when Y2 is past it or before Y1).
    pub bottom: u16,
}

impl WindowRect {
    /// The rectangle `winh` and `winv` hold: bits 0-7 the end, bits 8-15 the start.
    // TODO(hw-unverified): the limits are the DS screen's, 256 by 192, where GBATEK gives the GBA's
    // 240 by 160, and a window is judged by comparing the line against Y1 and Y2 as the line is
    // made (`windows.md`, "Unverified").
    fn decode(winh: u16, winv: u16) -> Self {
        let end = |reg: u16, limit: u16| {
            let (start, end) = (reg >> 8, reg & 0xFF);
            match end > limit || start > end {
                true => limit,
                false => end,
            }
        };
        Self {
            left: winh >> 8,
            right: end(winh, SCREEN_WIDTH as u16),
            top: winv >> 8,
            bottom: end(winv, SCREEN_HEIGHT as u16),
        }
    }
}

/// Both windows and the regions' layer sets, as an engine's registers and DISPCNT give them. A
/// debugger's BG panel (#58) shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Windows {
    /// Window 0's and window 1's rectangles.
    pub rects: [WindowRect; 2],
    /// Whether DISPCNT turns window 0, window 1 and the OBJ window on.
    pub enabled: [bool; 3],
    /// The layers inside window 0 and inside window 1 (WININ).
    pub inside: [LayerSet; 2],
    /// The layers outside every window (WINOUT, low byte).
    pub outside: LayerSet,
    /// The layers inside the OBJ window (WINOUT, high byte).
    pub obj_window: LayerSet,
}

impl Windows {
    /// Decodes the registers under `dispcnt`.
    pub(super) fn decode(
        dispcnt: u32,
        winh: [u16; 2],
        winv: [u16; 2],
        winin: u16,
        winout: u16,
    ) -> Self {
        let group = |reg: u16, shift: u32| LayerSet::new((reg >> shift) as u8);
        Self {
            rects: [0, 1].map(|n| WindowRect::decode(winh[n], winv[n])),
            enabled: [
                dispcnt & DISPCNT_WIN0 != 0,
                dispcnt & DISPCNT_WIN1 != 0,
                dispcnt & DISPCNT_OBJ_WINDOW != 0,
            ],
            inside: [group(winin, 0), group(winin, GROUP_SHIFT)],
            outside: group(winout, 0),
            obj_window: group(winout, GROUP_SHIFT),
        }
    }
}

/// The regions of one line, ready to be asked about each pixel.
pub(super) struct LineWindows {
    /// The columns each window covers on this line, empty when the window is off or the line is
    /// outside its rows.
    spans: [(usize, usize); 2],
    inside: [LayerSet; 2],
    outside: LayerSet,
}

impl LineWindows {
    /// The regions of line `y` (`windows.md`, "The four regions").
    // TODO(hw-unverified): the row test compares the line against Y1 and Y2 as the line is made
    // (`windows.md`, "Unverified").
    // TODO(phase4): the OBJ window region (#33) is not made yet; its pixels fall to the outside.
    pub(super) fn new(windows: &Windows, y: usize) -> Self {
        // With no window on at all there is no windowing: every layer shows everywhere.
        if !windows.enabled.iter().any(|&on| on) {
            return Self {
                spans: [(0, 0); 2],
                inside: [LayerSet::ALL; 2],
                outside: LayerSet::ALL,
            };
        }
        let span = |n: usize| {
            let rect = windows.rects[n];
            let on = windows.enabled[n] && (rect.top as usize..rect.bottom as usize).contains(&y);
            match on {
                true => (rect.left as usize, rect.right as usize),
                false => (0, 0),
            }
        };
        Self {
            spans: [span(0), span(1)],
            inside: windows.inside,
            outside: windows.outside,
        }
    }

    /// The layers column `x` shows: window 0's, else window 1's, else the outside set.
    pub(super) fn layers(&self, x: usize) -> LayerSet {
        let hit = |n: usize| (self.spans[n].0..self.spans[n].1).contains(&x);
        match (hit(0), hit(1)) {
            (true, _) => self.inside[0],
            (false, true) => self.inside[1],
            (false, false) => self.outside,
        }
    }
}
