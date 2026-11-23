// SPDX-License-Identifier: Apache-2.0
//! The calibration constant, from `docs/hw/firmware.md`: pixel -> ADC -> GBATEK's formula -> pixel.

use super::{ADC_PER_PIXEL, CALIBRATION};

/// GBATEK's conversion for one axis, in integers (the formula's division is exact here).
fn to_pixel(adc: i32, adc1: u16, adc2: u16, scr1: u8, scr2: u8) -> i32 {
    (adc - i32::from(adc1)) * (i32::from(scr2) - i32::from(scr1))
        / (i32::from(adc2) - i32::from(adc1))
        + (i32::from(scr1) - 1)
}

/// The pixels (axis, pixel) whose round trip through the ADC and the formula differs.
fn round_trip_failures() -> Vec<(char, i32, i32)> {
    let [(ax1, ay1), (ax2, ay2)] = CALIBRATION.adc;
    let [(sx1, sy1), (sx2, sy2)] = CALIBRATION.screen;
    let xs = (0..256).map(|p| {
        (
            'x',
            p,
            to_pixel(p * i32::from(ADC_PER_PIXEL), ax1, ax2, sx1, sx2),
        )
    });
    let ys = (0..192).map(|p| {
        (
            'y',
            p,
            to_pixel(p * i32::from(ADC_PER_PIXEL), ay1, ay2, sy1, sy2),
        )
    });
    xs.chain(ys).filter(|&(_, p, back)| p != back).collect()
}

#[test]
fn given_every_pixel_when_converted_to_adc_and_back_by_the_formula_then_it_is_exact() {
    assert_eq!(round_trip_failures(), Vec::<(char, i32, i32)>::new());
}

#[test]
fn given_every_pixel_when_converted_to_adc_then_the_value_fits_12_bits() {
    let too_wide: Vec<u32> = (0..256)
        .map(|p| p * u32::from(ADC_PER_PIXEL))
        .filter(|&adc| adc > 0xFFF)
        .collect();
    assert_eq!(too_wide, Vec::<u32>::new());
}

#[test]
fn given_the_calibration_when_the_two_points_are_read_then_adc_per_pixel_matches_both_axes() {
    let [(ax1, ay1), (ax2, ay2)] = CALIBRATION.adc;
    let [(sx1, sy1), (sx2, sy2)] = CALIBRATION.screen;
    let per_pixel = [
        (ax2 - ax1) / u16::from(sx2 - sx1),
        (ay2 - ay1) / u16::from(sy2 - sy1),
    ];
    assert_eq!(per_pixel, [ADC_PER_PIXEL, ADC_PER_PIXEL]);
}
