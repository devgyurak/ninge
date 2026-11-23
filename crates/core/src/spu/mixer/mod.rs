// SPDX-License-Identifier: Apache-2.0
//! The arithmetic of the channel mixer and the master stage, one pure function a step of
//! GBATEK's "Channel/Mixer Bit-Widths" table (`docs/hw/spu.md`, "Volume and panning", "The
//! mixer").
//!
//! Values between the stages are integers in units of 1/256 of a PCM16 step (8 fraction bits), the
//! width the table gives after the channel's rounding. All shifts are arithmetic, so a negative
//! value rounds toward minus infinity, "rounding down" in the table's words.
// TODO(phase6): the capture units (#46) read the mixer's output and can add channel 1 or 3 into
// channel 0 or 2; none of it is modeled here (`spu.md`, "What this note does not cover").

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

/// One channel's left and right values for the PCM16 `sample`, in 1/256 units: divide, multiply by
/// the volume, then by the panning, and drop the fraction beyond 8 bits (`spu.md`, "Volume and
/// panning"). The left factor is `(128 - N) / 128` and the right `N / 128`.
// TODO(hw-unverified): where each fraction is dropped is read from GBATEK's table
// (`spu.md`, "Unverified").
pub(super) fn channel(sample: i32, divider: u32, volume: u32, panning: u32) -> (i64, i64) {
    let divided = (i64::from(sample) << DIVIDER_BITS) >> DIVIDER_SHIFT[(divider & 3) as usize];
    let with_volume = divided * factor(volume);
    let pan = factor(panning);
    (
        (with_volume * (128 - pan)) >> CHANNEL_DROP,
        (with_volume * pan) >> CHANNEL_DROP,
    )
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
