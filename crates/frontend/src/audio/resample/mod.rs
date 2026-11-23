// SPDX-License-Identifier: Apache-2.0
//! Sample-rate conversion from the machine's output rate to the device's, and the rate control
//! that keeps the two clocks from drifting apart (#47, plan §4.3).
//!
//! The machine's rate is the rational 67,027,964 / 2048 Hz (`ninge_core::MASTER_CLOCK_HZ` per
//! `OUTPUT_SAMPLE_CYCLES`) and the device's is a whole number of Hz, so the step between two
//! output samples, in input samples, is a rational too. The resampler keeps its position as an
//! integer numerator over a fixed denominator and steps it by an integer numerator, so a rate
//! that divides exactly never accumulates an error, however long it runs. Only the trim, which
//! changes the step a little while the ring is off target, rounds, and by at most about a
//! millionth of a sample per step.
//!
//! The conversion is linear interpolation between the two input pairs around each output
//! instant. It carries the last input pair across calls so a frame boundary is not audible, and
//! it runs one input pair behind: the first output is the silence it started from.

use ninge_core::MASTER_CLOCK_HZ;
use ninge_core::spu::OUTPUT_SAMPLE_CYCLES;

/// Largest rate trim, in parts per million: ±0.5 %, far below what a listener hears as a pitch
/// change, and enough to absorb the difference between the machine's and the device's clocks.
pub const MAX_TRIM_PPM: i32 = 5_000;

/// Parts per million in one.
const PPM: u64 = 1_000_000;

/// A linear resampler for interleaved stereo `i16`.
#[derive(Clone, Debug)]
pub struct Resampler {
    /// The source rate is `source_num / source_den` Hz.
    source_num: u64,
    /// The unit of `phase` and `step`: one input pair is `scale` units.
    scale: u64,
    /// Input pairs advanced per output pair, in units of `1 / scale`, at the current trim.
    step: u64,
    /// Where the next output falls between `previous` and the next input pair, in `[0, scale)`
    /// once an input pair has arrived.
    phase: u64,
    /// The input pair before the next one, `(left, right)`.
    previous: [i16; 2],
    trim_ppm: i32,
}

impl Resampler {
    /// A resampler from `source_num / source_den` Hz to `dest_hz` Hz, with no trim.
    ///
    /// # Panics
    ///
    /// When a rate is zero.
    pub fn new(source_num: u64, source_den: u64, dest_hz: u32) -> Self {
        assert!(
            source_num > 0 && source_den > 0 && dest_hz > 0,
            "a sample rate is zero"
        );
        let mut resampler = Self {
            source_num,
            // One input pair is `source_den * dest_hz * PPM` units, so that a step of
            // `source_num / (source_den * dest_hz)` input pairs is the whole number
            // `source_num * PPM`.
            scale: source_den * u64::from(dest_hz) * PPM,
            step: 0,
            phase: 0,
            previous: [0, 0],
            trim_ppm: 0,
        };
        resampler.step = resampler.step_for(0);
        resampler
    }

    /// A resampler from the machine's output rate to `dest_hz` Hz.
    pub fn for_machine(dest_hz: u32) -> Self {
        Self::new(MASTER_CLOCK_HZ, OUTPUT_SAMPLE_CYCLES, dest_hz)
    }

    /// The step at a trim: a positive trim makes the output rate higher, so each output pair
    /// advances fewer input pairs.
    fn step_for(&self, trim_ppm: i32) -> u64 {
        let rate = u128::from(PPM) * u128::from(PPM)
            / u128::from(PPM.saturating_add_signed(i64::from(trim_ppm)));
        // `source_num * PPM` input-pair units at no trim, scaled by the trim.
        u64::try_from(u128::from(self.source_num) * rate).expect("a step fits 64 bits")
    }

    /// Trim the output rate by `ppm` parts per million, clamped to ±[`MAX_TRIM_PPM`]. Positive
    /// makes more output pairs from the same input.
    pub fn set_trim_ppm(&mut self, ppm: i32) {
        self.trim_ppm = ppm.clamp(-MAX_TRIM_PPM, MAX_TRIM_PPM);
        self.step = self.step_for(self.trim_ppm);
    }

    /// The trim in force, in parts per million.
    pub fn trim_ppm(&self) -> i32 {
        self.trim_ppm
    }

    /// Convert interleaved input samples and append the output to `out`. A trailing unpaired
    /// input sample is ignored.
    pub fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        for &next in input.as_chunks::<2>().0 {
            while self.phase < self.scale {
                out.push(self.lerp(self.previous[0], next[0]));
                out.push(self.lerp(self.previous[1], next[1]));
                self.phase += self.step;
            }
            self.phase -= self.scale;
            self.previous = next;
        }
    }

    /// The point `phase / scale` of the way from `from` to `to`.
    fn lerp(&self, from: i16, to: i16) -> i16 {
        let delta = i128::from(to) - i128::from(from);
        // Rounds toward `from`; the result lies between the two samples, so it fits `i16`.
        let value = i128::from(from) + delta * i128::from(self.phase) / i128::from(self.scale);
        i16::try_from(value).expect("an interpolated sample lies between its i16 neighbours")
    }
}

/// The trim for a ring holding `fill` pairs when it should hold `target`: proportional, full
/// scale when the ring is half a target away, clamped to ±[`MAX_TRIM_PPM`]. A ring above its
/// target gets a negative trim (make less), one below it a positive trim (make more). A zero
/// target has nothing to aim at and gives no trim.
pub fn trim_for_fill(fill: usize, target: usize) -> i32 {
    if target == 0 {
        return 0;
    }
    // Pair counts are far below i64's range, so the signed subtraction cannot overflow.
    let error = target as i64 - fill as i64;
    let half = (target as i64 / 2).max(1);
    let trim = error.saturating_mul(i64::from(MAX_TRIM_PPM)) / half;
    i32::try_from(trim.clamp(-i64::from(MAX_TRIM_PPM), i64::from(MAX_TRIM_PPM)))
        .expect("a clamped trim fits i32")
}

#[cfg(test)]
mod tests;
