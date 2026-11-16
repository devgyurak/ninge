// SPDX-License-Identifier: Apache-2.0
//! DISP_MMEM_FIFO, the main-memory display FIFO of engine A: the pixels a DMA channel streams in
//! from main RAM, a line at a time, for display mode 3 and for capture source B
//! (`docs/hw/display-capture.md`, "The main-memory display").

use crate::SCREEN_WIDTH;
use crate::io::device::Width;
use serde::{Deserialize, Serialize};

/// DISP_MMEM_FIFO, at offset `68h` of engine A's register block, a word (`display-capture.md`,
/// "Registers").
pub(super) const FIFO_OFFSET: u32 = 0x68;
/// Bytes of that register.
pub(super) const FIFO_BYTES: u32 = 4;
/// Bit 15 of a pixel is unused.
const COLOR_MASK: u32 = 0x7FFF;

/// The pixels waiting for the display, in arrival order. It holds one line.
// TODO(hw-unverified): the FIFO's depth is one line, and a pixel that does not fit is dropped
// (`display-capture.md`, "Unverified").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayFifo {
    pixels: Vec<u16>,
}

impl DisplayFifo {
    pub(super) fn new() -> Self {
        Self {
            pixels: Vec::with_capacity(SCREEN_WIDTH),
        }
    }

    /// How many pixels are waiting.
    pub fn len(&self) -> usize {
        self.pixels.len()
    }

    /// Whether no pixel is waiting.
    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }

    /// Whether the FIFO holds only what it can: a savestate that decodes to more is corrupt.
    pub(super) fn well_formed(&self) -> bool {
        self.pixels.len() <= SCREEN_WIDTH && self.pixels.iter().all(|&p| u32::from(p) <= COLOR_MASK)
    }

    fn push(&mut self, pixel: u32) {
        if self.pixels.len() < SCREEN_WIDTH {
            self.pixels.push((pixel & COLOR_MASK) as u16);
        }
    }

    /// A write to the register: a word pushes two pixels, the low halfword first, a halfword one
    /// pixel, and a byte nothing (`display-capture.md`, "The main-memory display").
    // TODO(hw-unverified): a halfword write pushes one pixel and a byte write nothing
    // (`display-capture.md`, "Unverified").
    pub(super) fn write(&mut self, width: Width, value: u32) {
        match width {
            Width::Word => {
                self.push(value & 0xFFFF);
                self.push(value >> 16);
            }
            Width::Half => self.push(value & 0xFFFF),
            Width::Byte => {}
        }
    }

    /// The next line of 256 pixels: what has arrived, then zeros for what has not (an underrun
    /// shows black). The FIFO is empty after it.
    // TODO(hw-unverified): the display takes a line out of the FIFO at the start of every visible
    // line, shown or not, and an underrun reads 0 (`display-capture.md`, "Unverified").
    pub(super) fn take_line(&mut self) -> [u16; SCREEN_WIDTH] {
        let mut line = [0; SCREEN_WIDTH];
        line.iter_mut()
            .zip(self.pixels.drain(..))
            .for_each(|(slot, pixel)| *slot = pixel);
        line
    }
}
