// SPDX-License-Identifier: Apache-2.0
//! The three sound tables of the ARM7 BIOS (`docs/hw/hle-swi.md`, "The sound table SWIs"):
//! GetSineTable, GetPitchTable and GetVolumeTable each return one entry.
//!
//! ninge has no BIOS to read the tables from, so each is computed from the formula the note gives,
//! at compile time and with whole numbers only: a float's `sin` or `powf` may differ in its last
//! bit from one platform to the next, and an entry that rounds the other way would make the same
//! program sound different on two hosts (AGENTS.md §2 rule 6). Values are fixed point with
//! [`FRACTION_BITS`] fraction bits in a `u128`.
// TODO(hw-unverified): every entry is computed, none was compared with a BIOS, and the pitch and
// volume formulas are inferred (`hle-swi.md`, "Unverified").

/// Entries of the sine table: a quarter wave.
const SINE_LEN: usize = 64;
/// Entries of the pitch table: 64 steps to each of an octave's 12 semitones.
const PITCH_LEN: usize = 768;
/// Entries of the volume table: tenths of a decibel, the last one the full level.
const VOLUME_LEN: usize = 724;

/// Fraction bits of the fixed-point values the tables are computed with. With 56, a value up to
/// [`POWER_CAP`] takes 62 bits, so the product of two of them, the largest product made before
/// its shift, takes 124.
const FRACTION_BITS: u32 = 56;
/// 1.0 in fixed point.
const ONE: u128 = 1 << FRACTION_BITS;
/// Where [`power`] stops: above every value a root is compared with (2 and 10), and small enough
/// that the product of two values at the cap still fits a `u128`.
const POWER_CAP: u128 = 64 * ONE;
/// A root is looked for between 1.0 and this: both roots taken here are a little above 1.
const ROOT_BELOW: u128 = 2 * ONE;

/// The sine table steps a right angle in 64, which is six halvings.
const SINE_HALVINGS: u32 = SINE_LEN.trailing_zeros();
/// The scale of the sine table.
// TODO(hw-unverified): `7FFFh`, where GBATEK writes `8000h`, to meet the largest entry it gives
// (`hle-swi.md`, "Unverified").
const SINE_SCALE: u128 = 0x7FFF;
/// The scale of the pitch table: an entry is a fraction of `10000h`.
const PITCH_SHIFT: u32 = 16;
/// The largest volume entry, the full level.
const VOLUME_MAX: u128 = 0x7F;
/// Tenths of a decibel in a factor of ten of the level: 20 dB.
const STEPS_PER_DECADE: u32 = 200;
/// The first volume entry that goes with no divider: the level is a half 60 tenths of a decibel
/// below the top entry, 723.
// TODO(hw-unverified): that the table is made for use with the divider, and where the divider
// changes, are inferred (`hle-swi.md`, "Unverified"); so are the two entries below.
const NO_DIVIDER_FROM: usize = 663;
/// The first volume entry that goes with /2: the level is a quarter 120 tenths below the top.
const HALF_FROM: usize = 603;
/// The first volume entry that goes with /4: the level is a sixteenth 240 tenths below the top,
/// and the entries under this one go with /16.
const QUARTER_FROM: usize = 483;

/// The product of two fixed-point values.
const fn times(a: u128, b: u128) -> u128 {
    (a * b) >> FRACTION_BITS
}

/// A fixed-point value rounded to the nearest whole number.
const fn rounded(value: u128) -> u128 {
    (value + ONE / 2) >> FRACTION_BITS
}

/// `base` (at least 1.0) to the power `exponent`, or [`POWER_CAP`] once the result has passed it:
/// the caller only compares the result with a smaller number, so past the cap its size no longer
/// matters, and stopping there keeps every product inside a `u128`.
const fn power(base: u128, exponent: u32) -> u128 {
    let (mut result, mut square, mut left) = (ONE, base, exponent);
    while left > 0 {
        if left & 1 == 1 {
            result = times(result, square);
            if result > POWER_CAP {
                return POWER_CAP;
            }
        }
        square = times(square, square);
        if square > POWER_CAP {
            square = POWER_CAP;
        }
        left >>= 1;
    }
    result
}

/// The `degree`th root of `of` (a whole number below the cap of [`power`]): the largest
/// fixed-point value between 1.0 and 2.0 whose `degree`th power is not above it, by bisection.
const fn root(of: u128, degree: u32) -> u128 {
    let (mut low, mut high) = (ONE, ROOT_BELOW);
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if power(middle, degree) <= of * ONE {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

/// The square root of a fixed-point value of at most 1.0, rounded down: the whole-number square
/// root of the value shifted up by the fraction bits, a bit at a time from the top.
const fn square_root(value: u128) -> u128 {
    let wide = value << FRACTION_BITS;
    let (mut result, mut bit) = (0u128, 1u128 << FRACTION_BITS);
    while bit > 0 {
        let candidate = result | bit;
        if candidate * candidate <= wide {
            result = candidate;
        }
        bit >>= 1;
    }
    result
}

/// The sine table: entry `i` is `sin(i * 90 degrees / 64) * 7FFFh`.
///
/// No series and no pi: the cosine of a right angle is 0, the half-angle rule
/// `cos(a / 2) = sqrt((1 + cos a) / 2)` taken six times gives the cosine of 90/64 degrees, and
/// turning a point by that angle 63 times walks the quarter wave.
// TODO(hw-unverified): the entries are computed, not read from a BIOS (`hle-swi.md`,
// "Unverified").
const fn sine_table() -> [u16; SINE_LEN] {
    let mut cos_step = 0;
    let mut halvings = 0;
    while halvings < SINE_HALVINGS {
        cos_step = square_root((ONE + cos_step) / 2);
        halvings += 1;
    }
    let sin_step = square_root(ONE - times(cos_step, cos_step));
    let mut table = [0; SINE_LEN];
    let (mut cos, mut sin) = (ONE, 0);
    let mut i = 0;
    while i < SINE_LEN {
        table[i] = rounded(sin * SINE_SCALE) as u16;
        // The cosine reaches 0 at the turn after the last entry; it never goes below.
        let turned = times(cos, cos_step).saturating_sub(times(sin, sin_step));
        sin = times(sin, cos_step) + times(cos, sin_step);
        cos = turned;
        i += 1;
    }
    table
}

/// The pitch table: entry `i` is `(2^(i/768) - 1) * 10000h`.
// TODO(hw-unverified): the formula is inferred from the table's size and its largest entry
// (`hle-swi.md`, "Unverified").
const fn pitch_table() -> [u16; PITCH_LEN] {
    let step = root(2, PITCH_LEN as u32);
    let mut table = [0; PITCH_LEN];
    let mut ratio = ONE;
    let mut i = 0;
    while i < PITCH_LEN {
        table[i] = rounded((ratio - ONE) << PITCH_SHIFT) as u16;
        ratio = times(ratio, step);
        i += 1;
    }
    table
}

/// The divider that goes with volume entry `index`, which the entry is multiplied by: the
/// register's /16, /4 and /2, and 1 for none.
const fn divider(index: usize) -> u128 {
    if index >= NO_DIVIDER_FROM {
        1
    } else if index >= HALF_FROM {
        2
    } else if index >= QUARTER_FROM {
        4
    } else {
        16
    }
}

/// The volume table: entry `i` is `127 * 10^((i - 723) / 200)` times the divider of its range, at
/// most `7Fh`. Worked from the top entry down, a tenth of a decibel at a time.
// TODO(hw-unverified): tenths of a decibel, and the level times a divider, are inferred from the
// table's size and range (`hle-swi.md`, "Unverified").
const fn volume_table() -> [u16; VOLUME_LEN] {
    let step = root(10, STEPS_PER_DECADE);
    let mut table = [0; VOLUME_LEN];
    let mut level = VOLUME_MAX * ONE;
    let mut i = VOLUME_LEN;
    while i > 0 {
        i -= 1;
        let entry = rounded(level * divider(i));
        let capped = if entry > VOLUME_MAX {
            VOLUME_MAX
        } else {
            entry
        };
        table[i] = capped as u16;
        level = (level << FRACTION_BITS) / step;
    }
    table
}

const SINE: [u16; SINE_LEN] = sine_table();
const PITCH: [u16; PITCH_LEN] = pitch_table();
const VOLUME: [u16; VOLUME_LEN] = volume_table();

/// Entry `index` of `table`, 0 outside it.
// TODO(hw-unverified): GBATEK calls what the BIOS returns there garbage (`hle-swi.md`,
// "Unverified").
fn entry(table: &[u16], index: u32) -> u32 {
    table
        .get(index as usize)
        .map_or(0, |&entry| u32::from(entry))
}

/// GetSineTable: entry `index` (`00h`-`3Fh`) of the sine table, 0 outside it.
pub(super) fn sine(index: u32) -> u32 {
    entry(&SINE, index)
}

/// GetPitchTable: entry `index` (`000h`-`2FFh`) of the pitch table, 0 outside it.
pub(super) fn pitch(index: u32) -> u32 {
    entry(&PITCH, index)
}

/// GetVolumeTable: entry `index` (`000h`-`2D3h`) of the volume table, 0 outside it.
pub(super) fn volume(index: u32) -> u32 {
    entry(&VOLUME, index)
}
