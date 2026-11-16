// SPDX-License-Identifier: Apache-2.0
//! The display capture unit of engine A: DISPCAPCNT, the latch that starts a capture at line 0 and
//! ends it at line 192, and the pure functions that make and store one captured line
//! (`docs/hw/display-capture.md`). The machine owns the VRAM it writes, so these functions take
//! the bank as a slice.

use super::{BLUE_SHIFT, CHANNEL_MASK, GREEN_SHIFT, with_byte};
use crate::SCREEN_WIDTH;
use serde::{Deserialize, Serialize};

/// DISPCAPCNT, at offset `64h` of engine A's register block, a word (`display-capture.md`,
/// "Registers").
pub(super) const CAPTURE_OFFSET: u32 = 0x64;
/// Bytes of that register.
pub(super) const CAPTURE_BYTES: u32 = 4;
/// The bits DISPCAPCNT has: EVA, EVB, the write block, offset and size, source A and B, the read
/// offset, the capture source and enable. The others read 0 (`display-capture.md`, "Registers").
// TODO(hw-unverified): the unused bits are dropped on write and read 0 (`display-capture.md`,
// "Unverified").
const CONTROL_BITS: u32 = 0xEF3F_1F1F;
/// Bit 31: enable when written, busy when read.
const ENABLE: u32 = 1 << 31;

const WRITE_BLOCK_SHIFT: u32 = 16;
const WRITE_OFFSET_SHIFT: u32 = 18;
const SIZE_SHIFT: u32 = 20;
const SOURCE_A_3D: u32 = 1 << 24;
const SOURCE_B_FIFO: u32 = 1 << 25;
const READ_OFFSET_SHIFT: u32 = 26;
const SOURCE_SHIFT: u32 = 29;
/// A 2-bit field.
const TWO_BITS: u32 = 0x3;

/// The step of a VRAM offset field: value `n` is `n * 8000h` bytes.
const OFFSET_STEP: usize = 0x8000;
/// An offset wraps at the end of the 128 KiB bank.
const BANK_MASK: usize = 0x1_FFFF;
/// Bit 15 of a stored halfword: the pixel's alpha.
const ALPHA: u16 = 0x8000;
/// The colour bits of a halfword.
const COLOR_MASK: u16 = 0x7FFF;
/// EVA and EVB are the low five bits of their fields, at bit 0 and bit 8, in sixteenths; a factor
/// above 16 counts as 16.
const EV_MASK: u32 = 0x1F;
const EVB_SHIFT: u32 = 8;
const FACTOR_ONE: u32 = 16;

/// DISPCAPCNT and the capture's latch. Plain data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    // DISPCAPCNT as read back: the bits that exist, bit 31 the busy bit.
    control: u32,
    // The enable was seen at line 0, so lines are being captured until line 192.
    running: bool,
}

/// What a capture does for one line: DISPCAPCNT decoded (`display-capture.md`, "The capture
/// pipeline"). A copy of the register, not state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaptureLine {
    control: u32,
    // DISPCNT bits 18-19: the VRAM block source B reads.
    read_block: usize,
    // DISPCNT's display mode is 2, where the read offset is ignored.
    vram_display: bool,
}

/// What the capture unit stores for a pixel, in the order of DISPCAPCNT bits 29-30.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    A,
    B,
    Blend,
}

impl Capture {
    /// Whether the state is one the unit can reach: only the bits that exist, and a capture runs
    /// only with the enable bit set. A savestate that decodes to anything else is corrupt.
    pub(super) fn well_formed(&self) -> bool {
        self.control & !CONTROL_BITS == 0 && (!self.running || self.control & ENABLE != 0)
    }

    /// The register as read.
    pub fn control(&self) -> u32 {
        self.control
    }

    /// Whether a capture has begun and lines are being captured: the latch the register's busy bit
    /// does not show (a capture enabled mid-frame is busy, not running, until line 0).
    pub fn running(&self) -> bool {
        self.running
    }

    pub(super) fn read_byte(&self, byte: u32) -> u8 {
        (self.control >> (byte * 8)) as u8
    }

    /// Writes one byte. Clearing the enable bit stops a running capture
    /// (`display-capture.md`, "Unverified").
    // TODO(hw-unverified): a write that clears bit 31 during a capture stops it.
    pub(super) fn write_byte(&mut self, byte: u32, value: u8) {
        self.control = with_byte(self.control, byte, value) & CONTROL_BITS;
        self.running &= self.control & ENABLE != 0;
    }

    /// Line `y` is about to be made: line 0 begins a capture that was enabled
    /// (`display-capture.md`, "The capture pipeline").
    // TODO(hw-unverified): the fields are read at each line; only the enable is latched at line 0
    // (`display-capture.md`, "Unverified").
    pub(super) fn begin_line(&mut self, y: usize) {
        if y == 0 && self.control & ENABLE != 0 {
            self.running = true;
        }
    }

    /// Line 192 begins: a running capture is over, whatever its size.
    pub(super) fn vblank_started(&mut self) {
        if self.running {
            self.running = false;
            self.control &= !ENABLE;
        }
    }

    /// What to capture on visible line `y`, given engine A's `dispcnt` (the VRAM read block and
    /// the display mode), or `None` when no capture runs or the size has no row `y`.
    // TODO(hw-unverified): the fields are read at each line (`display-capture.md`, "Unverified").
    pub(super) fn line(&self, y: usize, dispcnt: u32) -> Option<CaptureLine> {
        let line = CaptureLine {
            control: self.control,
            read_block: ((dispcnt >> super::DISPCNT_BLOCK_SHIFT) & super::DISPCNT_BLOCK_MASK)
                as usize,
            vram_display: (dispcnt >> super::DISPCNT_MODE_SHIFT) & super::DISPCNT_MODE_MASK
                == super::MODE_VRAM,
        };
        (self.running && y < line.height()).then_some(line)
    }
}

impl CaptureLine {
    fn field(&self, shift: u32) -> u32 {
        (self.control >> shift) & TWO_BITS
    }

    /// The capture's width in pixels.
    fn width(&self) -> usize {
        match self.field(SIZE_SHIFT) {
            0 => 128,
            _ => SCREEN_WIDTH,
        }
    }

    /// The capture's height in rows.
    fn height(&self) -> usize {
        match self.field(SIZE_SHIFT) {
            0 | 2 => 128,
            1 => 64,
            _ => 192,
        }
    }

    fn source(&self) -> Source {
        match self.field(SOURCE_SHIFT) {
            0 => Source::A,
            1 => Source::B,
            _ => Source::Blend,
        }
    }

    /// The VRAM bank (0-3 for A-D) the capture writes.
    pub(crate) fn write_block(&self) -> usize {
        self.field(WRITE_BLOCK_SHIFT) as usize
    }

    /// The VRAM bank (0-3 for A-D) source B reads: DISPCNT bits 18-19, the block display mode 2
    /// shows.
    pub(crate) fn read_block(&self) -> usize {
        self.read_block
    }

    /// Whether the line needs engine A's graphics screen.
    pub(crate) fn reads_source_a(&self) -> bool {
        self.source() != Source::B
    }

    /// The byte offset of pixel `(x, y)` in a bank, from the offset field at `shift` (`None` is an
    /// offset of 0). The stride is the capture width and the offset wraps at the end of the bank.
    // TODO(hw-unverified): the 128-wide size has a row stride of 128 pixels (`display-capture.md`,
    // "Unverified").
    fn offset_by(&self, shift: Option<u32>, x: usize, y: usize) -> usize {
        let base = shift.map_or(0, |shift| self.field(shift) as usize * OFFSET_STEP);
        (base + (y * self.width() + x) * 2) & BANK_MASK
    }

    /// The pixels source A gives for `composed`, the graphics screen: alpha 1 for every pixel.
    // TODO(hw-unverified): a 2D pixel's alpha is 1 (`display-capture.md`, "Unverified").
    // TODO(phase7): the 3D screen as source A is a transparent zero until the 3D engine exists.
    fn source_a(&self, composed: &[u16; SCREEN_WIDTH]) -> [u16; SCREEN_WIDTH] {
        if self.control & SOURCE_A_3D != 0 {
            return [0; SCREEN_WIDTH];
        }
        composed.map(|pixel| (pixel & COLOR_MASK) | ALPHA)
    }

    /// Source B from VRAM for row `y`: the halfwords of the read block, alpha in bit 15. A block
    /// that is not in LCDC mode (`None`) reads as zeros. The read offset is ignored in display
    /// mode 2 (`display-capture.md`, "The capture pipeline").
    // TODO(hw-unverified): a read block that is not in LCDC mode reads 0, and the 128-wide size
    // reads with a row stride of 128 pixels (`display-capture.md`, "Unverified").
    // TODO(phase4): the main-memory display FIFO as source B (#36).
    pub(crate) fn source_b(&self, y: usize, bank: Option<&[u8]>) -> [u16; SCREEN_WIDTH] {
        let mut line = [0; SCREEN_WIDTH];
        if self.control & SOURCE_B_FIFO != 0 {
            return line;
        }
        let shift = if self.vram_display {
            None
        } else {
            Some(READ_OFFSET_SHIFT)
        };
        line.iter_mut()
            .take(self.width())
            .enumerate()
            .for_each(|(x, pixel)| {
                let at = self.offset_by(shift, x, y);
                let bytes = bank.and_then(|bank| bank.get(at..at + 2));
                *pixel = bytes.map_or(0, |bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
            });
        line
    }

    /// One colour channel of the blend: `a` and `b` are the channels, `alpha_a` and `alpha_b` the
    /// pixels' alpha bits, `eva` and `evb` the factors.
    fn blend_channel(a: u32, alpha_a: u32, eva: u32, b: u32, alpha_b: u32, evb: u32) -> u16 {
        let sum = (a * alpha_a * eva + b * alpha_b * evb) / FACTOR_ONE;
        sum.min(u32::from(CHANNEL_MASK)) as u16
    }

    /// The pixel A+B blended gives (`display-capture.md`, "The capture pipeline"): per channel
    /// `(A * alphaA * EVA + B * alphaB * EVB) / 16`, saturating at 31, and the alpha is
    /// `(alphaA and EVA > 0) or (alphaB and EVB > 0)`.
    // TODO(hw-unverified): a factor above 16 counts as 16, and the sum saturates at 31
    // (`display-capture.md`, "Unverified").
    fn blend(&self, a: u16, b: u16) -> u16 {
        let eva = (self.control & EV_MASK).min(FACTOR_ONE);
        let evb = ((self.control >> EVB_SHIFT) & EV_MASK).min(FACTOR_ONE);
        let (alpha_a, alpha_b) = (u32::from(a >> 15), u32::from(b >> 15));
        let channel = |shift: u32| {
            Self::blend_channel(
                u32::from((a >> shift) & CHANNEL_MASK),
                alpha_a,
                eva,
                u32::from((b >> shift) & CHANNEL_MASK),
                alpha_b,
                evb,
            ) << shift
        };
        let alpha = u16::from((alpha_a == 1 && eva > 0) || (alpha_b == 1 && evb > 0));
        channel(0) | channel(GREEN_SHIFT) | channel(BLUE_SHIFT) | alpha << 15
    }

    /// The line the unit stores for the graphics screen line `composed` and source B's line `b`
    /// (`width` pixels of it are stored).
    pub(crate) fn pixels(
        &self,
        composed: &[u16; SCREEN_WIDTH],
        b: &[u16; SCREEN_WIDTH],
    ) -> [u16; SCREEN_WIDTH] {
        let a = self.source_a(composed);
        match self.source() {
            Source::A => a,
            Source::B => *b,
            Source::Blend => std::array::from_fn(|x| self.blend(a[x], b[x])),
        }
    }

    /// Stores the line `pixels` as row `y` of the capture into `bank`, the 128 KiB write block.
    pub(crate) fn store(&self, y: usize, bank: &mut [u8], pixels: &[u16; SCREEN_WIDTH]) {
        pixels
            .iter()
            .take(self.width())
            .enumerate()
            .for_each(|(x, pixel)| {
                let at = self.offset_by(Some(WRITE_OFFSET_SHIFT), x, y);
                if let Some(slot) = bank.get_mut(at..at + 2) {
                    slot.copy_from_slice(&pixel.to_le_bytes());
                }
            });
    }
}
