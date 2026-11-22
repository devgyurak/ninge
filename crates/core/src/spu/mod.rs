// SPDX-License-Identifier: Apache-2.0
//! The sound registers of the ARM7: the 16 channels, SOUNDCNT, SOUNDBIAS and the two capture
//! units (`docs/hw/sound-registers.md`).
//!
//! No sound is made. The registers are stored and read back, and the one thing a program can
//! observe without audio, the busy bit of a channel and of a capture, is computed lazily from an
//! end time recorded when the unit starts: no scheduler event and no event slot. A read compares
//! the master time of the access with the end time, so a savestate taken mid-playback clears the
//! bit at the same time as an uninterrupted run.
//!
//! Only the ARM7 sees these registers, so [`Spu`] is a plain type implementing `Device<false>`
//! (`AGENTS.md` §4, "Per-CPU devices"): the "seen by one CPU" class. It registers directly with
//! its own port, with no `io/` entrance, as `gpu2d` does (`AGENTS.md` §2, "Placement").
// TODO(phase6): the mixer, the sample output and the capture writes replace the lazy end time with
// the real playback position (`sound-registers.md`, "Scope").

use crate::io::device::{Checked, Cx, Device, Effect, Width, read_bytes, write_bytes};
use serde::{Deserialize, Serialize};

/// SOUND0CNT, the first register of the window, at `0400_0400h` (`sound-registers.md`, "The
/// channels").
pub const SOUND_REGS_ADDR: u32 = 0x0400_0400;
/// Bytes of the window the device owns: the channels, SOUNDCNT, SOUNDBIAS and the captures, up to
/// and including SNDCAP1LEN (`0400_051Fh`).
pub const BLOCK_BYTES: u32 = 0x120;
/// The number of sound channels.
pub const CHANNELS: usize = 16;
/// The number of capture units.
pub const CAPTURES: usize = 2;

/// A channel's registers are this many bytes apart.
const CHANNEL_STRIDE_SHIFT: u32 = 4;
/// Where the channels end and SOUNDCNT begins, in the device's offsets.
const SOUNDCNT_OFFSET: u32 = 0x100;
const SOUNDBIAS_OFFSET: u32 = 0x104;
const SNDCAP0CNT_OFFSET: u32 = 0x108;
const SNDCAP1CNT_OFFSET: u32 = 0x109;
const SNDCAP0DAD_OFFSET: u32 = 0x110;
const SNDCAP0LEN_OFFSET: u32 = 0x114;
const SNDCAP1DAD_OFFSET: u32 = 0x118;
const SNDCAP1LEN_OFFSET: u32 = 0x11C;

/// A channel's registers, as offsets inside its 16 bytes.
const CNT_OFFSET: u32 = 0x0;
const SAD_OFFSET: u32 = 0x4;
const TMR_OFFSET: u32 = 0x8;
const PNT_OFFSET: u32 = 0xA;
const LEN_OFFSET: u32 = 0xC;
/// Bytes of the window that belong to one channel.
const CHANNEL_BYTES: u32 = 0x10;

/// SOUNDxCNT bit 31: start / busy.
pub const CNT_START: u32 = 1 << 31;
/// SOUNDxCNT bits 27-28: the repeat mode.
const CNT_REPEAT_SHIFT: u32 = 27;
const CNT_REPEAT_MASK: u32 = 0b11;
/// Repeat mode 2: one-shot, the only mode that ends by itself.
const REPEAT_ONE_SHOT: u32 = 2;
/// SOUNDxCNT bits 29-30: the format.
const CNT_FORMAT_SHIFT: u32 = 29;
const CNT_FORMAT_MASK: u32 = 0b11;
const FORMAT_PCM8: u32 = 0;
const FORMAT_PCM16: u32 = 1;
const FORMAT_ADPCM: u32 = 2;
/// The bits SOUNDxCNT keeps: volume 0-6, divider 8-9, hold 15, panning 16-22, duty 24-26,
/// repeat 27-28, format 29-30 and start 31; bits 7, 10-14 and 23 read zero.
pub const CNT_MASK: u32 = 0xFF7F_837F;
/// SOUNDxSAD: bits 0-26, word aligned.
pub const SAD_MASK: u32 = 0x07FF_FFFC;
/// SOUNDxLEN: bits 0-21.
pub const LEN_MASK: u32 = 0x003F_FFFF;
/// SOUNDCNT: bits 0-6, 8-13 and 15.
pub const SOUNDCNT_MASK: u16 = 0xBF7F;
/// SOUNDBIAS: bits 0-9.
pub const SOUNDBIAS_MASK: u16 = 0x03FF;
/// SNDCAPxCNT bit 7: start / busy.
pub const CAP_START: u8 = 1 << 7;
/// SNDCAPxCNT bit 2: 0 = loop, 1 = one-shot.
const CAP_ONE_SHOT: u8 = 1 << 2;
/// SNDCAPxCNT bit 3: 0 = PCM16, 1 = PCM8.
const CAP_PCM8: u8 = 1 << 3;
/// SNDCAPxCNT: bits 0-3 and 7.
pub const CAP_MASK: u8 = 0x8F;
/// SNDCAPxDAD: bits 0-26, word aligned.
pub const CAP_DAD_MASK: u32 = SAD_MASK;

/// The shortest sum of PNT and LEN, in words, a one-shot sound can play: below it GBATEK says the
/// busy bit stays set for ever (`sound-registers.md`, "The status bit of a channel").
const MIN_WORDS: u32 = 4;
/// A sample takes `4 * (10000h - TMR)` master cycles: the sample clock is the ARM7 clock halved
/// and one ARM7 cycle is two master cycles (`sound-registers.md`, "Model").
const TICKS_PER_TMR_STEP: u64 = 4;
/// Counts in a TMR period, so a TMR of 0 is the longest sample.
const TMR_PERIOD: u64 = 0x1_0000;
/// Samples in a word of a capture: two for PCM16, four for PCM8.
const CAP_SAMPLES_PCM16: u64 = 2;
const CAP_SAMPLES_PCM8: u64 = 4;
/// The channel whose TMR capture 0 and capture 1 share.
const CAPTURE_TIMER_CHANNEL: [usize; CAPTURES] = [1, 3];

/// The end time of a unit that never ends by itself. An unstarted unit holds 0.
const NEVER: u64 = u64::MAX;

/// One sound channel's registers and the end time of its sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    cnt: u32,
    sad: u32,
    tmr: u16,
    pnt: u16,
    len: u32,
    /// Master time the busy bit clears, [`NEVER`] if it does not, 0 while stopped.
    end: u64,
}

impl Channel {
    /// SOUNDxCNT as stored: bit 31 set means started, even after the sound ended (the bit is
    /// folded away on the next write; a read goes through [`Self::cnt_at`]).
    pub const fn cnt(&self) -> u32 {
        self.cnt
    }

    /// SOUNDxCNT as a read at master time `now` sees it: the busy bit is clear once the sound has
    /// ended.
    pub const fn cnt_at(&self, now: u64) -> u32 {
        if self.busy(now) {
            self.cnt
        } else {
            self.cnt & !CNT_START
        }
    }

    /// SOUNDxSAD as written (write-only on the bus).
    pub const fn sad(&self) -> u32 {
        self.sad
    }

    /// SOUNDxTMR as written (write-only on the bus).
    pub const fn tmr(&self) -> u16 {
        self.tmr
    }

    /// SOUNDxPNT as written (write-only on the bus).
    pub const fn pnt(&self) -> u16 {
        self.pnt
    }

    /// SOUNDxLEN as written (write-only on the bus).
    pub const fn length(&self) -> u32 {
        self.len
    }

    /// The master time the busy bit clears: `u64::MAX` for a sound that never ends, 0 while the
    /// channel is stopped.
    pub const fn end(&self) -> u64 {
        self.end
    }

    /// Whether the channel is busy at master time `now`.
    pub const fn busy(&self, now: u64) -> bool {
        self.cnt & CNT_START != 0 && now < self.end
    }

    /// The end time of a sound that starts at `now` with the registers as they are.
    // TODO(hw-unverified): no start delay before the first sample, and TMR, PNT and LEN written
    // while busy do not move the end (`sound-registers.md`, "Unverified").
    // TODO(hw-unverified): the manual and prohibited repeat modes, and PSG and noise on any
    // channel, never end (`sound-registers.md`, "Unverified").
    fn end_from(&self, now: u64) -> u64 {
        let words = u64::from(u32::from(self.pnt) + self.len);
        let one_shot = (self.cnt >> CNT_REPEAT_SHIFT) & CNT_REPEAT_MASK == REPEAT_ONE_SHOT;
        let samples = match (self.cnt >> CNT_FORMAT_SHIFT) & CNT_FORMAT_MASK {
            FORMAT_PCM8 => 4 * words,
            FORMAT_PCM16 => 2 * words,
            FORMAT_ADPCM => 8 * words.saturating_sub(1),
            _ => return NEVER,
        };
        if !one_shot || words < u64::from(MIN_WORDS) {
            return NEVER;
        }
        now.saturating_add(samples * ticks_per_sample(self.tmr))
    }
}

/// Master cycles per sample for a timer value (`sound-registers.md`, "Model").
fn ticks_per_sample(tmr: u16) -> u64 {
    TICKS_PER_TMR_STEP * (TMR_PERIOD - u64::from(tmr))
}

/// One capture unit's registers and the end time of its capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    cnt: u8,
    dad: u32,
    len: u16,
    end: u64,
}

impl Capture {
    /// SNDCAPxCNT as stored, bit 7 as started.
    pub const fn cnt(&self) -> u8 {
        self.cnt
    }

    /// SNDCAPxCNT as a read at master time `now` sees it.
    pub const fn cnt_at(&self, now: u64) -> u8 {
        if self.busy(now) {
            self.cnt
        } else {
            self.cnt & !CAP_START
        }
    }

    /// SNDCAPxDAD as stored.
    pub const fn dad(&self) -> u32 {
        self.dad
    }

    /// SNDCAPxLEN as written (write-only on the bus).
    pub const fn length(&self) -> u16 {
        self.len
    }

    /// The master time the capture ends: `u64::MAX` for a loop, 0 while stopped.
    pub const fn end(&self) -> u64 {
        self.end
    }

    /// Whether the capture is busy at master time `now`.
    pub const fn busy(&self, now: u64) -> bool {
        self.cnt & CAP_START != 0 && now < self.end
    }

    /// The end time of a capture that starts at `now`, at the sample rate `tmr` of the shared
    /// channel.
    // TODO(hw-unverified): the capture starts at once and reads the rate when it starts
    // (`sound-registers.md`, "Unverified").
    fn end_from(&self, now: u64, tmr: u16) -> u64 {
        if self.cnt & CAP_ONE_SHOT == 0 {
            return NEVER;
        }
        // A length of 0 acts as 1.
        let words = u64::from(self.len.max(1));
        let per_word = if self.cnt & CAP_PCM8 != 0 {
            CAP_SAMPLES_PCM8
        } else {
            CAP_SAMPLES_PCM16
        };
        now.saturating_add(words * per_word * ticks_per_sample(tmr))
    }
}

/// The ARM7's sound registers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spu {
    channels: [Channel; CHANNELS],
    cnt: u16,
    bias: u16,
    captures: [Capture; CAPTURES],
}

/// `old` with byte `index` replaced by `byte`.
const fn with_byte(old: u32, index: u32, byte: u8) -> u32 {
    let shift = 8 * index;
    (old & !(0xFF << shift)) | ((byte as u32) << shift)
}

impl Spu {
    /// The registers at power-on: all zero.
    // TODO(hw-unverified): the values after direct boot are not known; all zero here
    // (`sound-registers.md`, "Unverified").
    pub fn new() -> Self {
        Self::default()
    }

    /// Channel `index` (0-15).
    pub fn channel(&self, index: usize) -> &Channel {
        &self.channels[index % CHANNELS]
    }

    /// Capture unit `index` (0 or 1).
    pub fn capture(&self, index: usize) -> &Capture {
        &self.captures[index % CAPTURES]
    }

    /// SOUNDCNT as stored.
    pub const fn soundcnt(&self) -> u16 {
        self.cnt
    }

    /// SOUNDBIAS as stored.
    pub const fn soundbias(&self) -> u16 {
        self.bias
    }

    /// The byte of the window at `offset`, as a read at master time `now` sees it. The
    /// write-only registers and the unused addresses read 0.
    // TODO(hw-unverified): the write-only registers read 0 and the unused addresses of the window
    // read 0 (`sound-registers.md`, "Unverified").
    fn byte_at(&self, offset: u32, now: u64) -> u8 {
        let byte = |value: u32, base: u32| (value >> (8 * (offset - base))) as u8;
        match offset {
            ..SOUNDCNT_OFFSET if offset % CHANNEL_BYTES < SAD_OFFSET => {
                let channel = &self.channels[(offset >> CHANNEL_STRIDE_SHIFT) as usize];
                byte(channel.cnt_at(now), offset & !(CHANNEL_BYTES - 1))
            }
            SOUNDCNT_OFFSET..=0x101 => byte(u32::from(self.cnt), SOUNDCNT_OFFSET),
            SOUNDBIAS_OFFSET..=0x105 => byte(u32::from(self.bias), SOUNDBIAS_OFFSET),
            SNDCAP0CNT_OFFSET => self.captures[0].cnt_at(now),
            SNDCAP1CNT_OFFSET => self.captures[1].cnt_at(now),
            SNDCAP0DAD_OFFSET..=0x113 => byte(self.captures[0].dad, SNDCAP0DAD_OFFSET),
            SNDCAP1DAD_OFFSET..=0x11B => byte(self.captures[1].dad, SNDCAP1DAD_OFFSET),
            _ => 0,
        }
    }

    /// One byte written to the window at `offset` at master time `now`.
    fn write_byte(&mut self, offset: u32, byte: u8, now: u64) {
        match offset {
            ..SOUNDCNT_OFFSET => self.write_channel_byte(offset, byte, now),
            SOUNDCNT_OFFSET..=0x101 => {
                let merged = with_byte(u32::from(self.cnt), offset - SOUNDCNT_OFFSET, byte);
                self.cnt = (merged as u16) & SOUNDCNT_MASK;
            }
            SOUNDBIAS_OFFSET..=0x105 => {
                let merged = with_byte(u32::from(self.bias), offset - SOUNDBIAS_OFFSET, byte);
                self.bias = (merged as u16) & SOUNDBIAS_MASK;
            }
            SNDCAP0CNT_OFFSET => self.write_capture_cnt(0, byte, now),
            SNDCAP1CNT_OFFSET => self.write_capture_cnt(1, byte, now),
            SNDCAP0DAD_OFFSET..=0x113 => {
                let dad = &mut self.captures[0].dad;
                *dad = with_byte(*dad, offset - SNDCAP0DAD_OFFSET, byte) & CAP_DAD_MASK;
            }
            SNDCAP0LEN_OFFSET..=0x115 => {
                let len = &mut self.captures[0].len;
                *len = with_byte(u32::from(*len), offset - SNDCAP0LEN_OFFSET, byte) as u16;
            }
            SNDCAP1DAD_OFFSET..=0x11B => {
                let dad = &mut self.captures[1].dad;
                *dad = with_byte(*dad, offset - SNDCAP1DAD_OFFSET, byte) & CAP_DAD_MASK;
            }
            SNDCAP1LEN_OFFSET..=0x11D => {
                let len = &mut self.captures[1].len;
                *len = with_byte(u32::from(*len), offset - SNDCAP1LEN_OFFSET, byte) as u16;
            }
            _ => {}
        }
    }

    fn write_channel_byte(&mut self, offset: u32, byte: u8, now: u64) {
        let channel = &mut self.channels[(offset >> CHANNEL_STRIDE_SHIFT) as usize];
        let reg = offset % CHANNEL_BYTES;
        match reg {
            CNT_OFFSET..SAD_OFFSET => {
                // A sound that ended on its own is stopped before the write sees it, so a program
                // that starts the channel again starts it from 0 -> 1.
                let was_busy = channel.busy(now);
                channel.cnt = with_byte(channel.cnt_at(now), reg - CNT_OFFSET, byte) & CNT_MASK;
                let started = channel.cnt & CNT_START != 0;
                // Writing 1 while busy does not restart the sound: the end stays.
                channel.end = match (was_busy, started) {
                    (false, true) => channel.end_from(now),
                    (true, true) => channel.end,
                    (_, false) => 0,
                };
            }
            SAD_OFFSET..TMR_OFFSET => {
                channel.sad = with_byte(channel.sad, reg - SAD_OFFSET, byte) & SAD_MASK;
            }
            TMR_OFFSET..PNT_OFFSET => {
                channel.tmr = with_byte(u32::from(channel.tmr), reg - TMR_OFFSET, byte) as u16;
            }
            PNT_OFFSET..LEN_OFFSET => {
                channel.pnt = with_byte(u32::from(channel.pnt), reg - PNT_OFFSET, byte) as u16;
            }
            _ => channel.len = with_byte(channel.len, reg - LEN_OFFSET, byte) & LEN_MASK,
        }
    }

    fn write_capture_cnt(&mut self, index: usize, byte: u8, now: u64) {
        let tmr = self.channels[CAPTURE_TIMER_CHANNEL[index]].tmr;
        let capture = &mut self.captures[index];
        let was_busy = capture.busy(now);
        capture.cnt = byte & CAP_MASK;
        let started = capture.cnt & CAP_START != 0;
        capture.end = match (was_busy, started) {
            (false, true) => capture.end_from(now, tmr),
            (true, true) => capture.end,
            (_, false) => 0,
        };
    }
}

impl Checked for Spu {
    /// Every register holds only the bits it keeps, and a stopped unit has no end time. A started
    /// unit may be past its end: the bit is folded away lazily.
    fn well_formed(&self, _now: u64) -> bool {
        let channels = self.channels.iter().all(|c| {
            c.cnt & !CNT_MASK == 0
                && c.sad & !SAD_MASK == 0
                && c.len & !LEN_MASK == 0
                && (c.cnt & CNT_START != 0 || c.end == 0)
        });
        let captures = self.captures.iter().all(|c| {
            c.cnt & !CAP_MASK == 0
                && c.dad & !CAP_DAD_MASK == 0
                && (c.cnt & CAP_START != 0 || c.end == 0)
        });
        channels && captures && self.cnt & !SOUNDCNT_MASK == 0 && self.bias & !SOUNDBIAS_MASK == 0
    }
}

impl Device<false> for Spu {
    fn peek(&self, offset: u32, width: Width, cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.byte_at(at, cx.now))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte(at, byte, cx.now);
            Effect::default()
        })
    }
}

#[cfg(test)]
mod tests;
