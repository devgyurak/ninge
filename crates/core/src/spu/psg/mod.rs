// SPDX-License-Identifier: Apache-2.0
//! The PSG square wave (channels 8-13) and the noise generator (channels 14-15)
//! (`docs/hw/spu.md`, "PSG (channels 8-13)", "Noise (channels 14-15)").
//!
//! The square wave is a pure function of the sample count and the duty. The noise register is a
//! state that advances with the sample clock, serialized with the channel
//! (`AGENTS.md` §2 rule 5).

use serde::{Deserialize, Serialize};

/// The PCM16 value of a HIGH sample, and of a LOW one: `+7FFFh` and `-7FFFh` (`spu.md`, "PSG").
pub(super) const HIGH: i32 = 0x7FFF;
pub(super) const LOW: i32 = -0x7FFF;
/// A PSG wave is eight samples long (`spu.md`, "PSG").
const WAVE_SAMPLES: u64 = 8;
/// Duty 7 is the one that is LOW for the whole wave (GBATEK's table: 0%).
const DUTY_ALWAYS_LOW: u32 = 7;
/// The noise register is 15 bits, started at `7FFFh`, and a 1 shifted out toggles these taps
/// (`spu.md`, "Noise").
const NOISE_START: u16 = 0x7FFF;
const NOISE_TAPS: u16 = 0x6000;

/// The PSG wave's level after `elapsed` samples with duty `duty` (0-7): the wave starts with its
/// LOW part, `7 - duty` samples, then `duty + 1` HIGH samples; duty 7 never goes HIGH
/// (`spu.md`, "PSG").
pub(super) const fn duty_level(elapsed: u64, duty: u32) -> i32 {
    let phase = elapsed % WAVE_SAMPLES;
    if duty != DUTY_ALWAYS_LOW && phase >= (DUTY_ALWAYS_LOW - duty) as u64 {
        HIGH
    } else {
        LOW
    }
}

/// A noise channel's generator (`spu.md`, "Noise"): the register and the last output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Noise {
    /// Steps taken since the sound started; 0 until the first.
    pub(super) ticks: u64,
    /// The 15-bit register.
    pub(super) x: u16,
    /// Whether the last output was HIGH.
    pub(super) high: bool,
}

impl Noise {
    /// The 15-bit register.
    pub const fn register(&self) -> u16 {
        self.x
    }

    /// The last output as a PCM16 value: `+7FFFh` for HIGH, `-7FFFh` for LOW; before the first step
    /// it is 0.
    pub(super) const fn level(&self) -> i32 {
        match (self.ticks, self.high) {
            (0, _) => 0,
            (_, true) => HIGH,
            (_, false) => LOW,
        }
    }

    /// One step at one sample of the clock: shift right; a 1 shifted out makes the output LOW and
    /// toggles the taps, otherwise the output is HIGH. The first step starts from `7FFFh`.
    // TODO(hw-unverified): the first output is the first step from `7FFFh`, not the start value
    // (`spu.md`, "Unverified").
    pub(super) fn step(&mut self) {
        if self.ticks == 0 {
            self.x = NOISE_START;
        }
        let carry = self.x & 1 != 0;
        self.x >>= 1;
        if carry {
            self.x ^= NOISE_TAPS;
        }
        self.high = !carry;
        self.ticks += 1;
    }

    /// Whether the register is one the generator can reach: 15 bits.
    pub(super) const fn well_formed(&self) -> bool {
        self.x <= NOISE_START
    }
}

#[cfg(test)]
mod tests;
