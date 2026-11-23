// SPDX-License-Identifier: Apache-2.0
//! The sound registers of the ARM7: the 16 channels, SOUNDCNT, SOUNDBIAS and the two capture
//! units (`docs/hw/sound-registers.md`).
//!
//! A channel's playback is a pure function of the master time it started, the registers latched at
//! the start and the time of the query (`docs/hw/spu.md`, "Position"): how many samples have
//! elapsed, and so which sample of the data is playing. The busy bit uses the same elapsed count (an
//! end time computed once at the start), so the bit and the output cannot disagree, and a savestate
//! taken mid-playback plays on exactly as an uninterrupted run. A capture unit is the same: its busy bit
//! and the slots it has written are one pure function of its start time and what it latched
//! (`docs/hw/sound-capture.md`, "Model").
//!
//! Only the ARM7 sees these registers, so [`Spu`] is a plain type implementing `Device<false>`
//! (`AGENTS.md` §4, "Per-CPU devices"): the "seen by one CPU" class. It registers directly with
//! its own port, with no `io/` entrance, as `gpu2d` does (`AGENTS.md` §2, "Placement").

mod adpcm;
mod mixer;
mod psg;

use crate::cpu::Bus;
use crate::io::device::{Checked, Cx, Device, Effect, Width, read_bytes, write_bytes};
use serde::{Deserialize, Serialize};

pub use adpcm::Adpcm;
pub use psg::Noise;

/// SOUND0CNT, the first register of the window, at `0400_0400h` (`sound-registers.md`, "The
/// channels").
pub const SOUND_REGS_ADDR: u32 = 0x0400_0400;
/// SOUNDBIAS, at `0400_0504h`.
pub const SOUNDBIAS_ADDR: u32 = SOUND_REGS_ADDR + SOUNDBIAS_OFFSET;
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
/// PSG on channels 8-13, noise on 14-15 (`spu.md`, "PSG", "Noise").
const FORMAT_PSG: u32 = 3;
const PSG_CHANNELS: std::ops::RangeInclusive<u8> = 8..=13;
const NOISE_CHANNELS: std::ops::RangeInclusive<u8> = 14..=15;
/// SOUNDCNT bits 0-6: the master volume; 8-9 and 10-11: where the left and the right output come
/// from; 12 and 13: channel 1 and channel 3 are not sent to the mixer; 15: master enable
/// (`spu.md`, "The mixer", "The gates").
const SOUNDCNT_VOLUME_MASK: u16 = 0x7F;
const SOUNDCNT_LEFT_SHIFT: u16 = 8;
const SOUNDCNT_RIGHT_SHIFT: u16 = 10;
const SOUNDCNT_SOURCE_MASK: u16 = 0b11;
const SOUNDCNT_CH1_NOT_MIXED: u16 = 1 << 12;
const SOUNDCNT_CH3_NOT_MIXED: u16 = 1 << 13;
const SOUNDCNT_ENABLE: u16 = 1 << 15;
/// The output sources of bits 8-11: the mixer, channel 1, channel 3, or both of them.
const SOURCE_CH1: u16 = 1;
const SOURCE_CH3: u16 = 2;
const SOURCE_BOTH: u16 = 3;
/// The channels SOUNDCNT can route around the mixer.
const CHANNEL_1: usize = 1;
const CHANNEL_3: usize = 3;
/// SOUNDxCNT bit 15: keep the last sample after a one-shot ends (`spu.md`, "Position").
const CNT_HOLD: u32 = 1 << 15;
/// SOUNDxCNT bits 0-6, 8-9 and 16-22: volume, divider, panning.
const CNT_VOLUME_MASK: u32 = 0x7F;
const CNT_DIVIDER_SHIFT: u32 = 8;
const CNT_DIVIDER_MASK: u32 = 0b11;
const CNT_PANNING_SHIFT: u32 = 16;
const CNT_PANNING_MASK: u32 = 0x7F;
/// SOUNDxCNT bits 24-26: the PSG wave duty.
const CNT_DUTY_SHIFT: u32 = 24;
const CNT_DUTY_MASK: u32 = 0b111;
/// Samples in a word: four PCM8 samples, two PCM16 samples (`spu.md`, "Position").
const PCM8_PER_WORD: u64 = 4;
const PCM16_PER_WORD: u64 = 2;
/// IMA-ADPCM: eight 4-bit samples in a word.
const ADPCM_PER_WORD: u64 = 8;
/// Master cycles between two output samples: 1024 ARM7 cycles, two master cycles each
/// (`spu.md`, "Output samples").
// TODO(hw-unverified): GBATEK's 32.768 kHz is not exactly 1024 ARM7 cycles (`spu.md`,
// "Unverified").
pub const OUTPUT_SAMPLE_CYCLES: u64 = 2048;
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
/// SNDCAPxCNT bit 0: add channel 1 or 3 to channel 0 or 2 (needs bit 7 as well).
const CAP_ADD: u8 = 1;
/// SNDCAPxCNT bit 1: 0 = the mixer, 1 = channel 0 or 2.
const CAP_SOURCE_CHANNEL: u8 = 1 << 1;
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

/// Where a channel is in its data (`Channel::position`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Position {
    /// Nothing plays.
    Silent,
    /// Data sample `n` plays (counted after the header word for ADPCM).
    At(u64),
    /// A one-shot ended with Hold set: the last data sample, `n`, stays.
    Last(u64),
}

/// The generators a channel advances with the sample clock: the IMA-ADPCM decoder and the noise
/// register. A channel uses at most one, by its format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoders {
    adpcm: Adpcm,
    noise: Noise,
}

/// What a channel latches when it starts: the registers that decide where each sample comes from.
/// A write to them while the sound plays does not move it (`spu.md`, "Model"). Volume, divider,
/// panning, hold and duty are not here: they are read live from SOUNDxCNT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Latched {
    sad: u32,
    pnt: u32,
    len: u32,
    format: u32,
    /// The channel's number, which decides what format 3 is.
    channel: u8,
    one_shot: bool,
    /// Master cycles per sample.
    period: u64,
}

/// One sound channel's registers, the master time it started and what it latched then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    cnt: u32,
    sad: u32,
    tmr: u16,
    pnt: u16,
    len: u32,
    /// Master time of the write that started the sound; 0 while stopped.
    start: u64,
    latched: Latched,
    /// The IMA-ADPCM decoder and the noise register, advanced as the sample clock runs; each is
    /// unused by the other formats.
    decoders: Decoders,
    /// Master time the busy bit clears, [`NEVER`] if it does not, 0 while stopped. Computed once at
    /// the start from the latched registers, so the busy bit and the output share one position.
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

    /// The master time the sound started, 0 while stopped.
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// SOUNDxCNT bits 0-6: the volume multiplier as written.
    pub const fn volume(&self) -> u32 {
        self.cnt & CNT_VOLUME_MASK
    }

    /// SOUNDxCNT bits 8-9: the volume divider selector as written.
    pub const fn divider(&self) -> u32 {
        (self.cnt >> CNT_DIVIDER_SHIFT) & CNT_DIVIDER_MASK
    }

    /// SOUNDxCNT bits 16-22: the panning as written.
    pub const fn panning(&self) -> u32 {
        (self.cnt >> CNT_PANNING_SHIFT) & CNT_PANNING_MASK
    }

    /// SOUNDxCNT bit 15: the hold flag.
    pub const fn hold(&self) -> bool {
        self.cnt & CNT_HOLD != 0
    }

    /// SOUNDxCNT bits 24-26: the PSG wave duty as written.
    pub const fn duty(&self) -> u32 {
        (self.cnt >> CNT_DUTY_SHIFT) & CNT_DUTY_MASK
    }

    /// SOUNDxCNT bits 27-28: the repeat mode as written.
    pub const fn repeat_mode(&self) -> u32 {
        (self.cnt >> CNT_REPEAT_SHIFT) & CNT_REPEAT_MASK
    }

    /// SOUNDxCNT bits 29-30: the format as written.
    pub const fn format(&self) -> u32 {
        (self.cnt >> CNT_FORMAT_SHIFT) & CNT_FORMAT_MASK
    }

    /// Samples elapsed since the start at master time `now`: the position of the sample clock, or
    /// `None` while the channel is stopped or `now` is before the start (the ARM7 can write at a
    /// time past the master clock, and a sample event at the master clock is then earlier than the
    /// write; `spu.md`, "Position").
    // TODO(hw-unverified): the sound starts at the write, with none of GBATEK's start delay (1, 3 or
    // 11 samples) and the busy bit clears at the end of the last sample, not at its begin
    // (`spu.md`, "Unverified").
    pub const fn elapsed(&self, now: u64) -> Option<u64> {
        if self.cnt & CNT_START == 0 || self.latched.period == 0 || now < self.start {
            return None;
        }
        Some((now - self.start) / self.latched.period)
    }

    /// Whether the latched channel is one that makes format 3 sound: PSG on 8-13, noise on 14-15
    /// (`spu.md`, "Format 3 on channels 0-7").
    // TODO(hw-unverified): format 3 on channels 0-7 is silent and busy (`spu.md`, "Unverified").
    fn generates(&self) -> bool {
        PSG_CHANNELS.contains(&self.latched.channel)
            || NOISE_CHANNELS.contains(&self.latched.channel)
    }

    /// Where the data position is after `elapsed` samples of the sample clock (`spu.md`,
    /// "Position"). The sizes come from the format; the loop part is what follows the PNT part.
    fn position(&self, elapsed: u64) -> Position {
        let latched = &self.latched;
        // PSG and noise have no length: the repeat mode, PNT and LEN are ignored (`spu.md`, "PSG").
        if latched.format == FORMAT_PSG {
            return if self.generates() {
                Position::At(elapsed)
            } else {
                Position::Silent
            };
        }
        let words = u64::from(latched.pnt) + u64::from(latched.len);
        // Below the minimum length nothing plays (`spu.md`, "Position").
        // TODO(hw-unverified): GBATEK gives this for a one-shot; a loop below it is silent here too
        // (`spu.md`, "Unverified").
        if words < u64::from(MIN_WORDS) {
            return Position::Silent;
        }
        let (total, pnt) = match latched.format {
            FORMAT_PCM8 => (
                PCM8_PER_WORD * words,
                PCM8_PER_WORD * u64::from(latched.pnt),
            ),
            FORMAT_PCM16 => (
                PCM16_PER_WORD * words,
                PCM16_PER_WORD * u64::from(latched.pnt),
            ),
            // The first word is the header, so the sound is a word shorter and the loop start,
            // counted from the header, a word earlier (`spu.md`, "Position").
            FORMAT_ADPCM => (
                ADPCM_PER_WORD * (words - 1),
                ADPCM_PER_WORD * u64::from(latched.pnt).saturating_sub(1),
            ),
            _ => return Position::Silent,
        };
        let len = total - pnt;
        if latched.one_shot {
            match (elapsed < total, self.hold()) {
                (true, _) => Position::At(elapsed),
                // TODO(hw-unverified): the Hold flag during a restart is not modeled
                // (`spu.md`, "Unverified").
                (false, true) => Position::Last(total - 1),
                (false, false) => Position::Silent,
            }
        } else if elapsed < pnt {
            Position::At(elapsed)
        } else if len == 0 {
            // An empty loop: the PNT part, then silence (`spu.md`, "Model").
            // TODO(hw-unverified): what GBATEK leaves open for LEN = 0 (`spu.md`, "Unverified").
            Position::Silent
        } else {
            Position::At(pnt + (elapsed - pnt) % len)
        }
    }

    /// The channel's sample at master time `now` as a PCM16 value (`-8000h`-`7FFFh`), or 0 while
    /// it is silent. `read` is the ARM7's side-effect-free byte read of the source data.
    ///
    /// PCM is a pure function of the start time, the latched registers and `now`; nothing is
    /// stepped. IMA-ADPCM and noise are the value their generator holds, so [`Channel::advanced`]
    /// must have caught up to `now` first. After a one-shot ends the sample is 0, or the last one with Hold set
    /// (`spu.md`, "Position").
    pub fn sample(&self, now: u64, read: impl Fn(u32) -> u8) -> i32 {
        let Some(elapsed) = self.elapsed(now) else {
            return 0;
        };
        let (Position::At(index) | Position::Last(index)) = self.position(elapsed) else {
            return 0;
        };
        if self.latched.format == FORMAT_PSG {
            return if PSG_CHANNELS.contains(&self.latched.channel) {
                psg::duty_level(elapsed, self.duty())
            } else {
                self.decoders.noise.level()
            };
        }
        let sad = self.latched.sad;
        // TODO(hw-unverified): a source address the ARM7 bus does not map reads what the bus's read
        // gives there (`spu.md`, "Unverified").
        match self.latched.format {
            FORMAT_PCM8 => i32::from(read(sad.wrapping_add(index as u32)) as i8) << 8,
            FORMAT_PCM16 => {
                let at = sad.wrapping_add((index * 2) as u32);
                i32::from(i16::from_le_bytes([read(at), read(at.wrapping_add(1))]))
            }
            FORMAT_ADPCM => self.decoders.adpcm.pcm(),
            _ => 0,
        }
    }

    /// The generators after catching up with the sample clock at master time `now`: the IMA-ADPCM
    /// decoder takes one decode per elapsed sample, the header read at the first (`spu.md`,
    /// "IMA-ADPCM"), and the noise register one step per elapsed sample (`spu.md`, "Noise"). A
    /// channel of another format, or one not playing, returns its state as it is. Pure: the caller
    /// stores the result ([`Spu::set_decoders`]), so the read of memory and the write of the state
    /// do not overlap.
    ///
    /// The cost is the samples elapsed since the last call, at most a few hundred between two
    /// output samples; no allocation.
    // TODO(hw-unverified): the ADPCM header is read when the first output sample after the start
    // catches up, not at the start, and what an ADPCM loop whose start is the header does
    // (`spu.md`, "Unverified").
    pub fn advanced(&self, now: u64, read: impl Fn(u32) -> u8) -> Decoders {
        let mut decoders = self.decoders;
        let Some(elapsed) = self.elapsed(now) else {
            return decoders;
        };
        match self.latched.format {
            FORMAT_ADPCM => decoders.adpcm = self.advance_adpcm(decoders.adpcm, elapsed, read),
            FORMAT_PSG if NOISE_CHANNELS.contains(&self.latched.channel) => {
                while decoders.noise.ticks <= elapsed {
                    decoders.noise.step();
                }
            }
            _ => {}
        }
        decoders
    }

    fn advance_adpcm(&self, mut state: Adpcm, elapsed: u64, read: impl Fn(u32) -> u8) -> Adpcm {
        let sad = self.latched.sad;
        let loop_start = ADPCM_PER_WORD * u64::from(self.latched.pnt).saturating_sub(1);
        while state.ticks <= elapsed {
            let tick = state.ticks;
            if tick == 0 {
                let word =
                    u32::from_le_bytes(std::array::from_fn(|i| read(sad.wrapping_add(i as u32))));
                state = Adpcm::from_header(word);
            }
            if let Position::At(index) = self.position(tick) {
                // The data follows the header word; two samples to a byte.
                let at = sad.wrapping_add(4).wrapping_add((index / 2) as u32);
                state.decode(index, loop_start, read(at));
            }
            state.ticks = tick + 1;
        }
        state
    }

    /// The IMA-ADPCM decoder state (`spu.md`, "IMA-ADPCM"), for the debugger.
    pub const fn adpcm(&self) -> &Adpcm {
        &self.decoders.adpcm
    }

    /// The noise register (`spu.md`, "Noise"), for the debugger.
    pub const fn noise(&self) -> &Noise {
        &self.decoders.noise
    }

    /// The end time of a sound that starts at `now` with the registers as they are.
    // TODO(hw-unverified): no start delay before the first sample, and TMR, PNT and LEN written
    // while busy do not move the end (`sound-registers.md`, "Unverified"; `spu.md`, "Unverified").
    // TODO(hw-unverified): the manual and prohibited repeat modes, and PSG and noise on any
    // channel, never end (`sound-registers.md`, "Unverified").
    fn end_from(&self, now: u64) -> u64 {
        let words = u64::from(self.latched.pnt + self.latched.len);
        let one_shot = self.latched.one_shot;
        let samples = match (self.cnt >> CNT_FORMAT_SHIFT) & CNT_FORMAT_MASK {
            FORMAT_PCM8 => 4 * words,
            FORMAT_PCM16 => 2 * words,
            FORMAT_ADPCM => 8 * words.saturating_sub(1),
            _ => return NEVER,
        };
        if !one_shot || words < u64::from(MIN_WORDS) {
            return NEVER;
        }
        now.saturating_add(samples * self.latched.period)
    }

    /// Latches the registers a started sound reads from, at master time `now`.
    // TODO(hw-unverified): SAD, PNT, LEN, TMR, the format and the repeat mode are read at the start
    // and a write while the sound plays does not move it (`spu.md`, "Unverified").
    fn latch(&mut self, now: u64, channel: u8) {
        self.start = now;
        self.decoders = Decoders::default();
        self.latched = Latched {
            sad: self.sad,
            pnt: u32::from(self.pnt),
            len: self.len,
            format: self.format(),
            channel,
            one_shot: self.repeat_mode() == REPEAT_ONE_SHOT,
            period: ticks_per_sample(self.tmr),
        };
    }
}

/// Master cycles per sample for a timer value (`sound-registers.md`, "Model").
fn ticks_per_sample(tmr: u16) -> u64 {
    TICKS_PER_TMR_STEP * (TMR_PERIOD - u64::from(tmr))
}

/// What a capture latches when it starts (`sound-capture.md`, "Model"): where it writes, how many
/// samples the buffer holds, the format and the repeat mode, and the sample period from the shared
/// TMR. Bit 0 and bit 1 of SNDCAPxCNT are read live, not here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct CaptureLatched {
    dad: u32,
    /// Samples the buffer holds: `LEN` words (0 counts as 1) of two PCM16 or four PCM8 samples.
    samples: u64,
    pcm8: bool,
    one_shot: bool,
    /// Master cycles per sample; 0 while stopped.
    period: u64,
}

/// One capture unit's registers, the master time it started and what it latched then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    cnt: u8,
    dad: u32,
    len: u16,
    /// Master time of the write that started the capture; 0 while stopped.
    start: u64,
    latched: CaptureLatched,
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
        if self.cnt & CAP_START == 0 {
            0
        } else if !self.latched.one_shot {
            NEVER
        } else {
            self.start
                .saturating_add(self.latched.samples * self.latched.period)
        }
    }

    /// Whether the capture is busy at master time `now`. A one-shot is busy until every slot of
    /// its buffer has been written, which is the moment [`Self::written`] reaches the buffer size.
    pub const fn busy(&self, now: u64) -> bool {
        self.cnt & CAP_START != 0 && now < self.end()
    }

    /// The slots written by master time `now`: the samples the shared clock has completed since
    /// the start, at most the whole buffer for a one-shot. Zero while stopped or before the start
    /// (the ARM7 can write at a time past the master clock). This is the one formula the busy bit
    /// and the writes share (`sound-capture.md`, "Model").
    // TODO(hw-unverified): a capture starts at once, with none of the start delay a channel has
    // (`sound-capture.md`, "Unverified").
    pub const fn written(&self, now: u64) -> u64 {
        if self.cnt & CAP_START == 0 || self.latched.period == 0 || now < self.start {
            return 0;
        }
        let done = (now - self.start) / self.latched.period;
        if self.latched.one_shot && done > self.latched.samples {
            self.latched.samples
        } else {
            done
        }
    }

    /// The slot the next sample goes to at master time `now`, inside the buffer: 0 when stopped.
    pub const fn position(&self, now: u64) -> u64 {
        if self.latched.samples == 0 {
            return 0;
        }
        self.written(now) % self.latched.samples
    }

    /// The address the next sample is written to at master time `now`: `DAD` when stopped.
    // The debugger's sound panel (#47) will show the position and this address.
    pub const fn next_address(&self, now: u64) -> u32 {
        if self.latched.samples == 0 {
            return self.dad;
        }
        self.latched.address(self.position(now))
    }

    /// Latches the registers a started capture reads from, at master time `now`, with the TMR of
    /// the channel it shares a timer with.
    // TODO(hw-unverified): TMR, LEN, DAD and the format are read at the start and a write while the
    // capture runs does not move it (`sound-capture.md`, "Unverified").
    fn latch(&mut self, now: u64, tmr: u16) {
        let per_word = if self.cnt & CAP_PCM8 != 0 {
            CAP_SAMPLES_PCM8
        } else {
            CAP_SAMPLES_PCM16
        };
        self.start = now;
        self.latched = CaptureLatched {
            dad: self.dad,
            // A length of 0 acts as 1.
            samples: u64::from(self.len.max(1)) * per_word,
            pcm8: self.cnt & CAP_PCM8 != 0,
            one_shot: self.cnt & CAP_ONE_SHOT != 0,
            period: ticks_per_sample(tmr),
        };
    }
}

impl Capture {
    /// A stopped capture holds nothing latched; a started one holds a buffer of 1 to `FFFFh` words
    /// and a period of at least one TMR step, so no divisor or product in [`Self::written`],
    /// [`Self::end`] or [`CaptureWrites::address`] is zero or can overflow.
    fn well_formed(&self) -> bool {
        if self.cnt & CAP_START == 0 {
            return self.start == 0 && self.latched == CaptureLatched::default();
        }
        let l = &self.latched;
        (1..=u64::from(u16::MAX) * CAP_SAMPLES_PCM8).contains(&l.samples)
            && (TICKS_PER_TMR_STEP..=TICKS_PER_TMR_STEP * TMR_PERIOD).contains(&l.period)
    }
}

impl CaptureLatched {
    /// The address of slot `index` of the buffer: a halfword apart for PCM16, a byte for PCM8.
    const fn address(&self, index: u64) -> u32 {
        let size = if self.pcm8 { 1 } else { 2 };
        // `index` is below the buffer size (at most 4 * FFFFh), so the narrowing is exact.
        self.dad.wrapping_add((index as u32).wrapping_mul(size))
    }
}

/// What one capture writes at an output sample: `count` slots from `first` with the same `value`
/// (`sound-capture.md`, "Model"). Plain data computed from `&Spu`, run afterwards against the
/// ARM7's view of memory, so the device is never borrowed while the bus is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaptureWrites {
    latched: CaptureLatched,
    first: u64,
    count: u64,
    /// The PCM16 or PCM8 sample, sign-extended.
    value: i32,
}

impl CaptureWrites {
    /// The address of slot `index`; the buffer wraps to `DAD`.
    fn address(&self, index: u64) -> u32 {
        self.latched.address(index % self.latched.samples)
    }

    /// Writes the slots through `bus`, a sample at a time as a DMA unit would
    /// (`sound-capture.md`, "Model").
    // TODO(hw-unverified): the FIFO between the capture and memory is not modeled; the samples are
    // written one at a time and at once (`sound-capture.md`, "Unverified").
    pub(crate) fn run<B: Bus>(&self, bus: &mut B) {
        for slot in self.first..self.first + self.count {
            let address = self.address(slot);
            if self.latched.pcm8 {
                bus.write8(address, self.value as u8);
            } else {
                bus.write16(address, self.value as u16);
            }
        }
    }
}

/// Whether capture `x`'s addition is on for the mixer, and whether channel `b` still counts in it
/// (`Spu::adding`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Adding {
    on: bool,
    b_mixed: bool,
}

impl Adding {
    /// No addition.
    pub(crate) const OFF: Self = Self {
        on: false,
        b_mixed: false,
    };
}

/// The 16 channels panned and summed: each channel's `(left, right)` and the two mixer sums
/// before the master stage, in 1/256 units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mix {
    routed: [(i64, i64); CHANNELS],
    sum: [i64; 2],
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

    /// Stores the generator states [`Channel::advanced`] returned, one per channel.
    pub(crate) fn set_decoders(&mut self, states: [Decoders; CHANNELS]) {
        self.channels
            .iter_mut()
            .zip(states)
            .for_each(|(channel, state)| channel.decoders = state);
    }

    /// Channel `index` (0-15).
    pub fn channel(&self, index: usize) -> &Channel {
        &self.channels[index % CHANNELS]
    }

    /// Capture unit `index` (0 or 1).
    pub fn capture(&self, index: usize) -> &Capture {
        &self.captures[index % CAPTURES]
    }

    /// Whether capture `x` adds channel `b` into channel `a` for the mixer at master time `time`,
    /// and whether `b` still counts (`sound-capture.md`, "The addition mode"): addition needs bit 0
    /// and bit 7 of SNDCAPxCNT, and channel `b` is muted when channel `a` is not busy or SOUNDCNT
    /// bit 12 or 13 sends it away from the mixer.
    // TODO(hw-unverified): "channel a disabled" is "not busy" here (`sound-capture.md`,
    // "Unverified").
    // TODO(hw-unverified): the addition for the mixer stops when the busy bit clears, while the final
    // slots of that run are still written with the capture-side addition (`sound-capture.md`,
    // "Unverified").
    pub(crate) fn adding(&self, time: u64) -> [Adding; CAPTURES] {
        std::array::from_fn(|x| {
            let on = self.captures[x].cnt & CAP_ADD != 0 && self.captures[x].busy(time);
            let (a, not_mixed) = (2 * x, [SOUNDCNT_CH1_NOT_MIXED, SOUNDCNT_CH3_NOT_MIXED][x]);
            Adding {
                on,
                b_mixed: self.channels[a].busy(time) && self.cnt & not_mixed == 0,
            }
        })
    }

    /// The 16 channels panned and summed into the two mixers, before the master stage
    /// (`spu.md`, "The mixer"). Channel 1 and 3 drop out of the sum when SOUNDCNT bit 12 or 13 says
    /// so. A unit whose addition is on sends channel `b` through channel `a`: `a`'s panning applies
    /// to the sum of the two, and `b` does not reach the mixer on its own. Pure and
    /// allocation-free: this runs once per output sample.
    // TODO(hw-unverified): what channel b's own path does in addition mode: here it reaches the
    // mixer only through channel a, and the output selection of SOUNDCNT bits 8-11 still sees its
    // own panned value (`sound-capture.md`, "Unverified").
    pub(crate) fn mix(&self, samples: &[i32; CHANNELS], adding: [Adding; CAPTURES]) -> Mix {
        let mut mix = Mix::default();
        let mut with_volume = [0i64; CHANNELS];
        let mut contribution = [(0i64, 0i64); CHANNELS];
        self.channels
            .iter()
            .zip(samples)
            .enumerate()
            .for_each(|(x, (channel, &sample))| {
                with_volume[x] = mixer::with_volume(sample, channel.divider(), channel.volume());
                let routed = mixer::pan(with_volume[x], channel.panning());
                mix.routed[x] = routed;
                let not_mixed = match x {
                    CHANNEL_1 => self.cnt & SOUNDCNT_CH1_NOT_MIXED != 0,
                    CHANNEL_3 => self.cnt & SOUNDCNT_CH3_NOT_MIXED != 0,
                    _ => false,
                };
                contribution[x] = if not_mixed { (0, 0) } else { routed };
            });
        adding
            .iter()
            .enumerate()
            .filter(|(_, unit)| unit.on)
            .for_each(|(x, unit)| {
                let (a, b) = (2 * x, 2 * x + 1);
                let added = with_volume[a] + if unit.b_mixed { with_volume[b] } else { 0 };
                contribution[a] = mixer::pan(added, self.channels[a].panning());
                contribution[b] = (0, 0);
            });
        contribution.iter().for_each(|&(left, right)| {
            mix.sum[0] += left;
            mix.sum[1] += right;
        });
        mix
    }

    /// The stereo output sample for the channels' PCM16 values `samples` (`spu.md`, "The mixer"),
    /// with no capture adding: `[left, right]` as `i16`, the medium level at zero. `audible` is the digital gates outside
    /// the unit, the speakers (POWCNT2) and the amplifier, both on; with it off the output is 0.
    /// With the master enable off the mixer is not used and the level is the bias alone.
    ///
    /// Pure and allocation-free: this runs once per output sample.
    // TODO(hw-unverified): a silenced output is 0, and the master-enable-off output is the bias
    // alone (`spu.md`, "Unverified").
    pub fn output(&self, samples: &[i32; CHANNELS], audible: bool) -> [i16; 2] {
        self.output_mixed(&self.mix(samples, [Adding::OFF; CAPTURES]), audible)
    }

    /// [`Self::output`] for a mix already made, so the capture units and the speakers read one.
    pub(crate) fn output_mixed(&self, mix: &Mix, audible: bool) -> [i16; 2] {
        if !audible {
            return [0, 0];
        }
        let bias = self.bias;
        if self.cnt & SOUNDCNT_ENABLE == 0 {
            let silent = mixer::to_i16(mixer::level(0, bias));
            return [silent, silent];
        }
        let routed = &mix.routed;
        let pick = |shift: u16, side: fn((i64, i64)) -> i64, mixed: i64| {
            let (ch1, ch3) = (side(routed[CHANNEL_1]), side(routed[CHANNEL_3]));
            match (self.cnt >> shift) & SOUNDCNT_SOURCE_MASK {
                SOURCE_CH1 => ch1,
                SOURCE_CH3 => ch3,
                SOURCE_BOTH => ch1 + ch3,
                _ => mixed,
            }
        };
        let master = u32::from(self.cnt & SOUNDCNT_VOLUME_MASK);
        let left = pick(SOUNDCNT_LEFT_SHIFT, |pair| pair.0, mix.sum[0]);
        let right = pick(SOUNDCNT_RIGHT_SHIFT, |pair| pair.1, mix.sum[1]);
        [left, right].map(|signal| mixer::to_i16(mixer::level(mixer::master(signal, master), bias)))
    }

    /// The value capture `x` takes at an output sample, in 1/256 units (`sound-capture.md`,
    /// "Sources" and "The two bugs"): the clipped mixer sum, or channel `a` before its panning.
    fn capture_source(&self, x: usize, mix: &Mix, samples: &[i32; CHANNELS]) -> i64 {
        // TODO(hw-unverified): the mixer source ignores SOUNDCNT bit 15 and the speaker gates, and
        // the clip keeps the fraction (`sound-capture.md`, "Unverified").
        if self.captures[x].cnt & CAP_SOURCE_CHANNEL == 0 {
            return mixer::clip_capture(mix.sum[x]);
        }
        // TODO(hw-unverified): a channel that is not playing is 0 here (`sound-capture.md`,
        // "Unverified").
        let before_pan = |channel: usize| {
            let c = &self.channels[channel];
            mixer::volumed(samples[channel], c.divider(), c.volume())
        };
        let (a, b) = (before_pan(2 * x), before_pan(2 * x + 1));
        if self.captures[x].cnt & CAP_ADD != 0 {
            // The capture side of the addition: always done, whatever SOUNDCNT bits 12 and 13 say,
            // and plain `b` while `a` is silent. The sum is not clipped (the overflow bug).
            return mixer::capture_overflow(a + b);
        }
        // The both-negative bug: channel `b` is not added, yet it still turns the value into -8000h
        // when both are below 0.
        if a < 0 && b < 0 {
            mixer::CAPTURE_BUG
        } else {
            a
        }
    }

    /// What each running capture writes at the output sample at master time `time`: every slot the
    /// shared clock completed since the previous output sample, all with this sample's source value
    /// (`sound-capture.md`, "Model"). `None` for a capture with nothing to write. Pure and
    /// allocation-free; the previous event is one grid step back, so no cursor is kept.
    pub(crate) fn capture_writes(
        &self,
        mix: &Mix,
        samples: &[i32; CHANNELS],
        time: u64,
    ) -> [Option<CaptureWrites>; CAPTURES] {
        let before = time.saturating_sub(OUTPUT_SAMPLE_CYCLES);
        // TODO(hw-unverified): the slots of one output sample all get its value, and the last slot
        // of a one-shot reaches memory at the event after the busy bit clears
        // (`sound-capture.md`, "Unverified").
        std::array::from_fn(|x| {
            let capture = &self.captures[x];
            let (first, last) = (capture.written(before), capture.written(time));
            if last <= first {
                return None;
            }
            let value = self.capture_source(x, mix, samples);
            let latched = capture.latched;
            let value = if latched.pcm8 {
                mixer::capture_pcm8(value)
            } else {
                mixer::capture_pcm16(value)
            };
            Some(CaptureWrites {
                latched,
                first,
                count: last - first,
                value: value as i32,
            })
        })
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
    // TODO(hw-unverified): a write to an unused address of the window is dropped, and a partial
    // write changes only its bytes without reading the write-only registers back to merge
    // (`sound-registers.md`, "Unverified").
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
                // TODO(hw-unverified): what a write of 1 to a running channel does on the hardware
                // (`spu.md`, "Unverified").
                match (was_busy, started) {
                    (false, true) => {
                        channel.latch(now, (offset >> CHANNEL_STRIDE_SHIFT) as u8);
                        channel.end = channel.end_from(now);
                    }
                    (true, true) => {}
                    (_, false) => {
                        channel.start = 0;
                        channel.latched = Latched::default();
                        channel.decoders = Decoders::default();
                        channel.end = 0;
                    }
                }
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
        // Writing 1 while busy keeps the current run, as a channel does.
        // TODO(hw-unverified): what a write of 1 to bit 7 of a running capture does
        // (`sound-capture.md`, "Unverified").
        match (was_busy, started) {
            (false, true) => capture.latch(now, tmr),
            (true, true) => {}
            (_, false) => {
                capture.start = 0;
                capture.latched = CaptureLatched::default();
            }
        }
    }
}

impl Checked for Spu {
    /// Every register holds only the bits it keeps, and a stopped unit has no end time. A started
    /// unit may be past its end (the bit is folded away lazily), and `end` is deliberately not
    /// checked against `now`: any end time is one a program can have reached.
    fn well_formed(&self, _now: u64) -> bool {
        let channels = self.channels.iter().all(|c| {
            c.cnt & !CNT_MASK == 0
                && c.sad & !SAD_MASK == 0
                && c.len & !LEN_MASK == 0
                && (c.cnt & CNT_START != 0 || (c.end == 0 && c.start == 0))
                && c.decoders.adpcm.well_formed()
                && c.decoders.noise.well_formed()
        });
        let captures = self
            .captures
            .iter()
            .all(|c| c.cnt & !CAP_MASK == 0 && c.dad & !CAP_DAD_MASK == 0 && c.well_formed());
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
