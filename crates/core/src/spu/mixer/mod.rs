// SPDX-License-Identifier: Apache-2.0
//! The arithmetic of the channel mixer and the master stage, one pure function a step of
//! GBATEK's "Channel/Mixer Bit-Widths" table (`docs/hw/spu.md`, "Volume and panning", "The
//! mixer").
//!
//! Values between the stages are integers in units of 1/256 of a PCM16 step (8 fraction bits), the
//! width the table gives after the channel's rounding. All shifts are arithmetic, so a negative
//! value rounds toward minus infinity, "rounding down" in the table's words.
//!
//! The capture units read a value before the master stage and make it PCM16 or PCM8
//! (`docs/hw/sound-capture.md`); [`volumed`], [`clip_capture`], [`capture_pcm16`] and
//! [`capture_pcm8`] are those steps.

/// A 7-bit volume or pan register means `N` = 0-126 as written, and 128 for 127 (GBATEK, "7bit
/// Volume and Panning Values"); the factor is `N / 128`.
pub(super) const fn factor(register: u32) -> i64 {
    if register == 127 {
        128
    } else {
        register as i64
    }
}

/// The volume divider's shift for the 2-bit selector: divide by 1, 2, 4 or 16.
const DIVIDER_SHIFT: [u32; 4] = [0, 1, 2, 4];
/// Fraction bits the divider step keeps (GBATEK's "16.4").
const DIVIDER_BITS: u32 = 4;
/// Fraction bits of a 7-bit factor: `N / 128`.
const FACTOR_BITS: u32 = 7;
/// Fraction bits the channel stage drops after the panning: 4 + 7 + 7 = 18 become 8.
const CHANNEL_DROP: u32 = DIVIDER_BITS + 2 * FACTOR_BITS - 8;
/// Fraction bits the master stage drops: 8 + 7 (`N / 128`) + 6 (`/ 64`) = 21 (GBATEK's "14.21").
const MASTER_DROP: u32 = 8 + FACTOR_BITS + 6;
/// The unsigned output has 10 bits (GBATEK, "DS Sound Control Registers").
const LEVEL_MAX: i64 = 0x3FF;
/// The level of a zero signal with the usual bias: the medium level.
const LEVEL_MEDIUM: i32 = 0x200;
/// The 10-bit level is scaled to the `i16` range by 64 (`spu.md`, "Output samples").
const LEVEL_SCALE: i32 = 64;

/// A channel's value after the divider and the volume factor, with the 11 fraction bits those two
/// steps leave (GBATEK's "16.11"): the value the panning (and, in addition mode, the addition)
/// takes as input. Divide by 1, 2, 4 or 16, then multiply by `N / 128` (`spu.md`, "Volume and
/// panning").
pub(super) fn with_volume(sample: i32, divider: u32, volume: u32) -> i64 {
    let divided = (i64::from(sample) << DIVIDER_BITS) >> DIVIDER_SHIFT[(divider & 3) as usize];
    divided * factor(volume)
}

/// The left and right values of `with_volume` panned, in 1/256 units: the fraction beyond 8 bits is
/// dropped (`spu.md`, "Volume and panning"). The left factor is `(128 - N) / 128` and the right
/// `N / 128`.
// TODO(hw-unverified): where each fraction is dropped is read from GBATEK's table
// (`spu.md`, "Unverified").
pub(super) fn pan(with_volume: i64, panning: u32) -> (i64, i64) {
    let pan = factor(panning);
    (
        (with_volume * (128 - pan)) >> CHANNEL_DROP,
        (with_volume * pan) >> CHANNEL_DROP,
    )
}

/// Fraction bits the divider and volume steps leave (4 + 7) and the 8 a capture keeps.
const VOLUMED_DROP: u32 = DIVIDER_BITS + FACTOR_BITS - 8;

/// A channel's value before its panning, in 1/256 units: the divider and the volume factor
/// applied and the fraction cut to 8 bits (`sound-capture.md`, "Sources"). It is what the capture
/// reads as `ch(a)` and `ch(b)`, and it is not clipped.
// TODO(hw-unverified): where the fraction is dropped between the volume factor and the capture
// (`sound-capture.md`, "Unverified").
pub(super) fn volumed(sample: i32, divider: u32, volume: u32) -> i64 {
    with_volume(sample, divider, volume) >> VOLUMED_DROP
}

/// The bounds of a capture value, `-8000h` and `7FFFh` with the 8 fraction bits kept.
const CAPTURE_MIN: i64 = -0x80_0000;
const CAPTURE_MAX: i64 = 0x7F_FFFF;
/// The value a capture of a channel writes in the both-negative bug (`sound-capture.md`, "The two
/// bugs"): `-8000h`.
pub(super) const CAPTURE_BUG: i64 = CAPTURE_MIN;

/// The overflow bug of a capture of channel `a` with the addition on (`sound-capture.md`, "The two
/// bugs"): the sum is ANDed with `FFFFh` instead of clipped. The 16-bit integer part wraps, so a sum
/// beyond the range loses its sign; the 8 fraction bits stay.
// TODO(hw-unverified): GBATEK gives the AND for a PCM16 capture; for PCM8 the model wraps the
// 16-bit integer part before the conversion (`sound-capture.md`, "Unverified").
pub(super) const fn capture_overflow(value: i64) -> i64 {
    // The truncation to 16 bits is the bug itself, not an accident.
    let whole = (value >> 8) as u16 as i16;
    ((whole as i64) << 8) | (value & 0xFF)
}

/// A 16.8 value clipped to the range a capture holds (`sound-capture.md`, "Sources").
pub(super) fn clip_capture(value: i64) -> i64 {
    value.clamp(CAPTURE_MIN, CAPTURE_MAX)
}

/// A 16.8 capture value as PCM16: the 8 fraction bits are dropped by flooring.
// TODO(hw-unverified): GBATEK states the rounding for PCM8 only; PCM16 floors here
// (`sound-capture.md`, "Unverified").
pub(super) const fn capture_pcm16(value: i64) -> i64 {
    value >> 8
}

/// Fraction bits of the 8.16 value a PCM8 capture makes from the 16.8 one, and the most significant
/// of them.
const PCM8_FRACTION_BITS: u32 = 16;
const PCM8_FRACTION_MSB: i64 = 1 << (PCM8_FRACTION_BITS - 1);

/// A 16.8 capture value as PCM8: divided by `100h` into 8.16; with the fraction's most significant
/// bit set the value is rounded toward zero (a negative one rounds up, a positive one down), and the
/// fraction is dropped (`sound-capture.md`, "Format").
// TODO(hw-unverified): the fraction is the two's complement one, so -5.5 steps becomes -5 and
// -5.50002 becomes -6 (`sound-capture.md`, "Unverified").
pub(super) const fn capture_pcm8(value: i64) -> i64 {
    let whole = value >> PCM8_FRACTION_BITS;
    let rounds_up = value < 0 && value & PCM8_FRACTION_MSB != 0;
    whole + rounds_up as i64
}

/// The master stage: `sum` (1/256 units) times `N / 128 / 64`, fraction dropped.
// TODO(hw-unverified): the rounding order of the master stage is read from GBATEK's table
// (`spu.md`, "Unverified").
pub(super) fn master(sum: i64, volume: u32) -> i64 {
    (sum * factor(volume)) >> MASTER_DROP
}

/// The bias added and the result clipped to `000h`-`3FFh`.
pub(super) fn level(signal: i64, bias: u16) -> u16 {
    (signal + i64::from(bias)).clamp(0, LEVEL_MAX) as u16
}

/// The 10-bit level as an `i16` centered on the medium level (`spu.md`, "Output samples").
// TODO(hw-unverified): the conversion to `i16` is ours; GBATEK gives only the 10-bit level
// (`spu.md`, "Unverified").
pub(super) const fn to_i16(level: u16) -> i16 {
    ((level as i32 - LEVEL_MEDIUM) * LEVEL_SCALE) as i16
}

#[cfg(test)]
mod tests;
