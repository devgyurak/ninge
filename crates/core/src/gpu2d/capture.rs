// SPDX-License-Identifier: Apache-2.0
//! The display capture unit of engine A: DISPCAPCNT, the latch that starts a capture at line 0 and
//! ends it at line 192, and the pure functions that make and store one captured line
//! (`docs/hw/display-capture.md`). The machine owns the VRAM it writes, so these functions take
//! the bank as a slice.

use super::with_byte;
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
}

/// What the capture unit stores for a pixel, in the order of DISPCAPCNT bits 29-30.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    A,
    B,
    Blend,
}

impl Capture {
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

    /// What to capture on visible line `y`, or `None` when no capture runs or the size has no row
    /// `y`.
    pub(super) fn line(&self, y: usize) -> Option<CaptureLine> {
        let line = CaptureLine {
            control: self.control,
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

    /// Whether the line needs engine A's graphics screen.
    pub(crate) fn reads_source_a(&self) -> bool {
        self.source() != Source::B
    }

    /// The byte offset of pixel `(x, y)` in a bank, from the offset field at `shift`. The stride is
    /// the capture width and the offset wraps at the end of the bank.
    // TODO(hw-unverified): the 128-wide size has a row stride of 128 pixels (`display-capture.md`,
    // "Unverified").
    fn offset(&self, shift: u32, x: usize, y: usize) -> usize {
        let base = self.field(shift) as usize * OFFSET_STEP;
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

    /// The line the unit stores, `width` pixels of it, for the graphics screen line `composed`.
    // TODO(phase4): the capture sources B and A+B (#36).
    pub(crate) fn pixels(&self, composed: &[u16; SCREEN_WIDTH]) -> [u16; SCREEN_WIDTH] {
        self.source_a(composed)
    }

    /// Stores the line `pixels` as row `y` of the capture into `bank`, the 128 KiB write block.
    pub(crate) fn store(&self, y: usize, bank: &mut [u8], pixels: &[u16; SCREEN_WIDTH]) {
        pixels
            .iter()
            .take(self.width())
            .enumerate()
            .for_each(|(x, pixel)| {
                let at = self.offset(WRITE_OFFSET_SHIFT, x, y);
                if let Some(slot) = bank.get_mut(at..at + 2) {
                    slot.copy_from_slice(&pixel.to_le_bytes());
                }
            });
    }
}
