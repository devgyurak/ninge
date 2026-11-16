// SPDX-License-Identifier: Apache-2.0
//! The window feature: the rectangles of WIN0 and WIN1, the layer sets WININ and WINOUT give each
//! region, and which set applies at a pixel (`docs/hw/windows.md`).

use crate::SCREEN_WIDTH;
use crate::io::timing::{FIRST_VBLANK_LINE, LINES};
use serde::{Deserialize, Serialize};

/// Only the low eight bits of the scanline number are compared with Y1 and Y2.
const LINE_MASK: u16 = 0xFF;

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

/// The vertical state of both windows: whether each is switched on at the line being made. A
/// window switches on at the line whose low eight bits equal Y1 and off at the one that equal Y2,
/// counting the VBlank lines (`windows.md`, "The vertical test").
// TODO(hw-unverified): the state follows the registers as they are when the lines are made; the
// VBlank lines are stepped together with the registers of the next frame's first line. The state
// moves whether or not DISPCNT turns the window on, and whether or not the engine is powered. A
// window starts off, Y2 is judged after Y1 on the same line, a Y2 of 192 to 255 switches the window
// off in VBlank, and `Y1 = Y2` leaves it off (`windows.md`, "Unverified").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Latches {
    on: [bool; 2],
}

impl Latches {
    // TODO(hw-unverified): with Y1 greater than Y2 the window stays on across VBlank, where the
    // GBA rule reads Y2 as the screen height (`windows.md`, "Unverified").
    /// Scanline `line` (0-262) goes by with the window rows `winv` hold: Y1 switches a window on,
    /// then Y2 switches it off, so `Y1 = Y2` leaves it off.
    fn step(&mut self, line: u16, winv: [u16; 2]) {
        let low = line & LINE_MASK;
        self.on.iter_mut().zip(winv).for_each(|(on, reg)| {
            *on = (*on || low == reg >> 8) && low != reg & LINE_MASK;
        });
    }

    /// Visible line `y` is about to be made. Line 0 first takes the VBlank lines that came after
    /// the last visible one.
    pub(super) fn begin_line(&mut self, y: usize, winv: [u16; 2]) {
        if y == 0 {
            (FIRST_VBLANK_LINE..LINES).for_each(|line| self.step(line, winv));
        }
        // `y` is a visible line, below 192, so it fits.
        self.step(y as u16, winv);
    }

    /// Whether window `n` is switched on at the line being made.
    fn on(self, n: usize) -> bool {
        self.on.get(n).copied().unwrap_or(false)
    }
}

/// One window's rectangle after the rules for odd values (`windows.md`, "The rectangles"): the
/// columns `left..right`, and Y1 and Y2 as written, which the vertical test uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowRect {
    /// The first column (X1).
    pub left: u16,
    /// One past the last column (X2, or the screen width when X2 is past it or before X1).
    pub right: u16,
    /// Y1, as written.
    pub top: u16,
    /// Y2, as written.
    pub bottom: u16,
}

impl WindowRect {
    /// The rectangle `winh` and `winv` hold: bits 0-7 the end, bits 8-15 the start.
    // TODO(hw-unverified): the width limit is the DS screen's, 256, where GBATEK gives the GBA's
    // 240; X1 greater than X2 reads as X2 = 256 (`windows.md`, "Unverified").
    fn decode(winh: u16, winv: u16) -> Self {
        let (start, end) = (winh >> 8, winh & 0xFF);
        Self {
            left: start,
            right: match start > end {
                true => SCREEN_WIDTH as u16,
                false => end,
            },
            top: winv >> 8,
            bottom: winv & 0xFF,
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
    /// Whether window 0 and window 1 are switched on at the line last made (`windows.md`, "The
    /// vertical test").
    pub latched: [bool; 2],
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
        latches: Latches,
    ) -> Self {
        let group = |reg: u16, shift: u32| LayerSet::new((reg >> shift) as u8);
        Self {
            rects: [0, 1].map(|n| WindowRect::decode(winh[n], winv[n])),
            enabled: [
                dispcnt & DISPCNT_WIN0 != 0,
                dispcnt & DISPCNT_WIN1 != 0,
                dispcnt & DISPCNT_OBJ_WINDOW != 0,
            ],
            latched: [latches.on(0), latches.on(1)],
            inside: [group(winin, 0), group(winin, GROUP_SHIFT)],
            outside: group(winout, 0),
            obj_window: group(winout, GROUP_SHIFT),
        }
    }
}

/// The regions of one line, ready to be asked about each pixel.
pub(super) struct LineWindows {
    /// The columns each window covers on this line, empty when the window is off or its latch is
    /// off.
    spans: [(usize, usize); 2],
    inside: [LayerSet; 2],
    /// The OBJ window's layers, when DISPCNT turns it on.
    obj_window: Option<LayerSet>,
    outside: LayerSet,
}

impl LineWindows {
    /// The regions of the line the `latches` stand at (`windows.md`, "The four regions").
    pub(super) fn new(windows: &Windows, latches: Latches) -> Self {
        // With no window on at all there is no windowing: every layer shows everywhere.
        if !windows.enabled.iter().any(|&on| on) {
            return Self {
                spans: [(0, 0); 2],
                inside: [LayerSet::ALL; 2],
                obj_window: None,
                outside: LayerSet::ALL,
            };
        }
        let span = |n: usize| {
            let rect = windows.rects[n];
            match windows.enabled[n] && latches.on(n) {
                true => (rect.left as usize, rect.right as usize),
                false => (0, 0),
            }
        };
        Self {
            spans: [span(0), span(1)],
            inside: windows.inside,
            obj_window: windows.enabled[2].then_some(windows.obj_window),
            outside: windows.outside,
        }
    }

    /// The layers column `x` shows: window 0's, else window 1's, else the OBJ window's where
    /// `obj_window` marks the column, else the outside set.
    pub(super) fn layers(&self, x: usize, obj_window: &[bool; SCREEN_WIDTH]) -> LayerSet {
        let hit = |n: usize| (self.spans[n].0..self.spans[n].1).contains(&x);
        match (hit(0), hit(1), self.obj_window) {
            (true, _, _) => self.inside[0],
            (false, true, _) => self.inside[1],
            (false, false, Some(set)) if obj_window[x] => set,
            _ => self.outside,
        }
    }
}
