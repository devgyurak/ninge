// SPDX-License-Identifier: Apache-2.0
//! The IMA-ADPCM decoder of a sound channel (`docs/hw/spu.md`, "IMA-ADPCM"): the two tables, the
//! one pure decode step and the decoder state a channel carries from sample to sample.
//!
//! The state cannot be computed from the sample count alone, so it is advanced as the count grows
//! (`Channel::advanced`) and serialized with the channel (`AGENTS.md` §2 rule 5).

use serde::{Deserialize, Serialize};

/// The largest table index a decode step keeps (`spu.md`, "IMA-ADPCM").
const MAX_INDEX: i32 = 88;
/// A header's table index is 7 bits (bits 16-22) and so can name an entry past [`MAX_INDEX`], whose
/// step size is 0 (`spu.md`, "IMA-ADPCM").
pub(super) const INDEX_MASK: u32 = 0x7F;
/// The largest and smallest PCM16 value a decode step produces; `-8000h` can only come from a
/// header (`spu.md`, "IMA-ADPCM").
const PCM_MAX: i32 = 0x7FFF;
const PCM_MIN: i32 = -0x7FFF;

/// How the table index moves for the low three bits of a 4-bit value (GBATEK, "IMA-ADPCM Format",
/// `spu.md`).
const INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// The step sizes by table index (GBATEK, "IMA-ADPCM Format", `spu.md`). The 89 entries GBATEK
/// lists, then 39 zeros: GBATEK's reproduction formula sets entries 89-127 to 0, which a header
/// index past 88 can reach.
const STEP_TABLE: [u16; 128] = {
    const LISTED: [u16; 89] = [
        0x0007, 0x0008, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x0010, 0x0011, 0x0013,
        0x0015, 0x0017, 0x0019, 0x001C, 0x001F, 0x0022, 0x0025, 0x0029, 0x002D, 0x0032, 0x0037,
        0x003C, 0x0042, 0x0049, 0x0050, 0x0058, 0x0061, 0x006B, 0x0076, 0x0082, 0x008F, 0x009D,
        0x00AD, 0x00BE, 0x00D1, 0x00E6, 0x00FD, 0x0117, 0x0133, 0x0151, 0x0173, 0x0198, 0x01C1,
        0x01EE, 0x0220, 0x0256, 0x0292, 0x02D4, 0x031C, 0x036C, 0x03C3, 0x0424, 0x048E, 0x0502,
        0x0583, 0x0610, 0x06AB, 0x0756, 0x0812, 0x08E0, 0x09C3, 0x0ABD, 0x0BD0, 0x0CFF, 0x0E4C,
        0x0FBA, 0x114C, 0x1307, 0x14EE, 0x1706, 0x1954, 0x1BDC, 0x1EA5, 0x21B6, 0x2515, 0x28CA,
        0x2CDF, 0x315B, 0x364B, 0x3BB9, 0x41B2, 0x4844, 0x4F7E, 0x5771, 0x602F, 0x69CE, 0x7462,
        0x7FFF,
    ];
    let mut table = [0; 128];
    let mut i = 0;
    while i < LISTED.len() {
        table[i] = LISTED[i];
        i += 1;
    }
    table
};

/// The first word of the data: the decoder's starting PCM16 value and table index.
pub(super) const fn header(word: u32) -> (i32, u8) {
    // Bits 0-15 are a signed PCM16 value; bits 16-22 the index; bits 23-31 are unused.
    (
        (word as u16 as i16) as i32,
        ((word >> 16) & INDEX_MASK) as u8,
    )
}

/// One decode step: the PCM16 value and table index after the 4-bit value `data` (only its low four
/// bits count), from `pcm` and `index` (`spu.md`, "IMA-ADPCM").
///
/// The step size is split in pieces that each drop their remainder, as the hardware does; the add
/// side stops at `7FFFh` and the subtract side at `-7FFFh`, so a starting `-8000h` survives an add
/// and is lifted by a subtract.
pub(super) fn step(pcm: i32, index: u8, data: u8) -> (i32, u8) {
    let size = i32::from(STEP_TABLE[usize::from(index) & 0x7F]);
    let diff = size / 8
        + if data & 1 != 0 { size / 4 } else { 0 }
        + if data & 2 != 0 { size / 2 } else { 0 }
        + if data & 4 != 0 { size } else { 0 };
    let pcm = if data & 8 == 0 {
        (pcm + diff).min(PCM_MAX)
    } else {
        (pcm - diff).max(PCM_MIN)
    };
    let index = (i32::from(index) + INDEX_TABLE[usize::from(data & 7)]).clamp(0, MAX_INDEX);
    (pcm, index as u8)
}

/// A channel's decoder state (`spu.md`, "IMA-ADPCM"): the value and index after the last decoded
/// sample, which data sample comes next, and the state saved at the loop start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adpcm {
    /// Samples of the sample clock the decoder has caught up with: 0 until the header is read.
    pub(super) ticks: u64,
    /// The data sample the next tick decodes (the first is 0, after the header word).
    pub(super) next: u64,
    pub(super) pcm: i32,
    pub(super) index: u8,
    /// The value and index at the loop start, put back each time the loop wraps.
    pub(super) loop_pcm: i32,
    pub(super) loop_index: u8,
}

impl Adpcm {
    /// The PCM16 value after the last decoded sample.
    pub const fn pcm(&self) -> i32 {
        self.pcm
    }

    /// The table index after the last decoded sample.
    pub const fn index(&self) -> u8 {
        self.index
    }

    /// The value and index saved at the loop start.
    pub const fn loop_state(&self) -> (i32, u8) {
        (self.loop_pcm, self.loop_index)
    }

    /// Whether the state is one the decoder can reach: an index of 7 bits and a PCM16 value.
    pub(super) const fn well_formed(&self) -> bool {
        self.index <= INDEX_MASK as u8
            && self.loop_index <= INDEX_MASK as u8
            && self.pcm >= i16::MIN as i32
            && self.pcm <= i16::MAX as i32
            && self.loop_pcm >= i16::MIN as i32
            && self.loop_pcm <= i16::MAX as i32
    }

    /// The state with the header decoded in: the first tick of a sound.
    pub(super) const fn from_header(word: u32) -> Self {
        let (pcm, index) = header(word);
        Self {
            ticks: 0,
            next: 0,
            pcm,
            index,
            loop_pcm: pcm,
            loop_index: index,
        }
    }

    /// Decodes data sample `at` (from the byte `data`, the low nibble for an even sample) at the
    /// next tick. At the loop start (`loop_start`, in data samples) the state is saved; reaching it
    /// by wrapping rather than by playing on puts the saved state back first, so each pass decodes
    /// the same values (`spu.md`, "IMA-ADPCM").
    pub(super) fn decode(&mut self, at: u64, loop_start: u64, data: u8) {
        if at == loop_start {
            if at != self.next {
                self.pcm = self.loop_pcm;
                self.index = self.loop_index;
            }
            self.loop_pcm = self.pcm;
            self.loop_index = self.index;
        }
        let nibble = if at.is_multiple_of(2) {
            data & 0xF
        } else {
            data >> 4
        };
        (self.pcm, self.index) = step(self.pcm, self.index, nibble);
        self.next = at + 1;
    }
}

#[cfg(test)]
mod tests;
