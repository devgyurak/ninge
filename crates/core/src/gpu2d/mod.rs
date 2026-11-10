// SPDX-License-Identifier: Apache-2.0
//! Graphics engine A's display registers and the Phase 2 picture: display off and the VRAM
//! display (`docs/hw/display-mode-vram.md`).
//!
//! The whole frame is drawn at once from the registers and VRAM as they are when it is asked
//! for. No allocation, formatting or collection happens while pixels are made.
//!
//! Engine A is wired to the ARM9 alone, so `Gpu2d` is a plain type with no CPU const generic: the
//! "seen by one CPU" class of `AGENTS.md` §4 ("Per-CPU devices").

use crate::io::device::{Device, Effect, Width, read_bytes, write_bytes};
use serde::{Deserialize, Serialize};

/// DISPCNT of engine A, at `0400_0000h` (`display-mode-vram.md`, "Registers").
pub const DISPCNT_A_ADDR: u32 = 0x0400_0000;
/// POWCNT1, at `0400_0304h` (`display-mode-vram.md`, "Registers").
pub const POWCNT1_ADDR: u32 = 0x0400_0304;
/// POWCNT1's offset in the register space of [`Gpu2d`], which starts at DISPCNT.
const POWCNT1_OFFSET: u32 = POWCNT1_ADDR - DISPCNT_A_ADDR;
/// The xRGB8888 value of a white pixel.
pub const WHITE: u32 = 0x00FF_FFFF;
/// The xRGB8888 value of a black pixel.
pub const BLACK: u32 = 0;

/// DISPCNT bits 16-17: the display mode.
const DISPCNT_MODE_SHIFT: u32 = 16;
const DISPCNT_MODE_MASK: u32 = 0x3;
/// Display mode 0: display off, a white screen.
const MODE_OFF: u32 = 0;
/// Display mode 2: the bitmap read from a VRAM block.
const MODE_VRAM: u32 = 2;
/// DISPCNT bits 18-19: the VRAM block mode 2 reads.
const DISPCNT_BLOCK_SHIFT: u32 = 18;
const DISPCNT_BLOCK_MASK: u32 = 0x3;
/// The block number of bank A, the only block modeled in Phase 2.
const BLOCK_A: u32 = 0;
/// POWCNT1 bit 15: display swap. Set sends engine A to the upper screen.
const POWCNT1_SWAP: u16 = 1 << 15;
/// POWCNT1 after direct boot: LCD, both 2D engines and both 3D units on, engine A on top.
// TODO(hw-unverified): a choice made for lack of evidence; the smoke golden depends on bit 15
// (`display-mode-vram.md`, "Unverified").
const POWCNT1_RESET: u16 = 0x820F;

/// Colour channel mask: five bits per channel in the 15-bit colour.
const CHANNEL_MASK: u16 = 0x1F;
/// Bit offsets of green and blue in the 15-bit colour; red is at bit 0.
const GREEN_SHIFT: u32 = 5;
const BLUE_SHIFT: u32 = 10;

/// Engine A's display registers (`display-mode-vram.md`, "Registers").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu2d {
    dispcnt_a: u32,
    powcnt1: u16,
}

/// Widens a 5-bit channel to 8 bits by repeating its top bits. A host-side choice, not a
/// hardware fact (`display-mode-vram.md`, "Colour expansion"; #68).
fn expand_channel(channel: u16) -> u32 {
    let c = u32::from(channel & CHANNEL_MASK);
    (c << 3) | (c >> 2)
}

/// A 15-bit colour as xRGB8888. Bit 15 is not part of the colour.
fn expand_color(halfword: u16) -> u32 {
    (expand_channel(halfword) << 16)
        | (expand_channel(halfword >> GREEN_SHIFT) << 8)
        | expand_channel(halfword >> BLUE_SHIFT)
}

/// `reg` with its byte `index` replaced by `value`.
fn with_byte(reg: u32, index: u32, value: u8) -> u32 {
    let shift = index * 8;
    (reg & !(0xFF << shift)) | (u32::from(value) << shift)
}

impl Gpu2d {
    /// Registers in their power-on state.
    // TODO(hw-unverified): DISPCNT starts at 0 and POWCNT1 at `820Fh`, neither stated by
    // GBATEK (`display-mode-vram.md`, "Unverified").
    pub fn new() -> Self {
        Self {
            dispcnt_a: 0,
            powcnt1: POWCNT1_RESET,
        }
    }

    /// DISPCNT of engine A.
    pub fn dispcnt_a(&self) -> u32 {
        self.dispcnt_a
    }

    /// POWCNT1.
    pub fn powcnt1(&self) -> u16 {
        self.powcnt1
    }

    /// Whether POWCNT1 bit 15 sends engine A to the upper screen.
    // TODO(hw-unverified): bits 0 (LCD enable) and 1 (engine A enable) have no effect on the
    // picture (`display-mode-vram.md`, "Unverified").
    // TODO(phaseN): the other POWCNT1 bits arrive with their units (3D: phase 7, engine B:
    // phase 4).
    pub fn engine_a_on_top(&self) -> bool {
        self.powcnt1 & POWCNT1_SWAP != 0
    }

    /// The byte at `offset` in the register space, which starts at DISPCNT; 0 where no register
    /// is.
    fn read_byte(&self, offset: u32) -> u8 {
        self.register_at(offset)
            .map_or(0, |(reg, base)| (reg >> ((offset - base) * 8)) as u8)
    }

    /// Writes one byte of DISPCNT or POWCNT1; any other offset is not ours and is ignored.
    // TODO(hw-unverified): POWCNT1's undefined bits are kept as written
    // (`display-mode-vram.md`, "Unverified").
    fn write_byte(&mut self, offset: u32, value: u8) {
        let powcnt1_index = offset.wrapping_sub(POWCNT1_OFFSET);
        if offset < 4 {
            self.dispcnt_a = with_byte(self.dispcnt_a, offset, value);
        } else if powcnt1_index < 2 {
            self.powcnt1 = with_byte(u32::from(self.powcnt1), powcnt1_index, value) as u16;
        }
    }

    /// The register that holds `offset` and its own offset.
    fn register_at(&self, offset: u32) -> Option<(u32, u32)> {
        if offset < 4 {
            Some((self.dispcnt_a, 0))
        } else if offset.wrapping_sub(POWCNT1_OFFSET) < 2 {
            Some((u32::from(self.powcnt1), POWCNT1_OFFSET))
        } else {
            None
        }
    }

    /// Draws engine A's picture into `out` (`SCREEN_WIDTH * SCREEN_HEIGHT` pixels, from the crate root), given the
    /// bytes of bank A when the CPU can see it as LCDC VRAM.
    // TODO(phase3): mode 1 draws the text backgrounds (#104 brings the line timing).
    // TODO(phase4): mode 1 completes, mode 3 draws the main-memory bitmap, and blocks 1-3
    // (banks B-D) join mode 2 (#35).
    // TODO(phase4): master brightness applies to the output of modes 2 and 3 (#34).
    // TODO(hw-unverified): DISPCNT bit 7 (forced blank) is stored but ignored
    // (`display-mode-vram.md`, "Unverified").
    // TODO(hw-unverified): a bank that is disabled or not in LCDC mode is drawn as zeros, i.e.
    // black (`display-mode-vram.md`, "Unverified").
    pub(crate) fn render_engine_a(&self, bank_a: Option<&[u8]>, out: &mut [u32]) {
        let mode = (self.dispcnt_a >> DISPCNT_MODE_SHIFT) & DISPCNT_MODE_MASK;
        let block = (self.dispcnt_a >> DISPCNT_BLOCK_SHIFT) & DISPCNT_BLOCK_MASK;
        if mode == MODE_OFF {
            out.fill(WHITE);
        } else if mode == MODE_VRAM && block == BLOCK_A {
            out.fill(BLACK);
            let (pairs, _) = bank_a.unwrap_or(&[]).as_chunks::<2>();
            out.iter_mut().zip(pairs).for_each(|(pixel, &bytes)| {
                *pixel = expand_color(u16::from_le_bytes(bytes));
            });
        } else {
            out.fill(BLACK);
        }
    }
}

/// Engine A is wired to the ARM9 alone (`AGENTS.md` §4, "Per-CPU devices"). Its registers are
/// bytes; a wider access is split into them.
impl Device<true> for Gpu2d {
    fn peek(&self, offset: u32, width: Width, _now: u64) -> u32 {
        read_bytes(offset, width, |at| self.read_byte(at))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, _now: u64) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte(at, byte);
            Effect::default()
        })
    }
}

impl Default for Gpu2d {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
